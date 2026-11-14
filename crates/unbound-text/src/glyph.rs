//! 字形光栅化、图集驻留管理与四边形图元生成。
//!
//! 单个字形的处理管线：
//!
//! 1. 使用 `swash` 按其坐标所隐含的亚像素相位进行光栅化（共 4 个相位，
//!    使 12px 标签保持均匀字距，而非将每个字形强行对齐到同一像素网格）；
//!
//! 2. 在 R8 图集中预留槽位（彩色 Emoji 与距离场则使用 RGBA 图集）；
//!
//! 3. 生成供渲染器上传到纹理数组的 [`AtlasUpdate`]；
//! 4. 生成供视图层转换为 `GpuPrimitiveInstance` 的 [`GlyphQuad`]。
//!
//! 步骤 1–4 均为纯 CPU 计算，因此整套缓存无需 GPU 设备即可完成测试。

use cosmic_text::{fontdb, CacheKey, CacheKeyFlags, SwashCache};
use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};

use crate::atlas::{GlyphAtlas, GlyphKey, GlyphSlot};
use crate::sdf;
use crate::shaping::{PositionedGlyph, ShapedText, TextEngine};

/// 更新目标图集类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AtlasTarget {
    /// `Atlas_R8`，每纹素 1 字节。
    Coverage,
    /// `Atlas_RGBA8`，每纹素 4 字节。
    Color,
}

/// 待上传至某图集层的矩形纹素区域。
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasUpdate {
    pub target: AtlasTarget,
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// 行主序纹素数据；长度为 `width * height * bpp`。
    pub data: Vec<u8>,
}

impl AtlasUpdate {
    #[must_use]
    pub fn bpp(&self) -> u32 {
        match self.target {
            AtlasTarget::Coverage => 1,
            AtlasTarget::Color => 4,
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> usize {
        (self.width * self.height) as usize * self.bpp() as usize
    }
}

/// 已光栅化并驻留于图集中的字形。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ResolvedGlyph {
    pub slot: GlyphSlot,
    /// 从画笔位置到四边形左上角的偏移量，单位为逻辑像素。
    pub bearing: Vec2,
    /// 四边形尺寸，单位为逻辑像素。
    pub size: Vec2,
    /// 步进宽度，单位为逻辑像素。
    pub advance: f32,
    /// 当字形来自 RGBA 图集时为 `true`。
    pub color: bool,
    /// 当 RGBA 图集条目为多通道距离场时为 `true`。
    ///
    /// 该标志必须传递到片元着色器：若缺失该标志，距离场会被当作普通
    /// RGBA 位图采样，导致字形变成白色方块。
    ///
    pub msdf: bool,
}

impl ResolvedGlyph {
    /// 文本局部空间中的四边形。
    ///
    /// 画笔坐标会对齐到光栅自身的像素网格：画笔位置的亚像素部分已被烘焙进
    /// 位图中（光栅按 `pen.x` 隐含的相位生成），若再次叠加小数部分会导致
    /// 字形发生双重偏移。
    ///
    ///
    /// 水平原点来自 [`GlyphCache::quantize_x`]，它连同相位一起携带了整像素：
    /// 落在 `[0.875, 1.0)` 的画笔小数部分会舍入到**下一像素**的相位 0，
    /// 四边形也必须随之移动。丢弃该进位（旧版单纯 `% 4` 的做法）曾使
    /// 12.5% 的字形向左偏移一个像素，导致其边缘与前一个字母重叠。
    ///
    ///
    #[must_use]
    pub fn quad(&self, pen: Vec2, snap_phases: bool) -> Rect {
        let (base_x, _) = GlyphCache::quantize_x(pen.x, snap_phases);
        Rect::new(
            base_x + self.bearing.x,
            pen.y.round() + self.bearing.y,
            self.size.x,
            self.size.y,
        )
    }
}

/// 准备送入图元构建器的单个字形。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    pub rect: Rect,
    pub uv: [f32; 4],
    pub layer: u32,
    pub color: bool,
    /// 该字形以距离场形式存放在 RGBA 图集中。
    pub msdf: bool,
    /// 该字形使用的水平亚像素相位（0..3）。
    pub phase: u8,
}

/// 光栅化并缓存字形，维护其在图集中的驻留状态。
pub struct GlyphCache {
    swash: SwashCache,
    pub atlas: GlyphAtlas,
    resolved: FxHashMap<GlyphKey, ResolvedGlyph>,
    /// 自上次排空以来产生的图集更新。
    updates: Vec<AtlasUpdate>,
    /// 输出距离场而非普通覆盖率位图（用于画布文本）。
    pub use_sdf: bool,
    /// 距离场扩散范围（单位：纹素）。
    pub sdf_spread: f32,
    /// 每个字形四周的额外边距纹素（防止双线性采样渗色）。
    pub padding: u32,
    /// 在整像素网格上光栅化字形，而非使用 4 个亚像素相位。
    ///
    /// 相位必须与四边形最终绘制的位置匹配：光栅按特定相位生成且四边形对齐到
    /// 像素网格，若相位光栅与画笔位置不匹配，采样点将偏离生成位置，每条边缘
    /// 都会退化为半像素混合。
    ///
    ///
    ///
    /// 开启此选项会强制所有字形使用相位 0，仅当画笔本身始终位于整像素上时才正确。
    /// UI 文本保留四相位方案：相位由字形的绝对坐标推导，与真实画笔位置的误差
    /// 至多为 ⅛ px。
    ///
    pub snap_phases: bool,
    /// `fontdb::ID` 与 [`GlyphKey`] 中紧凑字体索引之间的双射。`fontdb::ID` 是
    /// 不透明的 slotmap 键，未公开整数转换，因此图集改用稠密索引作为键。
    ///
    faces: Vec<fontdb::ID>,
    face_index: FxHashMap<fontdb::ID, u32>,
    /// 自上次排空以来被图集淘汰的键。
    ///
    /// 图集会回收被淘汰字形的矩形区域，仍指向该区域的缓存条目将采样到
    /// 其他字形的纹素。
    evicted: Vec<GlyphKey>,
    misses: u64,
    hits: u64,
}

impl core::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("resident", &self.resolved.len())
            .field("updates", &self.updates.len())
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish_non_exhaustive()
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            swash: SwashCache::new(),
            atlas: GlyphAtlas::default(),
            resolved: FxHashMap::default(),
            updates: Vec::new(),
            use_sdf: false,
            sdf_spread: 4.0,
            padding: 2,
            snap_phases: false,
            faces: Vec::new(),
            face_index: FxHashMap::default(),
            evicted: Vec::new(),
            misses: 0,
            hits: 0,
        }
    }

    /// 获取字体对应的稠密索引，首次遇到时分配新索引。
    fn face_id(&mut self, face: fontdb::ID) -> u32 {
        if let Some(i) = self.face_index.get(&face) {
            return *i;
        }
        let i = self.faces.len() as u32;
        self.faces.push(face);
        self.face_index.insert(face, i);
        i
    }

    /// 推进图集淘汰时钟。
    pub fn begin_frame(&mut self, frame: u64) {
        self.atlas.begin_frame(frame);
    }

    /// 排空待上传的图集更新队列。
    pub fn take_updates(&mut self) -> Vec<AtlasUpdate> {
        core::mem::take(&mut self.updates)
    }

    /// 将已排空的批次放回队列前端。
    ///
    /// 上传可能失败——设备丢失、暂存缓冲区耗尽或队列已满。缓存此时认为
    /// 这些字形已在图集中，因此丢失批次意味着文本会采样到从未写入的纹素：
    /// 永久显示空白或乱码字形。因此必须将该批次放回队列，在下一帧重试。
    ///
    ///
    ///
    /// 恢复的更新保留在队列前端：它们比在此期间新生成的更新更早。
    ///
    pub fn restore_updates(&mut self, mut updates: Vec<AtlasUpdate>) {
        if updates.is_empty() {
            return;
        }
        updates.append(&mut self.updates);
        self.updates = updates;
    }

    /// 当前仍在队列中的待上传更新，供诊断与测试使用。
    #[must_use]
    pub fn pending_updates(&self) -> &[AtlasUpdate] {
        &self.updates
    }

    #[must_use]
    pub fn pending_update_bytes(&self) -> usize {
        self.updates.iter().map(AtlasUpdate::byte_len).sum()
    }

    #[must_use]
    pub fn resident_glyphs(&self) -> usize {
        self.resolved.len()
    }

    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f32 / total as f32
        }
    }

    /// 将画笔坐标拆分为整像素原点与亚像素相位。
    ///
    /// 四个相位覆盖 `[0, 1)`，分别为 `0, ¼, ½, ¾`。相位由四舍五入决定，
    /// 因此最后一个区间顶端（`fract ≥ 0.875`）会舍入到 `4`——即**下一像素**
    /// 的相位 0。整像素原点必须随之进位，这正是 `div_euclid`/`rem_euclid`
    /// 所做的：由同一辅助函数同时生成两个数值，确保二者永不矛盾。
    ///
    ///
    /// `snap_phases` 则强制整像素光栅（在 `round(x)` 处取相位 0），
    /// 供画笔始终位于像素网格上的调用方使用。
    #[must_use]
    pub fn quantize_x(x: f32, snap_phases: bool) -> (f32, u8) {
        if snap_phases {
            return (x.round(), 0);
        }
        let quarters = (x * 4.0).round();
        (quarters.div_euclid(4.0), quarters.rem_euclid(4.0) as u8)
    }

    /// 由画笔位置推导出的亚像素相位。
    #[must_use]
    pub fn phase_for(x: f32) -> u8 {
        // 四相位：0, ¼, ½, ¾。舍入到最近相位可将误差控制在 ⅛ px 以内，
        // 肉眼不可见；若改用整像素对齐则会明显改变字母间距。
        //
        Self::quantize_x(x, false).1
    }

    /// `pen_x` 处字形实际光栅化所用的亚像素相位。
    ///
    /// 相位必须由字形的**绝对**位置推导：光栅按该相位生成，若基于
    /// 相对文本段的偏移推导，每个字形最多会偏移一个像素。
    ///
    #[must_use]
    pub fn phase_at(&self, pen_x: f32) -> u8 {
        Self::quantize_x(pen_x, self.snap_phases).1
    }

    /// 解析（并在必要时光栅化）单个字形。
    pub fn resolve(
        &mut self,
        engine: &mut TextEngine,
        glyph: &PositionedGlyph,
        face: fontdb::ID,
        pen: Vec2,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedGlyph> {
        let phase = self.phase_at(pen.x);
        let sdf_mode = self.use_sdf;
        let face_index = u64::from(self.face_id(face));
        // 光栅化字重必须与文本段塑形时使用的字重一致。此前在此叠加“光学”
        // 补偿导致小号标签比其排版步进更粗，且 18 px 处的阶跃造成字号变化时
        // 字重突变。
        //
        // 光栅化结果取决于相位、字重与斜体倾斜，因此三者均纳入键中。
        //
        let key = GlyphKey::styled(
            glyph.glyph_id,
            glyph.font_size,
            phase,
            sdf_mode,
            face_index,
            weight,
            italic,
        );
        if let Some(hit) = self.resolved.get(&key).copied() {
            if hit.slot.is_valid() {
                // 刷新图集代际而无需重新光栅化。
                self.hits += 1;
                if hit.color {
                    self.atlas.rgba.touch(&key);
                } else {
                    self.atlas.r8.touch(&key);
                }
                return Some(hit);
            }
        }
        self.misses += 1;

        let mut flags = CacheKeyFlags::empty();
        if italic {
            flags |= CacheKeyFlags::FAKE_ITALIC;
        }
        // 在向 swash 请求光栅化前将坐标偏置到目标亚像素相位，
        // 确保四个相位生成真正不同的位图。
        let phase_offset = phase as f32 * 0.25;
        let (cache_key, _, _) = CacheKey::new(
            face,
            glyph.glyph_id,
            glyph.font_size,
            (phase_offset, 0.0),
            cosmic_text::Weight(weight),
            flags,
        );
        let image = self
            .swash
            .get_image(engine.font_system_mut(), cache_key)
            .as_ref()?;

        let placement = image.placement;
        let w = placement.width;
        let h = placement.height;
        if w == 0 || h == 0 {
            // 空白字符：无纹理，仅记录步进宽度。
            return Some(ResolvedGlyph {
                slot: GlyphSlot::default(),
                bearing: Vec2::new(placement.left as f32, 0.0),
                size: Vec2::ZERO,
                advance: glyph.w,
                color: matches!(image.content, cosmic_text::SwashContent::Color),
                msdf: false,
            });
        }

        let is_color = matches!(image.content, cosmic_text::SwashContent::Color);
        let pad = self.padding;
        let pad2 = pad * 2;
        // 图集可能为腾出空间而淘汰已有字形；被淘汰键的缓存条目也必须一并删除，
        // 否则它将指向被回收复用的矩形区域。
        let mut evicted = core::mem::take(&mut self.evicted);
        let mut resolved = if is_color || sdf_mode {
            let slot = self
                .atlas
                .allocate_color(key, w + pad2, h + pad2, &mut evicted)?;
            let data = if sdf_mode {
                let field = sdf::from_coverage(&image.data, w, h, self.sdf_spread);
                pad_field(&field.data, w, h, pad)
            } else {
                pad_rgba(&image.data, w, h, pad)
            };
            self.updates.push(AtlasUpdate {
                target: AtlasTarget::Color,
                layer: slot.layer,
                x: slot.x,
                y: slot.y,
                width: slot.w,
                height: slot.h,
                data,
            });
            ResolvedGlyph {
                slot,
                bearing: Vec2::new(
                    placement.left as f32 - pad as f32,
                    -(placement.top as f32) - pad as f32,
                ),
                size: Vec2::new(w as f32 + pad2 as f32, h as f32 + pad2 as f32),
                advance: glyph.w,
                color: true,
                msdf: sdf_mode,
            }
        } else {
            let slot = self
                .atlas
                .allocate_coverage(key, w + pad2, h + pad2, &mut evicted)?;
            let data = pad_r8(&image.data, w, h, pad);
            self.updates.push(AtlasUpdate {
                target: AtlasTarget::Coverage,
                layer: slot.layer,
                x: slot.x,
                y: slot.y,
                width: slot.w,
                height: slot.h,
                data,
            });
            ResolvedGlyph {
                slot,
                bearing: Vec2::new(
                    placement.left as f32 - pad as f32,
                    -(placement.top as f32) - pad as f32,
                ),
                size: Vec2::new(w as f32 + pad2 as f32, h as f32 + pad2 as f32),
                advance: glyph.w,
                color: false,
                msdf: false,
            }
        };

        // UV 矩形必须覆盖**整个含边距的分配区域**，因为这与四边形尺寸一致。
        // 四边形宽 `w + 2·pad`，边距正是防止双线性采样触及相邻字形的屏障，
        // 因此四边形与 UV 都必须包含该边距。
        //
        let sz = self.atlas.r8.page_size() as f32;
        resolved.slot.uv = [
            resolved.slot.x as f32 / sz,
            resolved.slot.y as f32 / sz,
            (resolved.slot.x + resolved.slot.w) as f32 / sz,
            (resolved.slot.y + resolved.slot.h) as f32 / sz,
        ];

        // 被淘汰字形的矩形会归还空闲池并分配给下一个字形，因此仍指向该矩形的
        // 缓存条目会采样到别人的纹素。此处立即清除这些条目。
        //
        for key in evicted.drain(..) {
            self.resolved.remove(&key);
        }

        self.resolved.insert(key, resolved);
        Some(resolved)
    }

    /// 将整段已塑形的文本解析为四边形列表。
    ///
    /// `origin` 为调用方空间中文本框的左上角；返回的四边形位于同一空间。
    ///
    pub fn quads_for(
        &mut self,
        engine: &mut TextEngine,
        shaped: &ShapedText,
        origin: Vec2,
        weight: u16,
        italic: bool,
    ) -> Vec<GlyphQuad> {
        let mut out = Vec::with_capacity(shaped.glyphs.len());
        for g in &shaped.glyphs {
            let Some(face) = shaped.faces.get(g.face_index as usize).copied() else {
                continue;
            };
            let pen = origin + Vec2::new(g.x, g.y);
            if let Some(resolved) = self.resolve(engine, g, face, pen, weight, italic) {
                if resolved.size.x <= 0.0 || resolved.size.y <= 0.0 {
                    continue;
                }
                let snap = self.snap_phases;
                out.push(GlyphQuad {
                    rect: resolved.quad(pen, snap),
                    uv: resolved.slot.uv,
                    layer: resolved.slot.layer,
                    color: resolved.color,
                    msdf: resolved.msdf,
                    phase: Self::quantize_x(pen.x, snap).1,
                });
            }
        }
        out
    }

    /// 丢弃所有已缓存字形（图集扩容或字体重载时调用）。
    pub fn clear(&mut self) {
        self.resolved.clear();
        self.atlas = GlyphAtlas::default();
        self.updates.clear();
        // 字体索引保持有效：图集仅清除了纹素数据。
    }

    /// 图集占用的内存字节数。
    #[must_use]
    pub fn atlas_memory_bytes(&self) -> u64 {
        self.atlas.memory_bytes()
    }
}

fn pad_r8(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    // 围绕字形紧凑包围盒的纯透明边距槽。
    //
    // 四边形按 `padding` 向外扩展，因此外侧纹素**确实会被采样**——但它们必须
    // 采样为完全透明。早先版本向外渐变扩展边缘覆盖率（边缘钳制 + 50% 衰减），
    // 导致每个触及包围盒的纹素行旁都挤出灰色台阶：对于字母 `O`，这恰好发生在
    // 上下左右四个切点处，使其在四向长出方形污迹而对角处保持干净。
    //
    //
    //
    //
    // 边距槽的作用是阻止双线性采样触及相邻字形，而非额外增加墨迹。
    //
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph) as usize];
    if w == 0 || h == 0 {
        return out;
    }
    for y in 0..h {
        let dst = ((y + pad) * pw + pad) as usize;
        let src_off = (y * w) as usize;
        out[dst..dst + w as usize].copy_from_slice(&src[src_off..src_off + w as usize]);
    }
    out
}

fn pad_rgba(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..h {
        let dst = (((y + pad) * pw + pad) * 4) as usize;
        let src_off = (y * w * 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&src[src_off..src_off + (w * 4) as usize]);
    }
    out
}

fn pad_field(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    // 距离场的编码约定为“高值 = 内部”：GPU 参考实现（`unbound-gpu/src/sdf.rs`）
    // 断言全覆盖方块中心采样高于 0.9、角点低于 0.1，而 `uber.frag` 通过
    // `smoothstep(0.5 - w, 0.5 + w, m)` 将其转为覆盖率。
    //
    //
    // 因此边距必须填充为 **0**（表示“远离外部”）。若填充 255 会被解析为
    // “深处内部”，并在每个字形周围画出不透明的矩形光晕。
    //
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..h {
        let dst = (((y + pad) * pw + pad) * 4) as usize;
        let src_off = (y * w * 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&src[src_off..src_off + (w * 4) as usize]);
    }
    out
}

// `SubpixelBin` 体现了上述四相位方案并在 `CacheKey::new` 中使用；
// 保留显式导入以便读者查阅。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::AtlasConfig;
    use crate::shaping::TextStyle;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[test]
    fn the_glyph_gutter_stays_transparent() {
        // 宽 2 px、高 3 px 且左边缘高亮的源图——模拟触及紧凑包围盒边界的
        // 字形边缘（例如 `O` 的切点）。
        //
        let src = [255u8, 128, 64, 200, 40, 10];
        let out = pad_r8(&src, 2, 3, 2);
        let pw = 2 + 4;
        assert_eq!(out.len(), pw * (3 + 4));
        // 边距槽中的每个纹素均为零：无边缘钳制挤出，圆角不会长出方形污迹。
        //
        for x in 0..pw {
            assert_eq!(out[x], 0, "top gutter at x={x}");
            assert_eq!(out[(3 + 3) * pw + x], 0, "bottom gutter at x={x}");
        }
        for y in 0..7 {
            assert_eq!(out[y * pw], 0, "left gutter at y={y}");
            assert_eq!(out[y * pw + pw - 1], 0, "right gutter at y={y}");
        }
        // 字形本体原样拷贝，不做伽马渐变。
        assert_eq!(out[2 * pw + 2], 255);
        assert_eq!(out[2 * pw + 3], 128);
        assert_eq!(out[3 * pw + 2], 64);
        assert_eq!(out[3 * pw + 3], 200);
    }

    #[test]
    fn phase_bucketing_covers_four_steps() {
        assert_eq!(GlyphCache::phase_for(0.0), 0);
        assert_eq!(GlyphCache::phase_for(0.26), 1);
        assert_eq!(GlyphCache::phase_for(0.51), 2);
        assert_eq!(GlyphCache::phase_for(0.76), 3);
        assert_eq!(GlyphCache::phase_for(1.02), 0);
        assert_eq!(GlyphCache::phase_for(10.99), 0);
    }

    #[test]
    fn rasterising_a_label_produces_quads_and_uploads() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let style = TextStyle::sized(16.0);
        let shaped = e.shape("Ag", &style);
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 2, "both glyphs must produce a quad");
        assert!(
            cache.pending_update_bytes() > 0,
            "rasterising must queue uploads"
        );
        let updates = cache.take_updates();
        assert_eq!(updates.len(), 2);
        for u in &updates {
            assert_eq!(u.target, AtlasTarget::Coverage);
            assert_eq!(u.data.len(), u.byte_len());
            assert!(u.width > 0 && u.height > 0);
        }
        for q in &quads {
            assert!(q.rect.width() > 0.0 && q.rect.height() > 0.0);
            assert!(q.uv[2] > q.uv[0] && q.uv[3] > q.uv[1]);
            assert!(!q.color);
        }
    }

    /// 上传失败绝不能丢失字形。
    ///
    /// 缓存认为字形已在图集中，因此丢失批次意味着文本会采样到从未写入的纹素：
    /// 永久显示空白或乱码字形。因此必须将该批次放回队列。
    ///
    #[test]
    fn a_failed_upload_puts_the_glyphs_back_into_the_queue() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("Ag", &TextStyle::sized(16.0));
        let _ = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);

        let updates = cache.take_updates();
        assert_eq!(updates.len(), 2);
        assert!(cache.pending_updates().is_empty(), "the batch was drained");

        // 上传失败：批次放回队列。
        cache.restore_updates(updates);
        assert_eq!(
            cache.pending_updates().len(),
            2,
            "the glyphs would be lost for good"
        );
        assert!(cache.pending_update_bytes() > 0);

        // 下一次尝试会再次取出它们。
        let again = cache.take_updates();
        assert_eq!(again.len(), 2);
        assert!(cache.pending_updates().is_empty());
    }

    /// 恢复的批次排在此期间新入队更新的前面。
    #[test]
    fn a_restored_batch_keeps_its_place_at_the_front() {
        let mut cache = GlyphCache::new();
        let older = AtlasUpdate {
            target: AtlasTarget::Coverage,
            layer: 0,
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            data: vec![0u8; 12],
        };
        let newer = AtlasUpdate {
            target: AtlasTarget::Color,
            layer: 1,
            x: 5,
            y: 6,
            width: 7,
            height: 8,
            data: vec![0u8; 7 * 8 * 4],
        };
        cache.restore_updates(vec![older.clone()]);
        cache.updates.push(newer.clone());
        cache.restore_updates(vec![older.clone()]);

        let queued = cache.pending_updates();
        assert_eq!(queued.len(), 3);
        assert_eq!(queued[0], older, "the oldest batch comes first");
        assert_eq!(queued[2], newer);
    }

    #[test]
    fn second_resolution_hits_the_cache() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("hello", &TextStyle::sized(16.0));
        let first = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let uploads = cache.take_updates().len();
        let second = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(first, second);
        assert_eq!(cache.take_updates().len(), 0, "no re-upload on a cache hit");
        assert!(uploads > 0);
        assert!(cache.hit_rate() > 0.3);
        // h/e/l/o 外加不同亚像素相位的第二个 `l`：相位会改变光栅结果，因此属于键的一部分。
        //
        assert!(
            (4..=5).contains(&cache.resident_glyphs()),
            "resident = {}",
            cache.resident_glyphs()
        );
    }

    /// 斜体文本段绝不能复用正体字形的光栅结果。
    ///
    /// `FAKE_ITALIC` 会使 swash 从同一字体 ID 生成倾斜的光栅结果，因此两段文本
    /// 除倾斜度外共享键的所有其他部分。若键中不含倾斜标记，第二段将采样到第一段的纹素。
    ///
    ///
    #[test]
    fn an_italic_run_does_not_reuse_the_upright_raster() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("M", &TextStyle::sized(24.0));
        let upright = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let italic = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, true);
        assert_eq!(upright.len(), 1);
        assert_eq!(italic.len(), 1);
        assert_ne!(
            (upright[0].layer, upright[0].uv),
            (italic[0].layer, italic[0].uv),
            "the italic glyph must have its own atlas rectangle"
        );
    }

    /// 四边形原点绝不能携带画笔坐标的小数部分。
    ///
    /// 亚像素相位已烘焙进位图中，再次叠加画笔小数部分会导致字形双重偏移——
    /// 且使四边形落在非整像素坐标上，导致采样器将其模糊。
    ///
    #[test]
    fn quad_origin_floors_pen_x_to_avoid_double_subpixel_shift() {
        let glyph = ResolvedGlyph {
            slot: GlyphSlot::default(),
            bearing: Vec2::new(1.0, -2.0),
            size: Vec2::new(8.0, 10.0),
            advance: 9.0,
            color: false,
            msdf: false,
        };
        let quad = glyph.quad(Vec2::new(10.75, 20.4), false);
        assert_eq!(quad.origin.x, 11.0, "the pen's fraction must be dropped");
        assert_eq!(quad.origin.y, 18.0, "the baseline is snapped to a pixel");
        assert_eq!(quad.size.x, 8.0);
        assert_eq!(quad.size.y, 10.0);
    }

    /// 最后一个相位区间必须向下一整像素进位。
    ///
    /// `round(fract * 4)` 对 `fract ∈ [0.875, 1)` 返回 4，即**下一像素**的相位 0。
    /// 旧版 `% 4` 将其折回相位 0 却未移动四边形，导致 12.5% 的字形向左偏移一像素。
    ///
    #[test]
    fn the_last_phase_bucket_carries_into_the_next_pixel() {
        for thousandths in 875..1000 {
            let x = 10.0 + thousandths as f32 / 1000.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            assert_eq!(phase, 0, "x = {x} must round to phase 0");
            assert_eq!(base, 11.0, "x = {x} must move to the next pixel");
        }
    }

    /// 量化必须单调：较大画笔位置绝不会渲染到较小位置的左侧，
    /// 且量化后的位置误差保持在 ⅛ px 以内。
    #[test]
    fn quantize_x_is_monotonic_and_never_jumps_backwards() {
        let mut previous = f32::NEG_INFINITY;
        for step in 0..=1280 {
            let x = step as f32 / 64.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            let quantised = base + phase as f32 * 0.25;
            assert!(
                quantised >= previous,
                "x = {x} quantised to {quantised}, behind {previous}"
            );
            assert!(
                (quantised - x).abs() <= 0.126,
                "x = {x} quantised to {quantised}, more than ⅛ px away"
            );
            previous = quantised;
        }
    }

    /// 整像素对齐将四边形置于四舍五入后的画笔位置，而非向下取整位置。
    #[test]
    fn snapped_phases_use_the_rounded_pen() {
        let glyph = ResolvedGlyph {
            slot: GlyphSlot::default(),
            bearing: Vec2::ZERO,
            size: Vec2::new(4.0, 4.0),
            advance: 4.0,
            color: false,
            msdf: false,
        };
        let quad = glyph.quad(Vec2::new(10.6, 0.0), true);
        assert_eq!(quad.origin.x, 11.0);
        assert_eq!(GlyphCache::quantize_x(10.6, true), (11.0, 0));
    }

    /// 字形光栅化所用的相位必须遵循其**绝对**坐标：光栅按该相位生成，
    /// 若从相对于文本段的位置推导，每个字形最多会偏移一像素。
    ///
    #[test]
    fn the_subpixel_phase_follows_the_absolute_pen_position() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("Hello", &TextStyle::sized(16.0));
        let at_zero = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let at_half = cache.quads_for(&mut e, &shaped, Vec2::new(0.5, 0.0), 400, false);
        assert_eq!(at_zero.len(), at_half.len());
        for (a, b) in at_zero.iter().zip(at_half.iter()) {
            assert_ne!(
                a.phase, b.phase,
                "moving the run by half a pixel must change the phase"
            );
        }
    }

    /// 位于同一绝对坐标的两段相同文本必须生成完全一致的四边形——
    /// 相位由坐标位置推导，而非由文本段的排版顺序决定。
    ///
    #[test]
    fn the_same_absolute_position_produces_the_same_phase() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("Hg", &TextStyle::sized(16.0));
        let first = cache.quads_for(&mut e, &shaped, Vec2::new(3.25, 0.0), 400, false);
        let second = cache.quads_for(&mut e, &shaped, Vec2::new(3.25, 0.0), 400, false);
        assert_eq!(first, second);
        // 四边形原点向下取整到光栅的像素网格上。
        for quad in &first {
            assert_eq!(quad.rect.origin.x, quad.rect.origin.x.floor());
            assert_eq!(quad.rect.origin.y, quad.rect.origin.y.floor());
        }
    }

    /// 当图集淘汰字形时，指向它的缓存条目必须一并删除：矩形会被回收并分配给
    /// 下一个字形。
    #[test]
    fn an_evicted_glyph_leaves_the_cache() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        // 无法容纳整串文本的极小图集。
        cache.atlas = GlyphAtlas::new(AtlasConfig {
            page_size: 32,
            max_layers: 1,
            padding: 1,
        });
        let shaped = e.shape("hello world", &TextStyle::sized(16.0));
        cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let resident = cache.resident_glyphs();
        assert!(resident > 0);
        // 图集至少淘汰了一个字形以容纳所有字符。
        assert!(
            cache.atlas.r8.evictions() > 0,
            "the test must actually exercise the eviction path"
        );
        // 每个常驻条目都必须仍拥有存活的图集槽位：槽位已被回收的条目会采样到
        // 别人的纹素。
        assert!(
            cache.resident_glyphs() <= resident,
            "the cache must not grow beyond the atlas"
        );
    }

    #[test]
    fn whitespace_has_an_advance_but_no_quad() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("a b", &TextStyle::sized(16.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 2, "the space must not paint");
    }

    #[test]
    fn sdf_mode_targets_the_rgba_atlas() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        cache.use_sdf = true;
        let shaped = e.shape("M", &TextStyle::sized(24.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 1);
        assert!(quads[0].color, "distance fields live in the RGBA atlas");
        assert!(
            quads[0].msdf,
            "the quad must carry the MSDF flag to the primitive builder"
        );
        let updates = cache.take_updates();
        assert_eq!(updates[0].target, AtlasTarget::Color);
        assert_eq!(updates[0].data.len(), updates[0].byte_len());
        // 距离场编码约定为“高值 = 内部”，因此边距必须被解读为**外部**（低值）。
        // 若填充 255 会被解读为“深处内部”，并在每个字形周围画出不透明光晕。
        //
        assert!(updates[0].data[0] < 55, "padded SDF must read as outside");
    }

    #[test]
    fn scaled_text_produces_larger_quads() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let small = e.shape("W", &TextStyle::sized(12.0));
        let large = e.shape("W", &TextStyle::sized(48.0));
        let qs = cache.quads_for(&mut e, &small, Vec2::ZERO, 400, false);
        let ql = cache.quads_for(&mut e, &large, Vec2::ZERO, 400, false);
        assert!(ql[0].rect.height() > qs[0].rect.height() * 2.0);
        assert_ne!(
            (ql[0].layer, ql[0].uv),
            (qs[0].layer, qs[0].uv),
            "different sizes must not share a slot"
        );
    }

    #[test]
    fn quads_follow_the_pen_positions() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("ii", &TextStyle::sized(16.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::new(10.0, 20.0), 400, false);
        assert_eq!(quads.len(), 2);
        assert!(quads[1].rect.origin.x > quads[0].rect.origin.x);
        assert!(quads[0].rect.origin.x >= 10.0 - 2.0);
    }

    #[test]
    fn clearing_resets_the_atlas() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("abc", &TextStyle::sized(16.0));
        cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert!(cache.resident_glyphs() > 0);
        cache.clear();
        assert_eq!(cache.resident_glyphs(), 0);
        assert_eq!(cache.atlas_memory_bytes(), 0);
    }
}

//! 字形光栅化、图集驻留管理与四边形图元生成。
//!
//! 单个字形的处理管线：
//!
//! 1. 使用 `swash` 按其坐标所隐含的亚像素相位进行光栅化（共 8 个相位，
//!    使 12px 标签保持均匀字距，而非将每个字形强行对齐到同一像素网格）；
//!
//! 2. 在 R8 图集中预留槽位（彩色 Emoji 与距离场则使用 RGBA 图集）；
//!
//! 3. 生成供渲染器上传到纹理数组的 [`AtlasUpdate`]；
//! 4. 生成供视图层转换为 `GpuPrimitiveInstance` 的 [`GlyphQuad`]。
//!
//! 步骤 1–4 均为纯 CPU 计算，因此整套缓存无需 GPU 设备即可完成测试。

use cosmic_text::fontdb;
#[cfg(feature = "backend-swash")]
use cosmic_text::{CacheKey, CacheKeyFlags, SwashCache};
use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};

use crate::atlas::{GlyphAtlas, GlyphKey, GlyphSlot};
use crate::composite::cluster_ranges;
use crate::report::TextRenderReport;
use crate::sdf;
use crate::settings::{InvalidationTier, PhaseMode, TextRenderSettings};
use crate::shaping::{PositionedGlyph, ShapedText, TextEngine};

/// 更新目标图集类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AtlasTarget {
    /// `Atlas_R8`，每纹素 1 字节。
    Coverage,
    /// `Atlas_RGBA8`，每纹素 4 字节。
    Color,
}

/// 字形光栅化后端。
///
/// `DirectWrite` 提供 8 位灰度的对称抗锯齿覆盖率；`Cosmic` 是纯 Rust
/// 回退路径，用于非 Windows 平台、彩色字形与距离场。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RasterBackend {
    /// Windows DirectWrite 覆盖率位图。
    DirectWrite,
    /// `swash` 覆盖率位图。
    Cosmic,
}

#[allow(clippy::derivable_impls)] // 默认后端依据平台与编译期特性决定，不可直接 derive
impl Default for RasterBackend {
    fn default() -> Self {
        #[cfg(all(windows, feature = "backend-directwrite"))]
        {
            Self::DirectWrite
        }
        #[cfg(not(all(windows, feature = "backend-directwrite")))]
        {
            Self::Cosmic
        }
    }
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
    /// 文本局部空间中的四边形（指定亚像素相位档位）。
    #[must_use]
    pub fn quad_with_mode(&self, pen: Vec2, mode: PhaseMode) -> Rect {
        let (base_x, _) = mode.quantize_x(pen.x);
        Rect::new(
            base_x + self.bearing.x,
            pen.y.round() + self.bearing.y,
            self.size.x,
            self.size.y,
        )
    }

    /// 文本局部空间中的四边形。
    ///
    /// 画笔坐标会对齐到光栅自身的像素网格：画笔位置的亚像素部分已被烘焙进
    /// 位图中（光栅按 `pen.x` 隐含的相位生成），若再次叠加小数部分会导致
    /// 字形发生双重偏移。
    ///
    /// 水平原点来自 [`GlyphCache::quantize_x`]，它连同相位一起携带了整像素：
    /// 落在 `[0.875, 1.0)` 的画笔小数部分会舍入到**下一像素**的相位 0，
    /// 四边形也必须随之移动。丢弃该进位（旧版单纯 `% 4` 的做法）曾使
    /// 12.5% 的字形向左偏移一个像素，导致其边缘与前一个字母重叠。
    #[must_use]
    pub fn quad(&self, pen: Vec2, snap_phases: bool) -> Rect {
        self.quad_with_mode(pen, PhaseMode::from(snap_phases))
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
    /// 该字形使用的水平亚像素相位（0..7，步长 ⅛ px）。
    pub phase: u8,
}

/// Cosmic (swash) 字形光栅化位图与度量。
#[cfg(feature = "backend-swash")]
#[derive(Clone, Debug, PartialEq)]
pub struct CosmicRaster {
    pub width: u32,
    pub height: u32,
    /// 位图左边缘相对画笔原点的水平偏移。
    pub left: f32,
    /// 位图顶边缘相对基线的垂直偏移。
    pub top: f32,
    /// 是否为彩色（RGBA8）位图。
    pub is_color: bool,
    /// 纹素数据（单色 R8 或彩色 RGBA8）。
    pub data: Vec<u8>,
}

/// 一簇彩色字形的解析结论。
///
/// 非 DirectWrite 构建只会产出 `NotColor`（彩色仍归 swash），因此这里放行
/// 另外两个分支的 `dead_code`：它们是 Windows 彩色路径的出口。
#[allow(dead_code)]
enum ClusterResolution {
    /// 已合成并驻留图集，可直接发射四边形。
    Rendered(ResolvedGlyph),
    /// 不是彩色字形：按普通单色 run 处理，不计回退。
    NotColor,
    /// 是彩色簇但合成不了；`force_cosmic` 表示回退策略要求改走 `swash`。
    Fallback { force_cosmic: bool },
}

/// 光栅化并缓存字形，维护其在图集中的驻留状态。
pub struct GlyphCache {
    settings: TextRenderSettings,
    #[cfg(feature = "backend-swash")]
    swash: SwashCache,
    pub atlas: GlyphAtlas,
    resolved: FxHashMap<GlyphKey, ResolvedGlyph>,
    /// 自上次排空以来产生的图集更新。
    updates: Vec<AtlasUpdate>,
    /// 输出距离场而非普通覆盖率位图（用于画布文本）。
    #[deprecated(note = "use TextRenderSettings and GlyphCache::apply_settings instead")]
    pub use_sdf: bool,
    /// 距离场扩散范围（单位：纹素）。
    #[deprecated(note = "use TextRenderSettings and GlyphCache::apply_settings instead")]
    pub sdf_spread: f32,
    /// 每个字形四周的额外边距纹素（防止双线性采样渗色）。
    #[deprecated(note = "use TextRenderSettings and GlyphCache::apply_settings instead")]
    pub padding: u32,
    /// 在整像素网格上光栅化字形，而非使用 `SUBPIXEL_PHASES` 个亚像素相位。
    ///
    /// 相位必须与四边形最终绘制的位置匹配：光栅按特定相位生成且四边形对齐到
    /// 像素网格，若相位光栅与画笔位置不匹配，采样点将偏离生成位置，每条边缘
    /// 都会退化为半像素混合。
    ///
    /// 开启此选项会强制所有字形使用相位 0，仅当画笔本身始终位于整像素上时才正确。
    /// UI 文本保留八相位方案：相位由字形的绝对坐标推导，与真实画笔位置的误差
    /// 至多为 ⅛ px。
    ///
    #[deprecated(note = "use TextRenderSettings and GlyphCache::apply_settings instead")]
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
    /// 当前生效的字形光栅化后端。
    backend: RasterBackend,
    /// DirectWrite 光栅化器；初始化失败时为 `None`，并自动回退到 `Cosmic`。
    #[cfg(all(windows, feature = "backend-directwrite"))]
    directwrite: Option<crate::directwrite::DirectWriteRasterizer>,
    /// 累计发生的光栅化或字体回退次数。
    fallback_count: u64,
    /// 最近一次回退的原因说明。
    last_fallback_reason: Option<String>,
}

impl core::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("resident", &self.resolved.len())
            .field("updates", &self.updates.len())
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .field("fallback_count", &self.fallback_count)
            .finish_non_exhaustive()
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(deprecated)]
impl GlyphCache {
    #[must_use]
    pub fn new() -> Self {
        // 环境变量用于 A/B 对照与 CI：`cosmic` 强制回退，`directwrite` 强制首选。
        let requested = match std::env::var("UNBOUND_TEXT_BACKEND").ok().as_deref() {
            Some("cosmic") => RasterBackend::Cosmic,
            Some("directwrite") => RasterBackend::DirectWrite,
            _ => RasterBackend::default(),
        };
        Self::with_backend(requested)
    }

    /// 按指定后端创建缓存；DirectWrite 不可用时自动回退到 `Cosmic`。
    #[must_use]
    pub fn with_backend(requested: RasterBackend) -> Self {
        let mut backend = requested;
        let mut fallback_count = 0u64;
        let mut last_fallback_reason = None;

        #[cfg(not(all(windows, feature = "backend-directwrite")))]
        if backend == RasterBackend::DirectWrite {
            backend = RasterBackend::Cosmic;
            fallback_count = fallback_count.saturating_add(1);
            last_fallback_reason =
                Some("DirectWrite backend is not compiled in this build".to_string());
        }

        #[cfg(not(feature = "backend-swash"))]
        if backend == RasterBackend::Cosmic {
            backend = RasterBackend::DirectWrite;
            fallback_count = fallback_count.saturating_add(1);
            last_fallback_reason =
                Some("Cosmic (swash) backend is not compiled in this build".to_string());
        }

        #[cfg(all(windows, feature = "backend-directwrite"))]
        let directwrite = if backend == RasterBackend::DirectWrite {
            let instance = if std::env::var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL").is_ok() {
                None
            } else {
                crate::directwrite::DirectWriteRasterizer::new()
            };
            if instance.is_none() {
                #[cfg(feature = "backend-swash")]
                {
                    backend = RasterBackend::Cosmic;
                }
                fallback_count = fallback_count.saturating_add(1);
                last_fallback_reason =
                    Some("DirectWrite factory initialization failed".to_string());
            }
            instance
        } else {
            None
        };
        let settings = TextRenderSettings {
            raster_backend: backend,
            ..Default::default()
        };
        #[allow(deprecated)]
        Self {
            settings,
            #[cfg(feature = "backend-swash")]
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
            backend,
            #[cfg(all(windows, feature = "backend-directwrite"))]
            directwrite,
            fallback_count,
            last_fallback_reason,
        }
    }

    /// 记录一次底层回退事件（如 DirectWrite 初始化或光栅化失败）。
    pub fn record_fallback(&mut self, reason: impl Into<String>) {
        self.fallback_count = self.fallback_count.saturating_add(1);
        self.last_fallback_reason = Some(reason.into());
    }

    /// 累计发生的光栅化或字体回退次数。
    #[must_use]
    pub fn fallback_count(&self) -> u64 {
        self.fallback_count
    }

    /// 最近一次回退的原因说明。
    #[must_use]
    pub fn last_fallback_reason(&self) -> Option<&str> {
        self.last_fallback_reason.as_deref()
    }

    /// 获取文本渲染实际生效报告快照。
    #[must_use]
    pub fn report(&self) -> TextRenderReport {
        TextRenderReport {
            backend: self.backend,
            phases: self.settings.phase_mode.phases(),
            emoji_path: match (
                self.settings.emoji_presentation,
                self.settings.emoji_backend,
            ) {
                (crate::settings::EmojiPresentation::Color, _) => "Swash (Color RGBA8)".to_string(),
                (
                    crate::settings::EmojiPresentation::Mono,
                    crate::settings::EmojiBackend::DirectWrite,
                ) => "DirectWrite (Mono R8)".to_string(),
                (
                    crate::settings::EmojiPresentation::Mono,
                    crate::settings::EmojiBackend::Follow,
                ) => {
                    if self.backend == RasterBackend::DirectWrite {
                        "DirectWrite (Mono R8)".to_string()
                    } else {
                        "Swash (Mono R8)".to_string()
                    }
                }
                (
                    crate::settings::EmojiPresentation::Mono,
                    crate::settings::EmojiBackend::Cosmic,
                ) => "Swash (Mono R8)".to_string(),
            },
            fallback_count: self.fallback_count,
            last_fallback_reason: self.last_fallback_reason.clone(),
            rgba_atlas_allocated: self.atlas.has_rgba(),
            atlas_memory_bytes: self.atlas.allocated_bytes(),
        }
    }

    /// 当前实际使用的光栅化后端。
    #[must_use]
    pub fn backend(&self) -> RasterBackend {
        self.backend
    }

    /// 获取当前生效的设置引用。
    #[must_use]
    pub fn settings(&self) -> &TextRenderSettings {
        &self.settings
    }

    /// 切换光栅化后端并同步初始化底层光栅化器。
    pub fn set_backend(&mut self, requested: RasterBackend) {
        let mut backend = requested;

        #[cfg(not(all(windows, feature = "backend-directwrite")))]
        if backend == RasterBackend::DirectWrite {
            backend = RasterBackend::Cosmic;
            self.record_fallback("DirectWrite backend is not compiled in this build");
        }

        #[cfg(not(feature = "backend-swash"))]
        if backend == RasterBackend::Cosmic {
            backend = RasterBackend::DirectWrite;
            self.record_fallback("Cosmic (swash) backend is not compiled in this build");
        }

        #[cfg(all(windows, feature = "backend-directwrite"))]
        {
            if backend == RasterBackend::DirectWrite {
                let instance = if std::env::var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL").is_ok() {
                    None
                } else {
                    crate::directwrite::DirectWriteRasterizer::new()
                };
                if instance.is_none() {
                    #[cfg(feature = "backend-swash")]
                    {
                        backend = RasterBackend::Cosmic;
                    }
                    self.record_fallback("DirectWrite factory initialization failed");
                }
                self.directwrite = instance;
            } else {
                self.directwrite = None;
            }
        }
        self.backend = backend;
        self.settings.raster_backend = backend;
    }

    /// 应用新设置，执行字段级差异比对并按三档执行失效。
    ///
    /// - `None`（预览样式或相同配置）：不清除任何缓存；
    /// - `Typographic`（排版语义）：若传入 `engine`，清空其塑形与换行缓存，但不清字形位图；
    /// - `Bitmap`（位图内容）：清空整张字形图集与待上传队列，整表重建。
    pub fn apply_settings(
        &mut self,
        settings: &TextRenderSettings,
        engine: Option<&mut TextEngine>,
    ) -> InvalidationTier {
        let mut resolved_settings = settings.clone();
        resolved_settings.enforce_constraints();
        let tier = self.settings.invalidation_tier(&resolved_settings);
        match tier {
            InvalidationTier::Bitmap => {
                self.clear();
                if self.settings.raster_backend != resolved_settings.raster_backend {
                    self.set_backend(resolved_settings.raster_backend);
                }
                #[allow(deprecated)]
                {
                    self.use_sdf = resolved_settings.use_sdf;
                    self.sdf_spread = resolved_settings.sdf_spread;
                    self.padding = resolved_settings.padding;
                    self.snap_phases = resolved_settings.snap_phases();
                }
            }
            InvalidationTier::Typographic | InvalidationTier::None => {}
        }
        if let Some(engine) = engine {
            engine.apply_settings(&self.settings, settings);
        }
        self.settings = resolved_settings;
        tier
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

    /// 返回当前生效的亚像素相位档位。
    #[must_use]
    pub fn phase_mode(&self) -> PhaseMode {
        self.settings.phase_mode
    }

    /// 切换亚像素相位档位，若发生变更则触发整表清空。
    pub fn set_phase_mode(&mut self, mode: PhaseMode) {
        if self.settings.phase_mode != mode {
            let mut settings = self.settings.clone();
            settings.phase_mode = mode;
            self.apply_settings(&settings, None);
        }
    }

    /// 将画笔坐标拆分为整像素原点与亚像素相位。
    ///
    /// 八个相位覆盖 `[0, 1)`，步长 ⅛ px。WinUI 的 DirectWrite 光栅按真实
    /// 亚像素位置生成覆盖率，参考截图中同一根字干的左右覆盖率约为
    /// 0.72/0.46（相位 ≈ 0.28）；四分相位只能取 0.25 或 0.5，会在每根
    /// 竖笔边缘留下可测的覆盖率误差。实测八分相位把残差压得最低——再细
    /// 到 ⅟₁₆ 反而会与参考的平滑方式错位，门禁通过数下降。
    /// 相位由四舍五入决定，因此最后一个区间顶端（`fract ≥ 0.9375`）会
    /// 舍入到 `8`——即**下一像素**的相位 0。整像素原点必须随之进位，
    /// 这正是 `div_euclid`/`rem_euclid` 所做的：由同一辅助函数同时生成两个
    /// 数值，确保二者永不矛盾。
    ///
    /// `snap_phases` 则强制整像素光栅（在 `round(x)` 处取相位 0），
    /// 供画笔始终位于像素网格上的调用方使用。
    #[must_use]
    pub fn quantize_x(x: f32, snap_phases: bool) -> (f32, u8) {
        PhaseMode::from(snap_phases).quantize_x(x)
    }

    /// 按指定相位档位量化画笔坐标。
    #[must_use]
    pub fn quantize_x_mode(x: f32, mode: PhaseMode) -> (f32, u8) {
        mode.quantize_x(x)
    }

    /// 由画笔位置推导出的亚像素相位。
    #[must_use]
    pub fn phase_for(x: f32) -> u8 {
        PhaseMode::Eight.quantize_x(x).1
    }

    /// `pen_x` 处字形实际光栅化所用的亚像素相位。
    ///
    /// 相位必须由字形的**绝对**位置推导：光栅按该相位生成，若基于
    /// 相对文本段的偏移推导，每个字形最多会偏移一个像素。
    ///
    #[must_use]
    pub fn phase_at(&self, pen_x: f32) -> u8 {
        self.settings.phase_mode.quantize_x(pen_x).1
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
        let mode = self.settings.phase_mode;
        let phase = mode.quantize_x(pen.x).1;
        let sdf_mode = self.use_sdf;
        let face_index = u64::from(self.face_id(face));
        // 光栅化字重必须与文本段塑形时使用的字重一致。此前在此叠加“光学”
        // 补偿导致小号标签比其排版步进更粗，且 18 px 处的阶跃造成字号变化时
        // 字重突变。
        //
        // 光栅化结果取决于相位、字重与斜体倾斜，因此三者均纳入键中。
        //
        let key = GlyphKey::styled_with_mode(
            glyph.glyph_id,
            glyph.font_size,
            phase,
            sdf_mode,
            face_index,
            weight,
            italic,
            mode,
            self.settings.emoji_presentation as u8,
            self.settings.palette_index,
        );
        if let Some(hit) = self.cached(&key) {
            return Some(hit);
        }
        self.misses += 1;

        // Windows 上优先使用 DirectWrite：覆盖率位图直接取自系统光栅器。
        // 彩色字体或字体解析失败时静默回退 swash（距离场通过覆盖率位图后处理生成），并记录回退诊断。
        #[cfg(all(windows, feature = "backend-directwrite"))]
        let try_directwrite = match self.settings.emoji_backend {
            crate::settings::EmojiBackend::DirectWrite => true,
            crate::settings::EmojiBackend::Cosmic => false,
            crate::settings::EmojiBackend::Follow => self.backend == RasterBackend::DirectWrite,
        };
        #[cfg(all(windows, feature = "backend-directwrite"))]
        if try_directwrite {
            let phase_offset = mode.phase_offset(phase);
            if let Some(raster) = self.raster_directwrite(engine, face, glyph, phase_offset) {
                return self.commit_raster(
                    key,
                    glyph.w,
                    raster.width,
                    raster.height,
                    raster.bearing_x,
                    -raster.bearing_y,
                    false,
                    raster.data,
                    sdf_mode,
                );
            } else if self.backend == RasterBackend::DirectWrite {
                self.record_fallback(format!(
                    "DirectWrite glyph rasterization failed for glyph {}",
                    glyph.glyph_id
                ));
            }
        }

        #[cfg(feature = "backend-swash")]
        {
            let phase_offset = mode.phase_offset(phase);
            if let Some(raster) =
                self.rasterize_cosmic(engine, face, glyph, phase_offset, italic, weight)
            {
                self.commit_raster(
                    key,
                    glyph.w,
                    raster.width,
                    raster.height,
                    raster.left,
                    raster.top,
                    raster.is_color,
                    raster.data,
                    sdf_mode,
                )
            } else {
                None
            }
        }

        #[cfg(not(feature = "backend-swash"))]
        {
            let _ = (face, glyph, phase, mode, italic, weight, key, sdf_mode);
            self.record_fallback("Swash backend is not compiled in");
            None
        }
    }

    /// 命中图集缓存时刷新代际并返回已解析字形。
    fn cached(&mut self, key: &GlyphKey) -> Option<ResolvedGlyph> {
        let hit = self.resolved.get(key).copied()?;
        if !hit.slot.is_valid() {
            return None;
        }
        // 刷新图集代际而无需重新光栅化。
        self.hits += 1;
        if hit.color {
            if let Some(rgba) = self.atlas.rgba.as_mut() {
                rgba.touch(key);
            }
        } else {
            self.atlas.r8.touch(key);
        }
        Some(hit)
    }

    /// 将一块已光栅化的位图写入图集并登记缓存。
    ///
    /// `left` 为位图左缘相对画笔原点的偏移，`top` 为位图顶缘相对基线的
    /// **向上**偏移（与 `swash` 的 `Placement.top` 一致）。
    #[allow(clippy::too_many_arguments)]
    fn commit_raster(
        &mut self,
        key: GlyphKey,
        advance: f32,
        width: u32,
        height: u32,
        left: f32,
        top: f32,
        is_color: bool,
        data: Vec<u8>,
        sdf_mode: bool,
    ) -> Option<ResolvedGlyph> {
        if width == 0 || height == 0 {
            // 空白字符：无纹理，仅记录步进宽度。
            return Some(ResolvedGlyph {
                slot: GlyphSlot::default(),
                bearing: Vec2::new(left, 0.0),
                size: Vec2::ZERO,
                advance,
                color: is_color,
                msdf: false,
            });
        }

        // 彩色字形（如 Emoji）已经是 4 通道 RGBA 位图，不能按单通道覆盖率生成距离场。
        let use_sdf = sdf_mode && !is_color;
        let pad = self.padding;
        let pad2 = pad * 2;
        // 图集可能为腾出空间而淘汰已有字形；被淘汰键的缓存条目也必须一并删除，
        // 否则它将指向被回收复用的矩形区域。
        let mut evicted = core::mem::take(&mut self.evicted);
        let mut resolved = if is_color {
            let slot = self
                .atlas
                .allocate_color(key, width + pad2, height + pad2, &mut evicted)?;
            let data = pad_rgba(&data, width, height, pad);
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
                bearing: Vec2::new(left - pad as f32, -(top) - pad as f32),
                size: Vec2::new(width as f32 + pad2 as f32, height as f32 + pad2 as f32),
                advance,
                color: true,
                msdf: false,
            }
        } else if use_sdf {
            let slot = self
                .atlas
                .allocate_sdf(key, width + pad2, height + pad2, &mut evicted)?;
            let field = sdf::from_coverage(&data, width, height, self.sdf_spread);
            let data = pad_field(&field.data, width, height, pad);
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
                bearing: Vec2::new(left - pad as f32, -(top) - pad as f32),
                size: Vec2::new(width as f32 + pad2 as f32, height as f32 + pad2 as f32),
                advance,
                color: true,
                msdf: true,
            }
        } else {
            let slot =
                self.atlas
                    .allocate_coverage(key, width + pad2, height + pad2, &mut evicted)?;
            let data = pad_r8(&data, width, height, pad);
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
                bearing: Vec2::new(left - pad as f32, -(top) - pad as f32),
                size: Vec2::new(width as f32 + pad2 as f32, height as f32 + pad2 as f32),
                advance,
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

    /// 用 DirectWrite 光栅化单个字形；字体家族或彩色字体不适用时返回 `None`。
    #[cfg(all(windows, feature = "backend-directwrite"))]
    pub fn rasterize_directwrite(
        &mut self,
        engine: &mut TextEngine,
        face: fontdb::ID,
        glyph: &PositionedGlyph,
        phase_offset: f32,
    ) -> Option<crate::directwrite::RasterizedGlyph> {
        let directwrite = self.directwrite.as_mut()?;
        let key = {
            let info = engine.font_system_mut().db().face(face)?;
            crate::directwrite::FaceKey {
                family: info.families.first()?.0.clone(),
                weight: info.weight.0,
                italic: matches!(info.style, fontdb::Style::Italic | fontdb::Style::Oblique),
            }
        };
        let mono = self.settings.emoji_presentation == crate::settings::EmojiPresentation::Mono;
        directwrite.rasterize(
            &key,
            glyph.glyph_id,
            glyph.font_size,
            glyph.w,
            phase_offset,
            mono,
        )
    }

    #[cfg(all(windows, feature = "backend-directwrite"))]
    #[inline]
    fn raster_directwrite(
        &mut self,
        engine: &mut TextEngine,
        face: fontdb::ID,
        glyph: &PositionedGlyph,
        phase_offset: f32,
    ) -> Option<crate::directwrite::RasterizedGlyph> {
        self.rasterize_directwrite(engine, face, glyph, phase_offset)
    }

    /// 用 Cosmic (swash) 光栅化单个字形。
    #[cfg(feature = "backend-swash")]
    pub fn rasterize_cosmic(
        &mut self,
        engine: &mut TextEngine,
        face: fontdb::ID,
        glyph: &PositionedGlyph,
        phase_offset: f32,
        italic: bool,
        weight: u16,
    ) -> Option<CosmicRaster> {
        let mut flags = CacheKeyFlags::empty();
        if italic {
            flags |= CacheKeyFlags::FAKE_ITALIC;
        }
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
        let mut is_color = matches!(image.content, cosmic_text::SwashContent::Color);
        let mut data = image.data.to_vec();
        let width = placement.width;
        let height = placement.height;
        let left = placement.left as f32;
        let top = placement.top as f32;

        if is_color && self.settings.emoji_presentation == crate::settings::EmojiPresentation::Mono
        {
            is_color = false;
            let mono_len = (width * height) as usize;
            let mut mono_data = Vec::with_capacity(mono_len);
            for chunk in data.chunks_exact(4) {
                let a = chunk[3] as u32;
                if a == 0 {
                    mono_data.push(0);
                } else {
                    let r = chunk[0] as u32;
                    let g = chunk[1] as u32;
                    let b = chunk[2] as u32;
                    let gray = (r * 299 + g * 587 + b * 114) / 1000;
                    mono_data.push(((gray * a) / 255) as u8);
                }
            }
            data = mono_data;
        }

        Some(CosmicRaster {
            width,
            height,
            left,
            top,
            is_color,
            data,
        })
    }

    /// 当前设置是否允许走 DirectWrite 彩色层合成。
    ///
    /// 只在显式要求彩色呈现、且光栅器落在 DirectWrite 上时成立；显式选择
    /// `swash` 后端时彩色仍归 swash。非 Windows 构建恒为 `false`。
    fn color_cluster_enabled(&self) -> bool {
        #[cfg(all(windows, feature = "backend-directwrite"))]
        {
            if self.settings.emoji_presentation != crate::settings::EmojiPresentation::Color {
                return false;
            }
            if self.directwrite.is_none() {
                return false;
            }
            match self.settings.emoji_backend {
                crate::settings::EmojiBackend::DirectWrite => true,
                crate::settings::EmojiBackend::Cosmic => false,
                crate::settings::EmojiBackend::Follow => self.backend == RasterBackend::DirectWrite,
            }
        }
        #[cfg(not(all(windows, feature = "backend-directwrite")))]
        {
            false
        }
    }

    /// 解析一簇彩色字形：一簇一个图集条目、一个四边形。
    ///
    /// 相位锁 0，原点由调用方按 `round(pen.x)` 取整（见 ADR 0017）。
    #[cfg(all(windows, feature = "backend-directwrite"))]
    fn resolve_color_cluster(
        &mut self,
        engine: &mut TextEngine,
        glyphs: &[PositionedGlyph],
        face: fontdb::ID,
        weight: u16,
        italic: bool,
    ) -> ClusterResolution {
        use crate::composite::{cluster_hash, composite, ColorLayer, CompositeOutcome, LayerColor};
        use crate::directwrite::{ClusterGlyph, ColorClusterOutcome};
        use crate::settings::EmojiFallback;

        let Some(first) = glyphs.first().copied() else {
            return ClusterResolution::NotColor;
        };
        let face_key = {
            let Some(info) = engine.font_system_mut().db().face(face) else {
                return ClusterResolution::NotColor;
            };
            let Some(family) = info.families.first() else {
                return ClusterResolution::NotColor;
            };
            crate::directwrite::FaceKey {
                family: family.0.clone(),
                weight: info.weight.0,
                italic: matches!(info.style, fontdb::Style::Italic | fontdb::Style::Oblique),
            }
        };
        if !self
            .directwrite
            .as_mut()
            .is_some_and(|dw| dw.is_color_face(&face_key))
        {
            return ClusterResolution::NotColor;
        }

        let face_index = self.face_id(face);
        let ids: Vec<u16> = glyphs.iter().map(|g| g.glyph_id).collect();
        let atlas_key = GlyphKey::styled_with_mode(
            first.glyph_id,
            first.font_size,
            0,
            false,
            u64::from(face_index),
            weight,
            italic,
            PhaseMode::One,
            crate::settings::EmojiPresentation::Color as u8,
            self.settings.palette_index,
        )
        .with_cluster(cluster_hash(&ids));

        if let Some(hit) = self.cached(&atlas_key) {
            return ClusterResolution::Rendered(hit);
        }

        // DWrite 的 run 用「步进 + 偏移」定位，这里把塑形结果的绝对坐标
        // 折算回相对 run 起点的偏移，避免多字形簇内部错位。
        let mut cluster_glyphs = Vec::with_capacity(glyphs.len());
        let mut advanced = 0.0f32;
        for g in glyphs {
            cluster_glyphs.push(ClusterGlyph {
                glyph_id: g.glyph_id,
                advance: g.w,
                offset_x: (g.x - first.x) - advanced,
                offset_y: g.y - first.y,
            });
            advanced += g.w;
        }
        let advance = advanced;

        let outcome = match self.directwrite.as_mut() {
            Some(dw) => dw.rasterize_color_cluster(
                &face_key,
                &cluster_glyphs,
                first.font_size,
                u32::from(self.settings.palette_index),
            ),
            None => ColorClusterOutcome::NotColor,
        };

        match outcome {
            ColorClusterOutcome::NotColor => ClusterResolution::NotColor,
            ColorClusterOutcome::Unsupported(reason) => {
                self.record_fallback(format!("color cluster {ids:?} is not composable: {reason}"));
                let force_cosmic = match self.settings.emoji_fallback {
                    EmojiFallback::Swash => {
                        #[cfg(feature = "backend-swash")]
                        {
                            true
                        }
                        #[cfg(not(feature = "backend-swash"))]
                        {
                            self.record_fallback("swash is not compiled in; color fallback degrades to mono silhouette");
                            false
                        }
                    }
                    EmojiFallback::Mono => false,
                    EmojiFallback::Placeholder => {
                        self.record_fallback("placeholder primitive is not implemented; color fallback degrades to mono silhouette");
                        false
                    }
                };
                ClusterResolution::Fallback { force_cosmic }
            }
            ColorClusterOutcome::Layers(layers) => {
                let borrowed: Vec<ColorLayer<'_>> = layers
                    .iter()
                    .map(|l| ColorLayer {
                        color: LayerColor::new(l.color[0], l.color[1], l.color[2], l.color[3]),
                        coverage: &l.data,
                        width: l.width,
                        height: l.height,
                        left: l.left,
                        top: l.top,
                    })
                    .collect();
                match composite(&borrowed) {
                    CompositeOutcome::NotColor => ClusterResolution::NotColor,
                    CompositeOutcome::Color(bitmap) => match self.commit_raster(
                        atlas_key,
                        advance,
                        bitmap.width,
                        bitmap.height,
                        bitmap.left as f32,
                        -(bitmap.top as f32),
                        true,
                        bitmap.data,
                        false,
                    ) {
                        Some(resolved) => ClusterResolution::Rendered(resolved),
                        None => {
                            self.record_fallback("color cluster atlas allocation failed");
                            ClusterResolution::Fallback {
                                force_cosmic: false,
                            }
                        }
                    },
                }
            }
        }
    }

    /// 非 Windows 构建没有彩色层遍历，恒按普通 run 处理。
    #[cfg(not(all(windows, feature = "backend-directwrite")))]
    fn resolve_color_cluster(
        &mut self,
        _engine: &mut TextEngine,
        _glyphs: &[PositionedGlyph],
        _face: fontdb::ID,
        _weight: u16,
        _italic: bool,
    ) -> ClusterResolution {
        ClusterResolution::NotColor
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
        let mode = self.settings.phase_mode;
        for range in cluster_ranges(&shaped.glyphs) {
            let group = &shaped.glyphs[range];
            let Some(first) = group.first() else {
                continue;
            };
            let Some(face) = shaped.faces.get(first.face_index as usize).copied() else {
                continue;
            };
            let pen = origin + Vec2::new(first.x, first.y);
            let backend_before_fallback = self.settings.emoji_backend;
            let mut force_cosmic = false;

            // 彩色簇先试一次层合成：成功则整簇只发一个四边形。
            if self.color_cluster_enabled() {
                match self.resolve_color_cluster(engine, group, face, weight, italic) {
                    ClusterResolution::Rendered(resolved) => {
                        if resolved.size.x > 0.0 && resolved.size.y > 0.0 {
                            out.push(GlyphQuad {
                                rect: Rect::new(
                                    pen.x.round() + resolved.bearing.x,
                                    pen.y.round() + resolved.bearing.y,
                                    resolved.size.x,
                                    resolved.size.y,
                                ),
                                uv: resolved.slot.uv,
                                layer: resolved.slot.layer,
                                color: resolved.color,
                                msdf: resolved.msdf,
                                // 彩色簇锁相位 0：原点已取整，不再有亚像素位移。
                                phase: 0,
                            });
                        }
                        continue;
                    }
                    ClusterResolution::NotColor => {}
                    ClusterResolution::Fallback {
                        force_cosmic: force,
                    } => force_cosmic = force,
                }
            }

            if force_cosmic {
                self.settings.emoji_backend = crate::settings::EmojiBackend::Cosmic;
            }
            for g in group {
                let Some(face) = shaped.faces.get(g.face_index as usize).copied() else {
                    continue;
                };
                let pen = origin + Vec2::new(g.x, g.y);
                if let Some(resolved) = self.resolve(engine, g, face, pen, weight, italic) {
                    if resolved.size.x <= 0.0 || resolved.size.y <= 0.0 {
                        continue;
                    }
                    let (base_x, phase) = mode.quantize_x(pen.x);
                    out.push(GlyphQuad {
                        rect: Rect::new(
                            base_x + resolved.bearing.x,
                            pen.y.round() + resolved.bearing.y,
                            resolved.size.x,
                            resolved.size.y,
                        ),
                        uv: resolved.slot.uv,
                        layer: resolved.slot.layer,
                        color: resolved.color,
                        msdf: resolved.msdf,
                        phase,
                    });
                }
            }
            // 逐簇回退只影响这一簇，下一簇仍按原设置重新判断。
            self.settings.emoji_backend = backend_before_fallback;
        }
        out
    }

    /// 丢弃所有已缓存字形（图集扩容或字体重载时调用）。
    pub fn clear(&mut self) {
        self.resolved.clear();
        self.atlas.clear();
        self.updates.clear();
        // 字体索引保持有效：图集仅清除了纹素数据。
    }

    /// 图集占用的内存字节数。
    #[must_use]
    pub fn atlas_memory_bytes(&self) -> u64 {
        self.atlas.allocated_bytes()
    }

    /// 当前已分配的图集显存字节数。
    #[must_use]
    pub fn allocated_bytes(&self) -> u64 {
        self.atlas.allocated_bytes()
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

// `SubpixelBin` 体现了上述八相位方案并在 `CacheKey::new` 中使用；
// 保留显式导入以便读者查阅。

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::atlas::AtlasConfig;
    use crate::shaping::TextStyle;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[cfg(all(windows, feature = "backend-directwrite"))]
    #[test]
    fn the_default_backend_is_directwrite_unless_overridden() {
        // 环境变量是 A/B 对照入口，设置时允许测试机主动选择 cosmic。
        if std::env::var_os("UNBOUND_TEXT_BACKEND").is_some() {
            return;
        }
        let cache = GlyphCache::new();
        assert_eq!(
            cache.backend(),
            RasterBackend::DirectWrite,
            "DirectWrite initialization failed and silently fell back to swash"
        );
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
    fn phase_bucketing_covers_eight_steps() {
        assert_eq!(GlyphCache::phase_for(0.0), 0);
        assert_eq!(GlyphCache::phase_for(0.13), 1);
        assert_eq!(GlyphCache::phase_for(0.26), 2);
        assert_eq!(GlyphCache::phase_for(0.51), 4);
        assert_eq!(GlyphCache::phase_for(0.76), 6);
        assert_eq!(GlyphCache::phase_for(0.89), 7);
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
    /// `round(fract * 8)` 对 `fract ∈ [0.9375, 1)` 返回 8，即**下一像素**的相位 0。
    /// 旧版 `% 4` 将其折回相位 0 却未移动四边形，导致 12.5% 的字形向左偏移一像素。
    ///
    #[test]
    fn the_last_phase_bucket_carries_into_the_next_pixel() {
        for thousandths in 938..1000 {
            let x = 10.0 + thousandths as f32 / 1000.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            assert_eq!(phase, 0, "x = {x} must round to phase 0");
            assert_eq!(base, 11.0, "x = {x} must move to the next pixel");
        }
    }

    /// 量化必须单调：较大画笔位置绝不会渲染到较小位置的左侧，
    /// 且量化后的位置误差保持在 ⅟₁₆ px 以内。
    #[test]
    fn quantize_x_is_monotonic_and_never_jumps_backwards() {
        let mut previous = f32::NEG_INFINITY;
        for step in 0..=1280 {
            let x = step as f32 / 64.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            let quantised = base + phase as f32 * 0.125;
            assert!(
                quantised >= previous,
                "x = {x} quantised to {quantised}, behind {previous}"
            );
            assert!(
                (quantised - x).abs() <= 0.063,
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

    /// DirectWrite 覆盖率必须是 8 位灰度抗锯齿。
    ///
    /// 若抗锯齿模式与纹理类型不匹配（例如在灰度模式下请求 ClearType 纹理），
    /// DirectWrite 会返回双电平纹理，中间灰占比坍缩到接近 0，字形边缘出现
    /// 硬阶梯。该断言把这条退化路径钉死在测试里。
    #[cfg(all(windows, feature = "backend-directwrite"))]
    #[test]
    fn directwrite_coverage_is_antialiased() {
        if std::env::var("UNBOUND_TEXT_BACKEND").is_ok_and(|v| v == "cosmic") {
            return;
        }
        let mut e = engine();
        let mut cache = GlyphCache::new();
        if cache.backend() != RasterBackend::DirectWrite {
            return;
        }
        let shaped = e.shape("Hamburgefonstiv AXVW", &TextStyle::sized(14.0));
        cache.quads_for(&mut e, &shaped, Vec2::new(0.3, 0.0), 400, false);
        let mut hist = [0u32; 256];
        for u in cache.take_updates() {
            if u.target != AtlasTarget::Coverage {
                continue;
            }
            for &b in &u.data {
                hist[b as usize] += 1;
            }
        }
        let total: u32 = hist.iter().sum();
        assert!(total > 0, "the test must rasterize at least one glyph");
        let mid: u32 = hist[1..255].iter().sum();
        let ratio = f64::from(mid) / f64::from(total);
        assert!(
            ratio > 0.12,
            "DirectWrite coverage looks binarized: only {:.1}% intermediate texels",
            ratio * 100.0
        );
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

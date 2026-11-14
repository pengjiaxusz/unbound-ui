//! 双格式分页字形图集，支持按帧代际 LRU 淘汰
//! （`docs` §3.5）。
//!
//! 两个独立的 `Texture2DArray`：
//!
//! * `Atlas_R8` —— 每像素 1 字节，每个字形包含 4 个水平亚像素相位。
//!   标准 UI 文本仅需覆盖率，为其分配每纹素 4 字节会使图集体积膨胀四倍
//!   却无任何视觉收益。
//! * `Atlas_RGBA8` —— 彩色 Emoji 与多通道距离场，必须使用四通道存储。
//!
//!
//! 页内采用架子（shelf）分配策略并维护空闲矩形列表，使被淘汰的字形留下
//! 可复用的空洞而非永久碎片。淘汰依据**帧代际**而非挂钟时间：上一帧刚用过的
//! 字形绝不会比一千帧未用的字形先被淘汰，无论那些帧实际耗时多久。
//!
//!

use rustc_hash::FxHashMap;

/// 标识单个已光栅化的字形变体。
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    /// 字体内的字形 ID。
    pub glyph_id: u16,
    /// 水平亚像素相位（0..3）；非亚像素图集为 0。
    pub phase: u8,
    /// 1 表示距离场，0 表示普通覆盖率位图。
    pub sdf: u8,
    /// 舍入到 ¼ px 的字号，使 0.1px 的动画步进不会引发缓存抖动。
    pub size_q4: u16,
    /// 字形所属的字体（face）。
    pub face: u32,
    /// 光栅化该字形时使用的字重。
    ///
    /// 光栅化结果取决于字重，因此必须纳入键中：同一字形的两种字重对应两张不同图像。
    ///
    pub weight: u16,
    /// 当字形使用合成斜体倾斜光栅化时为 1。
    ///
    /// 即使字形 ID 与字体相同，倾斜后的光栅结果也是与正体不同的图像。
    ///
    pub italic: u8,
}

impl GlyphKey {
    #[must_use]
    pub fn new(glyph_id: u16, size: f32, phase: u8, sdf: bool, face: u64) -> Self {
        Self::styled(glyph_id, size, phase, sdf, face, 400, false)
    }

    /// 完整缓存键，包含字重与斜体倾斜标记。
    #[must_use]
    pub fn styled(
        glyph_id: u16,
        size: f32,
        phase: u8,
        sdf: bool,
        face: u64,
        weight: u16,
        italic: bool,
    ) -> Self {
        Self {
            glyph_id,
            phase: phase & 3,
            sdf: u8::from(sdf),
            size_q4: (size.max(0.0) * 4.0).round().clamp(0.0, 65535.0) as u16,
            face: (face & 0xffff_ffff) as u32,
            weight,
            italic: u8::from(italic),
        }
    }
}

/// 字形在图集中的驻留位置。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GlyphSlot {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// 归一化纹理坐标 `[u0, v0, u1, v1]`。
    pub uv: [f32; 4],
    /// 最近一次请求该槽位的帧号。
    pub last_used: u64,
}

impl GlyphSlot {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }
}

/// 图集尺寸与容量配置。
#[derive(Copy, Clone, Debug)]
pub struct AtlasConfig {
    pub page_size: u32,
    pub max_layers: u32,
    /// 字形间的透明边距，防止双线性采样将相邻字形混入当前字形。
    ///
    pub padding: u32,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            page_size: 2048,
            max_layers: 8,
            padding: 2,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    /// 当前架子上可分配的下一个 x 坐标。
    cursor: u32,
}

#[derive(Copy, Clone, Debug)]
struct FreeRect {
    layer: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Debug, Default)]
struct Page {
    shelves: Vec<Shelf>,
    /// 可新建架子的下一个 y 坐标（位于最后一个架子下方）。
    next_y: u32,
}

/// 单一格式的图集：`max_layers` 页，每页 `page_size²` 纹素。
#[derive(Debug)]
pub struct Atlas {
    config: AtlasConfig,
    pages: Vec<Page>,
    slots: FxHashMap<GlyphKey, GlyphSlot>,
    free: Vec<FreeRect>,
    clock: u64,
    evictions: u64,
    /// 当前已分配的纹素存储字节数（页数 × page_size² × bpp）。
    pub bytes_per_texel: u32,
}

impl Atlas {
    #[must_use]
    pub fn new(config: AtlasConfig, bytes_per_texel: u32) -> Self {
        Self {
            config,
            pages: Vec::new(),
            slots: FxHashMap::default(),
            free: Vec::new(),
            clock: 0,
            evictions: 0,
            bytes_per_texel,
        }
    }

    /// 推进帧时钟并将指定槽位标记为已使用。
    pub fn begin_frame(&mut self, frame: u64) {
        self.clock = frame;
    }

    #[must_use]
    pub fn page_size(&self) -> u32 {
        self.config.page_size
    }

    #[must_use]
    pub fn layer_count(&self) -> u32 {
        self.pages.len() as u32
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// 迄今已预留的纹素内存字节数。
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        u64::from(self.pages.len() as u32)
            * u64::from(self.config.page_size)
            * u64::from(self.config.page_size)
            * u64::from(self.bytes_per_texel)
    }

    /// 查找已存在的槽位并刷新其帧代际。
    pub fn touch(&mut self, key: &GlyphKey) -> Option<GlyphSlot> {
        let clock = self.clock;
        let slot = self.slots.get_mut(key)?;
        slot.last_used = clock;
        Some(*slot)
    }

    /// 为 `key` 预留 `w × h` 空间，空间不足时淘汰最冷槽位。
    ///
    /// 仅当请求尺寸无法放入单页时返回 `None`。
    ///
    /// 被淘汰字形的键会追加到 `evicted`，以便调用方清理自身缓存条目。
    ///
    pub fn allocate(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        if w == 0 || h == 0 || w > self.config.page_size || h > self.config.page_size {
            return None;
        }
        if let Some(existing) = self.touch(&key) {
            return Some(existing);
        }
        let pad = self.config.padding;
        let need_w = w + pad;
        let need_h = h + pad;

        // 首先尝试在不淘汰任何条目的情况下分配。
        if let Some(slot) = self.try_place(need_w, need_h, w, h) {
            self.slots.insert(key, slot);
            return Some(slot);
        }

        // 空间不足：释放足够多的最冷槽位以容纳当前请求。当新字形比最冷常驻字形
        // 更宽时，单次淘汰并不够——48 px 的字形无法塞进句号的 8 px 矩形中——
        // 若此时直接放弃，会导致该字形从本帧中悄然丢失。
        //
        //
        //
        // 批量淘汰具有事务性：仅当放置成功后才向调用方报告淘汰列表，
        // 若分配最终失败，则将所有槽位（及所有空闲矩形）原样回滚。
        //
        const MAX_EVICTIONS_PER_ALLOCATION: usize = 16;
        let free_before = self.free.clone();
        let mut batch: Vec<(GlyphKey, GlyphSlot)> = Vec::new();
        let placement = loop {
            if let Some(slot) = self.try_place(need_w, need_h, w, h) {
                break Some(slot);
            }
            if batch.len() >= MAX_EVICTIONS_PER_ALLOCATION {
                break None;
            }
            match self.evict_coldest() {
                Some(entry) => {
                    batch.push(entry);
                    // 相邻空洞只有合并后才有用：16 个独立的 8 px 矩形依然放不下 48 px 字形。
                    //
                    self.coalesce_free_rects();
                }
                None => break None,
            }
        };
        if let Some(slot) = placement {
            self.slots.insert(key, slot);
            evicted.extend(batch.into_iter().map(|(key, _)| key));
            return Some(slot);
        }
        // 最后手段：清空最冷的一整页。请求已确认能放入单页，
        // 因此空白页必定能够接纳它。
        if need_w <= self.config.page_size
            && need_h <= self.config.page_size
            && self.reset_coldest_page(&mut batch)
        {
            if let Some(slot) = self.try_place(need_w, need_h, w, h) {
                self.slots.insert(key, slot);
                evicted.extend(batch.into_iter().map(|(key, _)| key));
                return Some(slot);
            }
            debug_assert!(
                false,
                "an empty page must fit a request that fits in a page"
            );
            return None;
        }
        let evicted_count = batch.len() as u64;
        self.free = free_before;
        for (key, slot) in batch {
            self.slots.insert(key, slot);
        }
        self.evictions = self.evictions.saturating_sub(evicted_count);
        None
    }

    /// 合并共享同一条边的空闲矩形。
    ///
    /// 淘汰会按字形逐个归还矩形，释放一连串相邻字形后往往会留下一排大字形
    /// 无法使用的小空洞。将它们合并（水平方向同行等高，垂直方向同列等宽）
    /// 可将这些碎片重新整理为可用空间。
    ///
    fn coalesce_free_rects(&mut self) {
        let mut merged = true;
        while merged {
            merged = false;
            'restart: for i in 0..self.free.len() {
                for j in (i + 1)..self.free.len() {
                    let a = self.free[i];
                    let b = self.free[j];
                    if a.layer != b.layer {
                        continue;
                    }
                    if a.y == b.y && a.h == b.h && (a.x + a.w == b.x || b.x + b.w == a.x) {
                        self.free[i] = FreeRect {
                            layer: a.layer,
                            x: a.x.min(b.x),
                            y: a.y,
                            w: a.w + b.w,
                            h: a.h,
                        };
                        self.free.swap_remove(j);
                        merged = true;
                        break 'restart;
                    }
                    if a.x == b.x && a.w == b.w && (a.y + a.h == b.y || b.y + b.h == a.y) {
                        self.free[i] = FreeRect {
                            layer: a.layer,
                            x: a.x,
                            y: a.y.min(b.y),
                            w: a.w,
                            h: a.h + b.h,
                        };
                        self.free.swap_remove(j);
                        merged = true;
                        break 'restart;
                    }
                }
            }
        }
    }

    /// 淘汰最近最少使用（LRU）页上的所有字形并将该页置空。
    ///
    /// 仅当任意有界次数的单条目淘汰都无法满足请求（即图集确实已满）时触发。
    /// 若没有可重置的页则返回 `false`。
    ///
    fn reset_coldest_page(&mut self, batch: &mut Vec<(GlyphKey, GlyphSlot)>) -> bool {
        let mut best: Option<(u64, usize)> = None;
        for (index, page) in self.pages.iter().enumerate() {
            if page.shelves.is_empty() {
                continue;
            }
            let Some(oldest) = self
                .slots
                .values()
                .filter(|slot| slot.layer as usize == index)
                .map(|slot| slot.last_used)
                .min()
            else {
                continue;
            };
            if best.is_none_or(|(stamp, _)| oldest < stamp) {
                best = Some((oldest, index));
            }
        }
        let Some((_, layer)) = best else {
            return false;
        };
        let keys: Vec<GlyphKey> = self
            .slots
            .iter()
            .filter(|(_, slot)| slot.layer as usize == layer)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            if let Some(slot) = self.slots.remove(&key) {
                batch.push((key, slot));
                self.evictions += 1;
            }
        }
        self.pages[layer].shelves.clear();
        self.pages[layer].next_y = 0;
        self.free.retain(|rect| rect.layer as usize != layer);
        true
    }

    fn try_place(&mut self, need_w: u32, need_h: u32, w: u32, h: u32) -> Option<GlyphSlot> {
        let page_size = self.config.page_size;

        // 1. 从回收的空闲矩形中执行最佳适配（Best-fit）。
        let mut best: Option<usize> = None;
        for (i, r) in self.free.iter().enumerate() {
            if r.w >= need_w && r.h >= need_h {
                let better = best
                    .map(|b| {
                        let cur = self.free[b].w * self.free[b].h;
                        r.w * r.h < cur
                    })
                    .unwrap_or(true);
                if better {
                    best = Some(i);
                }
            }
        }
        if let Some(i) = best {
            let r = self.free.swap_remove(i);
            if r.w > need_w {
                self.free.push(FreeRect {
                    layer: r.layer,
                    x: r.x + need_w,
                    y: r.y,
                    w: r.w - need_w,
                    h: need_h,
                });
            }
            if r.h > need_h {
                self.free.push(FreeRect {
                    layer: r.layer,
                    x: r.x,
                    y: r.y + need_h,
                    w: r.w,
                    h: r.h - need_h,
                });
            }
            return Some(self.make_slot(r.layer, r.x, r.y, w, h));
        }

        // 2. 现有架子（首次适配）。先按索引找到目标，使 `self.pages` 的可变借用
        //    只发生一次。
        let mut hit: Option<(usize, usize, u32, u32)> = None;
        'outer: for (layer, page) in self.pages.iter().enumerate() {
            for (si, shelf) in page.shelves.iter().enumerate() {
                if shelf.height >= need_h && shelf.cursor + need_w <= page_size {
                    hit = Some((layer, si, shelf.cursor, shelf.y));
                    break 'outer;
                }
            }
        }
        if let Some((layer, si, x, y)) = hit {
            self.pages[layer].shelves[si].cursor = x + need_w;
            return Some(self.make_slot(layer as u32, x, y, w, h));
        }

        // 3. 在现有页底部新建一个架子。
        let mut new_shelf: Option<(usize, u32)> = None;
        for (layer, page) in self.pages.iter().enumerate() {
            if page.next_y + need_h <= page_size {
                new_shelf = Some((layer, page.next_y));
                break;
            }
        }
        if let Some((layer, y)) = new_shelf {
            let page = &mut self.pages[layer];
            page.shelves.push(Shelf {
                y,
                height: need_h,
                cursor: need_w,
            });
            page.next_y = y + need_h;
            return Some(self.make_slot(layer as u32, 0, y, w, h));
        }

        // 4. 分配全新的纹理层。
        if (self.pages.len() as u32) < self.config.max_layers {
            let mut page = Page::default();
            page.shelves.push(Shelf {
                y: 0,
                height: need_h,
                cursor: need_w,
            });
            page.next_y = need_h;
            self.pages.push(page);
            let layer = self.pages.len() as u32 - 1;
            return Some(self.make_slot(layer, 0, 0, w, h));
        }

        None
    }

    fn make_slot(&mut self, layer: u32, x: u32, y: u32, w: u32, h: u32) -> GlyphSlot {
        let sz = self.config.page_size as f32;
        GlyphSlot {
            layer,
            x,
            y,
            w,
            h,
            uv: [
                x as f32 / sz,
                y as f32 / sz,
                (x + w) as f32 / sz,
                (y + h) as f32 / sz,
            ],
            last_used: self.clock,
        }
    }

    /// 淘汰最近最少使用的槽位，并将其矩形归还空闲池。
    ///
    /// 同时返回被淘汰的键与槽位，以便调用方上报淘汰记录并在后续分配仍失败时
    /// 放回该字形：矩形归还空闲池后会立即分配给下一个字形，因此陈旧缓存项
    /// 会指向别人的纹素数据。
    ///
    ///
    fn evict_coldest(&mut self) -> Option<(GlyphKey, GlyphSlot)> {
        let (&key, &slot) = self
            .slots
            .iter()
            .min_by_key(|(_, s)| (s.last_used, s.layer, s.x, s.y))?;
        self.slots.remove(&key);
        let pad = self.config.padding;
        self.free.push(FreeRect {
            layer: slot.layer,
            x: slot.x,
            y: slot.y,
            w: slot.w + pad,
            h: slot.h + pad,
        });
        self.evictions += 1;
        Some((key, slot))
    }

    /// 丢弃指定字形（例如字体已被卸载）。
    pub fn remove(&mut self, key: &GlyphKey) -> Option<GlyphSlot> {
        let slot = self.slots.remove(key)?;
        let pad = self.config.padding;
        self.free.push(FreeRect {
            layer: slot.layer,
            x: slot.x,
            y: slot.y,
            w: slot.w + pad,
            h: slot.h + pad,
        });
        Some(slot)
    }

    /// 已分配纹理层的占用率，范围 `0..=1`。
    #[must_use]
    pub fn occupancy(&self) -> f32 {
        if self.pages.is_empty() {
            return 0.0;
        }
        let used: u64 = self.slots.values().map(|s| u64::from(s.w * s.h)).sum();
        let total = self.pages.len() as u64
            * u64::from(self.config.page_size)
            * u64::from(self.config.page_size);
        used as f32 / total as f32
    }
}

/// 渲染器绑定的一对图集（`Atlas_R8` + `Atlas_RGBA8`）。
#[derive(Debug)]
pub struct GlyphAtlas {
    /// 每个字形 4 个亚像素相位，每纹素 1 字节。
    pub r8: Atlas,
    /// 彩色 Emoji 与距离场，每纹素 4 字节。
    pub rgba: Atlas,
    /// 渲染器每帧递增的代际计数器。
    pub frame: u64,
}

/// R8 图集存储的水平亚像素相位数。
pub const SUBPIXEL_PHASES: u8 = 4;

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(AtlasConfig::default())
    }
}

impl GlyphAtlas {
    #[must_use]
    pub fn new(config: AtlasConfig) -> Self {
        Self {
            r8: Atlas::new(config, 1),
            rgba: Atlas::new(config, 4),
            frame: 0,
        }
    }

    /// 开始新一帧；同时刷新淘汰时钟。
    pub fn begin_frame(&mut self, frame: u64) {
        self.frame = frame;
        self.r8.begin_frame(frame);
        self.rgba.begin_frame(frame);
    }

    /// 为指定亚像素相位下的字形分配覆盖率存储空间。
    ///
    /// 被淘汰字形的键会追加到 `evicted`，以便调用方清理自身缓存条目。
    ///
    pub fn allocate_coverage(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        self.r8.allocate(key, w, h, evicted)
    }

    /// 分配 RGBA 存储空间（Emoji 或距离场）。
    ///
    /// 键中携带相位、字重与斜体倾斜标记，因为不同相位（或字重、倾斜度）
    /// 生成的光栅结果是完全不同的图像。遗漏其中任意一项都会导致两个变体
    /// 共享同一图集矩形，第二次上传将覆盖第一次的纹素。
    ///
    ///
    ///
    /// 被淘汰字形的键会追加到 `evicted`，以便调用方清理自身缓存条目。
    ///
    pub fn allocate_color(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        self.rgba.allocate(key, w, h, evicted)
    }

    /// 两个图集占用的总纹素内存。
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.r8.memory_bytes() + self.rgba.memory_bytes()
    }

    /// 渲染器必须绑定的纹理层数。
    #[must_use]
    pub fn layer_count(&self) -> u32 {
        self.r8.layer_count().max(self.rgba.layer_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> AtlasConfig {
        AtlasConfig {
            page_size: 256,
            max_layers: 2,
            padding: 1,
        }
    }

    #[test]
    fn allocation_returns_a_slot_inside_the_page() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(42, 16.0, 0, false, 1);
        let slot = a.allocate(key, 20, 24, &mut evicted).expect("slot");
        assert_eq!(slot.layer, 0);
        assert!(slot.x + slot.w <= 256 && slot.y + slot.h <= 256);
        assert_eq!(slot.w, 20);
        assert_eq!(slot.uv[0], 0.0);
        assert!((slot.uv[2] - 20.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn same_key_returns_the_same_slot_and_refreshes_it() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(7, 16.0, 2, false, 1);
        a.begin_frame(1);
        let first = a.allocate(key, 10, 10, &mut evicted).unwrap();
        a.begin_frame(50);
        let again = a.allocate(key, 10, 10, &mut evicted).unwrap();
        assert_eq!(
            (first.layer, first.x, first.y),
            (again.layer, again.x, again.y)
        );
        assert_eq!(again.last_used, 50);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn slots_never_overlap() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let mut rects = Vec::new();
        for i in 0..60u16 {
            let key = GlyphKey::new(i, 12.0, (i % 4) as u8, false, 1);
            if let Some(s) = a.allocate(key, 18, 22, &mut evicted) {
                for (l, x, y, w, h) in &rects {
                    let overlap = *l == s.layer
                        && s.x < x + w
                        && x < &(s.x + s.w)
                        && s.y < y + h
                        && y < &(s.y + s.h);
                    assert!(!overlap, "glyph {i} overlaps a previous slot");
                }
                rects.push((s.layer, s.x, s.y, s.w, s.h));
            }
        }
        assert!(rects.len() > 20, "only {} glyphs fit", rects.len());
    }

    #[test]
    fn eviction_recycles_the_coldest_glyph() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        // 用 16x16 字形填满单个 64x64 页。
        a.begin_frame(1);
        let mut keys = Vec::new();
        for i in 0..16u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(
                a.allocate(key, 16, 16, &mut evicted).is_some(),
                "glyph {i} should fit"
            );
            keys.push(key);
        }
        // 访问除字形 0 以外的所有字形，使字形 0 成为最冷条目。
        a.begin_frame(2);
        for key in keys.iter().skip(1) {
            assert!(a.allocate(*key, 16, 16, &mut evicted).is_some());
        }
        a.begin_frame(3);
        let fresh = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(
            a.allocate(fresh, 16, 16, &mut evicted).is_some(),
            "must evict to fit"
        );
        assert_eq!(a.evictions(), 1);
        assert!(
            a.touch(&keys[0]).is_none(),
            "glyph 0 must have been evicted"
        );
        assert!(a.touch(&keys[1]).is_some(), "glyph 1 was hot, keep it");
    }

    #[test]
    fn oversized_glyph_is_rejected_not_corrupted() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(1, 16.0, 0, false, 1);
        assert!(a.allocate(key, 512, 10, &mut evicted).is_none());
        assert!(a.allocate(key, 0, 10, &mut evicted).is_none());
        assert_eq!(a.len(), 0);
    }

    #[test]
    fn multiple_layers_are_used_before_evicting() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        for i in 0..40u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 40, 40, &mut evicted).is_some(), "glyph {i}");
        }
        assert!(a.layer_count() >= 2, "should spill into a second layer");
        assert_eq!(a.evictions(), 0);
    }

    #[test]
    fn phase_and_face_are_part_of_the_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let base = GlyphKey::new(5, 16.0, 0, false, 1);
        let phase = GlyphKey::new(5, 16.0, 2, false, 1);
        let face = GlyphKey::new(5, 16.0, 0, false, 2);
        let s0 = a.allocate(base, 8, 8, &mut evicted).unwrap();
        let s1 = a.allocate(phase, 8, 8, &mut evicted).unwrap();
        let s2 = a.allocate(face, 8, 8, &mut evicted).unwrap();
        assert_ne!((s0.x, s0.y), (s1.x, s1.y));
        assert_ne!((s0.x, s0.y), (s2.x, s2.y));
    }

    /// 字重和斜体倾斜会改变光栅结果，因此必须作为键的一部分。
    ///
    ///
    /// `FAKE_ITALIC` 会使 swash 从同一字体 ID 生成**倾斜**的光栅结果，
    /// 因此同一字形的斜体与正体在键的其余部分完全相同。若键中不含倾斜标记，
    /// 两者将共享同一个图集矩形，后者会覆盖前者。
    ///
    #[test]
    fn weight_and_italic_are_part_of_the_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let upright = GlyphKey::styled(5, 16.0, 0, false, 1, 400, false);
        let italic = GlyphKey::styled(5, 16.0, 0, false, 1, 400, true);
        let bold = GlyphKey::styled(5, 16.0, 0, false, 1, 700, false);
        let s0 = a.allocate(upright, 8, 8, &mut evicted).unwrap();
        let s1 = a.allocate(italic, 8, 8, &mut evicted).unwrap();
        let s2 = a.allocate(bold, 8, 8, &mut evicted).unwrap();
        assert_ne!((s0.x, s0.y), (s1.x, s1.y), "italic must not share a slot");
        assert_ne!((s0.x, s0.y), (s2.x, s2.y), "bold must not share a slot");
    }

    /// 彩色图集也必须将亚像素相位纳入键中。
    ///
    /// 在不同亚像素相位下光栅化的距离场（或彩色 Emoji）是不同的图像。
    /// 遗漏相位曾导致两个相位共享同一矩形，第二次上传覆盖了第一次的纹素。
    ///
    ///
    #[test]
    fn the_colour_atlas_keys_on_the_phase() {
        let mut evicted = Vec::new();
        let mut atlas = GlyphAtlas::new(small());
        atlas.begin_frame(1);
        let p0 = GlyphKey::styled(5, 16.0, 0, true, 1, 400, false);
        let p1 = GlyphKey::styled(5, 16.0, 2, true, 1, 400, false);
        let s0 = atlas.allocate_color(p0, 12, 12, &mut evicted).unwrap();
        let s1 = atlas.allocate_color(p1, 12, 12, &mut evicted).unwrap();
        assert_ne!(
            (s0.x, s0.y),
            (s1.x, s1.y),
            "two phases must not share an atlas rectangle"
        );
    }

    /// 被淘汰的字形必须上报，以便调用方清理缓存条目：矩形归还空闲池后
    /// 会被分配给下一个字形。
    ///
    #[test]
    fn eviction_reports_the_evicted_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        a.begin_frame(1);
        let mut keys = Vec::new();
        for i in 0..16u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 16, 16, &mut evicted).is_some());
            keys.push(key);
        }
        a.begin_frame(3);
        let fresh = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(
            a.allocate(fresh, 16, 16, &mut evicted).is_some(),
            "must evict"
        );
        assert_eq!(a.evictions(), 1);
        assert_eq!(evicted.len(), 1, "the evicted key must be reported");
        assert_eq!(evicted[0], keys[0], "the coldest glyph is evicted");
    }

    /// 失败的分配绝不能破坏已存活的字形。
    ///
    /// 为腾出空间而执行的淘汰具有破坏性：若重试依然失败，一个无关且完好的
    /// 字形就会白白丢失。
    #[test]
    fn a_failed_allocation_leaves_the_atlas_untouched() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 1,
            },
            1,
        );
        a.begin_frame(1);
        // 16 px 字形加上 1 px 边距：每条 64 px 架子可放 3 个，每页可放 3 条架子，
        // 因此 9 个字形恰好填满整页。
        for i in 0..9u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 16, 16, &mut evicted).is_some());
        }
        let before = a.len();
        assert_eq!(before, 9);
        // 64 px 加上 1 px 边距比整页更宽，因此该请求永远无法满足：
        // 途中尝试的所有淘汰都必须全部回滚。
        //
        let impossible = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(a.allocate(impossible, 64, 16, &mut evicted).is_none());
        assert_eq!(
            a.len(),
            before,
            "the failed allocation must not destroy a live glyph"
        );
        assert!(evicted.is_empty(), "nothing was really evicted");
        assert_eq!(a.evictions(), 0);
    }

    /// 批量淘汰使大于所有冷槽位的新字形也能成功分配。
    ///
    /// 单次淘汰仅释放最冷字形（通常是小标点符号）的矩形，重试仍会失败，
    /// 这曾导致新字形从该帧中完全丢失。
    ///
    #[test]
    fn a_glyph_larger_than_every_cold_slot_still_fits() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        a.begin_frame(1);
        for i in 0..64u16 {
            assert!(
                a.allocate(GlyphKey::new(i, 8.0, 0, false, 1), 8, 8, &mut evicted)
                    .is_some(),
                "glyph {i}"
            );
        }
        a.begin_frame(2);
        let large = GlyphKey::new(999, 48.0, 0, false, 1);
        let slot = a
            .allocate(large, 48, 48, &mut evicted)
            .expect("the atlas must free enough room for a large glyph");
        assert!(slot.x + 48 <= 64 && slot.y + 48 <= 64);
        assert!(evicted.len() > 1, "one 8 px slot cannot hold a 48 px glyph");
        assert!(a.touch(&large).is_some());
    }

    #[test]
    fn sub_quarter_pixel_size_changes_share_a_slot() {
        let _a = Atlas::new(small(), 1);
        let k1 = GlyphKey::new(5, 16.01, 0, false, 1);
        let k2 = GlyphKey::new(5, 16.10, 0, false, 1);
        assert_eq!(k1, k2, "sizes must bucket to ¼ px");
        let k3 = GlyphKey::new(5, 16.4, 0, false, 1);
        assert_ne!(k1, k3);
    }

    #[test]
    fn dual_atlas_tracks_both_formats() {
        let mut evicted = Vec::new();
        let mut atlas = GlyphAtlas::new(small());
        atlas.begin_frame(1);
        assert!(atlas
            .allocate_coverage(GlyphKey::new(1, 16.0, 0, false, 1), 12, 12, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_coverage(GlyphKey::new(1, 16.0, 1, false, 1), 12, 12, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_color(GlyphKey::new(2, 16.0, 0, false, 1), 16, 16, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_color(GlyphKey::new(3, 16.0, 0, true, 1), 32, 32, &mut evicted)
            .is_some());
        assert_eq!(atlas.r8.len(), 2);
        assert_eq!(atlas.rgba.len(), 2);
        assert!(atlas.memory_bytes() > 0);
        assert!(atlas.layer_count() >= 1);
    }

    #[test]
    fn removing_a_glyph_frees_its_rectangle() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(3, 16.0, 0, false, 1);
        let slot = a.allocate(key, 32, 32, &mut evicted).unwrap();
        assert!(a.remove(&key).is_some());
        assert_eq!(a.len(), 0);
        let next = a
            .allocate(GlyphKey::new(4, 16.0, 0, false, 1), 32, 32, &mut evicted)
            .unwrap();
        assert_eq!((next.layer, next.x, next.y), (slot.layer, slot.x, slot.y));
    }

    #[test]
    fn occupancy_grows_with_allocations() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        for i in 0..8u16 {
            a.allocate(GlyphKey::new(i, 16.0, 0, false, 1), 40, 40, &mut evicted);
        }
        let occ = a.occupancy();
        assert!(occ > 0.05 && occ < 0.9, "occupancy {occ}");
    }
}

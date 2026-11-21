//! 无限画布超高分辨率图像的多级瓦片金字塔（Tile Pyramid）纯 CPU LOD 调度器。
//!
//! 本模块保持纯 CPU、零平台与图形后端依赖，负责在有限显存槽位预算下完成以下调度计算：
//!
//! * 根据图像原始像素尺寸与逻辑瓦片边长建立多级金字塔几何（`level == 0` 为原始最高分辨率层，
//!   层级每增加 1，对应层级的图像宽高各自减半并向上取整）；
//! * 根据当前视口缩放比（`zoom`）选择目标 LOD 层级，并将视口世界矩形裁剪为可见瓦片集合与
//!   可选的预取环（`prefetch_ring`）；
//! * 为未驻留的可见瓦片向上回溯最近已驻留祖先瓦片，并计算精确的归一化 `sample_uv`
//!   （可选叠加 512+4px 等像素级 Halo 内缩映射，避免双线性采样跨瓦片边界渗色）；
//! * 按“可见区优先于预取环、近视口中心优先于边缘”输出加载队列（`load_queue`），
//!   并按 LRU 顺序输出冷瓦片淘汰队列（`evict_queue`），同时严格保护本帧绘制依赖的瓦片。

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::{FxHashMap, FxHashSet};
use unbound_math::Rect;

/// 默认逻辑瓦片边长（像素）。
pub const DEFAULT_TILE_SIZE: u32 = 512;

/// 默认单边 Halo 扩边像素数（512 + 2 * 4 = 520 px 物理纹理边长）。
pub const DEFAULT_TILE_HALO_PX: u32 = 4;

/// 瓦片金字塔中的唯一瓦片索引键。
///
/// `level == 0` 表示原始最高分辨率层；`level` 每增加 1，
/// 对应层级的图像宽高各自减半并向上取整。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// LOD 层级（`0` 为最高分辨率层）。
    pub level: u8,
    /// 该层级瓦片网格中的列索引（X 方向，从 `0` 开始）。
    pub x: u32,
    /// 该层级瓦片网格中的行索引（Y 方向，从 `0` 开始）。
    pub y: u32,
}

impl TileKey {
    /// 构造一个瓦片键。
    #[inline]
    #[must_use]
    pub const fn new(level: u8, x: u32, y: u32) -> Self {
        Self { level, x, y }
    }

    /// 返回上一级（更粗一级）父瓦片键；若 `level == u8::MAX` 则返回 `None`。
    #[inline]
    #[must_use]
    pub const fn parent(self) -> Option<Self> {
        match self.level.checked_add(1) {
            Some(next_level) => Some(Self {
                level: next_level,
                x: self.x / 2,
                y: self.y / 2,
            }),
            None => None,
        }
    }
}

/// 瓦片金字塔的静态几何规格。
///
/// 图像在画布世界坐标系中占据 `[0, image_width] × [0, image_height]`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TilePyramidSpec {
    /// 原始图像宽度（像素）。
    pub image_width: u32,
    /// 原始图像高度（像素）。
    pub image_height: u32,
    /// 单个逻辑瓦片的边长（像素，通常为 2 的幂次，如 `512`）。
    pub tile_size: u32,
}

impl TilePyramidSpec {
    /// 构造瓦片金字塔规格。
    #[inline]
    #[must_use]
    pub const fn new(image_width: u32, image_height: u32, tile_size: u32) -> Self {
        Self {
            image_width,
            image_height,
            tile_size,
        }
    }

    /// 使用默认瓦片边长（[`DEFAULT_TILE_SIZE`] = 512）构造金字塔规格。
    #[inline]
    #[must_use]
    pub const fn with_default_tile_size(image_width: u32, image_height: u32) -> Self {
        Self::new(image_width, image_height, DEFAULT_TILE_SIZE)
    }

    /// 判断金字塔规格是否有效（宽、高与瓦片边长均须大于零）。
    #[inline]
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.image_width > 0 && self.image_height > 0 && self.tile_size > 0
    }

    /// 返回整张图像在世界坐标系中的包围盒。
    #[inline]
    #[must_use]
    pub fn image_world_rect(&self) -> Rect {
        if !self.is_valid() {
            return Rect::ZERO;
        }
        Rect::new(0.0, 0.0, self.image_width as f32, self.image_height as f32)
    }

    /// 计算金字塔最大 LOD 层级（最粗层，满足整张图像落入单个 `1 × 1` 瓦片）。
    ///
    /// 若规格无效则返回 `0`。
    #[must_use]
    pub fn max_level(&self) -> u8 {
        if !self.is_valid() {
            return 0;
        }
        let mut level: u8 = 0;
        let mut w = self.image_width;
        let mut h = self.image_height;
        while w > self.tile_size || h > self.tile_size {
            w = w.div_ceil(2);
            h = h.div_ceil(2);
            level = level.saturating_add(1);
        }
        level
    }

    /// 计算指定 LOD 层级的像素宽高（每升一层边长减半向上取整）。
    ///
    /// 若规格无效则返回 `(0, 0)`。
    #[must_use]
    pub fn level_dimensions(&self, level: u8) -> (u32, u32) {
        if !self.is_valid() {
            return (0, 0);
        }
        (
            ceil_div_pow2(self.image_width, level),
            ceil_div_pow2(self.image_height, level),
        )
    }

    /// 计算指定 LOD 层级的瓦片网格行列数 `(cols, rows)`。
    ///
    /// 若规格无效则返回 `(0, 0)`。
    #[must_use]
    pub fn grid_size(&self, level: u8) -> (u32, u32) {
        if !self.is_valid() {
            return (0, 0);
        }
        let (lw, lh) = self.level_dimensions(level);
        (lw.div_ceil(self.tile_size), lh.div_ceil(self.tile_size))
    }

    /// 检查给定瓦片键是否落在本金字塔的合法层级与网格边界内。
    #[must_use]
    pub fn contains_key(&self, key: TileKey) -> bool {
        if !self.is_valid() || key.level > self.max_level() {
            return false;
        }
        let (cols, rows) = self.grid_size(key.level);
        key.x < cols && key.y < rows
    }

    /// 返回给定瓦片键在金字塔内的父级瓦片键；若已是最粗层或键非法则返回 `None`。
    #[must_use]
    pub fn parent_of(&self, key: TileKey) -> Option<TileKey> {
        if !self.contains_key(key) || key.level >= self.max_level() {
            return None;
        }
        key.parent()
    }

    /// 根据当前视口缩放比 `zoom` 选择最优目标 LOD 层级。
    ///
    /// * `zoom >= 1.0`（含 `+inf`）锁定最高分辨率层 `0`；
    /// * `zoom == 0.5` 映射至 `1`，`zoom == 0.25` 映射至 `2`，以此类推并钳制在 `max_level` 内；
    /// * `zoom <= 0.0` 或 `NaN` 等退化极值安全回退至 `max_level`，避免触发海量细层瓦片调度。
    #[must_use]
    pub fn select_level(&self, zoom: f32) -> u8 {
        if !self.is_valid() {
            return 0;
        }
        let max_level = self.max_level();
        if zoom.is_nan() || zoom <= 0.0 {
            return max_level;
        }
        if zoom >= 1.0 {
            return 0;
        }
        // 使用 f64 计算 -log2(zoom) 避免极小缩放比下的精度退化或溢出。
        let ideal = (-f64::from(zoom).log2()).floor();
        if !ideal.is_finite() || ideal <= 0.0 {
            return 0;
        }
        let clamped = ideal.min(f64::from(max_level));
        clamped as u8
    }

    /// 计算单个瓦片在世界坐标系中的实际覆盖矩形。
    ///
    /// 图像右边缘与下边缘的非整除瓦片会被精确裁剪至 `[0, image_width] × [0, image_height]`；
    /// 若规格无效或瓦片键越界，返回 [`Rect::ZERO`]。
    #[must_use]
    pub fn tile_world_rect(&self, key: TileKey) -> Rect {
        if !self.contains_key(key) {
            return Rect::ZERO;
        }
        let span = self.tile_world_span(key.level);
        let img_w = f64::from(self.image_width);
        let img_h = f64::from(self.image_height);

        let min_x = (f64::from(key.x) * span).min(img_w);
        let min_y = (f64::from(key.y) * span).min(img_h);
        let max_x = ((f64::from(key.x) + 1.0) * span).min(img_w);
        let max_y = ((f64::from(key.y) + 1.0) * span).min(img_h);

        Rect::new(
            min_x as f32,
            min_y as f32,
            (max_x - min_x).max(0.0) as f32,
            (max_y - min_y).max(0.0) as f32,
        )
    }

    /// 返回叠加单边 `halo_px` 像素扩边后的物理纹理边长（`tile_size + 2 * halo_px`）。
    #[inline]
    #[must_use]
    pub const fn physical_tile_size(&self, halo_px: u32) -> u32 {
        self.tile_size.saturating_add(halo_px.saturating_mul(2))
    }

    /// 将 `[0, 1] × [0, 1]` 逻辑瓦片 UV 子矩形映射到带单边 `halo_px` 扩边的物理纹理 UV 空间。
    ///
    /// 例如对 `512 px` 逻辑瓦片与 `4 px` Halo（物理边长 `520 px`），
    /// 逻辑矩形 `[0, 0, 1, 1]` 映射为 `[4/520, 4/520, 512/520, 512/520]`；
    /// 当 `halo_px == 0` 时返回恒等逻辑 UV。
    #[must_use]
    pub fn apply_halo_uv(&self, logical_uv: Rect, halo_px: u32) -> Rect {
        if !self.is_valid() || logical_uv.is_empty() || !logical_uv.is_finite() {
            return Rect::ZERO;
        }
        if halo_px == 0 {
            return logical_uv;
        }
        let tile = f64::from(self.tile_size);
        let halo = f64::from(halo_px);
        let padded = tile + halo * 2.0;

        let u0 = (halo + f64::from(logical_uv.origin.x) * tile) / padded;
        let v0 = (halo + f64::from(logical_uv.origin.y) * tile) / padded;
        let du = (f64::from(logical_uv.size.x) * tile) / padded;
        let dv = (f64::from(logical_uv.size.y) * tile) / padded;
        Rect::new(u0 as f32, v0 as f32, du as f32, dv as f32)
    }

    /// 计算目标瓦片 `target` 采样祖先瓦片 `ancestor` 时的归一化 UV 子矩形（可选叠加 `halo_px`）。
    ///
    /// 若任一瓦片键非法，或 `ancestor` 不是 `target` 自身或其合法祖先，返回 [`Rect::ZERO`]。
    #[must_use]
    pub fn fallback_sample_uv(&self, target: TileKey, ancestor: TileKey, halo_px: u32) -> Rect {
        if !self.contains_key(target) || !self.contains_key(ancestor) {
            return Rect::ZERO;
        }
        if ancestor.level < target.level {
            return Rect::ZERO;
        }
        let shift = ancestor.level - target.level;
        let expected_x = if shift >= 32 { 0 } else { target.x >> shift };
        let expected_y = if shift >= 32 { 0 } else { target.y >> shift };
        if ancestor.x != expected_x || ancestor.y != expected_y {
            return Rect::ZERO;
        }

        let logical_uv = if target == ancestor {
            Rect::new(0.0, 0.0, 1.0, 1.0)
        } else {
            let anc_rect = self.tile_world_rect(ancestor);
            let tgt_rect = self.tile_world_rect(target);
            let anc_w = f64::from(anc_rect.size.x);
            let anc_h = f64::from(anc_rect.size.y);
            if anc_w <= 0.0 || anc_h <= 0.0 {
                return Rect::ZERO;
            }
            let anc_x0 = f64::from(anc_rect.origin.x);
            let anc_y0 = f64::from(anc_rect.origin.y);
            let tgt_x0 = f64::from(tgt_rect.origin.x);
            let tgt_y0 = f64::from(tgt_rect.origin.y);
            let tgt_x1 = tgt_x0 + f64::from(tgt_rect.size.x);
            let tgt_y1 = tgt_y0 + f64::from(tgt_rect.size.y);

            let u0 = ((tgt_x0 - anc_x0) / anc_w).clamp(0.0, 1.0);
            let v0 = ((tgt_y0 - anc_y0) / anc_h).clamp(0.0, 1.0);
            let u1 = ((tgt_x1 - anc_x0) / anc_w).clamp(u0, 1.0);
            let v1 = ((tgt_y1 - anc_y0) / anc_h).clamp(v0, 1.0);
            Rect::new(u0 as f32, v0 as f32, (u1 - u0) as f32, (v1 - v0) as f32)
        };

        self.apply_halo_uv(logical_uv, halo_px)
    }

    /// 计算指定层级单个逻辑瓦片在世界坐标系中的名义边长。
    #[inline]
    fn tile_world_span(&self, level: u8) -> f64 {
        f64::from(self.tile_size) * 2.0_f64.powi(i32::from(level))
    }
}

/// 单条已解析的瓦片绘制指令。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedTileDraw {
    /// 目标瓦片在画布世界坐标系中的绘制矩形。
    pub target_world_rect: Rect,
    /// 实际参与采样的已驻留瓦片键（可能是目标瓦片自身，也可能是其祖先瓦片）。
    pub resident_key: TileKey,
    /// 在 `resident_key` 纹理内的归一化采样 UV 子矩形。
    pub sample_uv: Rect,
    /// 是否由粗级祖先瓦片回退绘制。
    pub is_fallback: bool,
}

/// 单帧瓦片金字塔调度计划。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileSchedulePlan {
    /// 本帧根据缩放比解析出的目标 LOD 层级。
    pub target_level: u8,
    /// 本帧可见瓦片的绘制列表（含直接命中与粗级回退）。
    pub draws: Vec<ResolvedTileDraw>,
    /// 本帧待加载瓦片队列（可见区优先于预取环，各组按距视口中心升序排列）。
    pub load_queue: Vec<TileKey>,
    /// 本帧超出预算时建议淘汰的冷瓦片队列（按 LRU 顺序，绝不包含本帧 `draws` 依赖瓦片）。
    pub evict_queue: Vec<TileKey>,
}

/// 基于显存槽位预算与 LRU 策略的纯 CPU 瓦片金字塔驻留缓存与调度器。
#[derive(Clone, Debug)]
pub struct TilePyramidCache {
    spec: TilePyramidSpec,
    capacity: usize,
    halo_px: u32,
    /// 已驻留瓦片及其最近一次访问的逻辑时间戳。
    resident: FxHashMap<TileKey, u64>,
    /// 单调递增逻辑时钟，用于精确刻画 LRU 访问次序。
    clock: u64,
}

impl TilePyramidCache {
    /// 创建一个瓦片金字塔调度缓存（默认 `halo_px = 0`，输出标准 `[0, 1]` 归一化 UV）。
    #[must_use]
    pub fn new(spec: TilePyramidSpec, capacity: usize) -> Self {
        Self::with_halo(spec, capacity, 0)
    }

    /// 创建一个带单边像素 Halo 映射的瓦片金字塔调度缓存（如 `512 + 4px` 扩边瓦片）。
    #[must_use]
    pub fn with_halo(spec: TilePyramidSpec, capacity: usize, halo_px: u32) -> Self {
        Self {
            spec,
            capacity,
            halo_px,
            resident: FxHashMap::default(),
            clock: 0,
        }
    }

    /// 返回当前金字塔几何规格。
    #[inline]
    #[must_use]
    pub const fn spec(&self) -> TilePyramidSpec {
        self.spec
    }

    /// 返回最大驻留瓦片预算。
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// 动态调整最大驻留瓦片预算（下次调用 [`Self::plan`] 时按新预算输出淘汰列表）。
    #[inline]
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
    }

    /// 返回当前单边 Halo 像素数。
    #[inline]
    #[must_use]
    pub const fn halo_px(&self) -> u32 {
        self.halo_px
    }

    /// 设置单边 Halo 像素数（为 `0` 时禁用 Halo UV 内缩）。
    #[inline]
    pub fn set_halo_px(&mut self, halo_px: u32) {
        self.halo_px = halo_px;
    }

    /// 返回当前已驻留的瓦片数量。
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.resident.len()
    }

    /// 判断当前是否没有任何已驻留瓦片。
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.resident.is_empty()
    }

    /// 清空全部已驻留记录并重置逻辑时钟。
    #[inline]
    pub fn clear(&mut self) {
        self.resident.clear();
        self.clock = 0;
    }

    /// 标记某个合法瓦片已驻留（若已存在则刷新其 LRU 热度）。
    ///
    /// 若 `key` 超出当前金字塔边界，调用将被安全忽略。
    pub fn mark_resident(&mut self, key: TileKey) {
        if !self.spec.contains_key(key) {
            return;
        }
        let tick = self.next_tick();
        self.resident.insert(key, tick);
    }

    /// 从驻留表中移除指定瓦片。
    pub fn evict(&mut self, key: TileKey) {
        self.resident.remove(&key);
    }

    /// 查询指定瓦片当前是否已驻留。
    #[inline]
    #[must_use]
    pub fn contains(&self, key: TileKey) -> bool {
        self.resident.contains_key(&key)
    }

    /// 针对当前视口世界矩形、缩放比与预取环宽度生成单帧调度计划。
    ///
    /// 本方法会刷新本帧绘制所依赖瓦片（直接命中与回退祖先）的 LRU 热度，
    /// 但不会直接增删驻留表——调用方在完成异步加载或显存回收后，
    /// 应分别调用 [`Self::mark_resident`] 与 [`Self::evict`] 同步状态。
    #[must_use]
    pub fn plan(
        &mut self,
        viewport_world: Rect,
        zoom: f32,
        prefetch_ring: u32,
    ) -> TileSchedulePlan {
        let target_level = self.spec.select_level(zoom);
        if !self.spec.is_valid() || !viewport_world.is_finite() || viewport_world.is_empty() {
            let evict_queue = self.collect_evictions(&FxHashSet::default());
            return TileSchedulePlan {
                target_level,
                draws: Vec::new(),
                load_queue: Vec::new(),
                evict_queue,
            };
        }

        let image_rect = self.spec.image_world_rect();
        let clipped = viewport_world.intersect(&image_rect);
        if clipped.is_empty() {
            let evict_queue = self.collect_evictions(&FxHashSet::default());
            return TileSchedulePlan {
                target_level,
                draws: Vec::new(),
                load_queue: Vec::new(),
                evict_queue,
            };
        }

        let (cols, rows) = self.spec.grid_size(target_level);
        let span = self.spec.tile_world_span(target_level);

        let min_x = f64::from(clipped.origin.x).max(0.0);
        let min_y = f64::from(clipped.origin.y).max(0.0);
        let max_x = (f64::from(clipped.origin.x) + f64::from(clipped.size.x))
            .min(f64::from(self.spec.image_width));
        let max_y = (f64::from(clipped.origin.y) + f64::from(clipped.size.y))
            .min(f64::from(self.spec.image_height));

        let vis_x0 = ((min_x / span).floor().max(0.0) as u64).min(u64::from(cols - 1)) as u32;
        let vis_y0 = ((min_y / span).floor().max(0.0) as u64).min(u64::from(rows - 1)) as u32;
        let vis_x1 = (((max_x / span).ceil().max(1.0) as u64).saturating_sub(1))
            .min(u64::from(cols - 1))
            .max(u64::from(vis_x0)) as u32;
        let vis_y1 = (((max_y / span).ceil().max(1.0) as u64).saturating_sub(1))
            .min(u64::from(rows - 1))
            .max(u64::from(vis_y0)) as u32;

        let vp_center = viewport_world.center();
        let vp_cx = f64::from(vp_center.x);
        let vp_cy = f64::from(vp_center.y);

        let mut draws = Vec::new();
        let mut visible_loads: Vec<(f64, TileKey)> = Vec::new();
        let mut protected: FxHashSet<TileKey> = FxHashSet::default();

        // 1. 解析可见区瓦片：已驻留直接绘制，未驻留则向上回退最近已驻留祖先并加入高优加载队列。
        for y in vis_y0..=vis_y1 {
            for x in vis_x0..=vis_x1 {
                let key = TileKey::new(target_level, x, y);
                let target_rect = self.spec.tile_world_rect(key);
                if target_rect.is_empty() {
                    continue;
                }
                if self.resident.contains_key(&key) {
                    self.touch(key);
                    protected.insert(key);
                    draws.push(ResolvedTileDraw {
                        target_world_rect: target_rect,
                        resident_key: key,
                        sample_uv: self.spec.fallback_sample_uv(key, key, self.halo_px),
                        is_fallback: false,
                    });
                } else {
                    let dist_sq = rect_center_dist_sq(target_rect, vp_cx, vp_cy);
                    visible_loads.push((dist_sq, key));
                    if let Some(ancestor) = self.find_resident_ancestor(key) {
                        self.touch(ancestor);
                        protected.insert(ancestor);
                        draws.push(ResolvedTileDraw {
                            target_world_rect: target_rect,
                            resident_key: ancestor,
                            sample_uv: self.spec.fallback_sample_uv(key, ancestor, self.halo_px),
                            is_fallback: true,
                        });
                    }
                }
            }
        }

        // 2. 收集预取环中的未驻留瓦片（优先级低于可见区）。
        let mut prefetch_loads: Vec<(f64, TileKey)> = Vec::new();
        if prefetch_ring > 0 {
            let pf_x0 = vis_x0.saturating_sub(prefetch_ring);
            let pf_y0 = vis_y0.saturating_sub(prefetch_ring);
            let pf_x1 = vis_x1.saturating_add(prefetch_ring).min(cols - 1);
            let pf_y1 = vis_y1.saturating_add(prefetch_ring).min(rows - 1);

            for y in pf_y0..=pf_y1 {
                for x in pf_x0..=pf_x1 {
                    if x >= vis_x0 && x <= vis_x1 && y >= vis_y0 && y <= vis_y1 {
                        continue;
                    }
                    let key = TileKey::new(target_level, x, y);
                    if self.resident.contains_key(&key) {
                        continue;
                    }
                    let rect = self.spec.tile_world_rect(key);
                    if rect.is_empty() {
                        continue;
                    }
                    let dist_sq = rect_center_dist_sq(rect, vp_cx, vp_cy);
                    prefetch_loads.push((dist_sq, key));
                }
            }
        }

        // 3. 稳定排序加载队列：可见区优先于预取环，组内按距视口中心升序，平局按坐标确定性打破。
        let cmp_load = |a: &(f64, TileKey), b: &(f64, TileKey)| {
            a.0.total_cmp(&b.0)
                .then_with(|| a.1.level.cmp(&b.1.level))
                .then_with(|| a.1.y.cmp(&b.1.y))
                .then_with(|| a.1.x.cmp(&b.1.x))
        };
        visible_loads.sort_by(cmp_load);
        prefetch_loads.sort_by(cmp_load);

        let mut load_queue = Vec::with_capacity(visible_loads.len() + prefetch_loads.len());
        load_queue.extend(visible_loads.into_iter().map(|(_, k)| k));
        load_queue.extend(prefetch_loads.into_iter().map(|(_, k)| k));

        // 4. 超出驻留预算时按 LRU 收集可淘汰的冷瓦片（严格保护本帧绘制依赖）。
        let evict_queue = self.collect_evictions(&protected);

        TileSchedulePlan {
            target_level,
            draws,
            load_queue,
            evict_queue,
        }
    }

    /// 向上回溯寻找最近的已驻留祖先瓦片。
    fn find_resident_ancestor(&self, mut key: TileKey) -> Option<TileKey> {
        while let Some(parent) = self.spec.parent_of(key) {
            if self.resident.contains_key(&parent) {
                return Some(parent);
            }
            key = parent;
        }
        None
    }

    /// 推进逻辑时钟并返回新的访问时间戳。
    #[inline]
    fn next_tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    /// 刷新已驻留瓦片的 LRU 访问时间戳。
    #[inline]
    fn touch(&mut self, key: TileKey) {
        let tick = self.next_tick();
        if let Some(slot) = self.resident.get_mut(&key) {
            *slot = tick;
        }
    }

    /// 根据当前预算与本帧受保护集合，按 LRU 顺序生成待淘汰冷瓦片列表。
    fn collect_evictions(&self, protected: &FxHashSet<TileKey>) -> Vec<TileKey> {
        let overflow = self.resident.len().saturating_sub(self.capacity);
        if overflow == 0 {
            return Vec::new();
        }
        let mut candidates: Vec<(u64, TileKey)> = self
            .resident
            .iter()
            .filter(|(key, _)| !protected.contains(key))
            .map(|(&key, &tick)| (tick, key))
            .collect();
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.level.cmp(&b.1.level))
                .then_with(|| a.1.y.cmp(&b.1.y))
                .then_with(|| a.1.x.cmp(&b.1.x))
        });
        candidates
            .into_iter()
            .take(overflow)
            .map(|(_, key)| key)
            .collect()
    }
}

/// 无溢出计算 `ceil(dim / 2^shift)`（对正整数 `dim` 恒返回至少 `1`）。
#[inline]
fn ceil_div_pow2(dim: u32, shift: u8) -> u32 {
    if dim == 0 {
        return 0;
    }
    if shift == 0 {
        return dim;
    }
    if shift >= 32 {
        return 1;
    }
    let d = u64::from(dim);
    let step = 1_u64 << shift;
    ((d + step - 1) >> shift).max(1) as u32
}

/// 计算矩形中心到给定视口中心 `(vp_cx, vp_cy)` 的欧氏距离平方。
#[inline]
fn rect_center_dist_sq(rect: Rect, vp_cx: f64, vp_cy: f64) -> f64 {
    let cx = f64::from(rect.origin.x) + f64::from(rect.size.x) * 0.5;
    let cy = f64::from(rect.origin.y) + f64::from(rect.size.y) * 0.5;
    let dx = cx - vp_cx;
    let dy = cy - vp_cy;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rect_approx(actual: Rect, expected: Rect, eps: f32) {
        assert!(
            (actual.origin.x - expected.origin.x).abs() <= eps
                && (actual.origin.y - expected.origin.y).abs() <= eps
                && (actual.size.x - expected.size.x).abs() <= eps
                && (actual.size.y - expected.size.y).abs() <= eps,
            "rect mismatch: actual={actual:?}, expected={expected:?}, eps={eps}"
        );
    }

    #[test]
    fn power_of_two_pyramid_geometry_is_exact() {
        let spec = TilePyramidSpec::new(4096, 2048, 512);
        assert_eq!(spec.max_level(), 3);

        assert_eq!(spec.level_dimensions(0), (4096, 2048));
        assert_eq!(spec.level_dimensions(1), (2048, 1024));
        assert_eq!(spec.level_dimensions(2), (1024, 512));
        assert_eq!(spec.level_dimensions(3), (512, 256));

        assert_eq!(spec.grid_size(0), (8, 4));
        assert_eq!(spec.grid_size(1), (4, 2));
        assert_eq!(spec.grid_size(2), (2, 1));
        assert_eq!(spec.grid_size(3), (1, 1));

        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 0, 0)),
            Rect::new(0.0, 0.0, 512.0, 512.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 7, 3)),
            Rect::new(3584.0, 1536.0, 512.0, 512.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(3, 0, 0)),
            Rect::new(0.0, 0.0, 4096.0, 2048.0),
            1e-5,
        );
        // 越界瓦片返回空矩形。
        assert_eq!(spec.tile_world_rect(TileKey::new(0, 8, 0)), Rect::ZERO);
        assert_eq!(spec.tile_world_rect(TileKey::new(4, 0, 0)), Rect::ZERO);
    }

    #[test]
    fn non_multiple_edge_tiles_clip_precisely_without_gaps() {
        let spec = TilePyramidSpec::new(1000, 700, 512);
        assert_eq!(spec.max_level(), 1);
        assert_eq!(spec.level_dimensions(0), (1000, 700));
        assert_eq!(spec.level_dimensions(1), (500, 350));
        assert_eq!(spec.grid_size(0), (2, 2));
        assert_eq!(spec.grid_size(1), (1, 1));

        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 0, 0)),
            Rect::new(0.0, 0.0, 512.0, 512.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 1, 0)),
            Rect::new(512.0, 0.0, 488.0, 512.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 0, 1)),
            Rect::new(0.0, 512.0, 512.0, 188.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(0, 1, 1)),
            Rect::new(512.0, 512.0, 488.0, 188.0),
            1e-5,
        );
        assert_rect_approx(
            spec.tile_world_rect(TileKey::new(1, 0, 0)),
            Rect::new(0.0, 0.0, 1000.0, 700.0),
            1e-5,
        );
    }

    #[test]
    fn extreme_and_degenerate_specs_never_overflow_or_divide_by_zero() {
        let invalid = TilePyramidSpec::new(0, 1024, 512);
        assert!(!invalid.is_valid());
        assert_eq!(invalid.max_level(), 0);
        assert_eq!(invalid.level_dimensions(0), (0, 0));
        assert_eq!(invalid.grid_size(0), (0, 0));
        assert_eq!(invalid.tile_world_rect(TileKey::new(0, 0, 0)), Rect::ZERO);

        let zero_tile = TilePyramidSpec::new(1024, 1024, 0);
        assert!(!zero_tile.is_valid());
        assert_eq!(zero_tile.max_level(), 0);
        assert_eq!(zero_tile.grid_size(0), (0, 0));

        // 极限 u32::MAX 尺寸不发生整数溢出。
        let huge = TilePyramidSpec::new(u32::MAX, u32::MAX, 512);
        let max_lvl = huge.max_level();
        assert_eq!(max_lvl, 23);
        assert_eq!(huge.grid_size(max_lvl), (1, 1));
        assert_eq!(huge.level_dimensions(32), (1, 1));
        assert_eq!(huge.level_dimensions(u8::MAX), (1, 1));
    }

    #[test]
    fn zoom_maps_to_expected_lod_across_normal_and_extreme_values() {
        let spec = TilePyramidSpec::new(4096, 4096, 512);
        assert_eq!(spec.max_level(), 3);

        // 放大或 1:1 锁定最高精度层 0。
        assert_eq!(spec.select_level(f32::INFINITY), 0);
        assert_eq!(spec.select_level(4.0), 0);
        assert_eq!(spec.select_level(1.0), 0);
        assert_eq!(spec.select_level(0.75), 0);

        // 按 2 的负整数次幂平滑切换粗层级。
        assert_eq!(spec.select_level(0.5), 1);
        assert_eq!(spec.select_level(0.3), 1);
        assert_eq!(spec.select_level(0.25), 2);
        assert_eq!(spec.select_level(0.125), 3);

        // 极小缩放、零、负数与 NaN 安全钳制到 max_level。
        assert_eq!(spec.select_level(0.01), 3);
        assert_eq!(spec.select_level(f32::MIN_POSITIVE), 3);
        assert_eq!(spec.select_level(0.0), 3);
        assert_eq!(spec.select_level(-1.0), 3);
        assert_eq!(spec.select_level(f32::NEG_INFINITY), 3);
        assert_eq!(spec.select_level(f32::NAN), 3);
    }

    #[test]
    fn halo_512_plus_4px_uv_mapping_is_exact() {
        let spec = TilePyramidSpec::with_default_tile_size(2048, 2048);
        assert_eq!(spec.tile_size, DEFAULT_TILE_SIZE);
        assert_eq!(spec.physical_tile_size(DEFAULT_TILE_HALO_PX), 520);

        let full = spec.apply_halo_uv(Rect::new(0.0, 0.0, 1.0, 1.0), DEFAULT_TILE_HALO_PX);
        assert_rect_approx(
            full,
            Rect::new(4.0 / 520.0, 4.0 / 520.0, 512.0 / 520.0, 512.0 / 520.0),
            1e-6,
        );

        // 启用 Halo 的调度器在父级回退时精确映射子象限 UV。
        let mut cache = TilePyramidCache::with_halo(spec, 16, DEFAULT_TILE_HALO_PX);
        let parent = TileKey::new(1, 0, 0);
        cache.mark_resident(parent);

        // 仅观察 level 0 右上象限 (1, 0) -> 世界坐标 [512, 1024] x [0, 512]。
        let plan = cache.plan(Rect::new(512.0, 0.0, 512.0, 512.0), 1.0, 0);
        assert_eq!(plan.target_level, 0);
        assert_eq!(plan.draws.len(), 1);
        let draw = plan.draws[0];
        assert!(draw.is_fallback);
        assert_eq!(draw.resident_key, parent);
        assert_rect_approx(
            draw.sample_uv,
            Rect::new(260.0 / 520.0, 4.0 / 520.0, 256.0 / 520.0, 256.0 / 520.0),
            1e-6,
        );
    }

    #[test]
    fn parent_and_grandparent_fallback_compute_exact_sample_uv() {
        let spec = TilePyramidSpec::new(2048, 2048, 512);
        let mut cache = TilePyramidCache::new(spec, 32);

        // 仅驻留祖父级 (level 2, 0, 0) 与一个父级 (level 1, 0, 0)。
        let grandparent = TileKey::new(2, 0, 0);
        let parent_tl = TileKey::new(1, 0, 0);
        cache.mark_resident(grandparent);
        cache.mark_resident(parent_tl);

        // 视口覆盖 (0, 1, 0)（属于 parent_tl）与 (0, 2, 1)（仅祖父级驻留）。
        let plan = cache.plan(Rect::new(600.0, 100.0, 900.0, 600.0), 1.0, 0);
        assert_eq!(plan.target_level, 0);

        // 检查 (0, 1, 0) 优先回退至更近的父级 parent_tl，UV 为右上象限 [0.5, 0.0, 0.5, 0.5]。
        let draw_1_0 = plan
            .draws
            .iter()
            .find(|d| d.target_world_rect == Rect::new(512.0, 0.0, 512.0, 512.0))
            .expect("draw for tile (0, 1, 0) must exist");
        assert!(draw_1_0.is_fallback);
        assert_eq!(draw_1_0.resident_key, parent_tl);
        assert_rect_approx(draw_1_0.sample_uv, Rect::new(0.5, 0.0, 0.5, 0.5), 1e-6);

        // 检查 (0, 2, 1) 回退至祖父级 grandparent，UV 为 [0.5, 0.25, 0.25, 0.25]。
        let draw_2_1 = plan
            .draws
            .iter()
            .find(|d| d.target_world_rect == Rect::new(1024.0, 512.0, 512.0, 512.0))
            .expect("draw for tile (0, 2, 1) must exist");
        assert!(draw_2_1.is_fallback);
        assert_eq!(draw_2_1.resident_key, grandparent);
        assert_rect_approx(draw_2_1.sample_uv, Rect::new(0.5, 0.25, 0.25, 0.25), 1e-6);
    }

    #[test]
    fn non_multiple_edge_tile_fallback_uv_covers_ancestor_seamlessly() {
        let spec = TilePyramidSpec::new(1000, 700, 512);
        let mut cache = TilePyramidCache::new(spec, 8);
        let root = TileKey::new(1, 0, 0);
        cache.mark_resident(root);

        let plan = cache.plan(Rect::new(0.0, 0.0, 1000.0, 700.0), 1.0, 0);
        assert_eq!(plan.draws.len(), 4);
        assert!(plan
            .draws
            .iter()
            .all(|d| d.is_fallback && d.resident_key == root));

        // 四个子块的 UV 面积之和应精确等于 1.0。
        let total_uv_area: f32 = plan.draws.iter().map(|d| d.sample_uv.area()).sum();
        assert!((total_uv_area - 1.0).abs() < 1e-5);

        let br = plan
            .draws
            .iter()
            .find(|d| d.target_world_rect == Rect::new(512.0, 512.0, 488.0, 188.0))
            .expect("bottom-right edge tile must exist");
        assert_rect_approx(
            br.sample_uv,
            Rect::new(512.0 / 1000.0, 512.0 / 700.0, 488.0 / 1000.0, 188.0 / 700.0),
            1e-6,
        );
    }

    #[test]
    fn load_queue_prioritizes_visible_before_prefetch_and_orders_by_center_distance() {
        let spec = TilePyramidSpec::new(2048, 2048, 512);
        let mut cache = TilePyramidCache::new(spec, 32);

        // 视口仅覆盖 (0, 1, 1) 与 (0, 2, 1)，且中心更偏向 (0, 1, 1) 的中心 (768, 768)。
        let viewport = Rect::new(600.0, 600.0, 450.0, 300.0);
        let plan = cache.plan(viewport, 1.0, 1);

        assert_eq!(plan.target_level, 0);
        // 无任何祖先驻留时，draws 应为空，仅排入 load_queue。
        assert!(plan.draws.is_empty());

        // 可见区为 x in 1..=2, y in 1..=1（共 2 个瓦片），必须排在队列最前且近中心者居先。
        assert_eq!(plan.load_queue[0], TileKey::new(0, 1, 1));
        assert_eq!(plan.load_queue[1], TileKey::new(0, 2, 1));

        // 预取环扩展至 x in 0..=3, y in 0..=2（共 4 * 3 - 2 = 10 个预取瓦片）。
        assert_eq!(plan.load_queue.len(), 12);

        // 验证预取段内部同样按距视口中心距离单调不减。
        let vp_c = viewport.center();
        let prefetch_dists: Vec<f64> = plan.load_queue[2..]
            .iter()
            .map(|&k| {
                rect_center_dist_sq(
                    spec.tile_world_rect(k),
                    f64::from(vp_c.x),
                    f64::from(vp_c.y),
                )
            })
            .collect();
        for pair in prefetch_dists.windows(2) {
            assert!(pair[0] <= pair[1] + 1e-6);
        }
    }

    #[test]
    fn lru_eviction_protects_active_and_fallback_tiles() {
        let spec = TilePyramidSpec::new(2048, 2048, 512);
        let mut cache = TilePyramidCache::new(spec, 2);

        let cold_oldest = TileKey::new(0, 3, 3);
        let cold_newer = TileKey::new(0, 3, 2);
        let fallback_parent = TileKey::new(1, 0, 0);
        let visible_hit = TileKey::new(0, 0, 0);

        // 按顺序写入：fallback_parent 最早，但本帧会被回退依赖，绝不可淘汰。
        cache.mark_resident(fallback_parent);
        cache.mark_resident(cold_oldest);
        cache.mark_resident(cold_newer);
        cache.mark_resident(visible_hit);
        assert_eq!(cache.len(), 4);

        // 视口覆盖 (0, 0, 0)（直接命中）与 (0, 1, 0)（回退依赖 fallback_parent）。
        let plan = cache.plan(Rect::new(0.0, 0.0, 700.0, 400.0), 1.0, 0);
        assert_eq!(plan.draws.len(), 2);

        // 超出预算 4 - 2 = 2 个，必须按 LRU 顺序淘汰两个冷瓦片，严禁淘汰受保护瓦片。
        assert_eq!(plan.evict_queue, vec![cold_oldest, cold_newer]);
        assert!(!plan.evict_queue.contains(&fallback_parent));
        assert!(!plan.evict_queue.contains(&visible_hit));

        for key in plan.evict_queue {
            cache.evict(key);
            assert!(!cache.contains(key));
        }
        assert_eq!(cache.len(), 2);
        assert!(cache.contains(fallback_parent));
        assert!(cache.contains(visible_hit));

        // 即使预算缩减为 1，仍绝不淘汰本帧 draws 依赖的 2 个瓦片。
        cache.set_capacity(1);
        let tight_plan = cache.plan(Rect::new(0.0, 0.0, 700.0, 400.0), 1.0, 0);
        assert!(tight_plan.evict_queue.is_empty());
    }

    #[test]
    fn empty_non_finite_and_out_of_bounds_viewports_are_handled_safely() {
        let spec = TilePyramidSpec::new(2048, 2048, 512);
        let mut cache = TilePyramidCache::new(spec, 1);
        let k1 = TileKey::new(0, 0, 0);
        let k2 = TileKey::new(0, 1, 0);
        cache.mark_resident(k1);
        cache.mark_resident(k2);

        // 空视口：无绘制、无加载，仍按 LRU 清理超出预算的冷瓦片。
        let empty_plan = cache.plan(Rect::ZERO, 1.0, 1);
        assert!(empty_plan.draws.is_empty());
        assert!(empty_plan.load_queue.is_empty());
        assert_eq!(empty_plan.evict_queue, vec![k1]);

        // 非有限视口：安全返回空绘制与空加载。
        let nan_plan = cache.plan(Rect::new(f32::NAN, 0.0, 500.0, 500.0), 1.0, 1);
        assert!(nan_plan.draws.is_empty());
        assert!(nan_plan.load_queue.is_empty());

        // 完全位于图像之外的视口：不产生绘制或加载。
        let outside_plan = cache.plan(Rect::new(5000.0, 5000.0, 500.0, 500.0), 1.0, 2);
        assert!(outside_plan.draws.is_empty());
        assert!(outside_plan.load_queue.is_empty());
    }
}

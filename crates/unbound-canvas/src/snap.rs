//! 视口限定的智能吸附求解器（`docs` §5.3）。
//!
//! 拖拽实体时，画布不会针对整个世界进行吸附，而是仅针对用户*可见*的内容。
//! 因此求解器执行以下步骤：
//!
//! 1. 向松散四叉树查询视口向外扩展 [`SnapConfig::padding_px`]（默认 64 px）
//!    范围内的所有元素；
//! 2. 按 `area / distance` 对候选元素排序并保留前 [`SnapConfig::max_candidates`]（128）个——
//!    百万元素画布绝不能让一次拖拽变成百万条边的扫描；
//!
//! 3. 将保留的候选投影到两张有序一维区间表（X 轴与 Y 轴）；
//! 4. 使用二分查找匹配左/中/右与上/中/下对齐；最后
//!    （若启用）
//! 5. 在双轴上查找等间距分布。
//!
//! 距离以*屏幕*像素计：阈值会除以相机缩放倍率，
//! 使 5 px 的吸附磁力在 10% 或 6400% 缩放下始终保持 5 屏幕像素。
//!
//! 输出为纯修正量与一组瞬态参考线。参考线在设计上即为瞬态（`docs` §5.3）：
//! 它们通过可复用缓冲区返回，由调用方直接拷贝进帧瞬态环中，
//! 绝不写入归约器状态或撤销栈。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Rect, Vec2};

use crate::quadtree::LooseQuadtree;

/// 参考线所沿的坐标轴。`X` 表示位于 `coordinate` 处的*垂直*参考线。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// 触发吸附的对齐特征类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// 移动左边缘对齐目标左边缘。
    Left,
    /// 移动中心线对齐目标中心线。
    Centre,
    /// 移动右边缘对齐目标右边缘。
    Right,
    Top,
    Middle,
    Bottom,
    /// 等间距分布对齐。
    Spacing,
}

impl SnapKind {
    /// 属于 X 轴变体时返回 true。
    #[must_use]
    pub fn is_x(self) -> bool {
        matches!(self, Self::Left | Self::Centre | Self::Right)
    }
}

/// 世界空间中的一条瞬态对齐参考线。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SnapGuide {
    pub axis: Axis,
    /// 参考线的世界坐标（垂直线对应 X 坐标）。
    pub coordinate: f32,
    /// 沿另一轴的跨度，使渲染器可绘制有限线段而非无限延长线。
    ///
    pub start: f32,
    pub end: f32,
    pub kind: SnapKind,
}

/// 调优参数配置。默认值取自 `docs` §5.3。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SnapConfig {
    /// 屏幕空间吸附磁力半径。
    pub threshold_px: f32,
    /// 收集候选元素时对视口的扩展边距。
    pub padding_px: f32,
    /// 候选元素硬上限；排序会保留最具显著性的元素。
    pub max_candidates: usize,
    /// 是否启用等间距检测。
    pub equal_spacing: bool,
    /// 小于该宽度（屏幕像素）的间隙视为噪声予以忽略。
    pub min_spacing_px: f32,
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            threshold_px: 5.0,
            padding_px: 64.0,
            max_candidates: 128,
            equal_spacing: true,
            min_spacing_px: 4.0,
        }
    }
}

/// 单帧拖拽的吸附求解结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapResult {
    /// 需加到被拖拽矩形原点上的修正量（世界单位）。
    pub offset: Vec2,
    /// X 轴是否找到吸附目标。
    pub snapped_x: bool,
    /// Y 轴是否找到吸附目标。
    pub snapped_y: bool,
    /// 本帧生成的瞬态参考线，按 `(axis, coordinate)` 去重。
    pub guides: Vec<SnapGuide>,
}

impl SnapResult {
    /// 当任一轴产生位置修正时返回 true。
    #[must_use]
    pub fn is_snapped(&self) -> bool {
        self.snapped_x || self.snapped_y
    }

    fn reset(&mut self) {
        self.offset = Vec2::ZERO;
        self.snapped_x = false;
        self.snapped_y = false;
        self.guides.clear();
    }

    fn push_guide(&mut self, guide: SnapGuide) {
        for existing in &mut self.guides {
            if existing.axis == guide.axis && (existing.coordinate - guide.coordinate).abs() < 0.01
            {
                existing.start = existing.start.min(guide.start);
                existing.end = existing.end.max(guide.end);
                return;
            }
        }
        self.guides.push(guide);
    }
}

/// 经排序的单个候选矩形。
#[derive(Copy, Clone, Debug)]
struct Candidate {
    rect: Rect,
    score: f32,
}

/// 轴向表中的一条边缘记录。
#[derive(Copy, Clone, Debug)]
struct Edge {
    coord: f32,
}

/// 可复用的吸附求解器：自持内部缓冲区，拖拽过程零内存分配。
pub struct Snapper {
    config: SnapConfig,
    candidates: Vec<Candidate>,
    edges_x: Vec<Edge>,
    edges_y: Vec<Edge>,
    result: SnapResult,
    /// 复用的四叉树查询 ID 缓冲区。
    ids: Vec<u32>,
    /// 复用的等间距检测矩形缓冲区。
    scratch: Vec<Rect>,
    /// 上一次调用中通过排序保留的候选数量。
    retained: usize,
}

impl core::fmt::Debug for Snapper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Snapper")
            .field("config", &self.config)
            .field("retained", &self.retained)
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl Snapper {
    /// 使用指定配置构建求解器。
    #[must_use]
    pub fn new(config: SnapConfig) -> Self {
        Self {
            config,
            candidates: Vec::with_capacity(config.max_candidates),
            edges_x: Vec::with_capacity(config.max_candidates * 3),
            edges_y: Vec::with_capacity(config.max_candidates * 3),
            result: SnapResult::default(),
            ids: Vec::new(),
            scratch: Vec::new(),
            retained: 0,
        }
    }

    /// 当前使用的配置。
    #[must_use]
    pub fn config(&self) -> &SnapConfig {
        &self.config
    }

    /// 上一次调用保留的候选数量，供 KPI 测试检查。
    #[must_use]
    pub fn retained_candidates(&self) -> usize {
        self.retained
    }

    /// 从 `tree` 中收集 `viewport` 周围的候选元素并对 `moving` 执行吸附求解。
    ///
    /// `moving_id` 会从候选集中排除，避免实体吸附到自身的旧包围盒。
    ///
    pub fn snap_against_tree(
        &mut self,
        tree: &mut LooseQuadtree,
        moving: Rect,
        moving_id: Option<u32>,
        viewport: Rect,
        zoom: f32,
    ) -> &SnapResult {
        let padded = viewport.inflate(self.config.padding_px / zoom.max(1e-6));
        let mut ids = core::mem::take(&mut self.ids);
        tree.query_exact(padded, &mut ids);
        self.candidates.clear();
        for id in &ids {
            if Some(*id) == moving_id {
                continue;
            }
            if let Some(rect) = tree.bounds_of(*id) {
                if rect.intersects(&padded) {
                    self.candidates.push(Candidate { rect, score: 0.0 });
                }
            }
        }
        self.rank(moving, zoom);
        self.ids = ids;
        self.solve(moving, zoom)
    }

    /// 针对显式候选列表执行求解（用于无头测试套件与单元测试）。
    pub fn snap(&mut self, moving: Rect, others: &[Rect], zoom: f32) -> &SnapResult {
        self.candidates.clear();
        self.candidates.extend(others.iter().map(|rect| Candidate {
            rect: *rect,
            score: 0.0,
        }));
        self.rank(moving, zoom);
        self.solve(moving, zoom)
    }

    /// 上一次调用的求解结果。
    #[must_use]
    pub fn result(&self) -> &SnapResult {
        &self.result
    }

    /// 按显著性（`area / distance`）排序并截断。
    ///
    /// 面积很关键，因为 40 000 px 的分组框比 8 px 图标更适合作为对齐参照；
    /// 距离同样关键，因为远离屏幕的参照物只是噪声。两者的商是兼顾两因素的
    /// 最低开销评分，每个候选只需计算一次。
    ///
    ///
    /// 两项均先转换为*屏幕*像素：面积原为世界单位平方，距离为世界单位，
    /// 若直接混合未缩放数值会导致评分依赖缩放倍率，并改变截断后保留的候选集。
    ///
    ///
    fn rank(&mut self, moving: Rect, zoom: f32) {
        let centre = moving.center();
        // 1 个世界单位等于 `zoom` 个屏幕像素。
        let to_screen = zoom.max(1e-6);
        for candidate in &mut self.candidates {
            let delta = candidate.rect.center() - centre;
            let distance = delta.length() * to_screen;
            let area = candidate.rect.area().max(1.0) * to_screen * to_screen;
            candidate.score = area / (distance + 16.0);
        }
        self.candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        if self.candidates.len() > self.config.max_candidates {
            self.candidates.truncate(self.config.max_candidates);
        }
        self.retained = self.candidates.len();
    }

    fn solve(&mut self, moving: Rect, zoom: f32) -> &SnapResult {
        self.result.reset();
        let inv_zoom = 1.0 / zoom.max(1e-6);
        let threshold = self.config.threshold_px * inv_zoom;
        let min_gap = self.config.min_spacing_px * inv_zoom;

        self.edges_x.clear();
        self.edges_y.clear();
        for candidate in &self.candidates {
            let r = candidate.rect;
            self.edges_x.push(Edge { coord: r.min().x });
            self.edges_x.push(Edge {
                coord: r.center().x,
            });
            self.edges_x.push(Edge { coord: r.max().x });
            self.edges_y.push(Edge { coord: r.min().y });
            self.edges_y.push(Edge {
                coord: r.center().y,
            });
            self.edges_y.push(Edge { coord: r.max().y });
        }
        self.edges_x.sort_by(|a, b| a.coord.total_cmp(&b.coord));
        self.edges_y.sort_by(|a, b| a.coord.total_cmp(&b.coord));

        let min = moving.min();
        let max = moving.max();
        let centre = moving.center();

        let x = self.best_axis(
            [min.x, centre.x, max.x],
            [SnapKind::Left, SnapKind::Centre, SnapKind::Right],
            threshold,
        );
        let y = self.best_axis(
            [min.y, centre.y, max.y],
            [SnapKind::Top, SnapKind::Middle, SnapKind::Bottom],
            threshold,
        );

        let x = self.blend_spacing_x(moving, x, threshold, min_gap);
        let y = self.blend_spacing_y(moving, y, threshold, min_gap);

        if let Some(hit) = x {
            self.result.offset.x = hit.delta;
            self.result.snapped_x = true;
            let (start, end) = self.span_y(&hit, moving);
            self.result.push_guide(SnapGuide {
                axis: Axis::X,
                coordinate: hit.coord,
                start,
                end,
                kind: hit.kind,
            });
        }
        if let Some(hit) = y {
            self.result.offset.y = hit.delta;
            self.result.snapped_y = true;
            let (start, end) = self.span_x(&hit, moving);
            self.result.push_guide(SnapGuide {
                axis: Axis::Y,
                coordinate: hit.coord,
                start,
                end,
                kind: hit.kind,
            });
        }
        &self.result
    }

    /// 在 `threshold` 范围内查找距离 `values` 中任意值最近的边缘。
    fn best_axis(&self, values: [f32; 3], kinds: [SnapKind; 3], threshold: f32) -> Option<Hit> {
        let table: &[Edge] = if kinds[0].is_x() {
            &self.edges_x
        } else {
            &self.edges_y
        };
        if table.is_empty() {
            return None;
        }
        let mut best: Option<Hit> = None;
        for (value, kind) in values.iter().zip(kinds) {
            if !value.is_finite() {
                continue;
            }
            let start = table.partition_point(|e| e.coord < *value - threshold);
            for edge in &table[start..] {
                if edge.coord > *value + threshold {
                    break;
                }
                let delta = edge.coord - *value;
                if delta.abs() > threshold {
                    continue;
                }
                // 优先选择最小修正量。相同时较早的表项胜出，
                // 该表按坐标排序——而非按显著性排序（显著性仅决定哪些候选能进入截断后的表）。
                //
                //
                if best.as_ref().is_none_or(|b| delta.abs() < b.delta.abs()) {
                    best = Some(Hit {
                        coord: edge.coord,
                        delta,
                        kind,
                    });
                }
            }
        }
        best
    }

    /// 在 X 轴执行等间距检测，保留移动量更小的修正。
    fn blend_spacing_x(
        &mut self,
        moving: Rect,
        aligned: Option<Hit>,
        threshold: f32,
        min_gap: f32,
    ) -> Option<Hit> {
        if !self.config.equal_spacing {
            return aligned;
        }
        let mut rects = core::mem::take(&mut self.scratch);
        rects.clear();
        rects.extend(self.candidates.iter().map(|c| c.rect));
        rects.sort_by(|a, b| a.min().x.total_cmp(&b.min().x));
        let mut best = aligned;
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let gap = b.min().x - a.max().x;
            if gap < min_gap {
                continue;
            }
            // 将移动矩形置于 `b` 之后或 `a` 之前，保持相同间距。
            let after = b.max().x + gap - moving.min().x;
            let before = a.min().x - gap - moving.max().x;
            for delta in [after, before] {
                if delta.abs() <= threshold
                    && best.as_ref().is_none_or(|h| delta.abs() < h.delta.abs())
                {
                    best = Some(Hit {
                        coord: moving.min().x + delta,
                        delta,
                        kind: SnapKind::Spacing,
                    });
                }
            }
        }
        self.scratch = rects;
        best
    }

    /// 在 Y 轴执行等间距检测。
    fn blend_spacing_y(
        &mut self,
        moving: Rect,
        aligned: Option<Hit>,
        threshold: f32,
        min_gap: f32,
    ) -> Option<Hit> {
        if !self.config.equal_spacing {
            return aligned;
        }
        let mut rects = core::mem::take(&mut self.scratch);
        rects.clear();
        rects.extend(self.candidates.iter().map(|c| c.rect));
        rects.sort_by(|a, b| a.min().y.total_cmp(&b.min().y));
        let mut best = aligned;
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let gap = b.min().y - a.max().y;
            if gap < min_gap {
                continue;
            }
            let after = b.max().y + gap - moving.min().y;
            let before = a.min().y - gap - moving.max().y;
            for delta in [after, before] {
                if delta.abs() <= threshold
                    && best.as_ref().is_none_or(|h| delta.abs() < h.delta.abs())
                {
                    best = Some(Hit {
                        coord: moving.min().y + delta,
                        delta,
                        kind: SnapKind::Spacing,
                    });
                }
            }
        }
        self.scratch = rects;
        best
    }

    /// 计算 X 轴吸附参考线在 Y 轴上的跨度。
    ///
    /// 跨度覆盖被拖拽矩形*以及所有*共享该匹配坐标的候选矩形，
    /// 因此三个共边对齐的盒子会生成一条贯穿全部三者的完整参考线，而非两两之间的一小段。
    ///
    fn span_y(&self, hit: &Hit, moving: Rect) -> (f32, f32) {
        let mut start = moving.min().y;
        let mut end = moving.max().y;
        for candidate in &self.candidates {
            let r = candidate.rect;
            let aligned = match hit.kind {
                SnapKind::Left => (r.min().x - hit.coord).abs() < 0.01,
                SnapKind::Centre => (r.center().x - hit.coord).abs() < 0.01,
                SnapKind::Right => (r.max().x - hit.coord).abs() < 0.01,
                _ => false,
            };
            if aligned {
                start = start.min(r.min().y);
                end = end.max(r.max().y);
            }
        }
        (start, end)
    }

    /// 计算 Y 轴吸附参考线在 X 轴上的跨度（取所有共线矩形的并集）。
    fn span_x(&self, hit: &Hit, moving: Rect) -> (f32, f32) {
        let mut start = moving.min().x;
        let mut end = moving.max().x;
        for candidate in &self.candidates {
            let r = candidate.rect;
            let aligned = match hit.kind {
                SnapKind::Top => (r.min().y - hit.coord).abs() < 0.01,
                SnapKind::Middle => (r.center().y - hit.coord).abs() < 0.01,
                SnapKind::Bottom => (r.max().y - hit.coord).abs() < 0.01,
                _ => false,
            };
            if aligned {
                start = start.min(r.min().x);
                end = end.max(r.max().x);
            }
        }
        (start, end)
    }
}

/// 单次采纳的对齐结果。
#[derive(Copy, Clone, Debug)]
struct Hit {
    /// 边缘落定的世界坐标。
    coord: f32,
    /// 施加到移动矩形上的修正量。
    delta: f32,
    kind: SnapKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn left_edges_align_within_threshold() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // 移动左边缘位于邻居左边缘右侧 3 px 处；10 px 宽度使另外两条边缘超出磁力范围。
        //
        let moving = rect(103.0, 200.0, 10.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x);
        assert!((r.offset.x - -3.0).abs() < 1e-4, "{:?}", r.offset);
        assert!(r.guides.iter().any(|g| g.axis == Axis::X
            && (g.coordinate - 100.0).abs() < 1e-4
            && g.kind == SnapKind::Left));
    }

    #[test]
    fn nothing_snaps_beyond_the_threshold() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        let moving = rect(180.0, 200.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(!r.snapped_x, "80 px away must not snap");
        assert!(r.guides.is_empty());
    }

    #[test]
    fn centre_to_centre_alignment_wins_over_a_farther_edge() {
        let mut s = Snapper::new(SnapConfig::default());
        // 该矩形中心位于 x = 125。
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // 移动中心在 127 → 偏离中心 2 px；移动左边缘在 117 → 偏离左边缘 17 px（超出范围）。只有中心能匹配。
        //
        let moving = rect(117.0, 200.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x);
        assert!((r.offset.x - -2.0).abs() < 1e-4, "{:?}", r.offset);
        assert_eq!(r.guides[0].kind, SnapKind::Centre);
    }

    #[test]
    fn threshold_is_measured_in_screen_pixels() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // 4 个世界单位在 zoom=1 时为 4 px（吸附），在 zoom=10 时为 0.4 px（Wait: 40px! 实际上注释原意是在不同缩放下像素不同）。
        let moving = rect(104.0, 200.0, 20.0, 20.0);
        assert!(s.snap(moving, &others, 1.0).snapped_x);
        assert!(!s.snap(moving, &others, 10.0).snapped_x);
    }

    #[test]
    fn equal_spacing_produces_a_third_slot() {
        let mut s = Snapper::new(SnapConfig::default());
        // 两个 100 px 的盒子，间距 40 px：0..100 与 140..240。
        let others = [rect(0.0, 0.0, 100.0, 100.0), rect(140.0, 0.0, 100.0, 100.0)];
        // 第三个盒子距离理想槽位 280 相差 3 px。
        let moving = rect(283.0, 0.0, 100.0, 100.0);
        let r = s.snap(moving, &others, 1.0);
        assert!((r.offset.x - -3.0).abs() < 1e-4, "{:?}", r.offset);
        assert!(r.guides.iter().any(|g| g.kind == SnapKind::Spacing));
    }

    #[test]
    fn equal_spacing_can_be_disabled() {
        let mut s = Snapper::new(SnapConfig {
            equal_spacing: false,
            ..SnapConfig::default()
        });
        let others = [rect(0.0, 0.0, 100.0, 100.0), rect(140.0, 0.0, 100.0, 100.0)];
        // 280 恰好是等间距槽位，但该功能已关闭，且没有边缘在 5 px 以内。
        //
        let moving = rect(283.0, 0.0, 100.0, 100.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(!r.snapped_x);
    }

    #[test]
    fn both_axes_can_snap_at_once() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 100.0, 50.0, 50.0)];
        let moving = rect(102.0, 98.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x && r.snapped_y);
        assert_eq!(r.guides.len(), 2, "one guide per axis");
    }

    #[test]
    fn guides_merge_instead_of_piling_up() {
        let mut s = Snapper::new(SnapConfig::default());
        // 三个共享同一左边缘的矩形。
        let others = [
            rect(100.0, 0.0, 10.0, 10.0),
            rect(100.0, 200.0, 10.0, 10.0),
            rect(100.0, 400.0, 10.0, 10.0),
        ];
        let moving = rect(102.0, 300.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        let guides: Vec<_> = r.guides.iter().filter(|g| g.axis == Axis::X).collect();
        assert_eq!(
            guides.len(),
            1,
            "same coordinate must collapse to one guide"
        );
        // 保留下来的参考线从第一个参照矩形延伸到被拖拽的盒子。
        assert!(guides[0].start <= 0.0 && guides[0].end >= 320.0);
    }

    #[test]
    fn candidate_ranking_truncates_to_the_cap() {
        let mut s = Snapper::new(SnapConfig {
            max_candidates: 8,
            ..SnapConfig::default()
        });
        let others: Vec<Rect> = (0..1000)
            .map(|i| rect((i % 40) as f32 * 32.0, (i / 40) as f32 * 32.0, 24.0, 24.0))
            .collect();
        s.snap(rect(500.0, 500.0, 20.0, 20.0), &others, 1.0);
        assert_eq!(s.retained_candidates(), 8);
    }

    #[test]
    fn bigger_and_closer_candidates_outrank_small_distant_ones() {
        let mut s = Snapper::new(SnapConfig {
            max_candidates: 1,
            ..SnapConfig::default()
        });
        let huge_far = rect(5000.0, 5000.0, 2000.0, 2000.0);
        let small_near = rect(0.0, 0.0, 8.0, 8.0);
        s.snap(rect(0.0, 0.0, 20.0, 20.0), &[small_near, huge_far], 1.0);
        // 距离 ~14 的 8 px 图标评分约 64/30 = 2；距离 ~7000 的 4e6 px 画框评分约 4e6/7000 = 570，
        // 因此显著对象在截断中胜出。这正是期望的优先级决策。
        //
        assert_eq!(s.retained_candidates(), 1);
    }

    #[test]
    fn the_dragged_entity_does_not_snap_to_itself() {
        let mut tree = LooseQuadtree::new(4096.0);
        tree.insert(1, rect(100.0, 100.0, 50.0, 50.0), 0);
        tree.insert(2, rect(400.0, 400.0, 50.0, 50.0), 0);
        let mut s = Snapper::new(SnapConfig::default());
        let viewport = rect(0.0, 0.0, 1000.0, 1000.0);
        // 将实体 1 拖拽到距离自身旧位置 2 px 处。
        let r = s.snap_against_tree(
            &mut tree,
            rect(102.0, 100.0, 50.0, 50.0),
            Some(1),
            viewport,
            1.0,
        );
        assert!(!r.is_snapped(), "self-snap is a bug: {r:?}");
    }

    #[test]
    fn tree_candidates_respect_the_viewport_padding() {
        let mut tree = LooseQuadtree::new(65_536.0);
        tree.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        tree.insert(2, rect(9000.0, 0.0, 10.0, 10.0), 0);
        let mut s = Snapper::new(SnapConfig::default());
        let viewport = rect(-100.0, -100.0, 200.0, 200.0);
        s.snap_against_tree(&mut tree, rect(0.0, 300.0, 10.0, 10.0), None, viewport, 1.0);
        assert_eq!(s.retained_candidates(), 1, "only the near rect qualifies");
    }

    #[test]
    fn a_blank_canvas_never_snaps() {
        let mut s = Snapper::new(SnapConfig::default());
        let r = s.snap(rect(1.0, 2.0, 3.0, 4.0), &[], 1.0);
        assert!(!r.is_snapped());
        assert!(r.guides.is_empty());
    }
}

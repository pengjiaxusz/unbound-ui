//! 结合重布局边界（RelayoutBoundary）的 Taffy 集成（`docs` §4.2）。
//!
//! 性能优化围绕两个核心思路展开：
//!
//! 1. **隔离性（Isolation）。** `RelayoutBoundary` 具有固定尺寸并裁剪其子节点，
//!    因此内部任何变化都不会影响外部。布局脏标记止步于此（见 `unbound-world`），
//!    本引擎只需重新求解该边界子树。
//!
//! 2. **并行性（Parallelism）。** 各个边界互不相交，因此当多个边界同时变脏时，
//!    可通过 Rayon 并发求解。每个工作线程为其子树构建独立的小型 `TaffyTree`，
//!    从而避免要求 `TaffyTree: Sync` 并保持工作线程完全独立。
//!
//!
//! 文本测量永不进入本模块：视图构建器会预先测量标签尺寸并将结果存放在节点上，
//! 因此布局引擎只处理数值。这使得文本塑形与布局可独立测试，
//! 同时保证布局求解绝不会意外阻塞在字体系统上。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use taffy::prelude::*;
pub use taffy::AvailableSpace;
use taffy::{Layout, LayoutInput, LayoutOutput, TaffyTree};
use unbound_abi::{Align, Dim, DimUnit, Display, FlexDirection, Justify, NodeFlags, Position};
use unbound_math::{Rect, Vec2};
use unbound_world::{NodeId, UiWorld};

/// 单次布局求解的结果，供 KPI 基准测试与调试覆盖层使用。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    pub nodes_visited: u32,
    pub boundaries_solved: u32,
    /// Taffy 内部耗时（微秒，不含树同步耗时）。
    pub taffy_us: u64,
    /// 拷入与拷出样式的耗时（微秒）。
    pub sync_us: u64,
}

impl LayoutStats {
    #[must_use]
    pub fn total_us(&self) -> u64 {
        self.taffy_us + self.sync_us
    }
}

// ---------------------------------------------------------------------------
// Descriptor -> Taffy 样式转换
// ---------------------------------------------------------------------------

fn dim(d: Dim) -> Dimension {
    match d.kind() {
        DimUnit::Px => Dimension::length(d.value),
        DimUnit::Percent => Dimension::percent((d.value / 100.0).max(0.0)),
        // `fr` 是网格概念；作为 flex basis 时，直接使用字面值是最合理的近似。
        //
        DimUnit::Fr => Dimension::length(d.value),
        DimUnit::Auto => Dimension::auto(),
    }
}

fn inset(d: Dim) -> LengthPercentageAuto {
    match d.kind() {
        DimUnit::Px => LengthPercentageAuto::length(d.value),
        DimUnit::Percent => LengthPercentageAuto::percent((d.value / 100.0).max(0.0)),
        _ => LengthPercentageAuto::auto(),
    }
}

fn fixed(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

/// 将 `Dim` 轨道映射为 Taffy 的最小/最大轨道尺寸对。
fn grid_track<S: taffy::style::CheapCloneStr>(d: Dim) -> taffy::style::GridTemplateComponent<S> {
    use taffy::style::{MaxTrackSizingFunction, MinTrackSizingFunction};
    use taffy::style_helpers;
    match d.kind() {
        DimUnit::Px => style_helpers::minmax(
            MinTrackSizingFunction::length(d.value.max(0.0)),
            MaxTrackSizingFunction::length(d.value.max(0.0)),
        ),
        DimUnit::Percent => {
            let percent = (d.value / 100.0).clamp(0.0, 1.0);
            style_helpers::minmax(
                MinTrackSizingFunction::percent(percent),
                MaxTrackSizingFunction::percent(percent),
            )
        }
        DimUnit::Fr => style_helpers::flex(d.value.max(0.0)),
        DimUnit::Auto => style_helpers::minmax(
            MinTrackSizingFunction::auto(),
            MaxTrackSizingFunction::auto(),
        ),
    }
}

/// 将基于 1 的起始行号与跨度转换为 Taffy 放置位置。
fn grid_placement(start: i16, span_count: u8) -> taffy::Line<taffy::style::GridPlacement> {
    use taffy::style::GridPlacement;
    use taffy::style_helpers::{line, span};
    let span_count = u16::from(span_count);
    if span_count > 0 {
        if start > 0 {
            taffy::Line {
                start: line(start),
                end: line(start + i16::try_from(span_count).unwrap_or(i16::MAX)),
            }
        } else {
            taffy::Line {
                start: GridPlacement::Auto,
                end: span(span_count),
            }
        }
    } else if start > 0 {
        taffy::Line {
            start: line(start),
            end: GridPlacement::Auto,
        }
    } else {
        taffy::Line::default()
    }
}

fn align_items(a: Align) -> taffy::AlignItems {
    match a {
        Align::FlexStart => taffy::AlignItems::FLEX_START,
        // Taffy 原生支持基线对齐；若将其折叠为 `FLEX_START`，
        // 会导致所有“标题紧邻说明文字”的行变为顶部对齐。
        Align::Baseline => taffy::AlignItems::BASELINE,
        Align::FlexEnd => taffy::AlignItems::FLEX_END,
        Align::Center => taffy::AlignItems::CENTER,
        Align::Stretch => taffy::AlignItems::STRETCH,
    }
}

/// 当描述符未指定对齐时，`align_self` 返回 `None`，使其从父节点的 `align_items` *继承*，
/// 行为与 CSS `align-self: auto` 完全一致。若将默认值映射为 `STRETCH`，
/// 会导致每个节点都忽略父节点的对齐方式——这是 Flex 布局中最常见的意外行为。
///
fn align_self(a: Align) -> Option<taffy::AlignSelf> {
    match a {
        Align::FlexStart => Some(taffy::AlignSelf::FLEX_START),
        Align::Baseline => Some(taffy::AlignSelf::BASELINE),
        Align::FlexEnd => Some(taffy::AlignSelf::FLEX_END),
        Align::Center => Some(taffy::AlignSelf::CENTER),
        Align::Stretch => None,
    }
}

fn justify(j: Justify) -> taffy::JustifyContent {
    match j {
        Justify::FlexStart => taffy::JustifyContent::START,
        Justify::FlexEnd => taffy::JustifyContent::END,
        Justify::Center => taffy::JustifyContent::CENTER,
        Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
    }
}

fn display(d: Display) -> taffy::Display {
    match d {
        Display::Flex => taffy::Display::Flex,
        Display::Grid => taffy::Display::Grid,
        Display::Block => taffy::Display::Block,
        Display::None => taffy::Display::None,
    }
}

fn direction(d: FlexDirection) -> taffy::FlexDirection {
    match d {
        FlexDirection::Row => taffy::FlexDirection::Row,
        FlexDirection::Column => taffy::FlexDirection::Column,
        FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
        FlexDirection::ColumnReverse => taffy::FlexDirection::ColumnReverse,
    }
}

/// 将单个节点的描述符转换为 Taffy 样式。
#[must_use]
pub fn style_for(world: &UiWorld, id: NodeId) -> Style {
    let Some(node) = world.get(id) else {
        return Style::default();
    };
    let d = &node.desc;
    let clips_x = d.is_clipping() || d.has(NodeFlags::SCROLL_X);
    let clips_y = d.is_clipping() || d.has(NodeFlags::SCROLL_Y);
    let mut style = Style {
        display: display(d.display_kind()),
        flex_direction: direction(d.direction_kind()),
        align_items: align_items(d.align_items_kind()),
        align_self: align_self(d.align_self_kind()),
        justify_content: justify(d.justify_kind()),
        flex_wrap: if d.has(NodeFlags::WRAP) {
            taffy::FlexWrap::Wrap
        } else {
            taffy::FlexWrap::NoWrap
        },
        size: taffy::Size {
            width: dim(d.width),
            height: dim(d.height),
        },
        min_size: taffy::Size {
            width: inset(d.min_width),
            height: inset(d.min_height),
        },
        max_size: taffy::Size {
            width: inset(d.max_width),
            height: inset(d.max_height),
        },
        padding: taffy::Rect {
            left: fixed(d.padding.left),
            top: fixed(d.padding.top),
            right: fixed(d.padding.right),
            bottom: fixed(d.padding.bottom),
        },
        margin: taffy::Rect {
            left: inset(Dim::px(d.margin.left)),
            top: inset(Dim::px(d.margin.top)),
            right: inset(Dim::px(d.margin.right)),
            bottom: inset(Dim::px(d.margin.bottom)),
        },
        border: taffy::Rect {
            left: fixed(d.border_width),
            top: fixed(d.border_width),
            right: fixed(d.border_width),
            bottom: fixed(d.border_width),
        },
        gap: taffy::Size {
            width: LengthPercentage::length(d.gap_column),
            height: LengthPercentage::length(d.gap_row),
        },
        flex_grow: d.flex_grow.max(0.0),
        flex_shrink: d.flex_shrink.max(0.0),
        flex_basis: dim(d.flex_basis),
        position: match d.position_kind() {
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
        },
        inset: taffy::Rect {
            left: inset(Dim::px(d.inset.left)),
            top: inset(Dim::px(d.inset.top)),
            right: inset(Dim::px(d.inset.right)),
            bottom: inset(Dim::px(d.inset.bottom)),
        },
        overflow: taffy::Point {
            x: if clips_x {
                taffy::Overflow::Hidden
            } else {
                taffy::Overflow::Visible
            },
            y: if clips_y {
                taffy::Overflow::Hidden
            } else {
                taffy::Overflow::Visible
            },
        },
        ..Default::default()
    };
    if d.aspect_ratio > 0.0 {
        style.aspect_ratio = Some(d.aspect_ratio);
    }
    // `fr` 表示“可用空间的份额”，而非像素值。若将其直接映射为字面 `1.0` 像素，
    // 会导致 `Dim::fr(1.0)` 节点宽度只有 1 像素；份额应表示为 `flex-grow` 配合零基准。
    //
    let width_fr = d.width.kind() == DimUnit::Fr;
    let height_fr = d.height.kind() == DimUnit::Fr;
    if width_fr || height_fr {
        // 只有比例轴贡献份额；另一轴仍保持长度（例如 `40 px` 的高度不应变成拉伸因子）。
        //
        style.flex_grow = match (width_fr, height_fr) {
            (true, true) => d.width.value.max(d.height.value),
            (true, false) => d.width.value,
            (false, true) => d.height.value,
            (false, false) => unreachable!(),
        }
        .max(0.0);
        style.flex_basis = Dimension::length(0.0);
        if width_fr {
            style.size.width = Dimension::auto();
        }
        if height_fr {
            style.size.height = Dimension::auto();
        }
    }
    if d.display_kind() == Display::Grid {
        let columns = usize::from(d.grid_col_count).min(d.grid_cols.len());
        let rows = usize::from(d.grid_row_count).min(d.grid_rows.len());
        style.grid_template_columns = d.grid_cols[..columns]
            .iter()
            .copied()
            .map(grid_track)
            .collect();
        style.grid_template_rows = d.grid_rows[..rows]
            .iter()
            .copied()
            .map(grid_track)
            .collect();
        style.grid_column = grid_placement(d.grid_col_start, d.grid_col_span);
        style.grid_row = grid_placement(d.grid_row_start, d.grid_row_span);
    }
    style
}

/// 从 Taffy 已知尺寸解析叶子节点尺寸，若未知则回退到预测量固有尺寸（文本、图像），
/// 最后回退为零。
fn measure_leaf(input: &LayoutInput, id: Option<NodeId>, world: &UiWorld) -> LayoutOutput {
    let intrinsic = id
        .and_then(|id| world.get(id))
        .and_then(|n| n.measure)
        .unwrap_or(Vec2::ZERO);
    let width = input.known_dimensions.width.unwrap_or(intrinsic.x).max(0.0);
    let height = input
        .known_dimensions
        .height
        .unwrap_or(intrinsic.y)
        .max(0.0);
    LayoutOutput::from_outer_size(taffy::Size { width, height })
}

fn available(viewport: Vec2) -> taffy::Size<AvailableSpace> {
    taffy::Size {
        width: AvailableSpace::Definite(viewport.x.max(1.0)),
        height: AvailableSpace::Definite(viewport.y.max(1.0)),
    }
}

/// 节点自身样式在各轴上固定的尺寸。
///
/// 明确指定的尺寸由节点自身决定；自动尺寸则由其内容决定。
///
fn definite_size(world: &UiWorld, id: NodeId) -> (Option<f32>, Option<f32>) {
    let Some(node) = world.get(id) else {
        return (None, None);
    };
    let px = |d: Dim| {
        if d.unit == DimUnit::Px as u8 {
            Some(d.value)
        } else {
            None
        }
    };
    (px(node.desc.width), px(node.desc.height))
}

fn rect_of(layout: &Layout) -> Rect {
    Rect::new(
        layout.location.x,
        layout.location.y,
        layout.size.width,
        layout.size.height,
    )
}

/// 非递归的前序遍历，依次产出 `(node, children)`。
fn preorder(world: &UiWorld, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        let mut kids = Vec::new();
        let mut child = world.get(id).map(|n| n.first_child);
        while let Some(c) = child {
            if c.is_none() {
                break;
            }
            kids.push(c);
            child = world.get(c).map(|n| n.next_sibling);
        }
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 布局引擎
// ---------------------------------------------------------------------------

/// 基于 UI 世界树（UiWorld）的持久化布局引擎。
pub struct LayoutEngine {
    taffy: TaffyTree<NodeId>,
    map: FxHashMap<NodeId, taffy::NodeId>,
    rects: FxHashMap<NodeId, Rect>,
    /// 上一次 `collect` 访问过的节点列表，在多次求解间复用以减少分配。
    changed: Vec<NodeId>,
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for LayoutEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LayoutEngine")
            .field("nodes", &self.map.len())
            .finish_non_exhaustive()
    }
}

impl LayoutEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            map: FxHashMap::default(),
            rects: FxHashMap::default(),
            changed: Vec::new(),
        }
    }

    /// 丢弃所有缓存节点（用于世界重建或视口重置）。
    pub fn clear(&mut self) {
        self.taffy = TaffyTree::new();
        self.map.clear();
        self.rects.clear();
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.map.len()
    }

    /// 创建缺失的 Taffy 节点并刷新所有样式。
    ///
    /// 样式写入是无条件的，因为做样式差异对比的开销与直接写入相当；
    /// 真正重要的脏跟踪发生在*上层*，由上层决定是否需要调用本方法。
    ///
    pub fn sync(&mut self, world: &UiWorld, root: NodeId) {
        let order = preorder(world, root);
        // 第一阶段：确保每个节点都存在并携带正确的样式。
        for id in &order {
            let style = style_for(world, *id);
            match self.map.get(id) {
                Some(t) => {
                    let _ = self.taffy.set_style(*t, style);
                }
                None => {
                    let t = self
                        .taffy
                        .new_leaf_with_context(style, *id)
                        .expect("taffy node allocation");
                    self.map.insert(*id, t);
                }
            }
        }
        // 第二阶段：在所有节点均已创建后挂接子节点。
        for id in &order {
            let Some(t) = self.map.get(id).copied() else {
                continue;
            };
            let kids: Vec<taffy::NodeId> = world
                .children(*id)
                .filter_map(|c| self.map.get(&c).copied())
                .collect();
            let _ = self.taffy.set_children(t, &kids);
        }
        // 第三阶段：移除已离开世界树的节点。
        //
        // `UiWorld::remove` 会释放槽位，但引擎此前会永久保留对应的 Taffy 节点、
        // 映射表项与缓存矩形：长时间运行中反复创建和销毁组件会导致每移除一个节点
        // 就泄漏一份上述数据。
        //
        self.map.retain(|id, taffy_id| {
            let alive = world.contains(*id);
            if !alive {
                let _ = self.taffy.remove(*taffy_id);
                self.rects.remove(id);
            }
            alive
        });
    }

    /// 在指定 `viewport` 下执行全量重算（用于拓扑变更或视口调整）。
    pub fn solve(&mut self, world: &mut UiWorld, root: NodeId, viewport: Vec2) -> LayoutStats {
        let sync_start = std::time::Instant::now();
        self.sync(world, root);
        let sync_us = sync_start.elapsed().as_micros() as u64;

        let Some(taffy_root) = self.map.get(&root).copied() else {
            return LayoutStats {
                sync_us,
                ..Default::default()
            };
        };

        let start = std::time::Instant::now();
        {
            // 拆分借用：Taffy 被可变借用，世界树仅只读借用。
            let world_ref: &UiWorld = world;
            let taffy = &mut self.taffy;
            let _ = taffy.compute_layout_with_measure(
                taffy_root,
                available(viewport),
                |input: LayoutInput,
                 _node: taffy::NodeId,
                 ctx: Option<&mut NodeId>,
                 _style: &Style| {
                    // Taffy 可能对其带上下文创建的任何节点调用测量函数（包括普通容器）。
                    // 它传入的已知尺寸具有权威性：若忽略它们，会导致所有固定尺寸的叶子节点
                    // 被静默折叠为 0。
                    //
                    measure_leaf(&input, ctx.map(|c| *c), world_ref)
                },
            );
        }
        let taffy_us = start.elapsed().as_micros() as u64;

        let visited = self.collect(world, root);
        world.apply_rects(&self.rects);
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: 1,
            taffy_us,
            sync_us,
        }
    }

    /// 执行全量重算，并允许调用方根据 Taffy 实际提供的宽度测量自动换行文本。
    ///
    ///
    /// 无文本或未启用 [`NodeFlags::WRAP`] 的节点继续使用预测量固有尺寸。
    /// 对于需要换行的节点，会以已知宽度（若有）及 Taffy 可用空间调用回调；
    /// 返回的尺寸会写回 `NodeData::measure`，以便调试覆盖层与后续帧获取解析后的段落尺寸。
    ///
    ///
    pub fn solve_with_measurer<F>(
        &mut self,
        world: &mut UiWorld,
        root: NodeId,
        viewport: Vec2,
        mut measure_fn: F,
    ) -> LayoutStats
    where
        F: FnMut(NodeId, Option<f32>, taffy::Size<AvailableSpace>) -> Vec2,
    {
        let sync_start = std::time::Instant::now();
        self.sync(world, root);
        let sync_us = sync_start.elapsed().as_micros() as u64;

        let Some(taffy_root) = self.map.get(&root).copied() else {
            return LayoutStats {
                sync_us,
                ..Default::default()
            };
        };

        let start = std::time::Instant::now();
        let mut measured: Vec<(NodeId, Vec2)> = Vec::new();
        {
            let world_ref: &UiWorld = world;
            let taffy = &mut self.taffy;
            let _ = taffy.compute_layout_with_measure(
                taffy_root,
                available(viewport),
                |input: LayoutInput,
                 _node: taffy::NodeId,
                 ctx: Option<&mut NodeId>,
                 _style: &Style| {
                    let Some(id) = ctx.map(|c| *c) else {
                        return measure_leaf(&input, None, world_ref);
                    };
                    let wraps = world_ref
                        .get(id)
                        .is_some_and(|node| node.desc.has(NodeFlags::WRAP))
                        && world_ref.text(id).is_some();
                    if !wraps {
                        return measure_leaf(&input, Some(id), world_ref);
                    }
                    let known_width = input.known_dimensions.width.or(input.parent_size.width);
                    let intrinsic = measure_fn(id, known_width, input.available_space);
                    measured.push((id, intrinsic));
                    let width = input.known_dimensions.width.unwrap_or(intrinsic.x).max(0.0);
                    let height = input
                        .known_dimensions
                        .height
                        .unwrap_or(intrinsic.y)
                        .max(0.0);
                    LayoutOutput::from_outer_size(taffy::Size { width, height })
                },
            );
        }
        for (id, size) in measured {
            if let Some(node) = world.get_mut(id) {
                node.measure = Some(size);
            }
        }
        let taffy_us = start.elapsed().as_micros() as u64;

        let visited = self.collect(world, root);
        world.apply_rects(&self.rects);
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: 1,
            taffy_us,
            sync_us,
        }
    }

    /// 仅重新求解指定的重布局边界子树。
    ///
    /// 单个边界直接使用持久化树（完全零分配）。多个边界由于结构上互不相交，
    /// 各自在私有树中并行求解。
    ///
    pub fn solve_boundaries(
        &mut self,
        world: &mut UiWorld,
        boundaries: &[NodeId],
        viewport: Vec2,
    ) -> LayoutStats {
        if boundaries.is_empty() {
            return LayoutStats::default();
        }
        if boundaries.len() == 1 {
            return self.solve_subtree(world, boundaries[0], viewport);
        }

        let start = std::time::Instant::now();
        let nested: Vec<Vec<(NodeId, Rect)>> = {
            use rayon::prelude::*;
            boundaries
                .par_iter()
                .map(|b| {
                    // 该边界作为根节点求解，因此为其传入原有空间：
                    // 使 `width: 100%` 的子节点能正确相对于包含块解析。
                    //
                    let size = world.node(*b).rect.size;
                    let space = Vec2::new(size.x.max(viewport.x), size.y.max(viewport.y));
                    solve_isolated(world, *b, space)
                })
                .collect()
        };
        let taffy_us = start.elapsed().as_micros() as u64;

        let mut visited = 0u32;
        for (chunk, boundary) in nested.into_iter().zip(boundaries.iter().copied()) {
            let placement = world.node(boundary).rect;
            let changed: Vec<NodeId> = chunk.iter().map(|(id, _)| *id).collect();
            visited += chunk.len() as u32;
            for (id, r) in chunk {
                self.rects.insert(id, r);
            }
            // 子树以该边界为根求解，因此边界自身的矩形位置会回到原点——
            // 但在完整树中它位于父节点放置的位置。保留父节点给定的位置，
            // 仅从本次求解中提取尺寸。
            //
            let (fixed_width, fixed_height) = definite_size(world, boundary);
            if let Some(rect) = self.rects.get_mut(&boundary) {
                rect.origin = placement.origin;
                // 样式固定的尺寸属于节点自身：Taffy 会根据内容计算根节点尺寸，
                // 因此必须恢复样式固定的尺寸。
                //
                if let Some(width) = fixed_width {
                    rect.size.x = width;
                }
                if let Some(height) = fixed_height {
                    rect.size.y = height;
                }
            }
            world.apply_rects_from(&self.rects, &changed, boundary);
        }
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: boundaries.len() as u32,
            taffy_us,
            sync_us: 0,
        }
    }

    /// 在持久化树上重新求解单个子树。
    pub fn solve_subtree(
        &mut self,
        world: &mut UiWorld,
        boundary: NodeId,
        viewport: Vec2,
    ) -> LayoutStats {
        if !self.map.contains_key(&boundary) {
            // 未知子树：全量求解是唯一正确的做法。
            let root = world.root();
            return self.solve(world, root, viewport);
        }
        let sync_start = std::time::Instant::now();
        self.sync(world, boundary);
        let sync_us = sync_start.elapsed().as_micros() as u64;

        let taffy_root = self.map[&boundary];
        // Taffy 会根据子节点计算*根*节点尺寸，因此必须以边界原有的空间作为约束求解子树：
        // 这样 `width: 100%` 的子节点才能相对于包含块解析，
        // 否则将无参照可依并塌缩为零。
        //
        let space = self.rects.get(&boundary).map_or(viewport, |r| r.size);
        let start = std::time::Instant::now();
        {
            let world_ref: &UiWorld = world;
            let taffy = &mut self.taffy;
            let _ = taffy.compute_layout_with_measure(
                taffy_root,
                available(space),
                |input: LayoutInput,
                 _node: taffy::NodeId,
                 ctx: Option<&mut NodeId>,
                 _style: &Style| {
                    // Taffy 可能对其带上下文创建的任何节点调用测量函数（包括普通容器）。
                    // 它传入的已知尺寸具有权威性：若忽略它们，会导致所有固定尺寸的叶子节点
                    // 被静默折叠为 0。
                    //
                    measure_leaf(&input, ctx.map(|c| *c), world_ref)
                },
            );
        }
        let taffy_us = start.elapsed().as_micros() as u64;

        let visited = self.collect(world, boundary);
        let changed = core::mem::take(&mut self.changed);
        // 子树以该边界为根求解，因此边界自身的矩形位置会回到原点——
        // 但在完整树中它位于父节点放置的位置。保留父节点给定的位置，
        // 仅从本次求解中提取尺寸。
        //
        let placement = world.node(boundary).rect;
        let (fixed_width, fixed_height) = definite_size(world, boundary);
        if let Some(rect) = self.rects.get_mut(&boundary) {
            rect.origin = placement.origin;
            // 样式固定的尺寸属于节点自身：Taffy 会根据内容计算根节点尺寸，
            // 因此必须恢复样式固定的尺寸。
            if let Some(width) = fixed_width {
                rect.size.x = width;
            }
            if let Some(height) = fixed_height {
                rect.size.y = height;
            }
        }
        world.apply_rects_from(&self.rects, &changed, boundary);
        self.changed = changed;
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: 1,
            taffy_us,
            sync_us,
        }
    }

    /// 将 `root` 子树的布局结果读入 `self.rects`。
    ///
    /// 该映射表*不会*被清空：增量求解仅刷新其触及的节点，
    /// 若丢弃未触及的几何信息，[`Self::rect`] 将无法回答树中其余节点的位置——
    /// 而这正是绘制帧的调用方所需要的。
    ///
    fn collect(&mut self, world: &UiWorld, root: NodeId) -> u32 {
        self.changed.clear();
        let mut visited = 0u32;
        for id in preorder(world, root) {
            if let Some(t) = self.map.get(&id) {
                if let Ok(layout) = self.taffy.layout(*t) {
                    self.rects.insert(id, rect_of(layout));
                    self.changed.push(id);
                    visited += 1;
                }
            }
        }
        visited
    }

    /// 节点上一次计算得到的矩形（位于父节点坐标空间内）。
    ///
    /// Taffy 报告节点在父节点中的位置，因此该矩形是相对坐标——
    /// 这也是 [`unbound_world::UiWorld::apply_rects`] 写回世界树的空间。
    /// 若调用方需要绝对坐标，请使用 [`Self::world_rect`]。
    ///
    #[must_use]
    pub fn rect(&self, id: NodeId) -> Option<Rect> {
        self.rects.get(&id).copied()
    }

    /// 节点上一次计算得到的矩形（绝对窗口坐标）。
    ///
    /// 世界树在应用求解结果时会将所有祖先节点的偏移量折叠进 `world_rect`，
    /// 因此这正是绘制器所需的矩形。
    #[must_use]
    pub fn world_rect(&self, world: &UiWorld, id: NodeId) -> Option<Rect> {
        world.get(id).map(|node| node.world_rect)
    }
}

/// 在私有 Taffy 树中求解 `root` 的子树。
///
/// 返回该子树的矩形映射，由调用方合并回世界树。这是并行计算的基本单元：
/// 无共享状态，也不要求 `TaffyTree` 实现 `Sync`。
fn solve_isolated(world: &UiWorld, root: NodeId, viewport: Vec2) -> Vec<(NodeId, Rect)> {
    let order = preorder(world, root);
    if order.is_empty() {
        return Vec::new();
    }
    let mut tree: TaffyTree<NodeId> = TaffyTree::new();
    let mut created: FxHashMap<NodeId, taffy::NodeId> =
        FxHashMap::with_capacity_and_hasher(order.len(), Default::default());
    for id in &order {
        let t = tree
            .new_leaf_with_context(style_for(world, *id), *id)
            .expect("leaf");
        created.insert(*id, t);
    }
    for id in &order {
        let kids: Vec<taffy::NodeId> = world
            .children(*id)
            .filter_map(|c| created.get(&c).copied())
            .collect();
        let _ = tree.set_children(created[id], &kids);
    }
    let root_t = created[&root];
    let _ = tree.compute_layout_with_measure(
        root_t,
        available(viewport),
        |input: LayoutInput, _node: taffy::NodeId, ctx: Option<&mut NodeId>, _style: &Style| {
            measure_leaf(&input, ctx.map(|c| *c), world)
        },
    );
    let mut out = Vec::with_capacity(order.len());
    for id in order {
        if let Ok(l) = tree.layout(created[&id]) {
            out.push((id, rect_of(l)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::{Edges, RawNodeDesc};

    fn loc() -> &'static core::panic::Location<'static> {
        core::panic::Location::caller()
    }

    /// 构建测试树：根节点（400x300）内含一个固定 200x100 的卡片及标签。
    fn fixture() -> (UiWorld, NodeId, NodeId, NodeId) {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(300.0);
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(200.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(
            card,
            RawNodeDesc::text().flags(NodeFlags::MEASURE_TEXT),
            loc(),
        );
        w.node_mut(label).measure = Some(Vec2::new(80.0, 16.0));
        (w, root, card, label)
    }

    /// 以边界为根重算子树时，边界自身的矩形位置会回到原点——
    /// 但在完整树中，边界仍位于父节点放置它的位置。
    ///
    ///
    /// 丢失该位置曾导致重算后的卡片跳回左上角并覆盖在兄弟节点之上。
    ///
    #[test]
    fn incremental_boundary_solve_preserves_the_boundary_origin() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(300.0);
        w.node_mut(root).desc.gap_row = 8.0;
        let first = w.insert(root, RawNodeDesc::container().size(200.0, 50.0), loc());
        let second = w.insert(root, RawNodeDesc::container().size(200.0, 50.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let placement = engine.rect(second).expect("rect").origin;
        assert!(placement.y > 0.0, "the second row is not at the top");

        // 单独重新求解第二行。
        engine.solve_subtree(&mut w, second, Vec2::new(400.0, 300.0));
        let after = engine.rect(second).expect("rect");
        assert_eq!(
            after.origin, placement,
            "the boundary must keep the position its parent gave it"
        );
        assert_eq!(
            after.size,
            Vec2::new(200.0, 50.0),
            "the size is still taken"
        );
        assert_ne!(first, second);
    }

    /// 离开世界树的节点绝不能残留在引擎的内部表中。
    #[test]
    fn sync_removes_deleted_world_nodes_from_taffy() {
        let (mut w, root, card, _label) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let before = engine.node_count();
        assert!(before >= 3);

        w.remove(card);
        engine.sync(&w, root);
        assert!(
            engine.node_count() < before,
            "the removed node must not stay in the engine: {} vs {before}",
            engine.node_count()
        );
        assert!(engine.rect(card).is_none());
    }

    #[test]
    fn a_fixed_card_gets_its_exact_size() {
        let (mut w, root, card, _label) = fixture();
        let mut engine = LayoutEngine::new();
        let stats = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert!(stats.nodes_visited >= 3);
        let rect = engine.rect(card).expect("card rect");
        assert_eq!(rect.size, Vec2::new(200.0, 100.0));
        assert_eq!(rect.origin, Vec2::ZERO);
        assert_eq!(w.node(card).rect.size, Vec2::new(200.0, 100.0));
    }

    #[test]
    fn column_flow_stacks_children_with_a_gap() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(200.0);
        w.node_mut(root).desc.gap_row = 8.0;
        let a = w.insert(root, RawNodeDesc::container().size(50.0, 20.0), loc());
        let b = w.insert(root, RawNodeDesc::container().size(50.0, 20.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 200.0));
        assert_eq!(engine.rect(a).unwrap().origin, Vec2::new(0.0, 0.0));
        assert_eq!(engine.rect(b).unwrap().origin, Vec2::new(0.0, 28.0));
    }

    #[test]
    fn row_flow_uses_the_column_gap() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(40.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        w.node_mut(root).desc.gap_column = 6.0;
        let a = w.insert(root, RawNodeDesc::container().size(30.0, 10.0), loc());
        let b = w.insert(root, RawNodeDesc::container().size(30.0, 10.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 40.0));
        assert_eq!(engine.rect(a).unwrap().origin, Vec2::new(0.0, 0.0));
        assert_eq!(engine.rect(b).unwrap().origin, Vec2::new(36.0, 0.0));
    }

    #[test]
    fn flex_grow_distributes_the_remaining_space() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(300.0);
        w.node_mut(root).desc.height = Dim::px(50.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let mut left = RawNodeDesc::container();
        left.width = Dim::px(100.0);
        left.height = Dim::px(10.0);
        let a = w.insert(root, left, loc());
        let mut right = RawNodeDesc::container();
        right.width = Dim::px(0.0);
        right.height = Dim::px(10.0);
        right.flex_grow = 1.0;
        let b = w.insert(root, right, loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(300.0, 50.0));
        assert_eq!(engine.rect(a).unwrap().size.x, 100.0);
        assert_eq!(engine.rect(b).unwrap().size.x, 200.0);
    }

    #[test]
    fn padding_shifts_the_content_box() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(100.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        w.node_mut(root).desc.padding = Edges::all(10.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let child = w.insert(root, RawNodeDesc::container().size(20.0, 20.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(100.0, 100.0));
        assert_eq!(engine.rect(child).unwrap().origin, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn percentage_sizes_resolve_against_the_parent() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let half = w.insert(
            root,
            RawNodeDesc::container()
                .width(Dim::percent(50.0))
                .height(Dim::px(10.0)),
            loc(),
        );
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 100.0));
        assert_eq!(engine.rect(half).unwrap().size.x, 100.0);
    }

    #[test]
    fn measured_text_sets_the_leaf_size() {
        let (mut w, root, card, label) = fixture();
        // 拉伸列会将标签拓宽至卡片内容盒宽度（符合 Flex 行为）；
        // 此处将其固定为固有宽度。
        w.node_mut(card).desc.align_items = Align::FlexStart as u8;
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let rect = engine.rect(label).expect("label rect");
        assert_eq!(rect.size, Vec2::new(80.0, 16.0));
    }

    #[test]
    fn absolute_positioning_uses_insets() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(200.0);
        let mut abs = RawNodeDesc::container().size(40.0, 40.0).absolute();
        abs.inset = Edges::trbl(10.0, 20.0, 0.0, 0.0);
        let child = w.insert(root, abs, loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 200.0));
        assert_eq!(engine.rect(child).unwrap().origin, Vec2::new(10.0, 20.0));
    }

    #[test]
    fn display_none_removes_the_node_from_flow() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(100.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        let a = w.insert(root, RawNodeDesc::container().size(10.0, 10.0), loc());
        let hidden = w.insert(
            root,
            RawNodeDesc::container()
                .size(10.0, 10.0)
                .display(unbound_abi::Display::None),
            loc(),
        );
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(100.0, 100.0));
        assert_eq!(engine.rect(hidden).unwrap().size, Vec2::ZERO);
        assert_eq!(engine.rect(a).unwrap().size, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn incremental_solve_touches_only_the_boundary() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(600.0);
        w.node_mut(root).desc.height = Dim::px(400.0);
        let cards: Vec<NodeId> = (0..4)
            .map(|_| {
                w.insert(
                    root,
                    RawNodeDesc::container()
                        .size(100.0, 100.0)
                        .flags(NodeFlags::CLIP),
                    loc(),
                )
            })
            .collect();
        let labels: Vec<NodeId> = cards
            .iter()
            .map(|c| {
                let l = w.insert(*c, RawNodeDesc::text(), loc());
                w.node_mut(l).measure = Some(Vec2::new(40.0, 12.0));
                l
            })
            .collect();

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(600.0, 400.0));
        w.clear_dirty();

        // 修改一个标签并仅求解其所在卡片。
        w.mark_layout_dirty(labels[1]);
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![cards[1]]);
        let stats = engine.solve_boundaries(&mut w, &boundaries, Vec2::new(600.0, 400.0));
        assert!(
            stats.nodes_visited <= 2,
            "only the card and its label should be visited, got {}",
            stats.nodes_visited
        );
        // 其他卡片保持其几何位置（通过世界树读取，因为 `engine.rect`
        // 仅报告最近一次求解的结果）。
        assert_eq!(w.node(cards[0]).rect.size, Vec2::new(100.0, 100.0));
    }

    #[test]
    fn multiple_boundaries_solve_in_parallel_with_identical_results() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(800.0);
        w.node_mut(root).desc.height = Dim::px(600.0);
        let cards: Vec<NodeId> = (0..6)
            .map(|_| {
                w.insert(
                    root,
                    RawNodeDesc::container()
                        .size(120.0, 80.0)
                        .flags(NodeFlags::CLIP),
                    loc(),
                )
            })
            .collect();
        for c in &cards {
            let l = w.insert(*c, RawNodeDesc::text(), loc());
            w.node_mut(l).measure = Some(Vec2::new(60.0, 14.0));
        }

        let mut full = LayoutEngine::new();
        full.solve(&mut w, root, Vec2::new(800.0, 600.0));
        let expected: Vec<Rect> = cards.iter().map(|c| full.rect(*c).unwrap()).collect();

        let mut parallel = LayoutEngine::new();
        parallel.sync(&w, root);
        let stats = parallel.solve_boundaries(&mut w, &cards, Vec2::new(800.0, 600.0));
        assert_eq!(stats.boundaries_solved, cards.len() as u32);
        for (card, want) in cards.iter().zip(expected) {
            let got = parallel.rect(*card).unwrap();
            assert_eq!(got.size, want.size, "size mismatch for {card:?}");
        }
    }

    #[test]
    fn solving_twice_is_idempotent() {
        let (mut w, root, card, _) = fixture();
        let mut engine = LayoutEngine::new();
        let first = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let r1 = engine.rect(card).unwrap();
        let second = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert_eq!(engine.rect(card).unwrap(), r1);
        assert_eq!(first.nodes_visited, second.nodes_visited);
    }

    #[test]
    fn clear_resets_the_engine() {
        let (mut w, root, _, _) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert!(engine.node_count() > 0);
        engine.clear();
        assert_eq!(engine.node_count(), 0);
        assert!(engine.rect(root).is_none());
    }

    #[test]
    fn text_node_inside_a_card_does_not_resize_the_card() {
        let (mut w, root, card, label) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        // 将标签设得极大；固定尺寸的卡片不得随之变大。
        w.node_mut(label).measure = Some(Vec2::new(900.0, 400.0));
        w.mark_layout_dirty(label);
        let boundaries = w.dirty_boundaries();
        engine.solve_boundaries(&mut w, &boundaries, Vec2::new(400.0, 300.0));
        assert_eq!(engine.rect(card).unwrap().size, Vec2::new(200.0, 100.0));
    }

    #[test]
    fn tag_specific_style_is_stable() {
        let mut w = UiWorld::new();
        let root = w.root();
        let t = w.insert(
            root,
            RawNodeDesc::text().flags(NodeFlags::MEASURE_TEXT),
            loc(),
        );
        let style = style_for(&w, t);
        assert_eq!(style.display, taffy::Display::Flex);
        assert_eq!(style.size.width, Dimension::auto());
    }

    /// 增量求解绝不能移动其所求解的边界本身。
    ///
    /// 子树以边界为*根*重新求解，因此边界自身的矩形回到原点——
    /// 而在树中它位于父节点放置的位置。若直接将该矩形写回，
    /// 会把卡片移至父节点左上角，随后的裁剪折叠也会传播错误位置。
    ///
    ///
    #[test]
    fn an_incremental_solve_keeps_the_boundary_where_it_was() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(600.0);
        w.node_mut(root).desc.height = Dim::px(400.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let first = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let second = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(second, RawNodeDesc::text(), loc());
        w.node_mut(label).measure = Some(Vec2::new(40.0, 12.0));

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(600.0, 400.0));
        let before = engine.rect(second).expect("second card");
        assert_eq!(
            before.origin,
            Vec2::new(100.0, 0.0),
            "the card is not at the origin"
        );

        // 修改标签并仅重新求解第二个卡片的边界。
        w.node_mut(label).measure = Some(Vec2::new(80.0, 12.0));
        w.mark_layout_dirty(label);
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![second]);
        engine.solve_boundaries(&mut w, &boundaries, Vec2::new(600.0, 400.0));

        let after = engine.rect(second).expect("second card");
        assert_eq!(
            after.origin, before.origin,
            "the incremental solve moved the boundary to the origin"
        );
        assert_eq!(w.node(second).rect.origin, before.origin);
        assert_eq!(w.node(first).rect.origin, Vec2::new(0.0, 0.0));
    }

    /// 长列表场景下表现出的同一缺陷。
    ///
    /// 画廊布局了 200 行并重算中间某一行。修复前该行会被瞬移到列表顶部——
    /// 它落入*视口内部*，导致空间索引多报告了一行：
    /// 一行被绘制在第一行之上，而原本属于该位置的行则消失了。
    ///
    ///
    #[test]
    fn a_row_re_solved_inside_a_list_stays_at_its_index() {
        const ROWS: usize = 200;
        const ROW_HEIGHT: f32 = 32.0;

        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(1280.0);
        w.node_mut(root).desc.height = Dim::px(720.0);
        let list = w.insert(
            root,
            RawNodeDesc::text().size(1280.0, 600.0).column(),
            loc(),
        );
        let mut rows = Vec::with_capacity(ROWS);
        for _ in 0..ROWS {
            let mut desc = RawNodeDesc::text().size(1280.0, ROW_HEIGHT).row();
            desc.flex_shrink = 0.0;
            rows.push(w.insert(list, desc, loc()));
        }

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(1280.0, 720.0));
        let middle = rows[ROWS / 2];
        let expected = w.node(middle).rect.origin;
        assert_eq!(
            expected,
            Vec2::new(0.0, (ROWS / 2) as f32 * ROW_HEIGHT),
            "the row must start at its own index"
        );

        // 像真实编辑那样重算中间行的边界：画廊直接将该行交给引擎处理。
        //
        engine.solve_boundaries(&mut w, &[middle], Vec2::new(1280.0, 720.0));

        assert_eq!(
            w.node(middle).rect.origin,
            expected,
            "the re-solved row was teleported to the top of the list"
        );

        // 原本位于顶部的行必须仍然留在顶部。
        assert_eq!(
            w.node(rows[0]).rect.origin,
            Vec2::new(0.0, 0.0),
            "the first row must not have been displaced"
        );
    }

    /// `fr` 表示可用空间的份额，而非像素值。
    #[test]
    fn a_fractional_size_grows_instead_of_collapsing_to_one_pixel() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(80.0);
        w.node_mut(root).desc.direction = FlexDirection::Row as u8;
        let mut first_desc = RawNodeDesc::container();
        first_desc.width = Dim::fr(1.0);
        first_desc.height = Dim::px(40.0);
        let mut second_desc = RawNodeDesc::container();
        second_desc.width = Dim::fr(2.0);
        second_desc.height = Dim::px(40.0);
        let first = w.insert(root, first_desc, loc());
        let second = w.insert(root, second_desc, loc());

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 80.0));
        let a = engine.rect(first).expect("rect").size.x;
        let b = engine.rect(second).expect("rect").size.x;
        assert!(a > 100.0, "1fr must not collapse to a pixel, got {a}");
        assert!(
            (b - 2.0 * a).abs() <= 1.0,
            "2fr must take twice 1fr: {a} / {b}"
        );
        assert!((a + b - 400.0).abs() <= 1.0, "{a} + {b} must fill the row");
    }

    /// `Align::Baseline` 必须作为基线对齐传递给 Taffy。
    #[test]
    fn baseline_alignment_is_not_folded_into_flex_start() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        w.node_mut(root).desc.direction = FlexDirection::Row as u8;
        w.node_mut(root).desc.align_items = Align::Baseline as u8;

        let mut tall_desc = RawNodeDesc::text();
        tall_desc.width = Dim::px(100.0);
        tall_desc.height = Dim::px(60.0);
        let tall = w.insert(root, tall_desc, loc());
        let mut short_desc = RawNodeDesc::text();
        short_desc.width = Dim::px(100.0);
        short_desc.height = Dim::px(20.0);
        let short = w.insert(root, short_desc, loc());

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 100.0));
        let a = engine.rect(tall).expect("rect");
        let b = engine.rect(short).expect("rect");
        assert!(
            b.origin.y > a.origin.y,
            "the shorter box must be pushed down onto the shared baseline"
        );
    }
}

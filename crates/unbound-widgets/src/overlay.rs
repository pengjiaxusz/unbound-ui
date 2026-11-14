//! 浮层定位与弹窗栈管理。
//!
//! 弹窗面临两个控件层其余部分没有的问题：
//!
//! 1. **绝不能超出窗口。** [`place_anchored_popup`] 与 [`place_tooltip_in_window`] 是纯几何方案：
//!    优先选择请求方向，空间不足时自动翻转（含主轴翻转与正交轴回退），最后将浮层钳制回视口安全边距内。
//! 2. **必须按正确顺序堆叠、关闭与清理。** [`OverlayStack`] 分配单调递增的排序键，
//!    在按下 `Escape` 或点击外部时关闭顶层，并在同一帧内清除锚点已消失的条目——
//!    弹窗绝不能比其触发器存活更久而留下幽灵命中区。

use unbound_gpu::data::PassLayer;
use unbound_math::{Rect, Vec2};

/// 锚点与弹窗之间的默认间距（逻辑像素）。
pub const POPUP_GAP: f32 = 4.0;

/// 工具提示距离窗口视口四边的默认安全内边距（逻辑像素）。
pub const TOOLTIP_VIEWPORT_MARGIN: f32 = 8.0;

/// 弹窗相对于锚点的期望出现位置。
///
/// 十二个变体覆盖四个方向及其起始、居中与末尾对齐。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// 位于锚点下方，与起始边对齐。
    #[default]
    BottomStart,
    /// 位于锚点下方，与末尾边对齐。
    BottomEnd,
    /// 位于锚点下方，水平居中。
    Bottom,
    /// 位于锚点上方，与起始边对齐。
    TopStart,
    /// 位于锚点上方，与末尾边对齐。
    TopEnd,
    /// 位于锚点上方，水平居中。
    Top,
    /// 位于锚点右侧，与顶边对齐。
    RightStart,
    /// 位于锚点右侧，与底边对齐。
    RightEnd,
    /// 位于锚点右侧，垂直居中。
    Right,
    /// 位于锚点左侧，与顶边对齐。
    LeftStart,
    /// 位于锚点左侧，与底边对齐。
    LeftEnd,
    /// 位于锚点左侧，垂直居中。
    Left,
}

impl Placement {
    /// 翻转后使用的对向位置。
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Self::BottomStart => Self::TopStart,
            Self::BottomEnd => Self::TopEnd,
            Self::Bottom => Self::Top,
            Self::TopStart => Self::BottomStart,
            Self::TopEnd => Self::BottomEnd,
            Self::Top => Self::Bottom,
            Self::RightStart => Self::LeftStart,
            Self::RightEnd => Self::LeftEnd,
            Self::Right => Self::Left,
            Self::LeftStart => Self::RightStart,
            Self::LeftEnd => Self::RightEnd,
            Self::Left => Self::Right,
        }
    }

    const fn vertical(self) -> bool {
        matches!(
            self,
            Self::BottomStart
                | Self::BottomEnd
                | Self::Bottom
                | Self::TopStart
                | Self::TopEnd
                | Self::Top
        )
    }

    const fn below(self) -> bool {
        matches!(self, Self::BottomStart | Self::BottomEnd | Self::Bottom)
    }

    const fn right(self) -> bool {
        matches!(self, Self::RightStart | Self::RightEnd | Self::Right)
    }

    /// 水平放置且与锚点末尾边对齐。
    const fn aligned_to_end(self) -> bool {
        matches!(self, Self::BottomEnd | Self::TopEnd)
    }

    /// 侧边放置且与锚点底边对齐。
    const fn aligned_y_to_end(self) -> bool {
        matches!(self, Self::RightEnd | Self::LeftEnd)
    }

    const fn centered(self) -> bool {
        matches!(self, Self::Bottom | Self::Top | Self::Right | Self::Left)
    }
}

/// 已放置弹窗的几何信息。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlacedPopup {
    /// 最终弹窗矩形（始终位于视口内）。
    pub rect: Rect,
    /// 弹窗实际落定的方向（请求方向，或在溢出时翻转后的方向）。
    pub chosen_placement: Placement,
    /// 弹窗边缘最靠近锚点的点，即箭头尖端位置。
    pub arrow_tip: Vec2,
}

/// 将弹窗放置在锚点旁，始终保持在视口内。
///
/// 返回的矩形不会大于视口：放不下的弹窗会先缩小再定位。
/// 翻转与平移确保结果对边缘或近边缘锚点均满足 `Rule::ViewportContainment`。
///
/// 新调用方可将结果绑定为 [`PlacedPopup`] 以获取选定方向与箭头尖端。
/// 仅需矩形的现有调用点仍可正常工作，因为实现了 `Rect: From<PlacedPopup>`：
///
/// ```
/// # use unbound_math::{Rect, Vec2};
/// # use unbound_widgets::overlay::{place_anchored_popup, Placement, PlacedPopup};
/// let anchor = Rect::new(10.0, 10.0, 40.0, 20.0);
/// let placed: PlacedPopup =
///     place_anchored_popup(anchor, Vec2::new(80.0, 60.0), anchor, Placement::Bottom);
/// assert_eq!(placed.chosen_placement, Placement::Bottom);
/// ```
#[must_use]
pub fn place_anchored_popup<T: From<PlacedPopup>>(
    anchor: Rect,
    popup_size: Vec2,
    viewport: Rect,
    placement: Placement,
) -> T {
    T::from(place_anchored_popup_detailed(
        anchor, popup_size, viewport, placement,
    ))
}

/// [`place_anchored_popup`] 的详细形式。
#[must_use]
pub fn place_anchored_popup_detailed(
    anchor: Rect,
    popup_size: Vec2,
    viewport: Rect,
    placement: Placement,
) -> PlacedPopup {
    let width = popup_size.x.max(0.0).min(viewport.width().max(0.0));
    let height = popup_size.y.max(0.0).min(viewport.height().max(0.0));

    let mut chosen = placement;
    let mut origin = candidate_origin(anchor, Vec2::new(width, height), placement, POPUP_GAP);

    // 当首选方向溢出且对向方向放得下时翻转。
    if placement.vertical() {
        let overflows = if placement.below() {
            origin.y + height > viewport.max().y
        } else {
            origin.y < viewport.origin.y
        };
        if overflows {
            let flipped_y = if placement.below() {
                anchor.origin.y - POPUP_GAP - height
            } else {
                anchor.max().y + POPUP_GAP
            };
            if flipped_y >= viewport.origin.y && flipped_y + height <= viewport.max().y {
                origin.y = flipped_y;
                chosen = placement.flip();
            }
        }
    } else {
        let overflows = if placement.right() {
            origin.x + width > viewport.max().x
        } else {
            origin.x < viewport.origin.x
        };
        if overflows {
            let flipped_x = if placement.right() {
                anchor.origin.x - POPUP_GAP - width
            } else {
                anchor.max().x + POPUP_GAP
            };
            if flipped_x >= viewport.origin.x && flipped_x + width <= viewport.max().x {
                origin.x = flipped_x;
                chosen = placement.flip();
            }
        }
    }

    // 将剩余溢出部分移回视口内。
    let max_x = (viewport.max().x - width).max(viewport.origin.x);
    let max_y = (viewport.max().y - height).max(viewport.origin.y);
    origin.x = origin.x.clamp(viewport.origin.x, max_x);
    origin.y = origin.y.clamp(viewport.origin.y, max_y);

    let rect = Rect::new(origin.x, origin.y, width, height);
    let arrow_tip = compute_arrow_tip(anchor, rect, chosen);

    PlacedPopup {
        rect,
        chosen_placement: chosen,
        arrow_tip,
    }
}

/// 计算指定 [`Placement`] 与 `gap` 下未钳制的浮层左上角坐标。
#[must_use]
fn candidate_origin(anchor: Rect, size: Vec2, placement: Placement, gap: f32) -> Vec2 {
    let width = size.x;
    let height = size.y;
    if placement.vertical() {
        let x = if placement.centered() {
            anchor.center().x - width * 0.5
        } else if placement.aligned_to_end() {
            anchor.max().x - width
        } else {
            anchor.origin.x
        };
        let y = if placement.below() {
            anchor.max().y + gap
        } else {
            anchor.origin.y - gap - height
        };
        Vec2::new(x, y)
    } else {
        let y = if placement.centered() {
            anchor.center().y - height * 0.5
        } else if placement.aligned_y_to_end() {
            anchor.max().y - height
        } else {
            anchor.origin.y
        };
        let x = if placement.right() {
            anchor.max().x + gap
        } else {
            anchor.origin.x - gap - width
        };
        Vec2::new(x, y)
    }
}

/// 判断候选位置在其主轴方向上是否完全落入 `bounds` 内。
#[must_use]
fn fits_primary_axis(origin: Vec2, size: Vec2, bounds: Rect, placement: Placement) -> bool {
    if placement.vertical() {
        origin.y >= bounds.origin.y && origin.y + size.y <= bounds.max().y
    } else {
        origin.x >= bounds.origin.x && origin.x + size.x <= bounds.max().x
    }
}

/// 计算 `placement` 所在侧相对于 `anchor` 在 `bounds` 内的可用主轴空间。
#[must_use]
fn available_primary_space(anchor: Rect, bounds: Rect, placement: Placement) -> f32 {
    if placement.vertical() {
        if placement.below() {
            bounds.max().y - anchor.max().y
        } else {
            anchor.origin.y - bounds.origin.y
        }
    } else if placement.right() {
        bounds.max().x - anchor.max().x
    } else {
        anchor.origin.x - bounds.origin.x
    }
}

#[must_use]
fn compute_arrow_tip(anchor: Rect, rect: Rect, chosen: Placement) -> Vec2 {
    let center = anchor.center();
    if chosen.vertical() {
        Vec2::new(
            center.x.clamp(rect.origin.x, rect.max().x),
            if chosen.below() {
                rect.origin.y
            } else {
                rect.max().y
            },
        )
    } else {
        Vec2::new(
            if chosen.right() {
                rect.origin.x
            } else {
                rect.max().x
            },
            center.y.clamp(rect.origin.y, rect.max().y),
        )
    }
}

/// 智能工具提示窗口内自动定位算法。
///
/// 确保工具提示气泡**始终完整显示在窗口视口内、绝不溢出窗口边界**：
/// 1. **视口安全边距（`viewport_margin`）**：自动在视口四周收缩 `viewport_margin`（当视口足够大时），
///    防止气泡紧贴窗口裁切边缘。
/// 2. **四向自适应翻转**：
///    - 优先尝试调用方请求的 `preferred` 方向；
///    - 若主轴空间不足，尝试对向翻转 `preferred.flip()`（如 `Top -> Bottom`、`Right -> Left`）；
///    - 若正反方向均放不下（例如触发器高度很大或贴近角落），自动尝试正交轴的两个方向（如 `Right`/`Left` 或 `Top`/`Bottom`），
///      优先选择剩余空间更大的一侧；
///    - 若四个方向均需钳制，则选择可用空间最大的一侧以最小化对触发器的遮挡。
/// 3. **双向边界钳制**：在次轴与主轴上将气泡矩形严格钳制在安全视口内，保证 `viewport.contains_rect(&placed.rect)` 恒成立。
#[must_use]
pub fn place_tooltip_in_window(
    anchor: Rect,
    tooltip_size: Vec2,
    viewport: Rect,
    preferred: Placement,
    gap: f32,
    viewport_margin: f32,
) -> PlacedPopup {
    let gap = gap.max(0.0);
    let margin = viewport_margin.max(0.0);
    let safe_viewport = if viewport.width() > margin * 2.0 && viewport.height() > margin * 2.0 {
        viewport.inflate(-margin)
    } else {
        viewport
    };

    let width = tooltip_size.x.max(0.0).min(safe_viewport.width().max(0.0));
    let height = tooltip_size.y.max(0.0).min(safe_viewport.height().max(0.0));
    let clamped_size = Vec2::new(width, height);

    let (cross_first, cross_second) = if preferred.vertical() {
        let space_right = available_primary_space(anchor, safe_viewport, Placement::Right);
        let space_left = available_primary_space(anchor, safe_viewport, Placement::Left);
        if space_right >= space_left {
            (Placement::Right, Placement::Left)
        } else {
            (Placement::Left, Placement::Right)
        }
    } else {
        let space_top = available_primary_space(anchor, safe_viewport, Placement::Top);
        let space_bottom = available_primary_space(anchor, safe_viewport, Placement::Bottom);
        if space_top >= space_bottom {
            (Placement::Top, Placement::Bottom)
        } else {
            (Placement::Bottom, Placement::Top)
        }
    };

    let candidates = [preferred, preferred.flip(), cross_first, cross_second];

    let mut chosen = preferred;
    let mut found_clean_fit = false;
    for candidate in candidates {
        let orig = candidate_origin(anchor, clamped_size, candidate, gap);
        if fits_primary_axis(orig, clamped_size, safe_viewport, candidate) {
            chosen = candidate;
            found_clean_fit = true;
            break;
        }
    }

    if !found_clean_fit {
        // 四个候选方向均超出安全视口主轴时，选择主轴可用空间最大的方向
        let mut best_space = f32::NEG_INFINITY;
        for candidate in candidates {
            let space = available_primary_space(anchor, safe_viewport, candidate);
            if space > best_space {
                best_space = space;
                chosen = candidate;
            }
        }
    }

    let mut origin = candidate_origin(anchor, clamped_size, chosen, gap);
    let max_x = (safe_viewport.max().x - width).max(safe_viewport.origin.x);
    let max_y = (safe_viewport.max().y - height).max(safe_viewport.origin.y);
    origin.x = origin.x.clamp(safe_viewport.origin.x, max_x);
    origin.y = origin.y.clamp(safe_viewport.origin.y, max_y);

    let rect = Rect::new(origin.x, origin.y, width, height);
    let arrow_tip = compute_arrow_tip(anchor, rect, chosen);

    PlacedPopup {
        rect,
        chosen_placement: chosen,
        arrow_tip,
    }
}

impl From<PlacedPopup> for Rect {
    fn from(placed: PlacedPopup) -> Self {
        placed.rect
    }
}

/// 渲染图背景模糊 Pass 的请求。
///
/// 需要毛玻璃背景的组件返回此请求。调用方收集一帧内的请求；
/// 若列表为空，渲染图会完全跳过模糊 Pass。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BackdropRequest {
    /// 拥有该效果的节点。
    pub node: u64,
    /// 需要模糊的背景矩形。
    pub rect: Rect,
    /// 模糊半径（逻辑像素）。
    pub blur: f32,
}

impl BackdropRequest {
    /// 构造请求；半径为零时返回 `None`。
    #[must_use]
    pub fn new(node: u64, rect: Rect, blur: f32) -> Option<Self> {
        (blur > 0.0 && !rect.is_empty()).then_some(Self { node, rect, blur })
    }
}

/// 浮层条目的类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// 下拉框、选择器或气泡框。
    Popover,
    /// 上下文或下拉菜单。
    Menu,
    /// 非交互式工具提示。
    Tooltip,
    /// 带遮罩的模态对话框。
    Modal,
}

impl OverlayKind {
    /// 该类型绘制所在的渲染层。
    #[must_use]
    pub const fn layer(self) -> PassLayer {
        match self {
            Self::Popover | Self::Menu | Self::Tooltip => PassLayer::Overlay,
            Self::Modal => PassLayer::Modal,
        }
    }

    /// 点击外部是否关闭。
    #[must_use]
    pub const fn light_dismiss(self) -> bool {
        !matches!(self, Self::Modal)
    }

    /// 按下 `Escape` 是否关闭。
    #[must_use]
    pub const fn escape_closes(self) -> bool {
        !matches!(self, Self::Tooltip)
    }
}

/// 浮层栈中的单个条目。
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayEntry {
    /// 弹窗自身的节点 ID。
    pub node: u64,
    /// 所锚定的触发器（若有）。
    pub anchor: Option<u64>,
    /// 类型，决定渲染层与关闭策略。
    pub kind: OverlayKind,
    /// 窗口空间下的弹窗边界。
    pub bounds: Rect,
    /// 模态框将 `Tab` 焦点限制在内的可聚焦节点列表。
    pub focus_scope: Vec<u64>,
    /// 锚点是否已滚出其裁剪区域。被裁剪的锚点会隐藏浮层但不关闭它，
    /// 以便锚点滚回视图时重新显示。
    pub anchor_clipped: bool,
}

impl OverlayEntry {
    /// 创建锚定到触发器的气泡框条目。
    #[must_use]
    pub fn popover(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Popover,
            bounds,
            focus_scope: Vec::new(),
            anchor_clipped: false,
        }
    }

    /// 创建锚定到触发器的菜单条目。
    #[must_use]
    pub fn menu(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Menu,
            bounds,
            focus_scope: Vec::new(),
            anchor_clipped: false,
        }
    }

    /// 创建锚定到触发器的工具提示条目。
    #[must_use]
    pub fn tooltip(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Tooltip,
            bounds,
            focus_scope: Vec::new(),
            anchor_clipped: false,
        }
    }

    /// 创建带焦点作用域的模态对话框条目。
    #[must_use]
    pub fn modal(node: u64, bounds: Rect, focus_scope: Vec<u64>) -> Self {
        Self {
            node,
            anchor: None,
            kind: OverlayKind::Modal,
            bounds,
            focus_scope,
            anchor_clipped: false,
        }
    }

    /// 标记锚点是否已滚出其裁剪区域。
    #[must_use]
    pub fn with_anchor_clipped(mut self, clipped: bool) -> Self {
        self.anchor_clipped = clipped;
        self
    }
}

/// 弹窗栈。
///
/// 条目按插入顺序排列。同层中后插入的条目绘制在先插入的条目之上，
/// 且 `PassLayer::Modal` 始终绘制在 `PassLayer::Overlay` 之上，
/// 使对话框内的选择器保持在对话框的视觉层级内。
#[derive(Clone, Debug, Default)]
pub struct OverlayStack {
    entries: Vec<OverlayEntry>,
    order: u32,
}

impl OverlayStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 推入浮层并返回其排序键。
    pub fn push(&mut self, entry: OverlayEntry) -> u64 {
        self.order = self.order.wrapping_add(1);
        let order = self.order;
        self.entries.push(entry);
        self.sort_key(self.entries.len() - 1, order)
    }

    /// 当前实际可见的最顶层条目。
    #[must_use]
    pub fn top(&self) -> Option<&OverlayEntry> {
        self.entries
            .iter()
            .filter(|entry| !entry.anchor_clipped)
            .max_by_key(|entry| self.key_of(entry))
    }

    /// 锚点仍在裁剪区域内的所有条目。
    ///
    /// 绘制时遍历此方法而非 [`Self::entries`]，以便锚点被裁剪时隐藏弹窗但保留在栈中。
    pub fn visible_entries(&self) -> impl Iterator<Item = &OverlayEntry> {
        self.entries.iter().filter(|entry| !entry.anchor_clipped)
    }

    /// 隐藏或显示浮层而不关闭它。
    ///
    /// 锚点被裁剪时会隐藏弹窗；条目保留在栈中，以便锚点滚回视图时重新显现。
    pub fn set_anchor_clipped(&mut self, node: u64, clipped: bool) -> bool {
        for entry in &mut self.entries {
            if entry.node == node {
                entry.anchor_clipped = clipped;
                return true;
            }
        }
        false
    }

    /// 当前打开的浮层数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按从底到顶顺序遍历所有条目。
    #[must_use]
    pub fn entries(&self) -> &[OverlayEntry] {
        &self.entries
    }

    /// 判断节点是否处于打开状态。
    #[must_use]
    pub fn contains(&self, node: u64) -> bool {
        self.entries.iter().any(|entry| entry.node == node)
    }

    /// 按节点移除一个浮层，返回其此前是否打开。
    pub fn close(&mut self, node: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.node != node);
        before != self.entries.len()
    }

    /// 关闭最顶层浮层，返回被关闭的节点。
    pub fn close_top(&mut self) -> Option<u64> {
        let node = self.top()?.node;
        self.close(node);
        Some(node)
    }

    /// 处理 `Escape`：关闭最顶层可关闭的浮层。
    pub fn close_on_escape(&mut self) -> Option<u64> {
        let node = self
            .entries
            .iter()
            .filter(|entry| !entry.anchor_clipped)
            .max_by_key(|entry| self.key_of(entry))
            .filter(|entry| entry.kind.escape_closes())?
            .node;
        self.close(node);
        Some(node)
    }

    /// 处理外部点击：当点击点位于最顶层可轻触关闭浮层的边界外时关闭它。
    pub fn close_outside(&mut self, point: Vec2) -> Option<u64> {
        let entry = self
            .entries
            .iter()
            .filter(|entry| !entry.anchor_clipped)
            .max_by_key(|entry| self.key_of(entry))?;
        if !entry.kind.light_dismiss() || entry.bounds.contains_point(point) {
            return None;
        }
        let node = entry.node;
        self.close(node);
        Some(node)
    }

    /// 移除锚点已不存在的浮层。
    ///
    /// 每帧重建场景后运行一次：触发器被移除时，其弹窗（及焦点陷阱）必须一并移除。
    pub fn prune_anchors(&mut self, mut valid: impl FnMut(u64) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| match entry.anchor {
            Some(anchor) => valid(anchor),
            None => true,
        });
        before - self.entries.len()
    }

    /// 若存在打开的模态框，返回最顶层模态框的焦点作用域。
    #[must_use]
    pub fn focus_scope(&self) -> Option<&[u64]> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == OverlayKind::Modal)
            .max_by_key(|entry| self.key_of(entry))
            .map(|entry| entry.focus_scope.as_slice())
    }

    /// 条目渲染层与顺序的排序前缀。
    #[must_use]
    pub fn sort_key(&self, _index: usize, order: u32) -> u64 {
        (order as u64) << 8
    }

    /// 条目的完整排序键（渲染层优先）。
    #[must_use]
    pub fn key_of(&self, entry: &OverlayEntry) -> u64 {
        let index = self
            .entries
            .iter()
            .position(|candidate| core::ptr::eq(candidate, entry))
            .unwrap_or(0);
        let layer = match entry.kind.layer() {
            PassLayer::Canvas => 0u64,
            PassLayer::Ui => 1,
            PassLayer::Overlay => 2,
            PassLayer::Modal => 3,
            PassLayer::Debug => 255,
        };
        (layer << 56) | self.sort_key(index, self.order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    fn assert_contained(popup: Rect, viewport: Rect) {
        assert!(
            viewport.contains_rect(&popup),
            "popup {popup:?} escaped viewport {viewport:?}"
        );
    }

    #[test]
    fn a_popup_prefers_the_requested_side() {
        let anchor = Rect::new(100.0, 100.0, 120.0, 36.0);
        let popup: Rect = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 160.0),
            viewport(),
            Placement::BottomStart,
        );
        assert_eq!(popup.origin.x, 100.0);
        assert_eq!(popup.origin.y, 140.0);
    }

    #[test]
    fn a_popup_flips_up_when_the_bottom_edge_has_no_room() {
        let anchor = Rect::new(100.0, 650.0, 120.0, 36.0);
        let popup: Rect = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 160.0),
            viewport(),
            Placement::BottomStart,
        );
        assert!(popup.max().y <= anchor.origin.y, "{popup:?}");
        assert_contained(popup, viewport());
    }

    #[test]
    fn a_popup_flips_left_when_the_right_edge_has_no_room() {
        let anchor = Rect::new(1200.0, 100.0, 60.0, 30.0);
        let popup: Rect = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 100.0),
            viewport(),
            Placement::RightStart,
        );
        assert!(popup.max().x <= anchor.origin.x);
        assert_contained(popup, viewport());
    }

    #[test]
    fn every_corner_and_edge_stays_contained() {
        let size = Vec2::new(240.0, 180.0);
        let anchors = [
            Rect::new(0.0, 0.0, 80.0, 30.0),
            Rect::new(1200.0, 0.0, 80.0, 30.0),
            Rect::new(0.0, 690.0, 80.0, 30.0),
            Rect::new(1200.0, 690.0, 80.0, 30.0),
            Rect::new(600.0, 0.0, 80.0, 30.0),
            Rect::new(600.0, 690.0, 80.0, 30.0),
            Rect::new(0.0, 350.0, 80.0, 30.0),
            Rect::new(1200.0, 350.0, 80.0, 30.0),
            Rect::new(-20.0, -10.0, 80.0, 30.0),
            Rect::new(1240.0, 710.0, 80.0, 30.0),
        ];
        let placements = [
            Placement::BottomStart,
            Placement::BottomEnd,
            Placement::Bottom,
            Placement::TopStart,
            Placement::TopEnd,
            Placement::Top,
            Placement::RightStart,
            Placement::RightEnd,
            Placement::Right,
            Placement::LeftStart,
            Placement::LeftEnd,
            Placement::Left,
        ];
        for anchor in anchors {
            for placement in placements {
                let popup: Rect = place_anchored_popup(anchor, size, viewport(), placement);
                assert_contained(popup, viewport());
                let tip = place_tooltip_in_window(anchor, size, viewport(), placement, 6.0, 8.0);
                assert_contained(tip.rect, viewport());
            }
        }
    }

    #[test]
    fn tooltip_in_window_falls_back_to_cross_axis_when_both_vertical_sides_lack_room() {
        let vp = Rect::new(0.0, 0.0, 800.0, 200.0);
        // 触发器高度占满上下空间，导致 Top 与 Bottom 均放不下 80px 高的气泡，应自动回退到右侧
        let anchor = Rect::new(40.0, 20.0, 80.0, 160.0);
        let placed =
            place_tooltip_in_window(anchor, Vec2::new(160.0, 80.0), vp, Placement::Top, 6.0, 8.0);
        assert_eq!(placed.chosen_placement, Placement::Right);
        assert_contained(placed.rect, vp.inflate(-8.0));
    }

    #[test]
    fn the_centred_placements_centre_on_the_anchor() {
        let anchor = Rect::new(400.0, 300.0, 100.0, 40.0);
        let below: PlacedPopup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 100.0),
            viewport(),
            Placement::Bottom,
        );
        assert_eq!(below.rect.center().x, anchor.center().x);
        assert_eq!(below.rect.origin.y, anchor.max().y + POPUP_GAP);

        let right: PlacedPopup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 100.0),
            viewport(),
            Placement::Right,
        );
        assert_eq!(right.rect.center().y, anchor.center().y);
        assert_eq!(right.rect.origin.x, anchor.max().x + POPUP_GAP);
    }

    #[test]
    fn a_flipped_popup_reports_the_chosen_side_and_an_arrow_tip_on_its_edge() {
        let anchor = Rect::new(100.0, 650.0, 120.0, 36.0);
        let placed: PlacedPopup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 160.0),
            viewport(),
            Placement::Bottom,
        );
        assert_eq!(placed.chosen_placement, Placement::Top);
        // 箭头从弹窗底边向下指向锚点。
        assert_eq!(placed.arrow_tip.y, placed.rect.max().y);
        assert_eq!(placed.arrow_tip.x, anchor.center().x);
    }

    #[test]
    fn an_oversized_popup_is_shrunk_to_the_viewport() {
        let anchor = Rect::new(0.0, 0.0, 10.0, 10.0);
        let popup: Rect = place_anchored_popup(
            anchor,
            Vec2::new(4000.0, 4000.0),
            viewport(),
            Placement::BottomStart,
        );
        assert_eq!(popup, viewport());
    }

    #[test]
    fn the_stack_orders_later_overlays_on_top() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::menu(2, 10, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.top().map(|entry| entry.node), Some(2));
    }

    #[test]
    fn escape_closes_only_the_top_layer() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::menu(2, 10, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.close_on_escape(), Some(2));
        assert!(stack.contains(1));
        assert_eq!(stack.close_on_escape(), Some(1));
        assert!(stack.is_empty());
    }

    #[test]
    fn a_click_outside_dismisses_a_popover_but_not_a_modal() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        ));
        assert_eq!(stack.close_outside(Vec2::new(500.0, 500.0)), Some(1));
        assert!(stack.is_empty());

        stack.push(OverlayEntry::modal(
            2,
            Rect::new(10.0, 10.0, 100.0, 100.0),
            vec![20, 21],
        ));
        assert_eq!(stack.close_outside(Vec2::new(500.0, 500.0)), None);
        assert!(stack.contains(2));
    }

    #[test]
    fn a_click_inside_the_popup_does_not_dismiss_it() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        ));
        assert_eq!(stack.close_outside(Vec2::new(50.0, 50.0)), None);
        assert!(stack.contains(1));
    }

    #[test]
    fn a_vanished_anchor_takes_its_popup_with_it() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::modal(
            2,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![],
        ));
        assert_eq!(stack.prune_anchors(|anchor| anchor != 10), 1);
        assert!(!stack.contains(1));
        assert!(stack.contains(2), "a modal has no anchor and survives");
    }

    #[test]
    fn a_modal_exposes_its_focus_scope_and_wins_over_a_dialog_popover() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::modal(
            1,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![7, 8],
        ));
        stack.push(OverlayEntry::popover(2, 7, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.focus_scope(), Some([7, 8].as_slice()));
    }
}

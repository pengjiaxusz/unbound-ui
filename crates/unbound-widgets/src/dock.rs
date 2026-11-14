//! 停靠区（DockArea）：支持可重排的分割面板与标签页树。
//!
//! 树采用扁平竞技场（arena）结构——[`DockNode::Split`] 持有两个子节点索引，
//! [`DockNode::Tabs`] 持有标签页列表——因此拖拽操作无需在事件循环中搬移递归结构。
//! 分割面板时将其节点克隆到新的竞技场槽位，并用新分割节点覆写旧槽位，从而保持既有子节点索引有效。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::scroll::ScrollAxis;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 标签栏高度（逻辑像素）。
pub const DOCK_TAB_BAR: f32 = 32.0;

/// 拖拽分割条时任意一侧可占用的最小比例。
const SPLIT_LIMIT: f32 = 0.1;

/// 单个可停靠标签页。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockTab {
    /// 稳定标识，同时用作标签页的语义与命中节点。
    pub id: u64,
    /// 可见标题。
    pub title: String,
    /// 标签页是否可关闭。
    pub closable: bool,
}

impl DockTab {
    /// 创建可关闭的标签页。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            closable: true,
        }
    }

    /// 创建不可关闭的同款标签页。
    #[must_use]
    pub fn pinned(mut self) -> Self {
        self.closable = false;
        self
    }
}

/// 停靠竞技场中的单个节点。
#[derive(Clone, Debug, PartialEq)]
pub enum DockNode {
    /// 由可拖拽分割条隔开的两个子节点。
    Split {
        /// `Horizontal` 表示左右并排放置两个子节点。
        axis: ScrollAxis,
        /// `first` 子节点所占比例。
        ratio: f32,
        /// 左/上侧子节点。
        first: usize,
        /// 右/下侧子节点。
        second: usize,
    },
    /// 共享同一面板的一组标签页。
    Tabs {
        /// 从左到右排列的标签页列表。
        tabs: Vec<DockTab>,
        /// 当前可见的标签页索引。
        active: usize,
    },
}

impl DockNode {
    /// 当节点为 `Tabs` 时返回其标签页列表。
    #[must_use]
    pub fn tabs(&self) -> Option<&[DockTab]> {
        match self {
            Self::Tabs { tabs, .. } => Some(tabs),
            Self::Split { .. } => None,
        }
    }

    /// 当节点为 `Tabs` 时返回其当前激活的标签页索引。
    #[must_use]
    pub fn active(&self) -> Option<usize> {
        match self {
            Self::Tabs { active, .. } => Some(*active),
            Self::Split { .. } => None,
        }
    }
}

/// 停靠面板的扁平树。
#[derive(Clone, Debug, PartialEq)]
pub struct DockTree {
    /// 所有节点；子节点索引指向此向量。
    pub nodes: Vec<DockNode>,
    /// 根节点索引。
    pub root: usize,
    /// 所有分割条的槽宽。
    pub handle: f32,
}

impl DockTree {
    /// 创建包含单个标签组的停靠树。
    #[must_use]
    pub fn new(tabs: Vec<DockTab>) -> Self {
        Self {
            nodes: vec![DockNode::Tabs { tabs, active: 0 }],
            root: 0,
            handle: 8.0,
        }
    }

    /// 获取 `index` 处的节点（若存在）。
    #[must_use]
    pub fn node(&self, index: usize) -> Option<&DockNode> {
        self.nodes.get(index)
    }

    /// 可变获取 `index` 处的节点。
    pub fn node_mut(&mut self, index: usize) -> Option<&mut DockNode> {
        self.nodes.get_mut(index)
    }

    /// 按竞技场顺序返回所有标签组。
    #[must_use]
    pub fn tab_groups(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| matches!(node, DockNode::Tabs { .. }).then_some(index))
            .collect()
    }

    /// 按标识定位标签页。
    #[must_use]
    pub fn find_tab(&self, id: u64) -> Option<(usize, usize)> {
        for (index, node) in self.nodes.iter().enumerate() {
            if let DockNode::Tabs { tabs, .. } = node {
                if let Some(position) = tabs.iter().position(|tab| tab.id == id) {
                    return Some((index, position));
                }
            }
        }
        None
    }

    /// 计算所有标签组的面板矩形。
    #[must_use]
    pub fn layout(&self, rect: Rect) -> Vec<DockPane> {
        let mut out = Vec::new();
        self.layout_into(self.root, rect, &mut out);
        out
    }

    fn layout_into(&self, index: usize, rect: Rect, out: &mut Vec<DockPane>) {
        match self.nodes.get(index) {
            Some(DockNode::Tabs { tabs, active }) => out.push(DockPane {
                node: index,
                rect,
                tabs: tabs.clone(),
                active: *active,
            }),
            Some(DockNode::Split {
                axis,
                ratio,
                first,
                second,
            }) => {
                let horizontal = !matches!(axis, ScrollAxis::Vertical);
                let length = if horizontal { rect.size.x } else { rect.size.y };
                let available = (length - self.handle).max(0.0);
                let ratio = ratio.clamp(SPLIT_LIMIT, 1.0 - SPLIT_LIMIT);
                let first_len = available * ratio;
                let (first_rect, second_rect) = if horizontal {
                    (
                        Rect::new(rect.origin.x, rect.origin.y, first_len, rect.size.y),
                        Rect::new(
                            rect.origin.x + first_len + self.handle,
                            rect.origin.y,
                            (available - first_len).max(0.0),
                            rect.size.y,
                        ),
                    )
                } else {
                    (
                        Rect::new(rect.origin.x, rect.origin.y, rect.size.x, first_len),
                        Rect::new(
                            rect.origin.x,
                            rect.origin.y + first_len + self.handle,
                            rect.size.x,
                            (available - first_len).max(0.0),
                        ),
                    )
                };
                self.layout_into(*first, first_rect, out);
                self.layout_into(*second, second_rect, out);
            }
            _ => {}
        }
    }

    /// 分割目标面板并将 `tabs` 放入新分出的半区。
    ///
    /// `Center` 则改为将标签页合并进目标组。返回新标签组的索引，若目标不是面板则返回 `None`。
    pub fn insert(&mut self, target: usize, zone: DockZone, tabs: Vec<DockTab>) -> Option<usize> {
        let node = self.nodes.get(target)?.clone();
        if !matches!(node, DockNode::Tabs { .. }) {
            return None;
        }
        if zone == DockZone::Center {
            if let Some(DockNode::Tabs {
                tabs: existing,
                active,
            }) = self.nodes.get_mut(target)
            {
                if !tabs.is_empty() {
                    *active = existing.len();
                }
                existing.extend(tabs);
            }
            return Some(target);
        }
        let existing = self.nodes.len();
        self.nodes.push(node);
        let fresh = self.nodes.len();
        self.nodes.push(DockNode::Tabs { tabs, active: 0 });
        let (axis, first, second) = match zone {
            DockZone::Left | DockZone::Top => (zone.axis(), fresh, existing),
            DockZone::Right | DockZone::Bottom => (zone.axis(), existing, fresh),
            DockZone::Center => unreachable!("centre merges above"),
        };
        self.nodes[target] = DockNode::Split {
            axis,
            ratio: 0.5,
            first,
            second,
        };
        Some(fresh)
    }

    /// 将标签页移动至目标面板与停靠区域。
    ///
    /// 拖放到自身所在组的 `Center` 区域保持原地不动；移入新组时会从旧组移除。
    /// 空组保留在原地：光标下突然消失的面板比等待接收新标签页的空面板更糟糕。
    pub fn move_tab(&mut self, id: u64, target: usize, zone: DockZone) -> bool {
        let Some((source, position)) = self.find_tab(id) else {
            return false;
        };
        if source == target && zone == DockZone::Center {
            return false;
        }
        let Some(DockNode::Tabs { tabs, active }) = self.nodes.get_mut(source) else {
            return false;
        };
        if position >= tabs.len() {
            return false;
        }
        let tab = tabs.remove(position);
        if *active >= tabs.len() && !tabs.is_empty() {
            *active = tabs.len() - 1;
        }
        self.insert(target, zone, vec![tab]).is_some()
    }

    /// 移除标签页。返回该标签页是否存在。
    pub fn close_tab(&mut self, id: u64) -> bool {
        let Some((group, position)) = self.find_tab(id) else {
            return false;
        };
        let Some(DockNode::Tabs { tabs, active }) = self.nodes.get_mut(group) else {
            return false;
        };
        tabs.remove(position);
        if *active >= tabs.len() {
            *active = tabs.len().saturating_sub(1);
        }
        true
    }

    /// 将拖拽比例应用到分割节点。
    pub fn set_split_ratio(&mut self, index: usize, ratio: f32) -> bool {
        let Some(DockNode::Split { ratio: slot, .. }) = self.nodes.get_mut(index) else {
            return false;
        };
        *slot = ratio.clamp(SPLIT_LIMIT, 1.0 - SPLIT_LIMIT);
        true
    }
}

/// 单个已布局的面板。
#[derive(Clone, Debug, PartialEq)]
pub struct DockPane {
    /// 该标签组的竞技场索引。
    pub node: usize,
    /// 面板矩形。
    pub rect: Rect,
    /// 组内的标签页切片。
    pub tabs: Vec<DockTab>,
    /// 当前可见标签页索引。
    pub active: usize,
}

impl DockPane {
    /// 面板内部的标签栏矩形。
    #[must_use]
    pub fn tab_bar(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            DOCK_TAB_BAR.min(self.rect.size.y),
        )
    }

    /// 标签栏下方的内容区域矩形。
    #[must_use]
    pub fn content(&self) -> Rect {
        let bar = self.tab_bar();
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + bar.size.y,
            self.rect.size.x,
            (self.rect.size.y - bar.size.y).max(0.0),
        )
    }

    /// 单个标签页的矩形，已裁剪到标签栏范围内。
    #[must_use]
    pub fn tab_rect(&self, index: usize, width: f32) -> Rect {
        let bar = self.tab_bar();
        Rect::new(
            bar.origin.x + index as f32 * width,
            bar.origin.y,
            width.min((bar.size.x - index as f32 * width).max(0.0)),
            bar.size.y,
        )
    }

    /// 单个标签页的关闭控件矩形。
    #[must_use]
    pub fn close_rect(&self, index: usize, width: f32) -> Rect {
        let tab = self.tab_rect(index, width);
        let size = (tab.size.y * 0.45).max(10.0);
        Rect::new(
            tab.max().x - size - 8.0,
            tab.origin.y + (tab.size.y - size) * 0.5,
            size,
            size,
        )
    }

    /// 当前可见的标签页（若组内存在）。
    #[must_use]
    pub fn active_tab(&self) -> Option<&DockTab> {
        self.tabs.get(self.active)
    }
}

/// 拖拽中的标签页在面板内的落点区域。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockZone {
    /// 分割面板并将标签页置于上方。
    Top,
    /// 分割面板并将标签页置于下方。
    Bottom,
    /// 分割面板并将标签页置于左侧。
    Left,
    /// 分割面板并将标签页置于右侧。
    Right,
    /// 合并到当前面板的标签组中。
    Center,
}

impl DockZone {
    /// 该落点区域产生的分割轴方向。
    #[must_use]
    pub fn axis(self) -> ScrollAxis {
        match self {
            Self::Left | Self::Right => ScrollAxis::Horizontal,
            Self::Top | Self::Bottom => ScrollAxis::Vertical,
            Self::Center => ScrollAxis::Both,
        }
    }

    /// 对面板内的指针位置进行区域分类。
    ///
    /// 每条边的外侧四分之一为分割区，中间为合并区。
    /// 距离最近的一条边胜出，因此靠近角落的指针仍能选中更明确的方向。
    #[must_use]
    pub fn from_pointer(rect: Rect, pointer: Vec2) -> Self {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 || !rect.contains_point(pointer) {
            return Self::Center;
        }
        let nx = (pointer.x - rect.origin.x) / rect.size.x;
        let ny = (pointer.y - rect.origin.y) / rect.size.y;
        let left = nx;
        let right = 1.0 - nx;
        let top = ny;
        let bottom = 1.0 - ny;
        let mut best = Self::Center;
        let mut distance = 0.25;
        if left < distance {
            distance = left;
            best = Self::Left;
        }
        if right < distance {
            distance = right;
            best = Self::Right;
        }
        if top < distance {
            distance = top;
            best = Self::Top;
        }
        if bottom < distance {
            best = Self::Bottom;
        }
        best
    }

    /// 该落点区域绘制的半透明预览矩形。
    #[must_use]
    pub fn preview(self, rect: Rect) -> Rect {
        match self {
            Self::Left => Rect::new(rect.origin.x, rect.origin.y, rect.size.x * 0.5, rect.size.y),
            Self::Right => Rect::new(
                rect.origin.x + rect.size.x * 0.5,
                rect.origin.y,
                rect.size.x * 0.5,
                rect.size.y,
            ),
            Self::Top => Rect::new(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y * 0.5),
            Self::Bottom => Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.y * 0.5,
                rect.size.x,
                rect.size.y * 0.5,
            ),
            Self::Center => rect,
        }
    }
}

/// 进行中的标签页拖拽状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockDrag {
    /// 被拖拽的标签页。
    pub tab: u64,
    /// 拖拽起始所在的标签组。
    pub from: usize,
    /// 当前指针位置。
    pub pointer: Vec2,
    /// 指针是否已离开所有停靠面板（调用方据此可将其转为浮动窗口）。
    pub detached: bool,
}

/// 拖拽悬停在面板上时产生的预览。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockHover {
    /// 目标标签组。
    pub target: usize,
    /// 组内的落点区域。
    pub zone: DockZone,
    /// 预览绘制的矩形。
    pub preview: Rect,
}

impl DockHover {
    /// 对停靠树上的指针位置进行分类。
    #[must_use]
    pub fn resolve(tree: &DockTree, root: Rect, pointer: Vec2) -> Option<Self> {
        let pane = tree
            .layout(root)
            .into_iter()
            .find(|pane| pane.rect.contains_point(pointer))?;
        let zone = DockZone::from_pointer(pane.rect, pointer);
        Some(Self {
            target: pane.node,
            zone,
            preview: zone.preview(pane.rect),
        })
    }
}

/// 停靠区的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct DockState {
    /// 面板树。
    pub tree: DockTree,
    /// 当前进行的拖拽（若有）。
    pub drag: Option<DockDrag>,
    /// 指针下方的落点预览（若有）。
    pub hover: Option<DockHover>,
}

impl DockState {
    /// 基于给定面板树创建停靠区状态。
    #[must_use]
    pub fn new(tree: DockTree) -> Self {
        Self {
            tree,
            drag: None,
            hover: None,
        }
    }

    /// 开始拖拽标签页。
    pub fn begin_drag(&mut self, tab: u64, from: usize, pointer: Vec2) {
        self.drag = Some(DockDrag {
            tab,
            from,
            pointer,
            detached: false,
        });
        self.hover = None;
    }

    /// 移动拖拽并重新计算落点预览。
    pub fn drag_to(&mut self, root: Rect, pointer: Vec2) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.pointer = pointer;
        self.hover = DockHover::resolve(&self.tree, root, pointer);
        drag.detached = self.hover.is_none();
    }

    /// 完成拖拽：将标签页移至预览区域。
    ///
    /// 返回面板树是否发生变化。
    pub fn drop_drag(&mut self) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        let Some(hover) = self.hover.take() else {
            return false;
        };
        self.tree.move_tab(drag.tab, hover.target, hover.zone)
    }

    /// 放弃拖拽，不做任何移动。
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.hover = None;
    }

    /// 判断当前是否正在拖拽标签页。
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// 激活指定组内的某个标签页。
    pub fn activate(&mut self, group: usize, index: usize) -> bool {
        let Some(DockNode::Tabs { tabs, active }) = self.tree.node_mut(group) else {
            return false;
        };
        if index >= tabs.len() || *active == index {
            return false;
        }
        *active = index;
        true
    }
}

/// 停靠区绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockTokens {
    /// 面板背景色。
    pub panel: Token,
    /// 面板边框色。
    pub border: Token,
    /// 标签栏背景色。
    pub tab_bar: Token,
    /// 闲置标签页背景色。
    pub tab: Token,
    /// 悬停标签页背景色。
    pub tab_hover: Token,
    /// 激活标签页背景色。
    pub tab_active: Token,
    /// 激活标签页文字颜色。
    pub tab_text_active: Token,
    /// 闲置标签页文字颜色。
    pub tab_text: Token,
    /// 拖放预览填充色。
    pub preview: Token,
    /// 拖放预览边框色。
    pub preview_border: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl DockTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            panel: Token::SURFACE,
            border: Token::OUTLINE,
            tab_bar: Token::HEADER_BG,
            tab: Token::NONE,
            tab_hover: Token::CONTROL_HOVER,
            tab_active: Token::CARD_BG,
            tab_text_active: Token::TEXT_PRIMARY,
            tab_text: Token::TEXT_SECONDARY,
            preview: Token::PRIMARY_SOFT,
            preview_border: Token::PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 发射面板框架：表面、边框与标签栏。
#[must_use]
pub fn emit_dock_pane(pane: &DockPane, tokens: DockTokens, z: u16) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    primitives.push(outlined(
        pane.rect,
        0.0,
        tokens.panel,
        tokens.border,
        1.0,
        0.0,
        z,
    ));
    primitives.push(surface(pane.tab_bar(), 0.0, tokens.tab_bar, z));
    if pane.active_tab().is_some() {
        // 激活标签页下方以强调色描线，这是停靠区唯一的常驻状态提示。
        let width = DOCK_TAB_BAR * 4.0;
        let rect = pane.tab_rect(pane.active, width);
        let line = Rect::new(rect.origin.x, rect.max().y - 2.0, rect.size.x, 2.0);
        primitives.push(instance(
            line,
            [0.0; 4],
            tokens.preview_border,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 发射单个停靠标签页。
///
/// 每个标签页拥有独立节点标识：标签栏是一组控件列表，而非带子区域的单个控件，
/// 因此辅助技术可以独立枚举并激活每个标签页。
#[must_use]
pub fn emit_dock_tab(
    desc: &DockPane,
    index: usize,
    width: f32,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: DockTokens,
    z: u16,
) -> WidgetOutput {
    let Some(tab) = desc.tabs.get(index) else {
        return WidgetOutput::default();
    };
    let rect = desc.tab_rect(index, width);
    let active = index == desc.active;
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    let fill = if active {
        tokens.tab_active
    } else if control.phase == PointerPhase::Hovered {
        tokens.tab_hover
    } else {
        tokens.tab
    };
    primitives.push(outlined(rect, 0.0, fill, Token::NONE, 0.0, 0.0, z));
    glyphs(
        label,
        if active {
            tokens.tab_text_active
        } else {
            tokens.tab_text
        },
        z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        2.0,
        tokens.focus_ring,
        2.0,
        z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(tab.id, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii: [0.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Tab, tab.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = active;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 发射半透明拖放预览。
#[must_use]
pub fn emit_dock_preview(hover: DockHover, tokens: DockTokens, z: u16) -> WidgetOutput {
    let rect = hover.preview;
    WidgetOutput {
        primitives: vec![outlined(
            rect,
            4.0,
            tokens.preview,
            tokens.preview_border,
            2.0,
            0.0,
            z,
        )],
        ..WidgetOutput::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> DockTree {
        DockTree::new(vec![
            DockTab::new(1, "Explorer"),
            DockTab::new(2, "Search"),
            DockTab::new(3, "Source"),
        ])
    }

    #[test]
    fn a_single_group_fills_the_root() {
        let tree = tree();
        let panes = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0));
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].rect, Rect::new(0.0, 0.0, 400.0, 300.0));
        assert_eq!(panes[0].content().origin.y, DOCK_TAB_BAR);
    }

    #[test]
    fn splitting_a_panel_produces_two_panels_with_a_gutter() {
        let mut tree = tree();
        let new = tree
            .insert(0, DockZone::Right, vec![DockTab::new(9, "Preview")])
            .expect("split a tab group");
        assert_eq!(new, 2);
        let panes = tree.layout(Rect::new(0.0, 0.0, 408.0, 300.0));
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].rect.size.x, 200.0);
        assert_eq!(panes[1].rect.origin.x, 208.0);
        assert_eq!(panes[1].rect.size.x, 200.0);
        assert_eq!(panes[0].tabs.len(), 3);
        assert_eq!(panes[1].tabs[0].id, 9);
    }

    #[test]
    fn centre_drop_merges_into_the_target_group() {
        let mut tree = tree();
        let preview = tree
            .insert(0, DockZone::Bottom, vec![DockTab::new(9, "Preview")])
            .expect("split");
        assert!(tree.move_tab(3, preview, DockZone::Center));
        assert_eq!(
            tree.node(preview)
                .expect("group")
                .tabs()
                .expect("tabs")
                .len(),
            2
        );
        assert_eq!(tree.find_tab(3).expect("still present").0, preview);
    }

    #[test]
    fn moving_a_tab_to_another_panel_removes_it_from_the_source() {
        let mut tree = tree();
        let preview = tree
            .insert(0, DockZone::Bottom, vec![DockTab::new(9, "Preview")])
            .expect("split");
        // 分割操作会将原标签组克隆到新槽位，因此源组不再位于索引 0。
        let source = tree
            .tab_groups()
            .into_iter()
            .find(|index| *index != preview)
            .expect("source group");
        assert!(tree.move_tab(2, preview, DockZone::Center));
        let source_node = tree.node(source).expect("source");
        assert_eq!(source_node.tabs().expect("tabs").len(), 2);
        let target = tree.node(preview).expect("target");
        assert_eq!(target.tabs().expect("tabs").len(), 2);
    }

    #[test]
    fn a_tab_dropped_on_its_own_group_stays_put() {
        let mut tree = tree();
        assert!(!tree.move_tab(1, 0, DockZone::Center));
        assert_eq!(tree.nodes[0].tabs().expect("tabs").len(), 3);
    }

    #[test]
    fn closing_a_tab_keeps_the_active_index_valid() {
        let mut tree = tree();
        tree.nodes[0] = DockNode::Tabs {
            tabs: vec![DockTab::new(1, "A"), DockTab::new(2, "B")],
            active: 1,
        };
        assert!(tree.close_tab(2));
        assert_eq!(tree.node(0).expect("group").active(), Some(0));
        assert!(!tree.close_tab(99));
    }

    #[test]
    fn the_zone_detection_picks_the_nearest_edge() {
        let rect = Rect::new(0.0, 0.0, 400.0, 400.0);
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(10.0, 200.0)),
            DockZone::Left
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(390.0, 200.0)),
            DockZone::Right
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 10.0)),
            DockZone::Top
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 390.0)),
            DockZone::Bottom
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 200.0)),
            DockZone::Center
        );
    }

    #[test]
    fn the_preview_covers_the_half_the_tab_would_occupy() {
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(
            DockZone::Left.preview(rect),
            Rect::new(0.0, 0.0, 200.0, 300.0)
        );
        assert_eq!(
            DockZone::Bottom.preview(rect),
            Rect::new(0.0, 150.0, 400.0, 150.0)
        );
        assert_eq!(DockZone::Center.preview(rect), rect);
    }

    #[test]
    fn a_drag_recomputes_the_preview_and_drops_the_tab() {
        let mut state = DockState::new(tree());
        let root = Rect::new(0.0, 0.0, 400.0, 300.0);
        state.begin_drag(2, 0, Vec2::new(60.0, 10.0));
        state.drag_to(root, Vec2::new(390.0, 150.0));
        let hover = state.hover.expect("a preview inside the panel");
        assert_eq!(hover.zone, DockZone::Right);
        assert!(state.drop_drag());
        assert!(!state.is_dragging());
        let panes = state.tree.layout(root);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[1].tabs[0].id, 2);
    }

    #[test]
    fn dragging_outside_every_panel_marks_the_drag_detached() {
        let mut state = DockState::new(tree());
        let root = Rect::new(0.0, 0.0, 400.0, 300.0);
        state.begin_drag(1, 0, Vec2::new(10.0, 10.0));
        state.drag_to(root, Vec2::new(900.0, 900.0));
        assert!(state.drag.expect("drag").detached);
        assert!(!state.drop_drag(), "a detached drop moves nothing");
    }

    #[test]
    fn a_tab_is_pickable_and_exposes_the_tab_role() {
        let tree = tree();
        let pane = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0)).remove(0);
        let tokens = DockTokens::standard();
        let output = emit_dock_tab(
            &pane,
            1,
            DOCK_TAB_BAR * 4.0,
            ControlState::default(),
            &[],
            tokens,
            10,
        );
        let hit = output.hit.expect("a tab is pickable");
        assert_eq!(hit.node, 2);
        let a11y = output.a11y.expect("a tab is reachable");
        assert_eq!(a11y.role, Role::Tab);
        assert_eq!(a11y.label, "Search");
        assert!(!a11y.selected);
    }

    #[test]
    fn the_active_tab_is_selected() {
        let tree = tree();
        let pane = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0)).remove(0);
        let output = emit_dock_tab(
            &pane,
            0,
            DOCK_TAB_BAR * 4.0,
            ControlState::default(),
            &[],
            DockTokens::standard(),
            10,
        );
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn the_split_ratio_is_clamped() {
        let mut tree = tree();
        tree.insert(0, DockZone::Right, vec![DockTab::new(9, "P")]);
        assert!(tree.set_split_ratio(0, 0.0));
        let DockNode::Split { ratio, .. } = tree.node(0).expect("split") else {
            panic!("root became a split");
        };
        assert_eq!(*ratio, SPLIT_LIMIT);
    }
}

//! 虚拟化树形视图：仅实例化视口内的行。
//!
//! 扁平化与槽位复用逻辑位于 [`unbound_world::virtualize::VirtualTree`]；
//! 本模块补充控件语义——选区、焦点、懒加载展开事件、行图标槽、紧凑密度、内联重命名与行绘制。
//! 因此十万节点的文档也仅维持恒定数量的存活行，且折叠或未加载的子树零遍历开销，
//! 因为 `children_of` 仅对已展开且已加载的节点调用。
//!

use accesskit::Role;
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_gpu::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::virtualize::{TreeRow, VirtualTree, WindowUpdate};

use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::shell::{emit_shell_icon_primitives, emit_widget_icon_primitives, ShellIcon, WidgetIcon};
use crate::switch::CONTROL_LAYER;
use crate::text_input::TextEditState;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认行高。对齐 WinUI `TreeViewItemMinHeight = 36`。
pub const TREE_ROW: f32 = 36.0;

/// 紧凑导航窗格行高（`28px`）。
pub const TREE_ROW_COMPACT: f32 = 28.0;

/// 每层缩进深度的水平缩进量。对齐 `TreeViewItem::UpdateIndentation`（depth × 16）。
pub const TREE_INDENT: f32 = 16.0;

/// 视口周围保持存活的额外行数。
pub const TREE_OVERSCAN: f32 = 1.0;

/// 行前置图标槽尺寸（`16×16px`）。
pub const TREE_ICON_SIZE: f32 = 16.0;

/// 行图标槽与文本标签之间的水平间距（`6px`）。
pub const TREE_ICON_GAP: f32 = 6.0;

/// 树形行前置图标槽类型别名（向后兼容 [`WidgetIcon`]）。
pub type TreeRowIcon = WidgetIcon;

/// 行右侧尾缀图标槽尺寸（`12×12px`，用于固定快捷项图钉等提示图标）。
pub const TREE_TRAILING_ICON_SIZE: f32 = 12.0;

/// 行右侧尾缀图标距行右沿的水平留白（`10px`）。
const TREE_TRAILING_ICON_INSET: f32 = 10.0;

/// 树形行节点级扩展展示选项（右侧尾缀图标槽与展开箭头显隐覆盖）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeRowOptions {
    /// 可选右侧尾缀图标（例如固定快捷项的 [`ShellIcon::Pin`]）。
    pub trailing_icon: Option<ShellIcon>,
    /// 是否强制隐藏该节点的展开/折叠箭头（即使 `has_children == true` 也不渲染 `>` 箭头与切换热区）。
    pub hide_expand_chevron: bool,
}

impl TreeRowOptions {
    /// 创建默认行选项（无尾缀图标、不隐藏展开箭头）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            trailing_icon: None,
            hide_expand_chevron: false,
        }
    }

    /// 配置右侧尾缀图标。
    #[must_use]
    pub const fn with_trailing_icon(mut self, icon: Option<ShellIcon>) -> Self {
        self.trailing_icon = icon;
        self
    }

    /// 配置是否强制隐藏展开/折叠箭头。
    #[must_use]
    pub const fn with_hide_expand_chevron(mut self, hide: bool) -> Self {
        self.hide_expand_chevron = hide;
        self
    }
}

/// 树形视图行高密度档位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TreeViewDensity {
    /// WinUI 3 标准密度（`36px` 行高，默认）。
    #[default]
    Standard,
    /// 紧凑导航窗格密度（`28px` 行高）。
    Compact,
}

impl TreeViewDensity {
    /// 返回该密度档位对应的逻辑像素行高。
    #[must_use]
    pub const fn row_height(self) -> f32 {
        match self {
            Self::Standard => TREE_ROW,
            Self::Compact => TREE_ROW_COMPACT,
        }
    }
}

/// 树形视图离散交互事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeViewEvent {
    /// 行被激活（双击或按 `Enter`）。
    Activated {
        occurrence_key: u64,
        logical_id: u64,
    },
    /// 选区发生变化（包含当前全部已选中的 `occurrence_key`）。
    SelectionChanged { selected: Vec<u64> },
    /// 首次展开未加载节点时发出的一次性异步加载请求。
    ExpandRequested {
        occurrence_key: u64,
        logical_id: u64,
    },
    /// 节点被折叠。
    Collapsed {
        occurrence_key: u64,
        logical_id: u64,
    },
    /// 请求在指定屏幕坐标弹出上下文菜单。
    ContextMenuRequested {
        occurrence_key: u64,
        logical_id: u64,
        screen_pos: [i32; 2],
    },
}

/// 单个已实例化的行。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeViewRow {
    /// 扁平化后的行数据。
    pub row: TreeRow,
    /// 该行在内容空间中的索引。
    pub item: usize,
    /// 该行在屏幕上的矩形。
    pub rect: Rect,
}

/// 树形视图的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TreeViewDesc {
    /// 视口矩形。
    pub rect: Rect,
    /// 行高。
    pub row_height: f32,
    /// 行高密度档位（默认 `Standard` `36px`）。
    pub density: TreeViewDensity,
    /// 每层深度的缩进量。
    pub indent: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 是否启用 Windows 11 资源管理器导航窗格预设风格（默认 `false`）。
    pub navigation_pane: bool,
    /// 是否绘制 1px 竖向缩进参考线（标准模式默认 `true`，导航窗格模式自动关闭）。
    pub indent_guides: bool,
    /// 是否绘制左侧 3px 蓝色选中强调指示条（标准模式默认 `true`，导航窗格模式自动关闭）。
    pub selection_indicator: bool,
}

impl TreeViewDesc {
    /// 使用默认尺寸创建的树形视口。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            row_height: TREE_ROW,
            density: TreeViewDensity::Standard,
            indent: TREE_INDENT,
            z: 450,
            navigation_pane: false,
            indent_guides: true,
            selection_indicator: true,
        }
    }

    /// 设置行高密度档位并同步更新 `row_height`。
    #[must_use]
    pub fn with_density(mut self, density: TreeViewDensity) -> Self {
        self.density = density;
        self.row_height = density.row_height();
        self
    }

    /// 启用或关闭 Windows 11 导航窗格预设风格。
    ///
    /// 开启时自动采用 [`TreeViewTokens::navigation_pane`] 配色，并关闭缩进参考线与左侧选中强调条；
    /// 关闭时恢复标准 `TreeView` 的默认视觉配置。
    #[must_use]
    pub fn with_navigation_pane(mut self, enabled: bool) -> Self {
        self.navigation_pane = enabled;
        if enabled {
            self.indent_guides = false;
            self.selection_indicator = false;
        } else {
            self.indent_guides = true;
            self.selection_indicator = true;
        }
        self
    }

    /// 显式开关 1px 竖向缩进参考线。
    #[must_use]
    pub fn with_indent_guides(mut self, enabled: bool) -> Self {
        self.indent_guides = enabled;
        self
    }

    /// 显式开关左侧 3px 蓝色选中强调指示条。
    #[must_use]
    pub fn with_selection_indicator(mut self, enabled: bool) -> Self {
        self.selection_indicator = enabled;
        self
    }

    /// 对应扁平化总行数的滚动描述符。
    #[must_use]
    pub fn scroll_desc(&self, rows: usize) -> ScrollViewDesc {
        ScrollViewDesc::new(
            self.rect,
            unbound_math::Vec2::new(self.rect.size.x, rows as f32 * self.row_height),
        )
    }

    /// 视口内可容纳的行数。
    #[must_use]
    pub fn visible_capacity(&self) -> usize {
        (self.rect.size.y / self.row_height).ceil().max(1.0) as usize
    }
}

/// 树形视图的实时状态。
#[derive(Debug)]
pub struct TreeViewState {
    /// 虚拟化窗口。
    pub tree: VirtualTree,
    /// 滚动弹簧。
    pub scroll: ScrollState,
    /// 已选中的行集合（按 `occurrence_key` 索引）。
    pub selected: FxHashSet<u64>,
    /// 拥有键盘焦点的行（`occurrence_key`）。
    pub focused: Option<u64>,
    /// 当前指针悬停的单行（`occurrence_key`），避免整棵树共享悬停态。
    pub hovered: Option<u64>,
    /// 当前正在重命名的行及其编辑器。
    pub renaming: Option<(u64, TextEditState)>,
    /// 待宿主消费的离散交互事件队列。
    pub pending_events: Vec<TreeViewEvent>,
    /// 按 `occurrence_key` 索引的行前置图标表。
    pub row_icons: FxHashMap<u64, TreeRowIcon>,
    /// 按 `occurrence_key` 索引的行右侧尾缀图标表（例如固定项的 [`ShellIcon::Pin`]）。
    pub row_trailing_icons: FxHashMap<u64, ShellIcon>,
    /// 强制隐藏展开/折叠箭头的出现键集合。
    row_hidden_chevrons: FxHashSet<u64>,
    /// 按 `occurrence_key` 索引的节点显示标签表。
    row_labels: FxHashMap<u64, String>,
    /// 需要在行顶部上方绘制水平分区分隔线的出现键集合。
    row_dividers_before: FxHashSet<u64>,
    /// 已发射过 `ExpandRequested` 且尚未完成加载的出现键集合（用于防重入）。
    requested_loads: FxHashSet<u64>,
    /// 所有扁平化行（用于键盘导航）。
    order: Vec<TreeRow>,
    /// 每个扁平化行的父节点映射（按 `occurrence_key` 索引）。
    parent: FxHashMap<u64, u64>,
    /// 上一次窗口更新统计，供调用方更新脏标记。
    pub last_window: WindowUpdate,
    /// 上一次调用 [`TreeViewState::rows`] 时使用的视口高度。
    viewport_height: f32,
}

impl TreeViewState {
    /// 创建空树状态。
    #[must_use]
    pub fn new(row_height: f32) -> Self {
        Self {
            tree: VirtualTree::new(row_height, TREE_OVERSCAN),
            scroll: ScrollState::new(),
            selected: FxHashSet::default(),
            focused: None,
            hovered: None,
            renaming: None,
            pending_events: Vec::new(),
            row_icons: FxHashMap::default(),
            row_trailing_icons: FxHashMap::default(),
            row_hidden_chevrons: FxHashSet::default(),
            row_labels: FxHashMap::default(),
            row_dividers_before: FxHashSet::default(),
            requested_loads: FxHashSet::default(),
            order: Vec::new(),
            parent: FxHashMap::default(),
            last_window: WindowUpdate::default(),
            viewport_height: 0.0,
        }
    }

    /// 按密度档位创建空树状态。
    #[must_use]
    pub fn new_with_density(density: TreeViewDensity) -> Self {
        Self::new(density.row_height())
    }

    /// 取出并清空待处理的树形交互事件。
    pub fn drain_events(&mut self) -> Vec<TreeViewEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// 为指定出现键配置行前置图标。
    pub fn set_row_icon(&mut self, occurrence_key: u64, icon: TreeRowIcon) {
        if icon == TreeRowIcon::None {
            self.row_icons.remove(&occurrence_key);
        } else {
            self.row_icons.insert(occurrence_key, icon);
        }
    }

    /// 查询指定出现键的行前置图标。
    #[must_use]
    pub fn row_icon(&self, occurrence_key: u64) -> TreeRowIcon {
        self.row_icons
            .get(&occurrence_key)
            .copied()
            .unwrap_or(TreeRowIcon::None)
    }

    /// 设置或清除指定行的右侧尾缀图标（例如固定快捷项的 [`ShellIcon::Pin`]）。
    pub fn set_row_trailing_icon(&mut self, occurrence_key: u64, icon: Option<ShellIcon>) {
        if let Some(icon) = icon {
            self.row_trailing_icons.insert(occurrence_key, icon);
        } else {
            self.row_trailing_icons.remove(&occurrence_key);
        }
    }

    /// 查询指定行的右侧尾缀图标。
    #[must_use]
    pub fn row_trailing_icon(&self, occurrence_key: u64) -> Option<ShellIcon> {
        self.row_trailing_icons.get(&occurrence_key).copied()
    }

    /// 设置指定行是否强制隐藏展开/折叠箭头。
    pub fn set_row_hide_expand_chevron(&mut self, occurrence_key: u64, hide: bool) {
        if hide {
            self.row_hidden_chevrons.insert(occurrence_key);
        } else {
            self.row_hidden_chevrons.remove(&occurrence_key);
        }
    }

    /// 查询指定行是否强制隐藏展开/折叠箭头。
    #[must_use]
    pub fn is_row_expand_chevron_hidden(&self, occurrence_key: u64) -> bool {
        self.row_hidden_chevrons.contains(&occurrence_key)
    }

    /// 汇总指定行的节点级扩展展示选项（右侧尾缀图标与展开箭头显隐覆盖）。
    #[must_use]
    pub fn row_options(&self, occurrence_key: u64) -> TreeRowOptions {
        TreeRowOptions {
            trailing_icon: self.row_trailing_icon(occurrence_key),
            hide_expand_chevron: self.is_row_expand_chevron_hidden(occurrence_key),
        }
    }

    /// 为指定行设置显示标签文本。
    pub fn set_row_label(&mut self, id: u64, label: impl Into<String>) {
        self.row_labels.insert(id, label.into());
    }

    /// 查询指定行的显示标签文本。
    #[must_use]
    pub fn row_label(&self, id: u64) -> Option<&str> {
        self.row_labels.get(&id).map(String::as_str)
    }

    /// 清空全部已注册的行显示标签。
    pub fn clear_row_labels(&mut self) {
        self.row_labels.clear();
    }

    /// 配置是否在指定行顶部上方绘制柔和水平分区分隔线。
    pub fn set_row_divider_before(&mut self, id: u64, enabled: bool) {
        if enabled {
            self.row_dividers_before.insert(id);
        } else {
            self.row_dividers_before.remove(&id);
        }
    }

    /// 查询指定行顶部上方是否启用了水平分区分隔线。
    #[must_use]
    pub fn has_row_divider_before(&self, id: u64) -> bool {
        self.row_dividers_before.contains(&id)
    }

    /// 清空全部行前置分隔线标记。
    pub fn clear_row_dividers(&mut self) {
        self.row_dividers_before.clear();
    }

    /// 返回当前指针悬停的单行出现键。
    #[must_use]
    pub fn hovered_row(&self) -> Option<u64> {
        self.hovered
    }

    /// 根据指针坐标执行视口内单行悬停命中测试，并更新 `hovered` 状态。
    pub fn hover_at(&mut self, desc: &TreeViewDesc, point: Vec2) -> Option<u64> {
        if !desc.rect.contains_point(point) {
            self.hovered = None;
            return None;
        }
        let hit = self
            .rows(desc)
            .into_iter()
            .find(|row| row.rect.contains_point(point))
            .map(|row| row.row.id);
        self.hovered = hit;
        hit
    }

    /// 清除当前单行悬停状态。
    pub fn clear_hover(&mut self) {
        self.hovered = None;
    }

    /// 替换数据模型。
    ///
    /// `children_of` 仅对已展开且已完成加载（`is_loaded == true`）的节点调用，
    /// 因此开销与树中可见部分成正比，而非整份文档。
    pub fn rebuild<F>(&mut self, roots: &[TreeRow], children_of: F)
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        self.order.clear();
        self.parent.clear();
        // 扁平化一次以记录 ArrowLeft 所需的父节点链接；虚拟树使用相同展开集合执行相同遍历。
        let mut stack: Vec<(TreeRow, Option<u64>)> =
            roots.iter().rev().map(|row| (*row, None)).collect();
        while let Some((mut row, parent)) = stack.pop() {
            row.expanded = self.tree.is_expanded(row.id);
            if row.is_loaded {
                self.requested_loads.remove(&row.id);
            }
            self.order.push(row);
            if let Some(parent) = parent {
                self.parent.insert(row.id, parent);
            }
            if row.expanded && row.is_loaded {
                for child in children_of(row.id).into_iter().rev() {
                    stack.push((child, Some(row.id)));
                }
            }
        }
        self.tree.rebuild(roots, children_of);
    }

    /// 扁平化后的总行数。
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.order.len()
    }

    /// 按出现键查找扁平化行。
    #[must_use]
    pub fn row(&self, id: u64) -> Option<TreeRow> {
        self.order.iter().copied().find(|row| row.id == id)
    }

    /// 获取当前可见行的父节点出现键。
    #[must_use]
    pub fn parent_of(&self, id: u64) -> Option<u64> {
        self.parent.get(&id).copied()
    }

    /// 当前滚动偏移量（含运动中的弹簧）。
    #[must_use]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset().y
    }

    /// 请求展开指定出现键对应的节点。
    ///
    /// 当节点 `has_children == true` 且 `is_loaded == false && !is_loading` 时，
    /// 乐观记录展开态并**精确发射一次** [`TreeViewEvent::ExpandRequested`]；
    /// 加载中或重复请求期间不会重入发射。
    pub fn request_expand(&mut self, occurrence_key: u64) -> bool {
        let Some(row) = self.row(occurrence_key) else {
            return false;
        };
        if !row.has_children {
            return false;
        }
        if !row.is_loaded {
            self.tree.set_expanded(occurrence_key, true);
            if !row.is_loading && self.requested_loads.insert(occurrence_key) {
                self.pending_events.push(TreeViewEvent::ExpandRequested {
                    occurrence_key,
                    logical_id: row.logical_id,
                });
                return true;
            }
            return false;
        }
        if !self.tree.is_expanded(occurrence_key) {
            self.tree.set_expanded(occurrence_key, true);
            return true;
        }
        false
    }

    /// 展开或折叠指定行，随后重建扁平化列表。
    pub fn set_expanded<F>(
        &mut self,
        id: u64,
        expanded: bool,
        roots: &[TreeRow],
        children_of: F,
    ) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        if self.tree.is_expanded(id) == expanded {
            return false;
        }
        let target_row = self.row(id);
        if expanded {
            if let Some(row) = target_row {
                if row.has_children && !row.is_loaded {
                    self.tree.set_expanded(id, true);
                    if !row.is_loading && self.requested_loads.insert(id) {
                        self.pending_events.push(TreeViewEvent::ExpandRequested {
                            occurrence_key: id,
                            logical_id: row.logical_id,
                        });
                    }
                    self.rebuild(roots, children_of);
                    return true;
                }
            }
        } else {
            self.requested_loads.remove(&id);
            if let Some(row) = target_row {
                self.pending_events.push(TreeViewEvent::Collapsed {
                    occurrence_key: id,
                    logical_id: row.logical_id,
                });
            }
        }
        self.tree.set_expanded(id, expanded);
        self.rebuild(roots, children_of);
        true
    }

    /// 切换指定行的展开状态，随后重建扁平化列表。
    pub fn toggle_expanded<F>(&mut self, id: u64, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let expanded = self.tree.is_expanded(id);
        self.set_expanded(id, !expanded, roots, children_of)
    }

    /// 选中指定行（按 `occurrence_key`）。若 `additive` 为 false，则替换先前选区。
    pub fn select(&mut self, id: u64, additive: bool) -> bool {
        let changed = if additive {
            if self.selected.contains(&id) {
                self.selected.remove(&id);
                true
            } else {
                self.selected.insert(id);
                true
            }
        } else {
            let changed = self.selected.len() != 1 || !self.selected.contains(&id);
            self.selected.clear();
            self.selected.insert(id);
            changed
        };
        self.focused = Some(id);
        if changed {
            let mut selected: Vec<u64> = self.selected.iter().copied().collect();
            selected.sort_unstable();
            self.pending_events
                .push(TreeViewEvent::SelectionChanged { selected });
        }
        changed
    }

    /// 激活指定行（双击或按 `Enter`），发射 [`TreeViewEvent::Activated`]。
    pub fn activate(&mut self, occurrence_key: u64) -> bool {
        let Some(row) = self.row(occurrence_key) else {
            return false;
        };
        self.pending_events.push(TreeViewEvent::Activated {
            occurrence_key,
            logical_id: row.logical_id,
        });
        true
    }

    /// 在指定屏幕坐标为目标行发射上下文菜单请求事件。
    pub fn request_context_menu(&mut self, occurrence_key: u64, screen_pos: [i32; 2]) -> bool {
        let Some(row) = self.row(occurrence_key) else {
            return false;
        };
        self.pending_events
            .push(TreeViewEvent::ContextMenuRequested {
                occurrence_key,
                logical_id: row.logical_id,
                screen_pos,
            });
        true
    }

    /// 清空选区。
    pub fn clear_selection(&mut self) {
        if !self.selected.is_empty() {
            self.selected.clear();
            self.pending_events.push(TreeViewEvent::SelectionChanged {
                selected: Vec::new(),
            });
        }
    }

    /// 按指定行数移动键盘焦点。
    pub fn focus_step(&mut self, delta: isize) -> bool {
        if self.order.is_empty() {
            return false;
        }
        let current = self
            .focused
            .and_then(|id| self.order.iter().position(|row| row.id == id))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.order.len() as isize - 1) as usize;
        let id = self.order[next].id;
        self.select(id, false);
        true
    }

    /// 将焦点移至父行；若当前聚焦行已展开，则先折叠它。
    /// 这是 `ArrowLeft` 的交互约定。
    pub fn focus_parent_or_collapse<F>(&mut self, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let Some(id) = self.focused else {
            return false;
        };
        if self.tree.is_expanded(id) {
            return self.set_expanded(id, false, roots, children_of);
        }
        let Some(parent) = self.parent_of(id) else {
            return false;
        };
        self.select(parent, false);
        let position = self
            .order
            .iter()
            .position(|row| row.id == parent)
            .unwrap_or(0);
        self.reveal(position);
        true
    }

    /// 展开当前聚焦行；若已展开且已加载，则聚焦其第一个子节点。
    pub fn expand_or_first_child<F>(&mut self, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let Some(id) = self.focused else {
            return false;
        };
        let Some(row) = self.row(id) else {
            return false;
        };
        if !row.has_children {
            return false;
        }
        if !self.tree.is_expanded(id) {
            return self.set_expanded(id, true, roots, children_of);
        }
        if !row.is_loaded {
            return self.request_expand(id);
        }
        let position = self.order.iter().position(|row| row.id == id).unwrap_or(0);
        if let Some(child) = self.order.get(position + 1).copied() {
            if self.parent_of(child.id) == Some(id) {
                self.select(child.id, false);
                return true;
            }
        }
        false
    }

    /// 开始内联重命名。
    pub fn begin_rename(&mut self, id: u64, current: impl Into<String>) -> bool {
        if self.row(id).is_none() {
            return false;
        }
        let mut editor = TextEditState::new();
        editor.set_text(current);
        self.renaming = Some((id, editor));
        self.focused = Some(id);
        true
    }

    /// 若当前正在重命名，返回重命名编辑器引用。
    #[must_use]
    pub fn rename_editor(&self) -> Option<&TextEditState> {
        self.renaming.as_ref().map(|(_, editor)| editor)
    }

    /// 完成重命名并返回新标题。
    pub fn finish_rename(&mut self) -> Option<(u64, String)> {
        self.renaming
            .take()
            .map(|(id, editor)| (id, editor.text().to_owned()))
    }

    /// 取消重命名。
    pub fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    /// 滚动使指定扁平化行可见。
    pub fn reveal(&mut self, item: usize) {
        let rows = self.order.len();
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 1.0, self.viewport_height),
            unbound_math::Vec2::new(1.0, rows as f32 * self.row_height()),
        );
        let y = item as f32 * self.row_height();
        if y < self.scroll_offset() {
            self.scroll.jump_to(&desc, unbound_math::Vec2::new(0.0, y));
        } else if y + self.row_height() > self.scroll_offset() + self.viewport_height {
            self.scroll.jump_to(
                &desc,
                unbound_math::Vec2::new(0.0, y + self.row_height() - self.viewport_height),
            );
        }
    }

    /// 虚拟化器使用的行高。
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.tree.row_height()
    }

    /// 上一次调用 [`Self::rows`] 时使用的视口高度。
    pub fn set_viewport_height(&mut self, height: f32) {
        self.viewport_height = height.max(0.0);
    }

    /// 单帧的实例化行列表。
    ///
    /// 仅返回视口内（及紧邻周围）的槽位，这是虚拟化器恒定实体数量的保证。
    pub fn rows(&mut self, desc: &TreeViewDesc) -> Vec<TreeViewRow> {
        self.set_viewport_height(desc.rect.size.y);
        if desc.rect.size.x <= 0.0 || desc.rect.size.y <= 0.0 {
            return Vec::new();
        }
        let scroll_desc = desc.scroll_desc(self.order.len());
        let clamped_scroll_y = scroll_desc.clamp_offset(self.scroll.offset()).y;
        self.last_window = self.tree.update(clamped_scroll_y, desc.rect.size.y);
        let mut out = Vec::with_capacity(self.tree.slot_count());
        for slot in 0..self.tree.slot_count() {
            let (Some(row), Some(item)) = (self.tree.row_at(slot), self.tree.slot_item(slot))
            else {
                continue;
            };
            let y = desc.rect.origin.y + self.tree.item_y(item) - clamped_scroll_y;
            let rect = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.row_height);
            if rect.max().y < desc.rect.origin.y || rect.origin.y > desc.rect.max().y {
                continue;
            }
            out.push(TreeViewRow { row, item, rect });
        }
        out
    }

    /// 扁平化行的总内容高度。
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.order.len() as f32 * self.row_height()
    }

    /// 按滚轮增量滚动。
    pub fn scroll_by(&mut self, desc: &TreeViewDesc, delta: f32) -> bool {
        let scroll_desc = desc.scroll_desc(self.order.len());
        self.scroll
            .scroll_by(&scroll_desc, unbound_math::Vec2::new(0.0, delta));
        self.scroll.is_animating()
    }

    /// 推进滚动弹簧步进，返回是否仍在运动。
    pub fn advance(&mut self, desc: &TreeViewDesc, dt: f32) -> bool {
        let scroll_desc = desc.scroll_desc(self.order.len());
        self.scroll.step(&scroll_desc, dt)
    }
}

/// 树形视图绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeViewTokens {
    /// 视口背景。
    pub background: Token,
    /// 空闲行。
    pub row: Token,
    /// 悬停行。
    pub row_hover: Token,
    /// 选中行。
    pub row_selected: Token,
    /// 空闲行文本。
    pub text: Token,
    /// 选中行文本。
    pub text_selected: Token,
    /// 缩进参考线。
    pub guide: Token,
    /// 展开箭头。
    pub arrow: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl TreeViewTokens {
    /// 标准配色集（对齐 WinUI 3 `TreeView` 基准）。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            background: Token::SURFACE,
            row: Token::NONE,
            row_hover: Token::HOVER, // TreeViewItemBackgroundPointerOver = SubtleFillColorSecondary
            row_selected: Token::HOVER, // TreeViewItemBackgroundSelected = SubtleFillColorSecondary
            text: Token::TEXT_PRIMARY,
            text_selected: Token::TEXT_PRIMARY, // TreeViewItemForegroundSelected = TextFillColorPrimary
            guide: Token::DIVIDER,
            arrow: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }

    /// 导航窗格配色集（使用 `Token::NAV_HOVER` / `Token::NAV_SELECTED`）。
    #[must_use]
    pub fn navigation_pane() -> Self {
        Self {
            background: Token::NAV_BG,
            row: Token::NONE,
            row_hover: Token::NAV_HOVER,
            row_selected: Token::NAV_SELECTED,
            text: Token::TEXT_PRIMARY,
            text_selected: Token::TEXT_PRIMARY,
            guide: Token::DIVIDER,
            arrow: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 计算单行文本标签的建议起始 X 坐标（对齐箭头热区与可选 `16×16` 图标槽）。
#[must_use]
pub fn tree_row_label_x(desc: &TreeViewDesc, depth: u16, has_icon: bool) -> f32 {
    let base = desc.rect.origin.x + f32::from(depth) * desc.indent + 32.0;
    if has_icon {
        base + TREE_ICON_SIZE + TREE_ICON_GAP
    } else {
        base
    }
}

/// 计算树形行右侧尾缀图标槽矩形（`12×12px`，垂直居中并距行右沿保留 `10px` 留白）。
#[must_use]
pub fn tree_row_trailing_icon_rect(row_rect: Rect) -> Rect {
    let size = TREE_TRAILING_ICON_SIZE
        .min(row_rect.size.x.max(0.0))
        .min(row_rect.size.y.max(0.0));
    let x = (row_rect.max().x - TREE_TRAILING_ICON_INSET - size).max(row_rect.origin.x);
    let y = row_rect.origin.y + (row_rect.size.y - size) * 0.5;
    Rect::new(x, y, size, size)
}

/// 计算单行文本标签的最大可用宽度（当存在右侧尾缀图标时自动预留尾缀槽位宽度与间距）。
#[must_use]
pub fn tree_row_label_max_width(
    desc: &TreeViewDesc,
    depth: u16,
    has_icon: bool,
    has_trailing_icon: bool,
) -> f32 {
    let label_x = tree_row_label_x(desc, depth, has_icon);
    let right_reserve = if has_trailing_icon {
        TREE_TRAILING_ICON_INSET + TREE_TRAILING_ICON_SIZE + TREE_ICON_GAP
    } else {
        8.0
    };
    (desc.rect.max().x - label_x - right_reserve).max(0.0)
}

/// 输出单行树节点：缩进参考线、展开箭头、标签与选中态（无行图标，保持既有签名向后兼容）。
#[must_use]
pub fn emit_tree_row(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    selected: bool,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TreeViewTokens,
) -> WidgetOutput {
    emit_tree_row_with_icon(desc, row, TreeRowIcon::None, selected, control, label, tokens)
}

/// 输出带前置图标槽的单行树节点（支持 `Vector(ShellIcon)` 与 `ExternalTexture(u64)` 双通道，
/// 并在 24 态交互组合下保持图元数严格恒定）。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn emit_tree_row_with_icon(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    icon: TreeRowIcon,
    selected: bool,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TreeViewTokens,
) -> WidgetOutput {
    emit_tree_row_with_options(
        desc,
        row,
        icon,
        TreeRowOptions::default(),
        selected,
        control,
        label,
        tokens,
    )
}

/// 输出带前置图标槽与节点级扩展选项（右侧尾缀图标、按节点隐藏展开箭头）的单行树节点。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn emit_tree_row_with_options(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    icon: TreeRowIcon,
    options: TreeRowOptions,
    selected: bool,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TreeViewTokens,
) -> WidgetOutput {
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(10 + label.len() + row.row.depth as usize);
    let mut payloads = Vec::with_capacity(label.len() + 4);
    let rect = row.rect;

    let fill = if !disabled && control.phase == PointerPhase::Pressed {
        Token::PRESSED
    } else if selected {
        tokens.row_selected
    } else if !disabled && control.phase == PointerPhase::Hovered {
        tokens.row_hover
    } else {
        tokens.row
    };
    // 行内容统一位于 `row_z = desc.z + 1`，与 `emit_tree_background`（`desc.z`）严格分离，
    // 消除文件树底板与行 Hover/选中胶囊同 `(z, SubElement::Background)` 互盖闪烁。
    let row_z = desc.z.saturating_add(1);

    // TreeViewItemPresenterMargin = 4,2：行背景在列表内左右各内缩 4、上下各 2。
    let background = Rect::new(
        rect.origin.x + 4.0,
        rect.origin.y + 2.0,
        (rect.size.x - 8.0).max(0.0),
        (rect.size.y - 4.0).max(0.0),
    );
    primitives.push(outlined(
        background,
        4.0, // ControlCornerRadius = 4
        fill,
        Token::NONE,
        0.0,
        0.0,
        row_z,
    ));

    if desc.selection_indicator {
        // 左侧选中强调指示条（未选中时透明度为 0，保持图元数恒定）
        // 对齐 WinUI 3 关键规范：3×16px 垂直居中、圆角 1.5px 的选中胶囊条，使用 Token::PRIMARY
        let pill_h = (rect.size.y - 12.0).clamp(12.0, 16.0);
        let indicator = Rect::new(
            rect.origin.x + 4.0,
            rect.origin.y + (rect.size.y - pill_h) * 0.5,
            3.0,
            pill_h,
        );
        let mut indicator_inst = instance(
            indicator,
            [1.5; 4],
            Token::PRIMARY,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            row_z,
        );
        if !selected {
            indicator_inst.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(indicator_inst.depth_u16(), 0.0);
        }
        primitives.push(indicator_inst);
    }

    // 每个深度层级一条参考线，停在箭头列之前（导航窗格模式下默认关闭）。
    let indent = desc.indent;
    if desc.indent_guides {
        for level in 0..row.row.depth as usize {
            let x = rect.origin.x + level as f32 * indent + indent * 0.5;
            let guide = Rect::new(x, rect.origin.y, 1.0, rect.size.y);
            primitives.push(instance(
                guide,
                [0.0; 4],
                tokens.guide,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Content,
                row_z,
            ));
        }
    }

    // ExpandCollapseChevron 左侧 Padding = 14，字形 12 × 12，独立热区 20 × 20。
    let show_chevron = row.row.has_children && !options.hide_expand_chevron;
    let arrow_x = rect.origin.x + row.row.depth as f32 * indent + 14.0;
    let centre = [arrow_x + 6.0, rect.origin.y + rect.size.y * 0.5];
    if show_chevron {
        // 朝右的箭头；展开状态时向下旋转。加载期间调低对比度提示正在拉取。
        let arrow_color = if disabled || row.row.is_loading {
            Token::TEXT_DISABLED
        } else {
            tokens.arrow
        };
        let arm = 4.0;
        let (a, b, c) = if row.row.expanded {
            (
                [centre[0] - arm, centre[1] - arm * 0.5],
                [centre[0], centre[1] + arm * 0.5],
                [centre[0] + arm, centre[1] - arm * 0.5],
            )
        } else {
            (
                [centre[0] - arm * 0.5, centre[1] - arm],
                [centre[0] + arm * 0.5, centre[1]],
                [centre[0] - arm * 0.5, centre[1] + arm],
            )
        };
        curve_stroke(
            a,
            b,
            1.5,
            arrow_color,
            row_z,
            &mut primitives,
            &mut payloads,
        );
        curve_stroke(
            b,
            c,
            1.5,
            arrow_color,
            row_z,
            &mut primitives,
            &mut payloads,
        );
    }

    // 16×16 前置行图标槽（位于箭头热区之后、标签文本之前）
    let icon_rect = Rect::new(
        arrow_x + 16.0,
        rect.origin.y + (rect.size.y - TREE_ICON_SIZE) * 0.5,
        TREE_ICON_SIZE,
        TREE_ICON_SIZE,
    );
    let icon_color = if disabled {
        Token::TEXT_DISABLED
    } else if selected {
        tokens.text_selected
    } else {
        tokens.text
    };
    emit_widget_icon_primitives(
        icon,
        icon_rect,
        icon_color,
        fill,
        disabled,
        row_z,
        &mut primitives,
        &mut payloads,
    );

    if let Some(trailing_icon) = options.trailing_icon {
        let trailing_rect = tree_row_trailing_icon_rect(rect);
        let trailing_color = if disabled {
            Token::TEXT_DISABLED
        } else {
            tokens.arrow
        };
        emit_shell_icon_primitives(
            trailing_icon,
            trailing_rect,
            trailing_color,
            fill,
            row_z,
            &mut primitives,
            &mut payloads,
        );
    }

    glyphs(
        label,
        if disabled {
            Token::TEXT_DISABLED
        } else if selected {
            tokens.text_selected
        } else {
            tokens.text
        },
        row_z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        5.0,
        tokens.focus_ring,
        2.0,
        row_z,
        control.focused && !disabled,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(row.row.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::TreeItem, "", rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = selected;
    a11y.level = u32::from(row.row.depth) + 1;
    if show_chevron {
        a11y.expanded = Some(row.row.expanded);
    }

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    };
    if show_chevron && !disabled {
        let toggle_key = tree_toggle_node(row.row.id);
        let toggle_h = 20.0f32.min(rect.size.y);
        let toggle_r = Rect::new(
            arrow_x - 4.0,
            rect.origin.y + (rect.size.y - toggle_h) * 0.5,
            20.0,
            toggle_h,
        );
        out.with_sub_control(
            toggle_key,
            HitRegion::new(toggle_key, CONTROL_LAYER, u32::from(desc.z) + 1, toggle_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Toggle branch", toggle_r).clickable(),
            false,
        );
    }
    out
}

/// 树行展开/折叠箭头的确定性子节点键。
#[must_use]
pub const fn tree_toggle_node(row_id: u64) -> u64 {
    row_id ^ 0x5452_4545_0001
}

/// 输出位于树行顶部上方的柔和 1px 水平分区分隔线（左右各留白 `12px`）。
#[must_use]
pub fn emit_tree_row_divider(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    tokens: TreeViewTokens,
) -> WidgetOutput {
    let inset = 12.0;
    let width = (row.rect.size.x - inset * 2.0).max(0.0);
    let primitives = if width > 0.0 {
        let line_rect = Rect::new(row.rect.origin.x + inset, row.rect.origin.y, width, 1.0);
        vec![instance(
            line_rect,
            [0.0; 4],
            tokens.guide,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Content,
            desc.z.saturating_add(1),
        )]
    } else {
        Vec::new()
    };
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 输出视口背景。
#[must_use]
pub fn emit_tree_background(desc: &TreeViewDesc, tokens: TreeViewTokens) -> WidgetOutput {
    WidgetOutput {
        primitives: vec![outlined(
            desc.rect,
            0.0,
            tokens.background,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        )],
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellIcon;

    fn root(id: u64) -> TreeRow {
        TreeRow::new(id, 0, true)
    }

    fn leaf(id: u64, depth: u16) -> TreeRow {
        TreeRow::new(id, depth, false)
    }

    fn children(id: u64) -> Vec<TreeRow> {
        match id {
            1 => vec![leaf(11, 1), leaf(12, 1)],
            2 => vec![leaf(21, 1)],
            _ => Vec::new(),
        }
    }

    fn state() -> TreeViewState {
        let mut state = TreeViewState::new(TREE_ROW);
        state.rebuild(&[root(1), root(2)], children);
        state
    }

    #[test]
    fn tree_row_trailing_icon_and_hidden_expand_chevron_work_per_node() {
        let mut state = state();
        state.set_row_trailing_icon(1, Some(ShellIcon::Pin));
        state.set_row_hide_expand_chevron(1, true);

        let opts1 = state.row_options(1);
        assert_eq!(opts1.trailing_icon, Some(ShellIcon::Pin));
        assert!(opts1.hide_expand_chevron);
        assert_eq!(state.row_options(2), TreeRowOptions::default());

        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 240.0, 200.0)).with_navigation_pane(true);
        let rows = state.rows(&desc);
        let tokens = TreeViewTokens::navigation_pane();

        // 节点 1（has_children = true，但隐藏展开箭头 + 带右侧图钉尾缀）
        let pinned_out = emit_tree_row_with_options(
            &desc,
            &rows[0],
            TreeRowIcon::Vector(ShellIcon::Folder),
            opts1,
            false,
            ControlState::default(),
            &[],
            tokens,
        );
        // 隐藏展开箭头后不注册 toggle 子热区且 a11y.expanded 为 None
        let toggle_key = tree_toggle_node(rows[0].row.id);
        assert!(
            !pinned_out.sub_hits.iter().any(|h| h.node == toggle_key),
            "hidden expand chevron must not register toggle hit region"
        );
        assert_eq!(
            pinned_out.a11y.as_ref().and_then(|a| a.expanded),
            None,
            "hidden expand chevron must clear a11y.expanded"
        );

        // 右侧尾缀图标应落在行右半区（与 tree_row_trailing_icon_rect 对齐）
        let trailing_rect = tree_row_trailing_icon_rect(rows[0].rect);
        assert!(trailing_rect.origin.x > rows[0].rect.center().x);
        assert!(trailing_rect.max().x <= rows[0].rect.max().x);
        assert!(
            pinned_out.primitives.iter().any(|p| {
                let cx = p.local_pos[0] + p.size[0] * 0.5;
                cx >= trailing_rect.origin.x - 4.0 && cx <= trailing_rect.max().x + 4.0
            }),
            "trailing Pin icon primitives must be emitted in the trailing icon rect"
        );

        // 存在尾缀图标时标签可用宽度应严格小于无尾缀图标
        let w_plain = tree_row_label_max_width(&desc, 0, true, false);
        let w_pinned = tree_row_label_max_width(&desc, 0, true, true);
        assert!(w_pinned < w_plain);

        // 节点 2 未隐藏展开箭头：仍正常注册 toggle 子热区与 a11y.expanded
        let normal_out = emit_tree_row_with_options(
            &desc,
            &rows[1],
            TreeRowIcon::Vector(ShellIcon::Folder),
            state.row_options(2),
            false,
            ControlState::default(),
            &[],
            tokens,
        );
        assert!(normal_out
            .sub_hits
            .iter()
            .any(|h| h.node == tree_toggle_node(rows[1].row.id)));
        assert_eq!(
            normal_out.a11y.as_ref().and_then(|a| a.expanded),
            Some(false)
        );
    }

    #[test]
    fn a_collapsed_tree_flattens_to_its_roots() {
        let state = state();
        assert_eq!(state.row_count(), 2);
        assert_eq!(state.row(1).expect("root visible").depth, 0);
    }

    #[test]
    fn expanding_a_root_adds_its_children() {
        let mut state = state();
        assert!(state.set_expanded(1, true, &[root(1), root(2)], children));
        assert_eq!(state.row_count(), 4);
        assert_eq!(state.parent_of(11), Some(1));
        assert!(!state.set_expanded(1, true, &[root(1), root(2)], children));
    }

    #[test]
    fn only_visible_rows_are_instantiated() {
        let mut state = TreeViewState::new(TREE_ROW);
        // 单个根节点下挂 10 000 个叶子节点。
        let kids: Vec<TreeRow> = (100..10_100).map(|id| leaf(id, 1)).collect();
        state.rebuild(
            &[root(1)],
            move |id| if id == 1 { kids.clone() } else { Vec::new() },
        );
        state.set_expanded(1, true, &[root(1)], move |id| {
            if id == 1 {
                (100..10_100).map(|id| leaf(id, 1)).collect()
            } else {
                Vec::new()
            }
        });
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        assert!(
            rows.len() <= 40,
            "a 10k-row tree must instantiate a viewport budget, got {}",
            rows.len()
        );
        assert!(rows.len() >= desc.visible_capacity());
    }

    #[test]
    fn scrolling_moves_the_window_without_growing_it() {
        let mut state = TreeViewState::new(TREE_ROW);
        let kids: Vec<TreeRow> = (100..10_100).map(|id| leaf(id, 1)).collect();
        let roots = [root(1)];
        state.rebuild(&roots, |_| kids.clone());
        state.set_expanded(1, true, &roots, |_| kids.clone());
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let first = state.rows(&desc).len();
        state.scroll_by(&desc, 2600.0);
        state.scroll.jump_to(
            &desc.scroll_desc(state.row_count()),
            unbound_math::Vec2::new(0.0, 2600.0),
        );
        let moved = state.rows(&desc);
        assert!(
            moved.len() <= 40,
            "scrolling must not grow the live set, got {}",
            moved.len()
        );
        assert!(
            (moved.len() as isize - first as isize).abs() <= 1,
            "the window keeps roughly the same size"
        );
        assert!(moved[0].item > 0);
    }

    #[test]
    fn selection_replaces_or_extends() {
        let mut state = state();
        assert!(state.select(1, false));
        assert!(state.select(2, true));
        assert_eq!(state.selected.len(), 2);
        assert!(state.select(2, true));
        assert_eq!(state.selected.len(), 1);
        state.clear_selection();
        assert!(state.selected.is_empty());
    }

    #[test]
    fn focus_steps_clamp_at_the_ends() {
        let mut state = state();
        assert!(state.focus_step(1));
        assert_eq!(state.focused, Some(2));
        assert!(state.focus_step(10));
        assert_eq!(state.focused, Some(2));
    }

    #[test]
    fn arrow_left_collapses_then_jumps_to_the_parent() {
        let roots = [root(1), root(2)];
        let mut state = TreeViewState::new(TREE_ROW);
        state.rebuild(&roots, children);
        state.set_expanded(1, true, &roots, children);
        state.select(11, false);
        assert!(state.focus_parent_or_collapse(&roots, children));
        assert_eq!(state.focused, Some(1));
        assert!(state.focus_parent_or_collapse(&roots, children));
        assert!(!state.tree.is_expanded(1));
        assert!(!state.focus_parent_or_collapse(&roots, children));
    }

    #[test]
    fn arrow_right_expands_then_enters_the_first_child() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.select(1, false);
        assert!(state.expand_or_first_child(&roots, children));
        assert!(state.tree.is_expanded(1));
        assert!(state.expand_or_first_child(&roots, children));
        assert_eq!(state.focused, Some(11));
        assert!(!state.expand_or_first_child(&roots, children));
    }

    #[test]
    fn rename_opens_with_the_current_title_and_commits() {
        let mut state = state();
        assert!(state.begin_rename(1, "Project"));
        assert_eq!(state.rename_editor().expect("editor").text(), "Project");
        let (id, title) = state.finish_rename().expect("committed");
        assert_eq!(id, 1);
        assert_eq!(title, "Project");
        assert!(state.rename_editor().is_none());
        assert!(!state.begin_rename(999, "missing"));
    }

    #[test]
    fn a_row_is_pickable_and_reports_its_depth() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.set_expanded(1, true, &roots, children);
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        let child = rows
            .iter()
            .find(|row| row.row.id == 11)
            .expect("child visible");
        let output = emit_tree_row(
            &desc,
            child,
            false,
            ControlState::default(),
            &[],
            TreeViewTokens::standard(),
        );
        let hit = output.hit.expect("a row is pickable");
        assert_eq!(hit.node, 11);
        let a11y = output.a11y.expect("a row is reachable");
        assert_eq!(a11y.role, Role::TreeItem);
        assert_eq!(a11y.level, 2);
    }

    #[test]
    fn an_expanded_row_reports_expansion() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.set_expanded(1, true, &roots, children);
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        let parent = rows.iter().find(|row| row.row.id == 1).expect("root");
        let output = emit_tree_row(
            &desc,
            parent,
            true,
            ControlState::default(),
            &[],
            TreeViewTokens::standard(),
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.expanded, Some(true));
        assert!(a11y.selected);
    }

    #[test]
    fn a_selected_row_uses_the_selected_token() {
        let state = state();
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = {
            let mut state = state;
            state.rows(&desc)
        };
        let tokens = TreeViewTokens::standard();
        let output = emit_tree_row(&desc, &rows[0], true, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.row_selected.raw() as u32
        );
    }

    #[test]
    fn occurrence_key_isolates_duplicate_logical_nodes_in_tree_view() {
        let quick = TreeRow::new(100, 0, true).with_logical_id(500);
        let drive = TreeRow::new(200, 0, true).with_logical_id(500);
        let kids = |occurrence_key: u64| match occurrence_key {
            100 => vec![TreeRow::new(101, 1, false).with_logical_id(501)],
            200 => vec![TreeRow::new(201, 1, false).with_logical_id(501)],
            _ => Vec::new(),
        };
        let mut state = TreeViewState::new_with_density(TreeViewDensity::Compact);
        state.rebuild(&[quick, drive], kids);
        assert!(state.set_expanded(100, true, &[quick, drive], kids));
        assert!(state.select(100, false));

        assert!(state.tree.is_expanded(100));
        assert!(!state.tree.is_expanded(200));
        assert!(state.selected.contains(&100));
        assert!(!state.selected.contains(&200));
        assert_eq!(state.row_count(), 3);
    }

    #[test]
    fn lazy_expand_emits_single_request_and_suppresses_reentry() {
        let mut state = TreeViewState::new_with_density(TreeViewDensity::Compact);
        let unloaded = TreeRow::new(300, 0, true)
            .with_logical_id(900)
            .with_loaded(false);
        let kids = |id: u64| {
            if id == 300 {
                vec![TreeRow::new(301, 1, false).with_logical_id(901)]
            } else {
                Vec::new()
            }
        };

        state.rebuild(&[unloaded], kids);
        assert!(state.request_expand(300));
        assert!(!state.request_expand(300), "duplicate expand must be suppressed");
        assert!(state.tree.is_expanded(300));

        let events = state.drain_events();
        assert_eq!(
            events,
            vec![TreeViewEvent::ExpandRequested {
                occurrence_key: 300,
                logical_id: 900,
            }]
        );

        // 加载期间即使重建也不渲染旧子节点，且 `is_loading == true` 抑制重入
        let loading = unloaded.with_loading(true);
        state.rebuild(&[loading], kids);
        assert_eq!(state.row_count(), 1);
        assert!(!state.request_expand(300));
        assert!(state.drain_events().is_empty());

        // 宿主回填并置 `is_loaded = true` 后自动展示子树
        let loaded = unloaded.with_loaded(true).with_loading(false);
        state.rebuild(&[loaded], kids);
        assert_eq!(state.row_count(), 2);
    }

    #[test]
    fn compact_density_uses_28px_rows_and_emits_icon_slots() {
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 140.0))
            .with_density(TreeViewDensity::Compact);
        assert_eq!(desc.density, TreeViewDensity::Compact);
        assert!((desc.row_height - TREE_ROW_COMPACT).abs() < f32::EPSILON);

        let mut state = TreeViewState::new_with_density(TreeViewDensity::Compact);
        state.set_row_icon(1, TreeRowIcon::Vector(ShellIcon::Folder));
        state.set_row_icon(2, TreeRowIcon::ExternalTexture(77));
        state.rebuild(&[root(1), root(2)], children);
        let rows = state.rows(&desc);
        assert_eq!(rows.len(), 2);
        assert!((rows[0].rect.size.y - 28.0).abs() < f32::EPSILON);

        let vec_out = emit_tree_row_with_icon(
            &desc,
            &rows[0],
            state.row_icon(1),
            true,
            ControlState::default(),
            &[],
            TreeViewTokens::navigation_pane(),
        );
        let base_out = emit_tree_row(
            &desc,
            &rows[0],
            true,
            ControlState::default(),
            &[],
            TreeViewTokens::navigation_pane(),
        );
        assert!(vec_out.primitives.len() > base_out.primitives.len());

        let tex_out = emit_tree_row_with_icon(
            &desc,
            &rows[1],
            state.row_icon(2),
            false,
            ControlState::default(),
            &[],
            TreeViewTokens::navigation_pane(),
        );
        assert!(tex_out
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::Image && p.border_token_or_tex == 77));
    }

    #[test]
    fn tree_row_keeps_constant_primitives_across_24_states() {
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 140.0))
            .with_density(TreeViewDensity::Compact);
        let row = TreeViewRow {
            row: TreeRow::new(1, 1, true),
            item: 0,
            rect: Rect::new(0.0, 0.0, 260.0, TREE_ROW_COMPACT),
        };
        let tokens = TreeViewTokens::navigation_pane();
        for icon in [
            TreeRowIcon::None,
            TreeRowIcon::Vector(ShellIcon::Folder),
            TreeRowIcon::ExternalTexture(42),
        ] {
            let base = emit_tree_row_with_icon(
                &desc,
                &row,
                icon,
                false,
                ControlState::default(),
                &[],
                tokens,
            )
            .primitives
            .len();
            for phase in [
                PointerPhase::Idle,
                PointerPhase::Hovered,
                PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let count = emit_tree_row_with_icon(
                                &desc, &row, icon, selected, control, &[], tokens,
                            )
                            .primitives
                            .len();
                            assert_eq!(count, base, "primitive count changed for {icon:?} {control:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn navigation_pane_hides_indent_guides_and_selection_indicator() {
        let std_desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 140.0))
            .with_density(TreeViewDensity::Compact);
        let nav_desc = std_desc.clone().with_navigation_pane(true);
        assert!(nav_desc.navigation_pane);
        assert!(!nav_desc.indent_guides);
        assert!(!nav_desc.selection_indicator);

        let row = TreeViewRow {
            row: TreeRow::new(11, 2, false),
            item: 0,
            rect: Rect::new(0.0, 0.0, 260.0, TREE_ROW_COMPACT),
        };
        let tokens = TreeViewTokens::navigation_pane();
        let std_out = emit_tree_row_with_icon(
            &std_desc,
            &row,
            TreeRowIcon::None,
            true,
            ControlState::default(),
            &[],
            tokens,
        );
        let nav_out = emit_tree_row_with_icon(
            &nav_desc,
            &row,
            TreeRowIcon::None,
            true,
            ControlState::default(),
            &[],
            tokens,
        );
        // 导航窗格模式关闭了 1 条蓝色左强调条与 2 条深度缩进参考线
        assert_eq!(nav_out.primitives.len() + 3, std_out.primitives.len());
        assert!(
            !nav_out
                .primitives
                .iter()
                .any(|p| p.prim_type() == PrimType::Line),
            "navigation pane must not emit vertical indent guide lines"
        );
        assert!(
            !nav_out
                .primitives
                .iter()
                .any(|p| p.fill_token_or_uv == Token::PRIMARY.raw() as u32),
            "navigation pane must not emit primary selection indicator pill"
        );
        assert_eq!(
            nav_out.primitives[0].fill_token_or_uv,
            tokens.row_selected.raw() as u32
        );
        assert_eq!(tokens.background, Token::NAV_BG);
        assert_eq!(tokens.row_selected, Token::NAV_SELECTED);

        // 显式开关可覆盖导航窗格默认值
        let custom_desc = nav_desc
            .with_indent_guides(true)
            .with_selection_indicator(true);
        assert!(custom_desc.indent_guides);
        assert!(custom_desc.selection_indicator);
    }

    #[test]
    fn row_labels_and_dividers_can_be_stored_and_emitted() {
        let mut state = state();
        assert_eq!(state.row_label(1), None);
        state.set_row_label(1, "Quick Access");
        state.set_row_label(2, "This PC");
        assert_eq!(state.row_label(1), Some("Quick Access"));
        assert_eq!(state.row_label(2), Some("This PC"));

        assert!(!state.has_row_divider_before(2));
        state.set_row_divider_before(2, true);
        assert!(state.has_row_divider_before(2));

        let desc = TreeViewDesc::new(Rect::new(16.0, 80.0, 240.0, 160.0))
            .with_navigation_pane(true);
        let rows = state.rows(&desc);
        let row2 = rows.iter().find(|r| r.row.id == 2).expect("row 2 visible");
        let div_out = emit_tree_row_divider(&desc, row2, TreeViewTokens::navigation_pane());
        assert_eq!(div_out.primitives.len(), 1);
        let prim = &div_out.primitives[0];
        assert_eq!(prim.fill_token_or_uv, Token::DIVIDER.raw() as u32);
        assert!((prim.local_pos[0] - (row2.rect.origin.x + 12.0)).abs() < 0.01);
        assert!((prim.local_pos[1] - row2.rect.origin.y).abs() < 0.01);
        assert!((prim.size[0] - (row2.rect.size.x - 24.0)).abs() < 0.01);
        assert!((prim.size[1] - 1.0).abs() < 0.01);

        state.set_row_divider_before(2, false);
        assert!(!state.has_row_divider_before(2));
        state.set_row_divider_before(2, true);
        state.clear_row_dividers();
        assert!(!state.has_row_divider_before(2));
        state.clear_row_labels();
        assert_eq!(state.row_label(1), None);
    }

    #[test]
    fn hover_at_tracks_single_row_and_respects_nonzero_origin() {
        let mut state = TreeViewState::new_with_density(TreeViewDensity::Compact);
        state.rebuild(&[root(1), root(2)], children);
        let desc = TreeViewDesc::new(Rect::new(24.0, 100.0, 220.0, 140.0))
            .with_density(TreeViewDensity::Compact)
            .with_navigation_pane(true);

        let rows = state.rows(&desc);
        assert_eq!(rows.len(), 2);
        assert!((rows[0].rect.origin.x - 24.0).abs() < f32::EPSILON);
        assert!((rows[0].rect.origin.y - 100.0).abs() < f32::EPSILON);
        assert!((rows[1].rect.origin.y - 128.0).abs() < f32::EPSILON);

        // 命中第一行
        assert_eq!(state.hover_at(&desc, Vec2::new(60.0, 110.0)), Some(1));
        assert_eq!(state.hovered_row(), Some(1));

        // 命中第二行
        assert_eq!(state.hover_at(&desc, Vec2::new(60.0, 136.0)), Some(2));
        assert_eq!(state.hovered_row(), Some(2));

        // 指针移出视口上方或右侧时清空悬停
        assert_eq!(state.hover_at(&desc, Vec2::new(60.0, 50.0)), None);
        assert_eq!(state.hovered_row(), None);

        // 手动 clear_hover
        assert_eq!(state.hover_at(&desc, Vec2::new(60.0, 110.0)), Some(1));
        state.clear_hover();
        assert_eq!(state.hovered_row(), None);
    }

    #[test]
    fn pressed_tree_row_uses_pressed_token_for_both_unselected_and_selected() {
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 140.0))
            .with_density(TreeViewDensity::Compact)
            .with_navigation_pane(true);
        let row = TreeViewRow {
            row: TreeRow::new(1, 0, true),
            item: 0,
            rect: Rect::new(0.0, 0.0, 260.0, TREE_ROW_COMPACT),
        };
        let tokens = TreeViewTokens::navigation_pane();
        let pressed_ctrl = ControlState {
            phase: PointerPhase::Pressed,
            ..ControlState::default()
        };

        // 未选中行按下时呈现 Token::PRESSED，不再回退透明闪烁
        let unselected_pressed = emit_tree_row_with_icon(
            &desc,
            &row,
            TreeRowIcon::None,
            false,
            pressed_ctrl,
            &[],
            tokens,
        );
        assert_eq!(
            unselected_pressed.primitives[0].fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );

        // 选中行按下时呈现柔和按压态 Token::PRESSED
        let selected_pressed = emit_tree_row_with_icon(
            &desc,
            &row,
            TreeRowIcon::None,
            true,
            ControlState {
                selected: true,
                ..pressed_ctrl
            },
            &[],
            tokens,
        );
        assert_eq!(
            selected_pressed.primitives[0].fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );

        // 禁用行按下时不进入按压反馈，保持既有静息/选中底色
        let disabled_pressed = emit_tree_row_with_icon(
            &desc,
            &row,
            TreeRowIcon::None,
            false,
            ControlState {
                disabled: true,
                ..pressed_ctrl
            },
            &[],
            tokens,
        );
        assert_eq!(
            disabled_pressed.primitives[0].fill_token_or_uv,
            u32::from(tokens.row.raw())
        );
    }

    #[test]
    fn tree_row_icon_alias_uses_token_none_and_disabled_opacity_for_external_texture() {
        let icon: TreeRowIcon = WidgetIcon::ExternalTexture(88);
        assert_eq!(icon, TreeRowIcon::ExternalTexture(88));

        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 140.0))
            .with_density(TreeViewDensity::Compact);
        let row = TreeViewRow {
            row: TreeRow::new(1, 0, false),
            item: 0,
            rect: Rect::new(0.0, 0.0, 260.0, TREE_ROW_COMPACT),
        };
        let tokens = TreeViewTokens::navigation_pane();

        let normal = emit_tree_row_with_icon(
            &desc,
            &row,
            icon,
            false,
            ControlState::default(),
            &[],
            tokens,
        );
        let img = normal
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("external texture image primitive");
        assert_eq!(img.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(img.border_token_or_tex, 88);
        assert!((img.opacity() - 1.0).abs() < 0.01);

        let disabled = emit_tree_row_with_icon(
            &desc,
            &row,
            icon,
            false,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        let disabled_img = disabled
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("disabled external texture image primitive");
        assert_eq!(disabled_img.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(disabled_img.border_token_or_tex, 88);
        assert!((disabled_img.opacity() - crate::WINUI_DISABLED_OPACITY).abs() < 0.02);
    }

    #[test]
    fn tree_row_capsule_and_icons_layer_strictly_above_tree_background() {
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 260.0, 200.0)).with_navigation_pane(true);
        let tokens = TreeViewTokens::navigation_pane();
        let bg_out = emit_tree_background(&desc, tokens);
        let bg_z = bg_out.primitives[0].depth_u16();

        let row = TreeViewRow {
            row: TreeRow::new(1, 0, true),
            item: 0,
            rect: Rect::new(0.0, 0.0, 260.0, TREE_ROW_COMPACT),
        };
        let row_out = emit_tree_row_with_options(
            &desc,
            &row,
            TreeRowIcon::Vector(ShellIcon::FolderOpen),
            TreeRowOptions::new().with_trailing_icon(Some(ShellIcon::Pin)),
            true,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );

        let capsule = row_out.primitives[0];
        assert!(
            capsule.depth_u16() > bg_z,
            "tree row capsule must sit strictly above tree background"
        );
        // 除首个胶囊与末尾焦点环外，所有图标与箭头均应处于 SubElement::Content 且 z >= capsule.depth_u16()。
        let content_slice = &row_out.primitives[1..row_out.primitives.len() - 1];
        assert!(!content_slice.is_empty());
        for prim in content_slice {
            assert!(prim.depth_u16() >= capsule.depth_u16());
            assert_eq!(
                (prim.prim_type_and_flags >> 16) & 0x7,
                u32::from(SubElement::Content as u8)
            );
        }
    }
}

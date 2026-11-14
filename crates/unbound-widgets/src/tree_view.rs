//! 虚拟化树形视图：仅实例化视口内的行。
//!
//! 扁平化与槽位复用逻辑位于 [`unbound_world::virtualize::VirtualTree`]；
//! 本模块补充控件语义——选区、焦点、内联重命名与行绘制。
//! 因此十万节点的文档也仅维持恒定数量的存活行，且折叠的子树零开销，
//! 因为 `children_of` 仅对展开的节点调用。
//!
//!

use accesskit::Role;
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::virtualize::{TreeRow, VirtualTree, WindowUpdate};

use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::switch::CONTROL_LAYER;
use crate::text_input::TextEditState;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认行高。
pub const TREE_ROW: f32 = 26.0;

/// 每层缩进深度的水平缩进量。
pub const TREE_INDENT: f32 = 16.0;

/// 视口周围保持存活的额外行数。
pub const TREE_OVERSCAN: f32 = 1.0;

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
    /// 每层深度的缩进量。
    pub indent: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl TreeViewDesc {
    /// 使用默认尺寸创建的树形视口。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            row_height: TREE_ROW,
            indent: TREE_INDENT,
            z: 450,
        }
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
    /// 已选中的行集合。
    pub selected: FxHashSet<u64>,
    /// 拥有键盘焦点的行。
    pub focused: Option<u64>,
    /// 当前正在重命名的行及其编辑器。
    pub renaming: Option<(u64, TextEditState)>,
    /// 所有扁平化行（用于键盘导航）。
    order: Vec<TreeRow>,
    /// 每个扁平化行的父节点映射。
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
            renaming: None,
            order: Vec::new(),
            parent: FxHashMap::default(),
            last_window: WindowUpdate::default(),
            viewport_height: 0.0,
        }
    }

    /// 替换数据模型。
    ///
    /// `children_of` 仅对展开的节点调用，因此开销与树中展开部分成正比，而非整份文档。
    pub fn rebuild<F>(&mut self, roots: &[TreeRow], children_of: F)
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        self.order.clear();
        self.parent.clear();
        // 扁平化一次以记录 ArrowLeft 所需的父节点链接；虚拟树使用相同展开集合执行相同遍历。
        //
        let mut stack: Vec<(TreeRow, Option<u64>)> =
            roots.iter().rev().map(|row| (*row, None)).collect();
        while let Some((mut row, parent)) = stack.pop() {
            row.expanded = self.tree.is_expanded(row.id);
            self.order.push(row);
            if let Some(parent) = parent {
                self.parent.insert(row.id, parent);
            }
            if row.expanded {
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

    /// 按节点标识查找扁平化行。
    #[must_use]
    pub fn row(&self, id: u64) -> Option<TreeRow> {
        self.order.iter().copied().find(|row| row.id == id)
    }

    /// 获取当前可见行的父节点。
    #[must_use]
    pub fn parent_of(&self, id: u64) -> Option<u64> {
        self.parent.get(&id).copied()
    }

    /// 当前滚动偏移量（含运动中的弹簧）。
    #[must_use]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset().y
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

    /// 选中指定行。若 `additive` 为 false，则替换先前选区。
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
        changed
    }

    /// 清空选区。
    pub fn clear_selection(&mut self) {
        self.selected.clear();
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
        self.focused = Some(id);
        self.selected.clear();
        self.selected.insert(id);
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

    /// 展开当前聚焦行；若已展开，则聚焦其第一个子节点。
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
        let position = self.order.iter().position(|row| row.id == id).unwrap_or(0);
        if let Some(child) = self.order.get(position + 1).copied() {
            self.select(child.id, false);
            return true;
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
        self.last_window = self.tree.update(self.scroll_offset(), desc.rect.size.y);
        let mut out = Vec::with_capacity(self.tree.slot_count());
        for slot in 0..self.tree.slot_count() {
            let (Some(row), Some(item)) = (self.tree.row_at(slot), self.tree.slot_item(slot))
            else {
                continue;
            };
            let y = desc.rect.origin.y + self.tree.item_y(item) - self.scroll_offset();
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
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            background: Token::SURFACE,
            row: Token::NONE,
            row_hover: Token::CONTROL_HOVER,
            row_selected: Token::PRIMARY_SOFT,
            text: Token::TEXT_PRIMARY,
            text_selected: Token::PRIMARY,
            guide: Token::DIVIDER,
            arrow: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 输出单行树节点：缩进参考线、展开箭头、标签与选中态。
#[must_use]
pub fn emit_tree_row(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    selected: bool,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TreeViewTokens,
) -> WidgetOutput {
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(6 + label.len() + row.row.depth as usize);
    let mut payloads = Vec::with_capacity(label.len());
    let rect = row.rect;

    let fill = if selected {
        tokens.row_selected
    } else if control.phase == PointerPhase::Hovered {
        tokens.row_hover
    } else {
        tokens.row
    };
    primitives.push(outlined(rect, 4.0, fill, Token::NONE, 0.0, 0.0, desc.z));

    // 每个深度层级一条参考线，停在箭头列之前。
    let indent = desc.indent;
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
            desc.z,
        ));
    }

    let arrow_x = rect.origin.x + row.row.depth as f32 * indent + 6.0;
    let centre = [arrow_x + 4.0, rect.origin.y + rect.size.y * 0.5];
    if row.row.has_children {
        // 朝右的箭头；展开状态时向下旋转。
        let arm = 3.5;
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
            tokens.arrow,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
        curve_stroke(
            b,
            c,
            1.5,
            tokens.arrow,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
    }

    glyphs(
        label,
        if selected {
            tokens.text_selected
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        5.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
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
    if row.row.has_children {
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
    if row.row.has_children && !disabled {
        let toggle_key = tree_toggle_node(row.row.id);
        let toggle_r = Rect::new(arrow_x - 2.0, rect.origin.y, 16.0, rect.size.y);
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

    fn root(id: u64) -> TreeRow {
        TreeRow {
            id,
            depth: 0,
            has_children: true,
            expanded: false,
        }
    }

    fn leaf(id: u64, depth: u16) -> TreeRow {
        TreeRow {
            id,
            depth,
            has_children: false,
            expanded: false,
        }
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
}

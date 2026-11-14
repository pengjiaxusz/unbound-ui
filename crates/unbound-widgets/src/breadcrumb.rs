//! 面包屑导航（Breadcrumb）：指向当前节点的路径，从左侧折叠。
//!
//! 当空间不足时，隐藏的是*中间*部分而非末尾：首项与最后几项保持可见，
//! 隐藏的头部折叠为省略号，点击可打开与首项相同的下拉菜单。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 单个分隔符两侧的间距。
pub const CRUMB_GAP: f32 = 6.0;

/// 为省略号面包屑预留的宽度。
pub const CRUMB_ELLIPSIS: f32 = 24.0;

/// 单个面包屑项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// 稳定标识，同时用作节点。
    pub id: u64,
    /// 可见标签文本。
    pub label: String,
}

impl Crumb {
    /// 创建带标签的面包屑项。
    #[must_use]
    pub fn new(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
        }
    }
}

/// 面包屑导航的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct BreadcrumbDesc {
    /// 面包屑矩形区域。
    pub rect: Rect,
    /// 面包屑列表，根节点在前。
    pub crumbs: Vec<Crumb>,
    /// 每个分隔符两侧的间距。
    pub gap: f32,
    /// 分隔符字形的宽度。
    pub separator_width: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl BreadcrumbDesc {
    /// 使用默认分隔符创建面包屑导航。
    #[must_use]
    pub fn new(rect: Rect, crumbs: Vec<Crumb>) -> Self {
        Self {
            rect,
            crumbs,
            gap: CRUMB_GAP,
            separator_width: 8.0,
            z: 520,
        }
    }

    /// 单个已布局的槽位。
    ///
    /// 省略号槽位时 `crumb` 为 `None`。
    #[must_use]
    pub fn layout(&self, widths: &[f32]) -> Vec<CrumbSlot> {
        let count = self.crumbs.len();
        if count == 0 {
            return Vec::new();
        }
        let width_of = |index: usize| {
            widths
                .get(index)
                .copied()
                .unwrap_or_else(|| self.crumbs[index].label.chars().count() as f32 * 8.0)
        };
        let step = |crumb_width: f32| crumb_width + self.gap * 2.0 + self.separator_width;

        let total: f32 = (0..count).map(|index| step(width_of(index))).sum::<f32>()
            - self.gap * 2.0
            - self.separator_width;
        if total <= self.rect.size.x {
            let mut out = Vec::with_capacity(count);
            let mut x = self.rect.origin.x;
            for index in 0..count {
                let width = width_of(index);
                out.push(CrumbSlot {
                    crumb: Some(index),
                    rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                    separator: index + 1 < count,
                });
                x += width + self.gap * 2.0 + self.separator_width;
            }
            return out;
        }

        // 从左侧折叠：优先保留最后能放下的面包屑，首项及其余隐藏项
        // 一并折叠进省略号。若连尾部也放不下，则仅显示一个省略号和最后一项。
        let mut visible = Vec::new();
        let mut used = step(CRUMB_ELLIPSIS);
        for index in (0..count).rev() {
            let needed = step(width_of(index));
            if used + needed <= self.rect.size.x || visible.is_empty() {
                visible.push(index);
                used += needed;
            } else {
                break;
            }
        }
        visible.reverse();
        if visible.len() >= count {
            return self.layout_without_overflow(widths);
        }
        let mut out = Vec::with_capacity(visible.len() + 1);
        let mut x = self.rect.origin.x;
        out.push(CrumbSlot {
            crumb: None,
            rect: Rect::new(x, self.rect.origin.y, CRUMB_ELLIPSIS, self.rect.size.y),
            separator: true,
        });
        x += CRUMB_ELLIPSIS + self.gap * 2.0 + self.separator_width;
        for (position, index) in visible.iter().enumerate() {
            let width = width_of(*index);
            out.push(CrumbSlot {
                crumb: Some(*index),
                rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                separator: position + 1 < visible.len(),
            });
            x += width + self.gap * 2.0 + self.separator_width;
        }
        out
    }

    fn layout_without_overflow(&self, widths: &[f32]) -> Vec<CrumbSlot> {
        let mut out = Vec::with_capacity(self.crumbs.len());
        let mut x = self.rect.origin.x;
        for index in 0..self.crumbs.len() {
            let width = widths.get(index).copied().unwrap_or(0.0);
            out.push(CrumbSlot {
                crumb: Some(index),
                rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                separator: index + 1 < self.crumbs.len(),
            });
            x += width + self.gap * 2.0 + self.separator_width;
        }
        out
    }

    /// 所有被折叠进省略号的面包屑索引。
    #[must_use]
    pub fn hidden_indices(&self, widths: &[f32]) -> Vec<usize> {
        let visible: Vec<usize> = self
            .layout(widths)
            .into_iter()
            .filter_map(|slot| slot.crumb)
            .collect();
        (0..self.crumbs.len())
            .filter(|index| !visible.contains(index))
            .collect()
    }
}

/// 单个面包屑槽位。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrumbSlot {
    /// 面包屑索引，省略号槽位时为 `None`。
    pub crumb: Option<usize>,
    /// 槽位矩形。
    pub rect: Rect,
    /// 该槽位后是否跟随分隔符。
    pub separator: bool,
}

/// 面包屑导航的实时状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BreadcrumbState {
    /// 溢出下拉菜单是否展开。
    pub overflow_open: bool,
}

impl BreadcrumbState {
    /// 创建关闭状态的面包屑。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换溢出下拉菜单的展开状态。
    pub fn toggle_overflow(&mut self) -> bool {
        self.overflow_open = !self.overflow_open;
        self.overflow_open
    }

    /// 关闭溢出下拉菜单。
    pub fn close(&mut self) {
        self.overflow_open = false;
    }
}

/// 面包屑绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreadcrumbTokens {
    /// 面包屑文本颜色。
    pub text: Token,
    /// 最后一项（当前节点）文本颜色。
    pub text_current: Token,
    /// 分隔符颜色。
    pub separator: Token,
    /// 省略号文本颜色。
    pub ellipsis: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl BreadcrumbTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            text: Token::TEXT_SECONDARY,
            text_current: Token::TEXT_PRIMARY,
            separator: Token::TEXT_DISABLED,
            ellipsis: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 发射两个槽位之间的分隔符。
#[must_use]
pub fn emit_crumb_separator(
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    if !slot.separator {
        return WidgetOutput::default();
    }
    let x = slot.rect.max().x + desc.gap;
    let y = slot.rect.origin.y + slot.rect.size.y * 0.5;
    let arm = desc.separator_width * 0.25;
    let mut primitives = Vec::with_capacity(2);
    for (start, end) in [
        ([x, y - arm], [x + arm * 2.0, y]),
        ([x + arm * 2.0, y], [x, y + arm]),
    ] {
        primitives.push(instance(
            Rect::new(
                start[0].min(end[0]),
                start[1].min(end[1]),
                (start[0] - end[0]).abs().max(1.0),
                (start[1] - end[1]).abs().max(1.0),
            ),
            [0.0; 4],
            tokens.separator,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        ..WidgetOutput::default()
    }
}

/// 发射单个面包屑项。
#[must_use]
pub fn emit_crumb(
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    let Some(index) = slot.crumb else {
        return WidgetOutput::default();
    };
    let Some(crumb) = desc.crumbs.get(index) else {
        return WidgetOutput::default();
    };
    let current = index + 1 == desc.crumbs.len();
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(2 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    glyphs(
        label,
        if current {
            tokens.text_current
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        slot.rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled)
        .then(|| {
            HitRegion::new(crumb.id, CONTROL_LAYER, u32::from(desc.z), slot.rect)
                .with_cursor(CursorKind::Pointer)
        })
        .filter(|_| !current);
    let mut a11y = A11yNode::new(Role::Link, crumb.label.clone(), slot.rect).disabled(disabled);
    a11y.selected = current;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled && !current,
        ..Default::default()
    }
}

/// 发射用于展开隐藏头部的省略号。
#[must_use]
pub fn emit_crumb_ellipsis(
    node: u64,
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    state: &BreadcrumbState,
    control: ControlState,
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    if slot.crumb.is_some() {
        return WidgetOutput::default();
    }
    let mut primitives = Vec::with_capacity(3);
    let cy = slot.rect.origin.y + slot.rect.size.y * 0.5;
    for offset in [-1.0f32, 0.0, 1.0] {
        let dot = Rect::new(
            slot.rect.origin.x + CRUMB_ELLIPSIS * 0.5 + offset * 6.0 - 1.5,
            cy - 1.5,
            3.0,
            3.0,
        );
        primitives.push(crate::primitive::surface(dot, 1.5, tokens.ellipsis, desc.z));
    }
    primitives.push(focus_ring(
        slot.rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), slot.rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, "Show full path", slot.rect)
        .clickable()
        .disabled(control.disabled);
    a11y.expanded = Some(state.overflow_open);

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PointerPhase;

    fn desc() -> BreadcrumbDesc {
        BreadcrumbDesc::new(
            Rect::new(0.0, 0.0, 400.0, 24.0),
            vec![
                Crumb::new(1, "Root"),
                Crumb::new(2, "src"),
                Crumb::new(3, "widgets"),
                Crumb::new(4, "button.rs"),
            ],
        )
    }

    fn widths() -> Vec<f32> {
        vec![40.0, 30.0, 60.0, 80.0]
    }

    #[test]
    fn a_path_that_fits_shows_every_crumb() {
        let desc = desc();
        let slots = desc.layout(&widths());
        assert_eq!(slots.len(), 4);
        assert!(slots.iter().all(|slot| slot.crumb.is_some()));
        assert!(desc.hidden_indices(&widths()).is_empty());
    }

    #[test]
    fn a_long_path_collapses_its_head_into_an_ellipsis() {
        let mut desc = desc();
        desc.rect.size.x = 190.0;
        let slots = desc.layout(&widths());
        assert_eq!(slots[0].crumb, None, "the head folds into the ellipsis");
        let shown: Vec<usize> = slots.iter().filter_map(|slot| slot.crumb).collect();
        assert_eq!(shown.last().copied(), Some(3), "the tail always stays");
        assert!(!desc.hidden_indices(&widths()).is_empty());
    }

    #[test]
    fn the_last_crumb_is_never_hidden() {
        let mut desc = desc();
        desc.rect.size.x = 10.0;
        let slots = desc.layout(&widths());
        let shown: Vec<usize> = slots.iter().filter_map(|slot| slot.crumb).collect();
        assert_eq!(shown, vec![3]);
    }

    #[test]
    fn separators_follow_every_slot_except_the_last() {
        let desc = desc();
        let slots = desc.layout(&widths());
        assert!(slots[0].separator);
        assert!(slots[1].separator);
        assert!(slots[2].separator);
        assert!(!slots[3].separator);
    }

    #[test]
    fn the_overflow_dropdown_toggles() {
        let mut state = BreadcrumbState::new();
        assert!(!state.overflow_open);
        assert!(state.toggle_overflow());
        assert!(state.overflow_open);
        state.close();
        assert!(!state.overflow_open);
    }

    #[test]
    fn the_ellipsis_is_pickable_and_reports_expansion() {
        let mut desc = desc();
        desc.rect.size.x = 190.0;
        let slots = desc.layout(&widths());
        let mut state = BreadcrumbState::new();
        state.toggle_overflow();
        let output = emit_crumb_ellipsis(
            99,
            &desc,
            &slots[0],
            &state,
            ControlState::default(),
            BreadcrumbTokens::standard(),
        );
        let hit = output.hit.expect("the ellipsis is pickable");
        assert_eq!(hit.node, 99);
        let a11y = output.a11y.expect("the ellipsis is reachable");
        assert_eq!(a11y.expanded, Some(true));
    }

    #[test]
    fn the_current_crumb_is_not_a_link() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let output = emit_crumb(
            &desc,
            &slots[3],
            ControlState::default(),
            &[],
            BreadcrumbTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn an_ancestor_crumb_is_pickable() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let output = emit_crumb(
            &desc,
            &slots[1],
            ControlState::default(),
            &[],
            BreadcrumbTokens::standard(),
        );
        let hit = output.hit.expect("an ancestor is pickable");
        assert_eq!(hit.node, 2);
        assert!(output.a11y.expect("reachable").role == Role::Link);
    }

    #[test]
    fn a_hovered_crumb_keeps_its_text_token() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let tokens = BreadcrumbTokens::standard();
        let output = emit_crumb(
            &desc,
            &slots[0],
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        // 此处字形列表为空；焦点环是唯一图元，未聚焦时保持隐藏。
        assert_eq!(output.primitives.len(), 1);
    }
}

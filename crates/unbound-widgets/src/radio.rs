//! 单选按钮：中心带圆点的圆形控件。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认圆圈尺寸。
pub const RADIO_SIZE: f32 = 18.0;

/// 圆圈与标签之间的间距。
pub const RADIO_GAP: f32 = 28.0;

/// 单选按钮绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioTokens {
    /// 外圆背景。
    pub circle: Token,
    /// 未选中圆圈的边框。
    pub border: Token,
    /// 已选中圆圈的边框。
    pub border_selected: Token,
    /// 中心圆点。
    pub dot: Token,
    /// 禁用状态下的圆圈。
    pub circle_disabled: Token,
    /// 禁用状态下的圆圈边框。
    pub border_disabled: Token,
    /// 禁用且选中状态下的中心圆点。
    pub dot_disabled: Token,
    /// 空闲状态的标签。
    pub label: Token,
    /// 悬停状态的标签。
    pub label_hover: Token,
    /// 禁用状态的标签。
    pub label_disabled: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl RadioTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            circle: Token::CARD_BG,
            border: Token::OUTLINE,
            border_selected: Token::PRIMARY,
            dot: Token::PRIMARY,
            circle_disabled: Token::CONTROL_DISABLED_BG,
            border_disabled: Token::CONTROL_DISABLED_BORDER,
            dot_disabled: Token::CONTROL_DISABLED_THUMB,
            label: Token::TEXT_SECONDARY,
            label_hover: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 单选按钮的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RadioDesc {
    /// 单选按钮占用的行矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 该选项是否处于选中状态。
    pub selected: bool,
    /// 圆圈尺寸。
    pub size: f32,
    /// 圆圈与标签之间的距离。
    pub gap: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl RadioDesc {
    /// 使用默认尺寸创建的单选按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            selected: false,
            size: RADIO_SIZE,
            gap: RADIO_GAP,
            z: 700,
        }
    }

    /// 更改所在矩形。
    #[must_use]
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 圆圈矩形：左对齐并在行内垂直居中。
    #[must_use]
    pub fn circle_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.y - self.size) * 0.5,
            self.size,
            self.size,
        )
    }

    /// 标签矩形。
    #[must_use]
    pub fn label_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gap,
            self.rect.origin.y,
            (self.rect.size.x - self.gap).max(0.0),
            self.rect.size.y,
        )
    }
}

/// 单选按钮组可识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioGroupKey {
    /// 移动到上一个选项。
    Up,
    /// 移动到下一个选项。
    Down,
    /// 移动到上一个选项。
    Left,
    /// 移动到下一个选项。
    Right,
}

/// 单选按钮组：选项列表及游走焦点索引。
///
/// 组是选中状态的*唯一*拥有者。单个单选按钮本身不持有焦点：
/// 恰有一个选项（当前选中项）进入 Tab 顺序，方向键同时移动选中项与游走焦点。
#[derive(Clone, Debug, PartialEq)]
pub struct RadioGroupDesc {
    /// 按视觉顺序排列的选项列表。
    pub items: Vec<RadioDesc>,
    /// 当前选中的选项（若有）。
    pub selected: Option<usize>,
}

impl RadioGroupDesc {
    /// 基于 `items` 及各自当前选中状态构建单选组。
    #[must_use]
    pub fn new(items: Vec<RadioDesc>) -> Self {
        let selected = items.iter().position(|item| item.selected);
        Self { items, selected }
    }

    /// 选项数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 组内是否没有选项。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 游走 Tab 停靠点：当前选中项；若尚未选中任何项，则为第一项。
    ///
    /// 并非所有单选按钮都参与 Tab 顺序——Tab 进入组一次，组内通过方向键移动。
    #[must_use]
    pub fn focus_order(&self) -> Vec<usize> {
        if self.items.is_empty() {
            return Vec::new();
        }
        vec![self.selected.unwrap_or(0).min(self.items.len() - 1)]
    }
}

/// 单次状态归约的输出结果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadioGroupOutcome {
    /// 按键后的选中索引。
    pub selected: Option<usize>,
    /// 选中项是否发生了变化。
    pub changed: bool,
}

/// 响应方向键推进单选组状态，到达两端时循环回绕。
///
/// 上/左键选择上一项，下/右键选择下一项；若当前未选中任何项，则首次按键选中第一项。
pub fn reduce_radio_group(group: &mut RadioGroupDesc, key: RadioGroupKey) -> RadioGroupOutcome {
    let len = group.items.len();
    if len == 0 {
        return RadioGroupOutcome::default();
    }
    let next = match group.selected {
        None => 0,
        Some(current) => {
            let current = current.min(len - 1);
            match key {
                RadioGroupKey::Up | RadioGroupKey::Left => (current + len - 1) % len,
                RadioGroupKey::Down | RadioGroupKey::Right => (current + 1) % len,
            }
        }
    };
    let changed = group.selected != Some(next);
    group.selected = Some(next);
    for (index, item) in group.items.iter_mut().enumerate() {
        item.selected = index == next;
    }
    RadioGroupOutcome {
        selected: Some(next),
        changed,
    }
}

/// 输出单选按钮：外圆、内点、标签与焦点环。
#[must_use]
pub fn emit_radio(
    node: u64,
    desc: &RadioDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: RadioTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let circle = desc.circle_rect();

    primitives.push(outlined(
        circle,
        desc.size * 0.5,
        if state.disabled {
            tokens.circle_disabled
        } else {
            tokens.circle
        },
        if state.disabled {
            tokens.border_disabled
        } else if desc.selected {
            tokens.border_selected
        } else {
            tokens.border
        },
        2.0,
        0.0,
        z,
    ));

    // 中心圆点：居中的较小圆形。
    if desc.selected {
        primitives.push(surface(
            circle.inflate(-5.0),
            4.0,
            if state.disabled {
                tokens.dot_disabled
            } else {
                tokens.dot
            },
            z,
        ));
    }

    let label_color = if state.disabled {
        tokens.label_disabled
    } else if state.phase == PointerPhase::Hovered {
        tokens.label_hover
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // 焦点环始终输出：焦点变化仅改变其不透明度而非图元池结构，因此聚焦只是绘制变更而非重传。
    //
    primitives.push(focus_ring(
        circle.inflate(3.0),
        desc.size * 0.5 + 3.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.size * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::RadioButton, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = desc.selected;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> RadioDesc {
        RadioDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Solid")
    }

    #[test]
    fn an_unselected_circle_keeps_the_plain_border() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(1, &desc(), ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border.raw() as u32
        );
        // 圆圈及其焦点环——焦点环始终输出，仅在控件聚焦时不透明。
        //
        assert_eq!(output.primitives.len(), 2);
        assert!(
            output
                .primitives
                .iter()
                .all(|primitive| primitive.fill_token_or_uv != tokens.dot.raw() as u32),
            "没有选中就没有圆点"
        );
    }

    #[test]
    fn a_selected_circle_paints_a_dot() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
            1,
            &desc().with_selected(true),
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border_selected.raw() as u32
        );
        // 圆圈、圆点与焦点环。
        assert_eq!(output.primitives.len(), 3);
        assert_eq!(
            output.primitives[1].fill_token_or_uv,
            tokens.dot.raw() as u32
        );
    }

    #[test]
    fn the_circle_is_left_aligned_and_centred() {
        let desc = desc();
        let circle = desc.circle_rect();
        assert_eq!(circle.origin.x, 0.0);
        assert_eq!(circle.origin.y, 7.0);
    }

    #[test]
    fn the_semantic_node_reports_the_selection() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
            1,
            &desc().with_selected(true),
            ControlState::default(),
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a radio button is always reachable");
        assert_eq!(a11y.role, Role::RadioButton);
        assert!(a11y.selected);
    }

    #[test]
    fn a_disabled_radio_button_leaves_the_hit_set() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

//! ListBox：经典单选列表。
//!
//! 每行始终输出「行底 + 选中指示条」两个图元，行高由描述符决定。
//! 选中与悬停只改变填充与指示条令牌，因此滑动选择不会重组场景结构。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 标准行高。
pub const LIST_BOX_ROW_HEIGHT: f32 = 32.0;
/// 选中指示条宽度。
pub const LIST_BOX_INDICATOR: f32 = 3.0;
/// 控件圆角。
pub const LIST_BOX_RADIUS: f32 = 4.0;

/// 单选列表框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ListBoxDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 行文本。
    pub items: Vec<String>,
    /// 行高。
    pub row_height: f32,
    /// 当前选中行。
    pub selected: Option<usize>,
    /// 当前悬停行。
    pub hovered: Option<usize>,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl ListBoxDesc {
    /// 创建行高为标准值的列表框。
    #[must_use]
    pub fn new(rect: Rect, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            rect,
            items: items.into_iter().map(Into::into).collect(),
            row_height: LIST_BOX_ROW_HEIGHT,
            selected: None,
            hovered: None,
            disabled: false,
            z: 714,
        }
    }

    /// 设置选中行。
    #[must_use]
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// 设置悬停行。
    #[must_use]
    pub fn hovered(mut self, index: usize) -> Self {
        self.hovered = Some(index);
        self
    }

    /// 覆盖行高。
    #[must_use]
    pub fn with_row_height(mut self, height: f32) -> Self {
        self.row_height = height.max(12.0);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 第 `index` 行的矩形；行高会自动收缩以适配控件高度。
    #[must_use]
    pub fn row_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.items.len() {
            return None;
        }
        let rows = self.items.len().max(1) as f32;
        let height = self.row_height.min(self.rect.height() / rows).max(0.0);
        Some(Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + index as f32 * height,
            self.rect.width(),
            height,
        ))
    }
}

/// 输出单选列表框的图元、命中区域与语义节点。
#[must_use]
pub fn emit_list_box(
    node: u64,
    desc: &ListBoxDesc,
    control: ControlState,
    item_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.focused {
        Token::FOCUS_RING
    } else if hovered {
        Token::INPUT_BORDER_HOVER
    } else {
        Token::CONTROL_STROKE
    };

    let mut primitives = Vec::with_capacity(3 + desc.items.len() * 2 + item_glyphs.len());
    let mut payloads = Vec::with_capacity(item_glyphs.len());

    primitives.push(outlined(
        rect,
        LIST_BOX_RADIUS,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        border,
        1.0,
        0.0,
        z,
    ));

    for index in 0..desc.items.len() {
        let Some(row) = desc.row_rect(index) else {
            continue;
        };
        let selected = desc.selected == Some(index);
        let row_hovered = desc.hovered == Some(index);
        let fill = if disabled {
            Token::CONTROL_DISABLED_BG
        } else if selected {
            Token::PRIMARY_SOFT
        } else if row_hovered || hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        primitives.push(surface(row, 0.0, fill, z + 1));
        let indicator = Rect::new(row.origin.x, row.origin.y, LIST_BOX_INDICATOR, row.height());
        primitives.push(surface(
            indicator,
            0.0,
            if selected && !disabled {
                Token::PRIMARY
            } else {
                Token::NONE
            },
            z + 2,
        ));
    }

    emit_glyphs(
        item_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        LIST_BOX_RADIUS,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [LIST_BOX_RADIUS; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::ListBox, "List box", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for (index, label) in desc.items.iter().enumerate() {
            if let Some(row) = desc.row_rect(index) {
                let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x700 + index as u64);
                let mut option = A11yNode::new(Role::ListItem, label.clone(), row).clickable();
                option.selected = desc.selected == Some(index);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(z) + 1, row)
                        .with_cursor(CursorKind::Pointer),
                    option,
                    true,
                );
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ListBoxDesc {
        ListBoxDesc::new(
            Rect::new(0.0, 0.0, 200.0, 128.0),
            ["Alpha", "Beta", "Gamma", "Delta"],
        )
        .selected(1)
    }

    #[test]
    fn rows_stack_without_gaps() {
        let d = desc();
        let first = d.row_rect(0).expect("row 0");
        let second = d.row_rect(1).expect("row 1");
        assert!((first.max().y - second.origin.y).abs() < 1e-4);
        assert_eq!(first.height(), 32.0);
        assert!(d.row_rect(4).is_none());
    }

    #[test]
    fn the_selected_row_draws_a_primary_indicator() {
        let d = desc();
        let output = emit_list_box(1, &d, ControlState::default(), &mut []);
        // 外框 1 个图元，随后每行 2 个；选中行是第 2 行，其指示条位于下标 4。
        let indicator = output.primitives[4].fill_token_or_uv as u16;
        assert_eq!(indicator, Token::PRIMARY.raw());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().hovered(2);
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let state = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(emit_list_box(1, &d, state, &mut []).primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

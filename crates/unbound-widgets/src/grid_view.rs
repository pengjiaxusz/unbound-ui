//! GridView：等宽网格集合视图。
//!
//! 每个单元始终输出「填充 + 选中描边」两个图元：选中与悬停只改令牌，
//! 因此换行、改列或切换状态都不会让后续图元重新编号。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 网格单元间距。
pub const GRID_VIEW_GAP: f32 = 8.0;
/// 单元格圆角。
pub const GRID_VIEW_CELL_RADIUS: f32 = 6.0;

/// 网格集合视图描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct GridViewDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 列数（至少 1）。
    pub columns: usize,
    /// 行数（至少 1）。
    pub rows: usize,
    /// 单元间距。
    pub gap: f32,
    /// 当前选中单元索引。
    pub selected: Option<usize>,
    /// 当前悬停单元索引。
    pub hovered: Option<usize>,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl GridViewDesc {
    /// 创建 `columns × rows` 的网格视图。
    #[must_use]
    pub fn new(rect: Rect, columns: usize, rows: usize) -> Self {
        Self {
            rect,
            columns: columns.max(1),
            rows: rows.max(1),
            gap: GRID_VIEW_GAP,
            selected: None,
            hovered: None,
            disabled: false,
            z: 716,
        }
    }

    /// 设置选中单元。
    #[must_use]
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// 设置悬停单元。
    #[must_use]
    pub fn hovered(mut self, index: usize) -> Self {
        self.hovered = Some(index);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 单元总数。
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.columns * self.rows
    }

    /// 第 `index` 个单元的矩形。
    #[must_use]
    pub fn cell_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.cell_count() {
            return None;
        }
        let col = index % self.columns;
        let row = index / self.columns;
        let cell_w = (self.rect.width() - self.gap * (self.columns.saturating_sub(1)) as f32)
            / self.columns as f32;
        let cell_h = (self.rect.height() - self.gap * (self.rows.saturating_sub(1)) as f32)
            / self.rows as f32;
        Some(Rect::new(
            self.rect.origin.x + col as f32 * (cell_w + self.gap),
            self.rect.origin.y + row as f32 * (cell_h + self.gap),
            cell_w.max(0.0),
            cell_h.max(0.0),
        ))
    }
}

/// 输出网格集合视图的图元、命中区域与语义节点。
#[must_use]
pub fn emit_grid_view(
    node: u64,
    desc: &GridViewDesc,
    control: ControlState,
    cell_glyphs: &mut [GlyphQuad],
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

    let mut primitives = Vec::with_capacity(3 + desc.cell_count() * 2 + cell_glyphs.len());
    let mut payloads = Vec::with_capacity(cell_glyphs.len());

    primitives.push(outlined(
        rect,
        8.0,
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

    for index in 0..desc.cell_count() {
        let cell = match desc.cell_rect(index) {
            Some(cell) => cell,
            None => continue,
        };
        let selected = desc.selected == Some(index);
        let cell_hovered = desc.hovered == Some(index);
        let fill = if disabled {
            Token::CONTROL_DISABLED_BG
        } else if selected {
            Token::PRIMARY_SOFT
        } else if cell_hovered || hovered {
            Token::CONTROL_HOVER
        } else {
            Token::SURFACE_SUNKEN
        };
        let cell_border = if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if selected {
            Token::PRIMARY
        } else if cell_hovered || hovered {
            Token::OUTLINE
        } else {
            Token::NONE
        };
        primitives.push(surface(cell, GRID_VIEW_CELL_RADIUS, fill, z + 1));
        primitives.push(outlined(
            cell,
            GRID_VIEW_CELL_RADIUS,
            Token::NONE,
            cell_border,
            if selected { 2.0 } else { 1.0 },
            0.0,
            z + 2,
        ));
    }

    emit_glyphs(
        cell_glyphs,
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
        8.0,
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
                .with_shape(Shape::Rounded { radii: [8.0; 4] })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Grid, "Grid view", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for index in 0..desc.cell_count() {
            if let Some(cell) = desc.cell_rect(index) {
                let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x600 + index as u64);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(z) + 1, cell)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::ListItem, format!("Item {}", index + 1), cell).clickable(),
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

    fn desc() -> GridViewDesc {
        GridViewDesc::new(Rect::new(0.0, 0.0, 240.0, 160.0), 3, 2)
    }

    #[test]
    fn cells_fill_the_grid_without_overlap() {
        let d = desc();
        assert_eq!(d.cell_count(), 6);
        let first = d.cell_rect(0).expect("cell 0");
        let second = d.cell_rect(1).expect("cell 1");
        assert!(first.max().x <= second.origin.x);
        assert!(d.rect.contains_rect(&first));
        assert!(d.rect.contains_rect(&d.cell_rect(5).expect("cell 5")));
        assert!(d.cell_rect(6).is_none());
    }

    #[test]
    fn a_selected_cell_uses_the_primary_soft_fill() {
        let d = desc().selected(2);
        let output = emit_grid_view(1, &d, ControlState::default(), &mut []);
        // 外框占 1 个图元，随后每单元 2 个（填充在前）。
        let cell_fill = output.primitives[1 + 2 * 2].fill_token_or_uv as u16;
        assert_eq!(cell_fill, Token::PRIMARY_SOFT.raw());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().selected(1).hovered(2);
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
                        counts.push(emit_grid_view(1, &d, state, &mut []).primitives.len());
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

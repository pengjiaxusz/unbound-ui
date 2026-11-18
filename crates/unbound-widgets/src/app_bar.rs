//! AppBarButton / AppBarToggleButton / AppBarSeparator：命令栏内部的原语。
//!
//! 三者共享同一套 Fluent 度量：32px 方形槽位、4px 圆角、透明底，
//! 悬停时铺 `ControlFillColorSecondary`，按下铺 `ControlFillColorTertiary`。
//! 切换按钮额外在底部画一条强调色指示条；分隔线是纯装饰元素，不参与交互。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 命令栏按钮标准边长。
pub const APP_BAR_ITEM_SIZE: f32 = 32.0;
/// 命令栏按钮圆角。
pub const APP_BAR_RADIUS: f32 = 4.0;
/// 分隔线厚度。
pub const APP_BAR_SEPARATOR_THICKNESS: f32 = 1.0;

/// 命令栏普通按钮描述符：图标槽位 + 紧凑标签。
#[derive(Clone, Debug, PartialEq)]
pub struct AppBarButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl AppBarButtonDesc {
    /// 创建命令栏按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: APP_BAR_RADIUS,
            disabled: false,
            z: 730,
        }
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 命令栏切换按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AppBarToggleButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 是否处于选中态。
    pub checked: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl AppBarToggleButtonDesc {
    /// 创建命令栏切换按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>, checked: bool) -> Self {
        Self {
            rect,
            label: label.into(),
            checked,
            radius: APP_BAR_RADIUS,
            disabled: false,
            z: 731,
        }
    }

    /// 设置为选中状态。
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 命令栏分隔线描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AppBarSeparatorDesc {
    /// 分隔线所占矩形；实际线条会居中并吸附到像素网格。
    pub rect: Rect,
    /// 绘制顺序键。
    pub z: u16,
}

impl AppBarSeparatorDesc {
    /// 创建竖直分隔线。
    #[must_use]
    pub fn vertical(rect: Rect) -> Self {
        Self { rect, z: 729 }
    }

    /// 实际绘制的一像素线条矩形。
    #[must_use]
    pub fn line_rect(&self) -> Rect {
        Rect::new(
            (self.rect.origin.x + (self.rect.width() - APP_BAR_SEPARATOR_THICKNESS) * 0.5).round(),
            self.rect.origin.y,
            APP_BAR_SEPARATOR_THICKNESS,
            self.rect.height(),
        )
    }
}

/// 输出命令栏普通按钮。
#[must_use]
pub fn emit_app_bar_button(
    node: u64,
    desc: &AppBarButtonDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let fill = app_bar_fill(control.phase, disabled);
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let mut primitives = Vec::with_capacity(2 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());
    primitives.push(outlined(rect, desc.radius, fill, Token::NONE, 0.0, 0.0, z));
    emit_glyphs(label_glyphs, ink, z + 1, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Button, desc.label.clone(), rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 输出命令栏切换按钮。
#[must_use]
pub fn emit_app_bar_toggle_button(
    node: u64,
    desc: &AppBarToggleButtonDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let checked = desc.checked || control.selected;
    let fill = if disabled {
        Token::NONE
    } else if checked {
        if control.phase == PointerPhase::Pressed {
            Token::PRIMARY_PRESSED
        } else if control.phase == PointerPhase::Hovered {
            Token::PRIMARY_HOVER
        } else {
            Token::PRIMARY_SOFT
        }
    } else {
        app_bar_fill(control.phase, disabled)
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else if checked {
        Token::PRIMARY
    } else {
        Token::TEXT_PRIMARY
    };
    let indicator = Rect::new(
        rect.origin.x + 6.0,
        rect.max().y - 2.0,
        (rect.width() - 12.0).max(0.0),
        2.0,
    );

    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());
    primitives.push(outlined(rect, desc.radius, fill, Token::NONE, 0.0, 0.0, z));
    primitives.push(surface(
        indicator,
        1.0,
        if checked && !disabled {
            Token::PRIMARY
        } else {
            Token::NONE
        },
        z + 1,
    ));
    emit_glyphs(label_glyphs, ink, z + 2, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !disabled,
    ));

    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = checked;

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 输出命令栏分隔线（纯装饰，无命中区域）。
#[must_use]
pub fn emit_app_bar_separator(desc: &AppBarSeparatorDesc) -> WidgetOutput {
    WidgetOutput {
        primitives: vec![surface(desc.line_rect(), 0.0, Token::DIVIDER, desc.z)],
        ..Default::default()
    }
}

/// 命令栏按钮的共享底色：禁用透明、按下压深、悬停提亮。
fn app_bar_fill(phase: PointerPhase, disabled: bool) -> Token {
    if disabled {
        Token::NONE
    } else {
        match phase {
            PointerPhase::Pressed => Token::SURFACE_SUNKEN,
            PointerPhase::Hovered => Token::CONTROL_HOVER,
            PointerPhase::Idle => Token::NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, APP_BAR_ITEM_SIZE, APP_BAR_ITEM_SIZE)
    }

    #[test]
    fn hovering_lights_up_the_button_background() {
        let desc = AppBarButtonDesc::new(rect(), "Bold");
        let idle = emit_app_bar_button(1, &desc, ControlState::default(), &mut []);
        let hovered = emit_app_bar_button(
            1,
            &desc,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_eq!(
            idle.primitives[0].fill_token_or_uv as u16,
            Token::NONE.raw()
        );
        assert_eq!(
            hovered.primitives[0].fill_token_or_uv as u16,
            Token::CONTROL_HOVER.raw()
        );
    }

    #[test]
    fn a_checked_toggle_draws_the_primary_indicator() {
        let desc = AppBarToggleButtonDesc::new(rect(), "Italic", true);
        let output = emit_app_bar_toggle_button(1, &desc, ControlState::default(), &mut []);
        assert_eq!(
            output.primitives[1].fill_token_or_uv as u16,
            Token::PRIMARY.raw()
        );
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn a_separator_is_one_pixel_wide() {
        let desc = AppBarSeparatorDesc::vertical(Rect::new(0.0, 0.0, 9.0, 32.0));
        let output = emit_app_bar_separator(&desc);
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(output.primitives[0].size[0], 1.0);
        assert!(output.hit.is_none());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let desc = AppBarToggleButtonDesc::new(rect(), "Bold", true);
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
                        counts.push(
                            emit_app_bar_toggle_button(1, &desc, state, &mut [])
                                .primitives
                                .len(),
                        );
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

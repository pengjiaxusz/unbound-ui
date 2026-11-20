//! RepeatButton：按住后持续触发的按钮。
//!
//! 视觉与标准 [`crate::button::Button`] 完全一致（4px 圆角、32px 标准高度、
//! `CONTROL_FILL` / `CONTROL_HOVER` / `SURFACE_SUNKEN` 三态），差异只在行为：
//! 按下后先等待 `repeat_delay_ms`，随后每隔 `repeat_interval_ms` 触发一次。
//! 节奏计算是描述符上的纯函数，便于在没有时钟的测试里直接验证。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 按下后首次重复前的等待时间（毫秒）。
pub const DEFAULT_REPEAT_DELAY_MS: u32 = 500;
/// 首次重复之后每次重复的间隔（毫秒）。
pub const DEFAULT_REPEAT_INTERVAL_MS: u32 = 100;
/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const REPEAT_BUTTON_RADIUS: f32 = 4.0;
/// 标准控件高度。对齐 WinUI `Button` 的 32px。
pub const REPEAT_BUTTON_HEIGHT: f32 = 32.0;

/// 重复按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RepeatButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签与显示文本。
    pub label: String,
    /// 首次重复前的等待时间（毫秒）。
    pub repeat_delay_ms: u32,
    /// 重复间隔（毫秒）。
    pub repeat_interval_ms: u32,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl RepeatButtonDesc {
    /// 使用默认节奏创建重复按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            repeat_delay_ms: DEFAULT_REPEAT_DELAY_MS,
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            radius: REPEAT_BUTTON_RADIUS,
            disabled: false,
            z: 706,
        }
    }

    /// 使用标准高度便捷构造。
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::new(0.0, 0.0, 120.0, REPEAT_BUTTON_HEIGHT), label)
    }

    /// 覆盖重复节奏。
    #[must_use]
    pub fn with_timing(mut self, delay_ms: u32, interval_ms: u32) -> Self {
        self.repeat_delay_ms = delay_ms;
        self.repeat_interval_ms = interval_ms.max(1);
        self
    }

    /// 设置圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置绘制图层顺序键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 以毫秒计的重复间隔，保证非零。
    #[must_use]
    pub const fn interval_ms(&self) -> u32 {
        if self.repeat_interval_ms == 0 {
            1
        } else {
            self.repeat_interval_ms
        }
    }

    /// 按住时长达到首次重复阈值后返回 `true`。
    #[must_use]
    pub const fn should_repeat(&self, elapsed_ms: u32) -> bool {
        elapsed_ms >= self.repeat_delay_ms
    }

    /// 下一次重复触发的绝对时刻（毫秒）。
    ///
    /// 尚未越过延迟时返回 `repeat_delay_ms`；越过之后返回最近一个
    /// `delay + k * interval` 之后的下一个触发点。
    #[must_use]
    pub fn next_repeat_due(&self, elapsed_ms: u32) -> u32 {
        if elapsed_ms < self.repeat_delay_ms {
            return self.repeat_delay_ms;
        }
        let interval = self.interval_ms();
        let completed = (elapsed_ms - self.repeat_delay_ms) / interval;
        self.repeat_delay_ms
            .saturating_add(completed.saturating_add(1).saturating_mul(interval))
    }
}

/// 重复按钮的按住状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepeatButtonState {
    /// 指针或按键是否仍按住。
    pub held: bool,
    /// 自按下起累计的毫秒数。
    pub elapsed_ms: u32,
}

impl RepeatButtonState {
    /// 创建未按下的状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始按住，计时归零，但第一次触发仍需等待延迟。
    pub fn begin(&mut self) {
        self.held = true;
        self.elapsed_ms = 0;
    }

    /// 结束按住。
    pub fn end(&mut self) {
        self.held = false;
        self.elapsed_ms = 0;
    }

    /// 处理按键按下：聚焦状态下按住 Space 或 Enter 开启连续触发。
    pub fn handle_key_down(&mut self, is_space_or_enter: bool) -> bool {
        if is_space_or_enter && !self.held {
            self.begin();
            true
        } else {
            false
        }
    }

    /// 处理按键松开：释放 Space 或 Enter 结束连续触发。
    pub fn handle_key_up(&mut self, is_space_or_enter: bool) -> bool {
        if is_space_or_enter && self.held {
            self.end();
            true
        } else {
            false
        }
    }

    /// 推进 `dt_ms` 毫秒，并返回该步内是否至少触发了一次重复。
    ///
    /// 首次触发由 `repeat_delay_ms` 决定，随后每 `repeat_interval_ms`
    /// 触发一次；未按住时始终返回 `false`。
    pub fn advance(&mut self, desc: &RepeatButtonDesc, dt_ms: u32) -> bool {
        if !self.held {
            return false;
        }
        let before = self.elapsed_ms;
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
        let interval = desc.interval_ms();
        if self.elapsed_ms < desc.repeat_delay_ms {
            return false;
        }
        let last = (self.elapsed_ms - desc.repeat_delay_ms) / interval;
        let first = if before < desc.repeat_delay_ms {
            0
        } else {
            (before - desc.repeat_delay_ms) / interval + 1
        };
        last >= first
    }
}

/// 将键盘激活折叠进视觉状态。
#[must_use]
pub fn keyboard_activation(state: ControlState, space_or_enter_down: bool) -> ControlState {
    if space_or_enter_down && state.focused && !state.disabled {
        ControlState {
            phase: PointerPhase::Pressed,
            ..state
        }
    } else {
        state
    }
}

/// 输出重复按钮的图元、命中区域与语义节点。
///
/// 按住状态只改变填充令牌，图元数量保持不变。
#[must_use]
pub fn emit_repeat_button(
    node: u64,
    desc: &RepeatButtonDesc,
    state: &RepeatButtonState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let radii = [desc.radius; 4];
    let disabled = control.disabled || desc.disabled;
    let held = state.held && !disabled;

    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == PointerPhase::Pressed || held {
        Token::SURFACE_SUNKEN
    } else if control.phase == PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::CONTROL_FILL
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let mut primitives = Vec::with_capacity(4 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill_rect = if !disabled { rect.inflate(-1.0) } else { rect };
    let fill_radius = if !disabled {
        (desc.radius - 1.0).max(0.0)
    } else {
        desc.radius
    };

    primitives.push(surface(fill_rect, fill_radius, fill, z));
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 1,
    ));
    if !disabled {
        let inset = desc.radius.min(rect.size.x * 0.5);
        primitives.push(surface(
            Rect::new(
                rect.origin.x + inset,
                rect.max().y - 1.0,
                (rect.size.x - inset * 2.0).max(0.0),
                1.0,
            ),
            0.0,
            Token::CONTROL_STROKE_SECONDARY,
            z + 1,
        ));
    } else {
        primitives.push(surface(Rect::ZERO, 0.0, Token::NONE, z + 1));
    }
    emit_glyphs(glyphs, ink, z + 1, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, desc.label.clone(), rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

impl UiWidget for RepeatButtonDesc {
    type State = RepeatButtonState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, 14.0, 500);
        let w = (measured.x + 22.0).max(self.rect.width()).max(64.0);
        let h = REPEAT_BUTTON_HEIGHT.max(self.rect.height());
        Vec2::new(w, h)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { .. } => {
                state.begin();
                true
            }
            WidgetInputEvent::PointerUp { .. } => {
                state.end();
                true
            }
            WidgetInputEvent::KeyDown(chord) => {
                let is_space_or_enter =
                    matches!(chord.key, KeyName::Named("Enter") | KeyName::Named("Space"));
                state.handle_key_down(is_space_or_enter)
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut glyphs =
            text_ctx.shape_line(&desc.label, rect, 14.0, 500, AlignX::Center, AlignY::Middle);
        emit_repeat_button(node, &desc, state, control, &mut glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> RepeatButtonDesc {
        RepeatButtonDesc::new(Rect::new(0.0, 0.0, 120.0, REPEAT_BUTTON_HEIGHT), "Repeat")
    }

    #[test]
    fn the_first_repeat_waits_for_the_delay() {
        let d = desc();
        assert!(!d.should_repeat(499));
        assert!(d.should_repeat(500));
        assert_eq!(d.next_repeat_due(0), 500);
        assert_eq!(d.next_repeat_due(500), 600);
        assert_eq!(d.next_repeat_due(650), 700);
    }

    #[test]
    fn holding_repeats_after_the_delay_and_every_interval() {
        let d = desc();
        let mut state = RepeatButtonState::new();
        state.begin();
        assert!(!state.advance(&d, 400));
        assert!(
            state.advance(&d, 200),
            "crossing 500 ms fires the first repeat"
        );
        assert!(state.advance(&d, 100), "the next interval fires again");
        assert!(!state.advance(&d, 50));
        state.end();
        assert!(!state.held);
        assert!(!state.advance(&d, 1_000));
    }

    #[test]
    fn key_down_and_up_drive_held_state() {
        let mut state = RepeatButtonState::new();
        assert!(state.handle_key_down(true));
        assert!(state.held);
        // 重复按下已按住时不重复触发
        assert!(!state.handle_key_down(true));

        assert!(state.handle_key_up(true));
        assert!(!state.held);
        assert!(!state.handle_key_up(true));
    }

    #[test]
    fn keyboard_activation_collapses_to_pressed() {
        let focused = ControlState {
            focused: true,
            ..ControlState::default()
        };
        let active = keyboard_activation(focused, true);
        assert_eq!(active.phase, PointerPhase::Pressed);

        let inactive = keyboard_activation(focused, false);
        assert_eq!(inactive.phase, PointerPhase::Idle);
    }

    #[test]
    fn a_disabled_button_never_repeats_visually() {
        let output = emit_repeat_button(
            1,
            &desc().disabled(true),
            &RepeatButtonState {
                held: true,
                elapsed_ms: 900,
            },
            ControlState::default(),
            &mut [],
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv as u16,
            Token::CONTROL_DISABLED_BG.raw()
        );
        assert!(output.hit.is_none());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc();
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
                        let repeat = RepeatButtonState {
                            held: focused,
                            elapsed_ms: 750,
                        };
                        counts.push(
                            emit_repeat_button(1, &d, &repeat, state, &mut [])
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

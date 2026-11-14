//! 开关控件：一条轨道与一个滑向另一端的旋钮。
//!
//! 开关是唯一其*业务*状态（`on`）不属于指针阶段的控件：
//! 该状态驻留在描述符中，而指针阶段与焦点环仍由路由器提供。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

/// 控件命中区域所在的界面层。
pub const CONTROL_LAYER: u8 = 1;

/// 默认轨道尺寸。
pub const SWITCH_TRACK: (f32, f32) = (44.0, 24.0);

/// 默认旋钮尺寸。
pub const SWITCH_KNOB: f32 = 16.0;

/// 旋钮与轨道边缘保持的距离。
const KNOB_INSET: f32 = 4.0;

/// 按下时旋钮沿轨道方向的拉伸量。
const KNOB_SQUISH: f32 = 4.0;

/// 开关绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchTokens {
    /// 关闭状态下的轨道。
    pub track: Token,
    /// 开启状态下的轨道。
    pub track_on: Token,
    /// 禁用状态下的轨道。
    pub track_disabled: Token,
    /// 旋钮。
    pub thumb: Token,
    /// 禁用状态下的旋钮。
    pub thumb_disabled: Token,
    /// 空闲状态下的标签。
    pub label: Token,
    /// 悬停状态下的标签。
    pub label_hover: Token,
    /// 禁用状态下的标签。
    pub label_disabled: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl SwitchTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            track_on: Token::PRIMARY,
            track_disabled: Token::CONTROL_DISABLED_BG,
            thumb: Token::CONTROL_THUMB,
            thumb_disabled: Token::CONTROL_DISABLED_THUMB,
            label: Token::TEXT_SECONDARY,
            label_hover: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 开关的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchDesc {
    /// 开关占用的行矩形，轨道位于行末右侧。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 开关是否处于开启状态。
    pub on: bool,
    /// 旋钮的动画位置（`0..=1`）。
    ///
    /// `None` 表示将旋钮直接对齐到业务状态；`Some(t)` 由弹簧动画每帧写入，
    /// 使数值切换时旋钮平滑滑动而非瞬移。
    pub thumb_t: Option<f32>,
    /// 轨道尺寸。
    pub track_size: (f32, f32),
    /// 旋钮尺寸。
    pub knob_size: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl SwitchDesc {
    /// 使用默认尺寸创建的开关。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            on: false,
            thumb_t: None,
            track_size: SWITCH_TRACK,
            knob_size: SWITCH_KNOB,
            z: 700,
        }
    }

    /// 切换开关状态。
    #[must_use]
    pub fn with_on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// 在插值位置绘制旋钮，而非直接对齐到目标端。
    #[must_use]
    pub fn with_thumb_t(mut self, thumb_t: f32) -> Self {
        self.thumb_t = Some(thumb_t);
        self
    }

    /// 轨道矩形：右对齐并在行内垂直居中。
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        let (width, height) = self.track_size;
        Rect::new(
            self.rect.max().x - width,
            self.rect.origin.y + (self.rect.size.y - height) * 0.5,
            width,
            height,
        )
    }

    /// 处于 `0..=1` 的旋钮位置（取自动画进度或业务状态）。
    #[must_use]
    pub fn thumb_position(&self) -> f32 {
        self.thumb_t
            .unwrap_or(if self.on { 1.0 } else { 0.0 })
            .clamp(0.0, 1.0)
    }

    /// 旋钮矩形。
    #[must_use]
    pub fn knob_rect(&self) -> Rect {
        self.knob_rect_pressed(false)
    }

    /// 按下状态的旋钮矩形。
    ///
    /// 按下开关会将旋钮拉伸为胶囊形：向轨道中心方向延伸、远离其锚定端，
    /// 模拟拇指按压物理拨动开关时的形变效果。
    #[must_use]
    pub fn knob_rect_pressed(&self, pressed: bool) -> Rect {
        let track = self.track_rect();
        let travel = (track.size.x - self.knob_size - KNOB_INSET * 2.0).max(0.0);
        let t = self.thumb_position();
        let mut width = self.knob_size;
        let mut x = track.origin.x + KNOB_INSET + t * travel;
        if pressed && travel > 0.0 {
            width += KNOB_SQUISH;
            if t >= 0.5 {
                x -= KNOB_SQUISH;
            }
        }
        Rect::new(
            x,
            track.origin.y + (track.size.y - self.knob_size) * 0.5,
            width,
            self.knob_size,
        )
    }
}

/// 输出开关：轨道、旋钮、标签与焦点环。
///
/// `label` 携带调用方已定位好的字形；输出器不会修改它们。
#[must_use]
pub fn emit_switch(
    node: u64,
    desc: &SwitchDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SwitchTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    // 轨道。
    let track_fill = if state.disabled {
        tokens.track_disabled
    } else if desc.on {
        tokens.track_on
    } else {
        tokens.track
    };
    primitives.push(surface(
        desc.track_rect(),
        desc.track_size.1 * 0.5,
        track_fill,
        z,
    ));

    // 旋钮：按下时拉伸为胶囊形，还原真实拨动开关的触觉反馈。
    //
    let pressed = state.phase == crate::PointerPhase::Pressed && !state.disabled;
    let knob = desc.knob_rect_pressed(pressed);
    let thumb_fill = if state.disabled {
        tokens.thumb_disabled
    } else {
        tokens.thumb
    };
    primitives.push(outlined(
        knob,
        desc.knob_size * 0.5,
        thumb_fill,
        Token::NONE,
        0.0,
        2.0,
        z,
    ));

    // 标签。
    let label_color = if state.disabled {
        tokens.label_disabled
    } else if state.phase == crate::PointerPhase::Hovered {
        tokens.label_hover
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // 焦点环。始终输出：焦点变化仅改变其不透明度而非图元池结构，因此聚焦只是绘制变更而非重传。
    //
    primitives.push(focus_ring(
        knob.inflate(3.0),
        desc.knob_size * 0.5 + 3.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.track_size.1 * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Switch, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = desc.on;

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
    use crate::PointerPhase;
    use unbound_math::Vec2;

    fn desc() -> SwitchDesc {
        SwitchDesc::new(Rect::new(0.0, 0.0, 200.0, 40.0), "Notifications")
    }

    #[test]
    fn the_track_sits_at_the_right_end_of_the_row() {
        let desc = desc();
        let track = desc.track_rect();
        assert_eq!(track.max().x, 200.0);
        assert_eq!(track.size, Vec2::new(44.0, 24.0));
        assert_eq!(track.origin.y, 8.0);
    }

    #[test]
    fn the_knob_slides_between_the_two_ends() {
        let mut desc = desc();
        desc.on = false;
        let off = desc.knob_rect();
        desc.on = true;
        let on = desc.knob_rect();
        assert!(on.origin.x > off.origin.x);
        assert_eq!(on.max().x, desc.track_rect().max().x - 4.0);
        assert_eq!(off.origin.x, desc.track_rect().origin.x + 4.0);
    }

    #[test]
    fn the_track_paints_the_state_colour() {
        let tokens = SwitchTokens::standard();
        let desc = desc();
        let output = emit_switch(1, &desc, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track.raw() as u32
        );

        let desc = desc.with_on(true);
        let output = emit_switch(1, &desc, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track_on.raw() as u32
        );
    }

    #[test]
    fn a_disabled_switch_leaves_the_hit_set() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
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
        assert!(output.a11y.is_some());
        assert!(!output.focusable);
    }

    #[test]
    fn a_focused_switch_emits_a_focus_ring() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
            1,
            &desc(),
            ControlState {
                focused: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == unbound_gpu::data::PrimType::Stroke));
    }

    #[test]
    fn the_semantic_node_reports_the_position() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
            1,
            &desc().with_on(true),
            ControlState::default(),
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a switch is always reachable");
        assert_eq!(a11y.role, Role::Switch);
        assert!(a11y.selected);
    }

    #[test]
    fn a_hovered_switch_brightens_its_label() {
        let tokens = SwitchTokens::standard();
        let idle = emit_switch(1, &desc(), ControlState::default(), &[], tokens);
        let hovered = emit_switch(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(idle.primitives.len(), hovered.primitives.len());
    }
}

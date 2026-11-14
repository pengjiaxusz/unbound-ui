//! 侧边面板与抽屉（Drawer）。
//!
//! 抽屉是从屏幕边缘滑入的模态框。它与模态框共享遮罩层、层级与焦点陷阱；
//! 唯一不同的是几何位置，且该几何位置仅是单个数值（`0.0..=1.0` 展开度）的纯函数。
//! 将动画收敛为单个标量意味着滑动过程绝不重建控件树：仅移动同一张卡片。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::modal::MODAL_LAYER;
use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

/// 抽屉滑入的边缘方向。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerSide {
    /// 从左边缘滑入。
    Left,
    /// 从右边缘滑入。
    #[default]
    Right,
    /// 从上边缘滑入。
    Top,
    /// 从下边缘滑入。
    Bottom,
}

/// 默认背景模糊半径。
pub const DRAWER_BLUR: f32 = 12.0;
/// 默认内侧圆角半径。
pub const DRAWER_RADIUS: f32 = 12.0;
const DRAWER_SHADOW: f32 = 32.0;

/// 抽屉描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DrawerDesc {
    /// 抽屉在其中滑动的视口。
    pub viewport: Rect,
    /// 滑入的边缘方向。
    pub side: DrawerSide,
    /// 抽屉伸入视口的深度。
    pub size: f32,
    /// 内侧边缘圆角半径。
    pub radius: f32,
    /// 背景模糊半径；`0.0` 表示禁用。
    pub backdrop_blur: f32,
    /// 按下 `Escape` 与点击遮罩层是否可关闭抽屉。
    pub dismissible: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl DrawerDesc {
    /// 创建带模糊背景的可关闭抽屉。
    #[must_use]
    pub fn new(viewport: Rect, side: DrawerSide, size: f32) -> Self {
        Self {
            viewport,
            side,
            size,
            radius: DRAWER_RADIUS,
            backdrop_blur: DRAWER_BLUR,
            dismissible: true,
            z: 930,
        }
    }

    /// 设置显式背景模糊半径。
    #[must_use]
    pub fn with_backdrop(mut self, blur: f32) -> Self {
        self.backdrop_blur = blur;
        self
    }

    /// 设为仅通过显式操作关闭的抽屉。
    #[must_use]
    pub fn permanent(mut self) -> Self {
        self.dismissible = false;
        self
    }

    /// 抽屉内侧圆角朝向的边。
    #[must_use]
    pub const fn inner_axis(&self) -> bool {
        matches!(self.side, DrawerSide::Left | DrawerSide::Right)
    }
}

/// 单个抽屉的展开/收起动画状态。
#[derive(Clone, Debug, PartialEq)]
pub struct DrawerState {
    openness: f32,
    velocity: f32,
    target: f32,
}

impl DrawerState {
    /// 创建初始为打开或关闭状态的抽屉。
    #[must_use]
    pub fn new(open: bool) -> Self {
        let openness = if open { 1.0 } else { 0.0 };
        Self {
            openness,
            velocity: 0.0,
            target: openness,
        }
    }

    /// 当前可见程度，`0.0..=1.0`。
    #[must_use]
    pub fn openness(&self) -> f32 {
        self.openness
    }

    /// 是否已完全打开。
    #[must_use]
    pub fn is_open(&self) -> bool {
        (self.openness - 1.0).abs() < 0.001
    }

    /// 是否已完全关闭。
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.openness < 0.001
    }

    /// 请求打开抽屉。
    pub fn open(&mut self) {
        self.target = 1.0;
    }

    /// 请求关闭抽屉。
    pub fn close(&mut self) {
        self.target = 0.0;
    }

    /// 步进弹簧；返回展开度是否发生变化。
    pub fn step(&mut self, dt: f32) -> bool {
        if dt <= 0.0 {
            return false;
        }
        let dt = dt.min(0.05);
        let before = self.openness;
        let force = (self.target - self.openness) * 170.0;
        self.velocity = (self.velocity + force * dt) * (-22.0 * dt).exp();
        self.openness += self.velocity * dt;
        if (self.target - self.openness).abs() < 0.001 && self.velocity.abs() < 0.01 {
            self.openness = self.target;
            self.velocity = 0.0;
        }
        self.openness = self.openness.clamp(0.0, 1.0);
        (self.openness - before).abs() > f32::EPSILON
    }
}

/// 给定展开度下的卡片矩形。
#[must_use]
pub fn drawer_rect(desc: &DrawerDesc, openness: f32) -> Rect {
    let openness = openness.clamp(0.0, 1.0);
    let viewport = desc.viewport;
    let size = desc.size.max(0.0);
    match desc.side {
        DrawerSide::Left => {
            let width = size * openness;
            Rect::new(viewport.origin.x, viewport.origin.y, width, viewport.size.y)
        }
        DrawerSide::Right => {
            let width = size * openness;
            Rect::new(
                viewport.max().x - width,
                viewport.origin.y,
                width,
                viewport.size.y,
            )
        }
        DrawerSide::Top => {
            let height = size * openness;
            Rect::new(
                viewport.origin.x,
                viewport.origin.y,
                viewport.size.x,
                height,
            )
        }
        DrawerSide::Bottom => {
            let height = size * openness;
            Rect::new(
                viewport.origin.x,
                viewport.max().y - height,
                viewport.size.x,
                height,
            )
        }
    }
}

/// 单个抽屉的发射结果。
#[derive(Clone, Debug)]
pub struct DrawerOutput {
    /// 卡片及其命中区域。
    pub output: WidgetOutput,
    /// 遮罩层（在卡片之外单独安装）。
    pub scrim: HitRegion,
    /// 调用方需推入的浮层条目。
    pub entry: OverlayEntry,
    /// 模糊半径非零时的模糊请求。
    pub backdrop: Option<BackdropRequest>,
    /// 本帧的卡片矩形。
    pub rect: Rect,
}

/// 按当前展开度发射抽屉。
#[must_use]
pub fn emit_drawer(
    node: u64,
    desc: &DrawerDesc,
    state: &DrawerState,
    focus_scope: &[u64],
    _drawer_glyphs: &[GlyphQuad],
    control: ControlState,
) -> DrawerOutput {
    let rect = drawer_rect(desc, state.openness());
    let radius = desc.radius.max(0.0);
    let primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(desc.viewport, 0.0, Token::SCRIM, desc.z),
        instance(
            rect,
            [radius; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            DRAWER_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, radius, Token::SURFACE_RAISED, desc.z),
        instance(
            rect,
            [radius; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];

    let a11y = A11yNode::new(Role::Dialog, String::new(), rect).disabled(control.disabled);

    DrawerOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit: Some(HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), rect)),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        scrim: HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, desc.viewport),
        entry: OverlayEntry::modal(node, rect, focus_scope.to_vec()),
        backdrop: BackdropRequest::new(node, desc.viewport, desc.backdrop_blur),
        rect,
    }
}

/// `Escape` 是否允许关闭该抽屉。
#[must_use]
pub const fn escape_closes(desc: &DrawerDesc) -> bool {
    desc.dismissible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    #[test]
    fn an_open_sheet_hugs_its_edge() {
        let left = DrawerDesc::new(viewport(), DrawerSide::Left, 320.0);
        let rect = drawer_rect(&left, 1.0);
        assert_eq!(rect.origin.x, 0.0);
        assert_eq!(rect.size.x, 320.0);
        assert_eq!(rect.size.y, 720.0);

        let right = DrawerDesc::new(viewport(), DrawerSide::Right, 320.0);
        let rect = drawer_rect(&right, 1.0);
        assert_eq!(rect.max().x, 1280.0);
        assert_eq!(rect.size.x, 320.0);

        let bottom = DrawerDesc::new(viewport(), DrawerSide::Bottom, 240.0);
        let rect = drawer_rect(&bottom, 1.0);
        assert_eq!(rect.max().y, 720.0);
        assert_eq!(rect.size.y, 240.0);

        let top = DrawerDesc::new(viewport(), DrawerSide::Top, 240.0);
        let rect = drawer_rect(&top, 1.0);
        assert_eq!(rect.origin.y, 0.0);
        assert_eq!(rect.size.y, 240.0);
    }

    #[test]
    fn a_closed_sheet_has_no_area() {
        let desc = DrawerDesc::new(viewport(), DrawerSide::Right, 320.0);
        let rect = drawer_rect(&desc, 0.0);
        assert_eq!(rect.size.x, 0.0);
        assert_eq!(rect.max().x, 1280.0);
    }

    #[test]
    fn the_slide_is_monotonic_and_settles() {
        let mut state = DrawerState::new(false);
        state.open();
        let mut previous = state.openness();
        for _ in 0..240 {
            state.step(1.0 / 60.0);
            assert!(state.openness() + 1e-5 >= previous);
            previous = state.openness();
        }
        assert!(state.is_open());
        assert_eq!(state.openness(), 1.0);
    }

    #[test]
    fn closing_returns_the_sheet_to_its_edge() {
        let mut state = DrawerState::new(true);
        state.close();
        for _ in 0..240 {
            state.step(1.0 / 60.0);
        }
        assert!(state.is_closed());
    }

    #[test]
    fn the_emitter_reports_scrim_card_entry_and_backdrop() {
        let desc = DrawerDesc::new(viewport(), DrawerSide::Right, 320.0);
        let state = DrawerState::new(true);
        let output = emit_drawer(5, &desc, &state, &[10, 11], &[], ControlState::default());
        assert_eq!(output.scrim.layer, PORTAL_LAYER);
        assert_eq!(output.output.hit.as_ref().unwrap().layer, MODAL_LAYER);
        assert_eq!(output.entry.focus_scope, vec![10, 11]);
        assert_eq!(
            output.backdrop.map(|request| request.blur),
            Some(DRAWER_BLUR)
        );
        assert_eq!(
            output.output.a11y.as_ref().map(|node| node.role),
            Some(Role::Dialog)
        );
        assert_eq!(output.rect.size.x, 320.0);
    }

    #[test]
    fn a_permanent_sheet_ignores_escape_and_the_backdrop_can_be_disabled() {
        let desc = DrawerDesc::new(viewport(), DrawerSide::Bottom, 200.0)
            .permanent()
            .with_backdrop(0.0);
        assert!(!escape_closes(&desc));
        assert!(emit_drawer(
            5,
            &desc,
            &DrawerState::new(true),
            &[],
            &[],
            ControlState::default()
        )
        .backdrop
        .is_none());
    }
}

//! Side sheets and drawers.
//!
//! A drawer is a modal that enters from an edge. It shares the modal's scrim,
//! layer and focus trap; the only thing that differs is geometry, and that
//! geometry is a pure function of one number — the openness in `0.0..=1.0`.
//! Keeping the animation in a scalar means a slide never rebuilds the widget
//! tree: it only moves the same card.

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

/// The edge a drawer enters from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerSide {
    /// From the left edge.
    Left,
    /// From the right edge.
    #[default]
    Right,
    /// From the top edge.
    Top,
    /// From the bottom edge.
    Bottom,
}

/// The default backdrop blur.
pub const DRAWER_BLUR: f32 = 12.0;
/// The default inner corner radius.
pub const DRAWER_RADIUS: f32 = 12.0;
const DRAWER_SHADOW: f32 = 32.0;

/// A drawer descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawerDesc {
    /// The viewport the sheet slides inside.
    pub viewport: Rect,
    /// The edge it enters from.
    pub side: DrawerSide,
    /// How far the sheet reaches into the viewport.
    pub size: f32,
    /// The corner radius on the inner edge.
    pub radius: f32,
    /// The backdrop blur radius; `0.0` disables the pass.
    pub backdrop_blur: f32,
    /// Whether `Escape` and the scrim close it.
    pub dismissible: bool,
    /// The draw order key.
    pub z: u16,
}

impl DrawerDesc {
    /// A dismissible sheet with a blur.
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

    /// The same sheet with an explicit blur.
    #[must_use]
    pub fn with_backdrop(mut self, blur: f32) -> Self {
        self.backdrop_blur = blur;
        self
    }

    /// A sheet that only closes through an explicit action.
    #[must_use]
    pub fn permanent(mut self) -> Self {
        self.dismissible = false;
        self
    }

    /// The side the sheet's inner corner faces.
    #[must_use]
    pub const fn inner_axis(&self) -> bool {
        matches!(self.side, DrawerSide::Left | DrawerSide::Right)
    }
}

/// The open/closed animation of one drawer.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawerState {
    openness: f32,
    velocity: f32,
    target: f32,
}

impl DrawerState {
    /// A drawer that starts open or closed.
    #[must_use]
    pub fn new(open: bool) -> Self {
        let openness = if open { 1.0 } else { 0.0 };
        Self {
            openness,
            velocity: 0.0,
            target: openness,
        }
    }

    /// How visible the sheet is, `0.0..=1.0`.
    #[must_use]
    pub fn openness(&self) -> f32 {
        self.openness
    }

    /// Whether the sheet is fully open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        (self.openness - 1.0).abs() < 0.001
    }

    /// Whether the sheet is fully closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.openness < 0.001
    }

    /// Ask the sheet to open.
    pub fn open(&mut self) {
        self.target = 1.0;
    }

    /// Ask the sheet to close.
    pub fn close(&mut self) {
        self.target = 0.0;
    }

    /// Advance the spring; returns whether the openness changed.
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

/// The card rectangle at a given openness.
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

/// What one drawer emits.
#[derive(Clone, Debug)]
pub struct DrawerOutput {
    /// The card and its hit region.
    pub output: WidgetOutput,
    /// The scrim, installed in addition to the card.
    pub scrim: HitRegion,
    /// The overlay entry the caller pushes.
    pub entry: OverlayEntry,
    /// The blur request, when the blur is non-zero.
    pub backdrop: Option<BackdropRequest>,
    /// The card rectangle this frame.
    pub rect: Rect,
}

/// Emit a drawer at its current openness.
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
        },
        scrim: HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, desc.viewport),
        entry: OverlayEntry::modal(node, rect, focus_scope.to_vec()),
        backdrop: BackdropRequest::new(node, desc.viewport, desc.backdrop_blur),
        rect,
    }
}

/// Whether `Escape` may close the drawer.
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

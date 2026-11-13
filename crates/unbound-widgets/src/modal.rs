//! Modal dialogs: scrim, card and focus scope.
//!
//! A modal is not just a floating card. It owns three things at once:
//!
//! * a full-viewport **scrim** hit region that swallows every pointer event
//!   aimed at the UI beneath it,
//! * a **card** whose children receive normal input,
//! * a **focus scope**, which the caller installs into
//!   [`unbound_input::FocusManager::set_trap`] so `Tab` cannot reach the
//!   background.
//!
//! Putting all three in one emitter keeps them impossible to drift apart: a
//! modal that renders without a scrim or without a focus scope is not
//! representable here.

use accesskit::Role;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{FocusCause, FocusManager, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of a modal: above ordinary portals.
pub const MODAL_LAYER: u8 = PORTAL_LAYER + 1;
const MODAL_SHADOW: f32 = 40.0;
/// The standard title-bar close button size.
pub const MODAL_CLOSE_SIZE: f32 = 24.0;
/// The default footer action width.
pub const MODAL_ACTION_WIDTH: f32 = 88.0;
/// The gap between two footer actions.
pub const MODAL_ACTION_GAP: f32 = 8.0;
/// The footer's horizontal padding.
pub const MODAL_FOOTER_PADDING: f32 = 16.0;

/// A modal dialog descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct ModalDesc {
    /// The dialog card rectangle.
    pub rect: Rect,
    /// The accessible title.
    pub label: String,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
    /// Backdrop blur radius behind the scrim; `0.0` disables the pass.
    pub backdrop_blur: f32,
    /// Paint the standard title-bar close button.
    pub close_button: bool,
    /// The title-bar height the close button centres in.
    pub title_bar_height: f32,
    /// The footer action-bar height; `0.0` hides the footer.
    pub footer_height: f32,
}

impl ModalDesc {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            radius: 14.0,
            z: 950,
            backdrop_blur: 0.0,
            close_button: false,
            title_bar_height: 44.0,
            footer_height: 0.0,
        }
    }

    /// The same dialog with a title.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The same dialog with the standard title-bar close button.
    #[must_use]
    pub fn with_close_button(mut self) -> Self {
        self.close_button = true;
        self
    }

    /// The same dialog with a footer action bar of the given height.
    #[must_use]
    pub fn with_footer(mut self, height: f32) -> Self {
        self.footer_height = height;
        self
    }

    /// The title-bar rectangle.
    #[must_use]
    pub fn title_bar_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.width(),
            self.title_bar_height,
        )
    }

    /// The close button rectangle, when the button is enabled.
    #[must_use]
    pub fn close_button_rect(&self) -> Option<Rect> {
        self.close_button.then(|| {
            Rect::new(
                self.rect.max().x - 12.0 - MODAL_CLOSE_SIZE,
                self.rect.origin.y + (self.title_bar_height - MODAL_CLOSE_SIZE) * 0.5,
                MODAL_CLOSE_SIZE,
                MODAL_CLOSE_SIZE,
            )
        })
    }

    /// The footer rectangle, when a footer height is configured.
    #[must_use]
    pub fn footer_rect(&self) -> Option<Rect> {
        (self.footer_height > 0.0).then(|| {
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - self.footer_height,
                self.rect.width(),
                self.footer_height,
            )
        })
    }

    /// The rectangles of `count` equal-width footer actions.
    ///
    /// The actions are right-aligned in the footer and read left to right in
    /// the order the caller emits them, so the primary action is the last
    /// rectangle.
    #[must_use]
    pub fn footer_actions(&self, count: usize) -> Vec<Rect> {
        let Some(footer) = self.footer_rect() else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        let total = count as f32 * MODAL_ACTION_WIDTH + (count - 1) as f32 * MODAL_ACTION_GAP;
        let first_x = footer.max().x - MODAL_FOOTER_PADDING - total;
        let y = footer.origin.y + (footer.height() - 32.0) * 0.5;
        (0..count)
            .map(|index| {
                Rect::new(
                    first_x + index as f32 * (MODAL_ACTION_WIDTH + MODAL_ACTION_GAP),
                    y,
                    MODAL_ACTION_WIDTH,
                    32.0,
                )
            })
            .collect()
    }
}

/// The focus stack of one or more open modals.
///
/// Opening pushes the node that had focus; closing pops it and restores it.
/// The stack nests, so an "are you sure?" dialog opened from another dialog
/// returns the keyboard to the first one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModalFocus {
    saved: Vec<Option<u64>>,
}

impl ModalFocus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a focus trap and remember the current focus.
    pub fn open(&mut self, focus: &mut FocusManager, scope: &[u64]) {
        self.saved.push(focus.focused());
        focus.set_trap(scope.iter().copied());
    }

    /// Drop the trap and restore the focus saved by [`Self::open`].
    ///
    /// Returns the restored node, if there was one.
    pub fn close(&mut self, focus: &mut FocusManager) -> Option<u64> {
        focus.clear_trap();
        match self.saved.pop() {
            Some(Some(key)) => {
                focus.focus(key, FocusCause::Programmatic);
                Some(key)
            }
            Some(None) => {
                focus.blur();
                None
            }
            None => None,
        }
    }

    /// The saved focus slots, outermost first.
    #[must_use]
    pub fn saved(&self) -> &[Option<u64>] {
        &self.saved
    }

    /// How many modals are open.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.saved.len()
    }
}

/// A modal descriptor bound to an identity.
#[derive(Clone, Debug)]
pub struct ModalWidget {
    /// The card node.
    pub node: u64,
    /// The viewport the scrim covers.
    pub viewport: Rect,
    /// The descriptor.
    pub desc: ModalDesc,
    /// The focusable nodes inside the dialog.
    pub focus_scope: Vec<u64>,
}

impl WidgetDesc for ModalWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_modal(
            self.node,
            self.viewport,
            &self.desc,
            &self.focus_scope,
            state,
        )
        .output
    }
}

/// What one modal emits.
#[derive(Clone, Debug)]
pub struct ModalOutput {
    /// The card and its hit region.
    pub output: WidgetOutput,
    /// The scrim, which must be installed **in addition** to the card.
    pub scrim: HitRegion,
    /// The stack entry the caller pushes.
    pub entry: OverlayEntry,
    /// The blur request, when `backdrop_blur > 0`.
    pub backdrop: Option<BackdropRequest>,
}

/// Emit a modal dialog.
#[must_use]
pub fn emit_modal(
    node: u64,
    viewport: Rect,
    desc: &ModalDesc,
    focus_scope: &[u64],
    state: ControlState,
) -> ModalOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    // The scrim is painted, then the shadow, then the card and its border.
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(viewport, 0.0, Token::SCRIM, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            MODAL_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::SURFACE_RAISED, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads: Vec<GpuPayload> = Vec::new();

    if let Some(button) = desc.close_button_rect() {
        let inset = 6.0;
        let a = (button.origin.x + inset, button.origin.y + inset);
        let b = (button.max().x - inset, button.max().y - inset);
        let c = (button.max().x - inset, button.origin.y + inset);
        let d = (button.origin.x + inset, button.max().y - inset);
        for (from, to) in [(a, b), (c, d)] {
            let (primitive, payload) = close_stroke(from, to, Token::TEXT_SECONDARY, desc.z);
            primitives.push(primitive);
            payloads.push(payload);
        }
    }

    let card_hit = HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), rect);
    let scrim = HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, viewport);
    let a11y = unbound_a11y::A11yNode::new(Role::Dialog, desc.label.clone(), rect)
        .disabled(state.disabled);
    let entry = OverlayEntry::modal(node, rect, focus_scope.to_vec());
    let backdrop = BackdropRequest::new(node, viewport, desc.backdrop_blur);

    ModalOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: Some(card_hit),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        scrim,
        entry,
        backdrop,
    }
}

/// One quadratic stroke of the close cross.
fn close_stroke(
    from: (f32, f32),
    to: (f32, f32),
    color: Token,
    z: u16,
) -> (GpuPrimitiveInstance, GpuPayload) {
    let origin = Rect::new(
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs().max(1.0),
        (from.1 - to.1).abs().max(1.0),
    );
    let primitive = instance(
        origin,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    let payload = GpuPayload::curve(
        [from.0 - origin.origin.x, from.1 - origin.origin.y],
        [
            (from.0 + to.0) * 0.5 - origin.origin.x,
            (from.1 + to.1) * 0.5 - origin.origin.y,
        ],
        [to.0 - origin.origin.x, to.1 - origin.origin.y],
        1.5,
    );
    (primitive, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    #[test]
    fn a_modal_paints_a_scrim_card_and_border() {
        let desc = ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)).with_label("Settings");
        let output = emit_modal(5, viewport(), &desc, &[10, 11], ControlState::default());
        assert_eq!(output.output.primitives.len(), 4);
        assert_eq!(output.scrim.bounds, viewport());
        assert_eq!(output.scrim.layer, PORTAL_LAYER);
        assert_eq!(output.output.hit.as_ref().unwrap().layer, MODAL_LAYER);
        assert_eq!(output.entry.focus_scope, vec![10, 11]);
        assert_eq!(
            output.output.a11y.as_ref().map(|node| node.role),
            Some(Role::Dialog)
        );
    }

    #[test]
    fn a_blur_request_is_only_emitted_when_asked_for() {
        let desc = ModalDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(
            emit_modal(1, viewport(), &desc, &[], ControlState::default())
                .backdrop
                .is_none()
        );
        let blurred = ModalDesc {
            backdrop_blur: 12.0,
            ..desc
        };
        let output = emit_modal(1, viewport(), &blurred, &[], ControlState::default());
        assert_eq!(output.backdrop.map(|request| request.blur), Some(12.0));
    }
}

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
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of a modal: above ordinary portals.
pub const MODAL_LAYER: u8 = PORTAL_LAYER + 1;
const MODAL_SHADOW: f32 = 40.0;

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
        }
    }

    /// The same dialog with a title.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
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
    let primitives: Vec<GpuPrimitiveInstance> = vec![
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

    let card_hit = HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), rect);
    let scrim = HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, viewport);
    let a11y = unbound_a11y::A11yNode::new(Role::Dialog, desc.label.clone(), rect)
        .disabled(state.disabled);
    let entry = OverlayEntry::modal(node, rect, focus_scope.to_vec());
    let backdrop = BackdropRequest::new(node, viewport, desc.backdrop_blur);

    ModalOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit: Some(card_hit),
            a11y: Some(a11y),
            focusable: false,
        },
        scrim,
        entry,
        backdrop,
    }
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

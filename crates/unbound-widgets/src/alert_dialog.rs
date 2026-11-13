//! Modal confirmation dialogs.
//!
//! An alert dialog is a [`crate::modal::ModalDesc`] with a fixed three-part
//! structure: an icon and a title, a body message, and a footer with exactly
//! two actions. That fixed shape is what lets the widget make the *safe*
//! choice for the caller: a destructive dialog puts the initial keyboard focus
//! on **Cancel**, so an inattentive `Enter` cannot delete anything.
//!
//! The dialog emits its scrim separately from the card, exactly like the
//! generic modal, because both must be installed as hit regions.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::modal::{MODAL_ACTION_GAP, MODAL_ACTION_WIDTH, MODAL_LAYER};
use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

/// The dialog body padding.
pub const DIALOG_PADDING: f32 = 20.0;
/// The footer height.
pub const DIALOG_FOOTER: f32 = 56.0;
/// The action button height.
pub const DIALOG_BUTTON_HEIGHT: f32 = 32.0;
/// The severity icon diameter.
pub const DIALOG_ICON: f32 = 22.0;
/// The default corner radius.
pub const DIALOG_RADIUS: f32 = 14.0;
const DIALOG_SHADOW: f32 = 40.0;

/// The emphasis of the confirm action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmTone {
    /// A neutral, reversible action.
    #[default]
    Primary,
    /// A destructive action: red, and the cancel button takes initial focus.
    Destructive,
}

impl ConfirmTone {
    /// The confirm button fill.
    #[must_use]
    pub fn fill(self) -> Token {
        match self {
            Self::Primary => Token::PRIMARY,
            Self::Destructive => Token::ERROR,
        }
    }

    /// The accent used by the warning icon.
    #[must_use]
    pub fn accent(self) -> Token {
        match self {
            Self::Primary => Token::PRIMARY,
            Self::Destructive => Token::ERROR,
        }
    }
}

/// Which footer action holds the keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlertFocus {
    /// The safe action.
    Cancel,
    /// The committing action.
    #[default]
    Confirm,
}

/// The keys the dialog handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertDialogKey {
    /// Cancel and close.
    Escape,
    /// Commit and close.
    Enter,
    /// Move the focus to the other action.
    Tab,
    /// Move the focus backwards.
    ShiftTab,
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertDialogOutcome {
    /// The key was not handled.
    Ignored,
    /// The dialog asks to close without committing.
    Cancelled,
    /// The dialog asks to commit and close.
    Confirmed,
    /// The focus moved to another action.
    FocusChanged(AlertFocus),
}

/// A confirmation dialog descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertDialogDesc {
    /// The dialog card rectangle.
    pub rect: Rect,
    /// The headline.
    pub title: String,
    /// The body copy.
    pub message: String,
    /// The cancel label.
    pub cancel_label: String,
    /// The confirm label.
    pub confirm_label: String,
    /// The confirm emphasis.
    pub tone: ConfirmTone,
    /// The corner radius.
    pub radius: f32,
    /// The backdrop blur radius; `0.0` disables the pass.
    pub backdrop_blur: f32,
    /// The draw order key.
    pub z: u16,
}

impl AlertDialogDesc {
    /// A dialog with a title.
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            message: String::new(),
            cancel_label: "取消".to_owned(),
            confirm_label: "确认".to_owned(),
            tone: ConfirmTone::Primary,
            radius: DIALOG_RADIUS,
            backdrop_blur: 8.0,
            z: 960,
        }
    }

    /// The same dialog with body copy.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// The same dialog with an explicit action pair.
    #[must_use]
    pub fn with_actions(mut self, cancel: impl Into<String>, confirm: impl Into<String>) -> Self {
        self.cancel_label = cancel.into();
        self.confirm_label = confirm.into();
        self
    }

    /// The same dialog with a destructive confirm action.
    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.tone = ConfirmTone::Destructive;
        self
    }

    /// The focus the dialog opens with.
    ///
    /// A destructive action never takes the initial focus: the safe choice is
    /// the default, which is the whole point of the dialog.
    #[must_use]
    pub fn initial_focus(&self) -> AlertFocus {
        match self.tone {
            ConfirmTone::Destructive => AlertFocus::Cancel,
            ConfirmTone::Primary => AlertFocus::Confirm,
        }
    }

    /// The index of the initial focus in `[cancel, confirm]`.
    #[must_use]
    pub fn initial_focus_index(&self) -> usize {
        match self.initial_focus() {
            AlertFocus::Cancel => 0,
            AlertFocus::Confirm => 1,
        }
    }

    /// The geometry of every part of the dialog.
    #[must_use]
    pub fn layout(&self) -> AlertDialogLayout {
        let pad = DIALOG_PADDING;
        let icon = Rect::new(
            self.rect.origin.x + pad,
            self.rect.origin.y + pad,
            DIALOG_ICON,
            DIALOG_ICON,
        );
        let title_x = icon.max().x + 12.0;
        let title = Rect::new(
            title_x,
            self.rect.origin.y + pad,
            (self.rect.max().x - pad - title_x).max(0.0),
            DIALOG_ICON,
        );
        let message = Rect::new(
            self.rect.origin.x + pad,
            icon.max().y + 14.0,
            (self.rect.size.x - pad * 2.0).max(0.0),
            (self.rect.max().y - DIALOG_FOOTER - icon.max().y - 14.0 - pad).max(0.0),
        );
        let footer = Rect::new(
            self.rect.origin.x,
            self.rect.max().y - DIALOG_FOOTER,
            self.rect.size.x,
            DIALOG_FOOTER,
        );
        let confirm = Rect::new(
            footer.max().x - pad - MODAL_ACTION_WIDTH,
            footer.origin.y + (DIALOG_FOOTER - DIALOG_BUTTON_HEIGHT) * 0.5,
            MODAL_ACTION_WIDTH,
            DIALOG_BUTTON_HEIGHT,
        );
        let cancel = Rect::new(
            confirm.origin.x - MODAL_ACTION_GAP - MODAL_ACTION_WIDTH,
            confirm.origin.y,
            MODAL_ACTION_WIDTH,
            DIALOG_BUTTON_HEIGHT,
        );
        AlertDialogLayout {
            icon,
            title,
            message,
            footer,
            cancel,
            confirm,
        }
    }
}

/// The computed rectangles of an alert dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertDialogLayout {
    /// The severity icon.
    pub icon: Rect,
    /// The headline.
    pub title: Rect,
    /// The body copy.
    pub message: Rect,
    /// The footer bar.
    pub footer: Rect,
    /// The safe action.
    pub cancel: Rect,
    /// The committing action.
    pub confirm: Rect,
}

/// The keyboard state of one dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertDialogState {
    focus: AlertFocus,
}

impl AlertDialogState {
    /// A dialog opened with the descriptor's safe default.
    #[must_use]
    pub fn new(desc: &AlertDialogDesc) -> Self {
        Self {
            focus: desc.initial_focus(),
        }
    }

    /// The focused action.
    #[must_use]
    pub fn focus(&self) -> AlertFocus {
        self.focus
    }

    /// Move the focus explicitly.
    pub fn set_focus(&mut self, focus: AlertFocus) {
        self.focus = focus;
    }

    /// Handle one key.
    pub fn handle_key(&mut self, key: AlertDialogKey) -> AlertDialogOutcome {
        match key {
            AlertDialogKey::Escape => AlertDialogOutcome::Cancelled,
            AlertDialogKey::Enter => AlertDialogOutcome::Confirmed,
            AlertDialogKey::Tab | AlertDialogKey::ShiftTab => {
                self.focus = match self.focus {
                    AlertFocus::Cancel => AlertFocus::Confirm,
                    AlertFocus::Confirm => AlertFocus::Cancel,
                };
                AlertDialogOutcome::FocusChanged(self.focus)
            }
        }
    }
}

/// What one dialog emits.
#[derive(Clone, Debug)]
pub struct AlertDialogOutput {
    /// The card, title, message and buttons.
    pub output: WidgetOutput,
    /// The scrim, installed in addition to the card.
    pub scrim: HitRegion,
    /// The cancel and confirm hit regions, in that order.
    pub actions: [HitRegion; 2],
    /// The overlay entry the caller pushes.
    pub entry: OverlayEntry,
    /// The blur request, when `backdrop_blur > 0`.
    pub backdrop: Option<BackdropRequest>,
    /// The computed layout.
    pub layout: AlertDialogLayout,
}

/// Deterministic sub-node id for the alert dialog Cancel button.
#[must_use]
pub const fn alert_cancel_node(node: u64) -> u64 {
    node ^ (1 << 62)
}

/// Deterministic sub-node id for the alert dialog Confirm button.
#[must_use]
pub const fn alert_confirm_node(node: u64) -> u64 {
    node ^ (1 << 61)
}

/// Emit a confirmation dialog.
#[must_use]
pub fn emit_alert_dialog(
    node: u64,
    viewport: Rect,
    desc: &AlertDialogDesc,
    glyphs: &[GlyphQuad],
    control: ControlState,
) -> AlertDialogOutput {
    let layout = desc.layout();
    let accent = desc.tone.accent();
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(viewport, 0.0, Token::SCRIM, desc.z),
        instance(
            desc.rect,
            [desc.radius; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            DIALOG_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(desc.rect, desc.radius, Token::SURFACE_RAISED, desc.z),
        instance(
            desc.rect,
            [desc.radius; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
        surface(layout.icon, layout.icon.size.x * 0.5, accent, desc.z + 1),
        surface(
            Rect::new(
                layout.icon.origin.x + layout.icon.size.x * 0.5 - 1.0,
                layout.icon.origin.y + 5.0,
                2.0,
                8.0,
            ),
            1.0,
            Token::ON_PRIMARY,
            desc.z + 2,
        ),
        surface(
            Rect::new(
                layout.icon.origin.x + layout.icon.size.x * 0.5 - 1.0,
                layout.icon.max().y - 7.0,
                2.0,
                2.0,
            ),
            1.0,
            Token::ON_PRIMARY,
            desc.z + 2,
        ),
        surface(
            Rect::new(
                layout.footer.origin.x,
                layout.footer.origin.y,
                layout.footer.size.x,
                1.0,
            ),
            0.0,
            Token::DIVIDER,
            desc.z + 1,
        ),
        // Cancel: an outlined surface.
        instance(
            layout.cancel,
            [8.0; 4],
            Token::SURFACE_RAISED,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            desc.z + 1,
        ),
        // Confirm: the tone fill.
        surface(layout.confirm, 8.0, desc.tone.fill(), desc.z + 1),
    ];
    let mut payloads = Vec::with_capacity(glyphs.len());
    crate::primitive::glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y =
        A11yNode::new(Role::AlertDialog, desc.title.clone(), desc.rect).disabled(control.disabled);
    a11y.description = desc.message.clone();

    let card_hit = HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), desc.rect);
    let scrim = HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, viewport);
    let cancel_key = alert_cancel_node(node);
    let confirm_key = alert_confirm_node(node);
    let actions = [
        HitRegion::new(
            cancel_key,
            MODAL_LAYER,
            u32::from(desc.z) + 1,
            layout.cancel,
        )
        .with_cursor(CursorKind::Pointer),
        HitRegion::new(
            confirm_key,
            MODAL_LAYER,
            u32::from(desc.z) + 1,
            layout.confirm,
        )
        .with_cursor(CursorKind::Pointer),
    ];

    let mut output = WidgetOutput {
        primitives,
        payloads,
        hit: Some(card_hit),
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    };
    if !control.disabled {
        output.with_sub_control(
            cancel_key,
            actions[0].clone(),
            A11yNode::new(Role::Button, desc.cancel_label.clone(), layout.cancel).clickable(),
            true,
        );
        output.with_sub_control(
            confirm_key,
            actions[1].clone(),
            A11yNode::new(Role::Button, desc.confirm_label.clone(), layout.confirm).clickable(),
            true,
        );
    }

    AlertDialogOutput {
        output,
        scrim,
        actions,
        entry: OverlayEntry::modal(node, desc.rect, Vec::new()),
        backdrop: BackdropRequest::new(node, viewport, desc.backdrop_blur),
        layout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> AlertDialogDesc {
        AlertDialogDesc::new(Rect::new(340.0, 300.0, 420.0, 200.0), "Delete workspace?")
            .with_message("This cannot be undone.")
    }

    #[test]
    fn a_destructive_dialog_opens_on_cancel() {
        let desc = desc().destructive();
        assert_eq!(desc.initial_focus(), AlertFocus::Cancel);
        assert_eq!(desc.initial_focus_index(), 0);
        assert_eq!(AlertDialogState::new(&desc).focus(), AlertFocus::Cancel);
    }

    #[test]
    fn a_neutral_dialog_opens_on_confirm() {
        let desc = desc();
        assert_eq!(desc.initial_focus(), AlertFocus::Confirm);
        assert_eq!(desc.initial_focus_index(), 1);
    }

    #[test]
    fn enter_and_escape_report_the_two_outcomes() {
        let mut state = AlertDialogState::new(&desc());
        assert_eq!(
            state.handle_key(AlertDialogKey::Escape),
            AlertDialogOutcome::Cancelled
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::Enter),
            AlertDialogOutcome::Confirmed
        );
    }

    #[test]
    fn tab_moves_between_the_two_actions() {
        let mut state = AlertDialogState::new(&desc());
        assert_eq!(
            state.handle_key(AlertDialogKey::Tab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Cancel)
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::Tab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Confirm)
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::ShiftTab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Cancel)
        );
    }

    #[test]
    fn the_actions_sit_right_aligned_inside_the_footer() {
        let layout = desc().layout();
        assert!(desc().rect.contains_rect(&layout.cancel));
        assert!(desc().rect.contains_rect(&layout.confirm));
        assert_eq!(layout.confirm.max().x, desc().rect.max().x - DIALOG_PADDING);
        assert_eq!(layout.confirm.origin.y, layout.cancel.origin.y);
        assert!(layout.cancel.max().x < layout.confirm.origin.x);
        assert_eq!(layout.footer.size.x, desc().rect.size.x);
    }

    #[test]
    fn the_emitter_paints_a_scrim_card_and_two_buttons() {
        let output = emit_alert_dialog(
            7,
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            &desc(),
            &[],
            ControlState::default(),
        );
        assert_eq!(output.scrim.layer, PORTAL_LAYER);
        assert_eq!(output.output.hit.as_ref().unwrap().layer, MODAL_LAYER);
        assert_eq!(output.actions.len(), 2);
        let a11y = output.output.a11y.expect("dialog");
        assert_eq!(a11y.role, Role::AlertDialog);
        assert_eq!(a11y.label, "Delete workspace?");
        assert!(!output.output.primitives.is_empty());
    }

    #[test]
    fn a_destructive_confirm_paints_the_error_fill() {
        let output = emit_alert_dialog(
            7,
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            &desc().destructive(),
            &[],
            ControlState::default(),
        );
        let confirm = output.layout.confirm;
        let primitive = output
            .output
            .primitives
            .iter()
            .find(|primitive| {
                primitive.local_pos == [confirm.origin.x, confirm.origin.y]
                    && primitive.size == [confirm.size.x, confirm.size.y]
            })
            .expect("confirm surface");
        assert_eq!(primitive.fill_token_or_uv, u32::from(Token::ERROR.raw()));
    }

    #[test]
    fn the_backdrop_request_follows_the_blur_setting() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let output = emit_alert_dialog(7, viewport, &desc(), &[], ControlState::default());
        assert_eq!(output.backdrop.map(|request| request.blur), Some(8.0));
        let sharp = AlertDialogDesc {
            backdrop_blur: 0.0,
            ..desc()
        };
        let output = emit_alert_dialog(7, viewport, &sharp, &[], ControlState::default());
        assert!(output.backdrop.is_none());
    }
}

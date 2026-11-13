//! Rich hover cards and empty states.
//!
//! A tooltip is information only and passes the pointer straight through. A
//! hover card is the interactive sibling: the pointer may travel *into* the
//! card and use whatever it holds. That changes the timing model — leaving the
//! trigger no longer hides the card, it starts a grace period that only
//! expires once the pointer is outside both the trigger and the card.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::overlay::{place_anchored_popup, OverlayEntry, PlacedPopup, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

/// How long the pointer must rest on the trigger.
pub const HOVER_CARD_DELAY_MS: f32 = 500.0;
/// How long the card survives after the pointer leaves both regions.
pub const HOVER_CARD_GRACE_MS: f32 = 150.0;
/// The card corner radius.
pub const HOVER_CARD_RADIUS: f32 = 10.0;
const HOVER_CARD_SHADOW: f32 = 22.0;

/// A hover card descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverCardDesc {
    /// The trigger rectangle.
    pub anchor: Rect,
    /// The viewport the card stays inside.
    pub viewport: Rect,
    /// The measured card size.
    pub size: Vec2,
    /// Where the card prefers to sit.
    pub placement: Placement,
    /// The delay before it appears.
    pub delay_ms: f32,
    /// The grace period after the pointer leaves.
    pub grace_ms: f32,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
}

impl HoverCardDesc {
    /// A card below its trigger.
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::BottomStart,
            delay_ms: HOVER_CARD_DELAY_MS,
            grace_ms: HOVER_CARD_GRACE_MS,
            radius: HOVER_CARD_RADIUS,
            z: 935,
        }
    }

    /// The same card on another side.
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// The placed card rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        let placed: PlacedPopup =
            place_anchored_popup(self.anchor, self.size, self.viewport, self.placement);
        placed.rect
    }
}

/// The open/close timing of one hover card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverCardState {
    visible: bool,
    anchor: Option<u64>,
    now_ms: f32,
    hovered_since: Option<f32>,
    hide_at: Option<f32>,
}

impl HoverCardState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the card is currently shown.
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// Advance one frame.
    ///
    /// `hovered` is the trigger node under the pointer, `over_card` whether the
    /// pointer is inside the card. Returns whether the visibility changed.
    pub fn update(
        &mut self,
        desc: &HoverCardDesc,
        hovered: Option<u64>,
        over_card: bool,
        dt_ms: f32,
    ) -> bool {
        let was = self.visible;
        let previous = self.now_ms;
        self.now_ms += dt_ms.max(0.0);

        match hovered {
            Some(node) => {
                if self.anchor != Some(node) {
                    // A different trigger restarts the delay and hides the
                    // previous card: its content does not apply any more.
                    self.anchor = Some(node);
                    self.visible = false;
                    self.hide_at = None;
                    self.hovered_since = Some(previous);
                }
            }
            None => {
                if self.anchor.is_some() {
                    self.anchor = None;
                    self.hovered_since = None;
                }
            }
        }

        if !self.visible {
            if let Some(since) = self.hovered_since {
                if self.anchor.is_some() && self.now_ms - since >= desc.delay_ms.max(0.0) {
                    self.visible = true;
                }
            }
        }

        if self.visible {
            if hovered.is_some() || over_card {
                self.hide_at = None;
            } else {
                let hide_at = *self
                    .hide_at
                    .get_or_insert(previous + desc.grace_ms.max(0.0));
                if self.now_ms >= hide_at {
                    self.visible = false;
                    self.hide_at = None;
                    self.hovered_since = None;
                    self.anchor = None;
                }
            }
        }

        self.visible != was
    }

    /// Hide immediately.
    pub fn hide(&mut self) {
        self.visible = false;
        self.anchor = None;
        self.hovered_since = None;
        self.hide_at = None;
    }
}

/// Emit a hover card. The output **does** carry a hit region: the card is
/// interactive, which is what distinguishes it from a tooltip.
#[must_use]
pub fn emit_hover_card(
    node: u64,
    anchor_node: u64,
    desc: &HoverCardDesc,
    card_glyphs: &[GlyphQuad],
    state: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect();
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            [desc.radius; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            HOVER_CARD_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::OVERLAY_BG, desc.z),
        instance(
            rect,
            [desc.radius; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads = Vec::with_capacity(card_glyphs.len());
    crate::primitive::glyphs(
        card_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );

    let a11y = A11yNode::new(Role::Group, String::new(), rect).disabled(state.disabled);

    (
        WidgetOutput {
            primitives,
            payloads,
            hit: Some(
                HitRegion::new(node, PORTAL_LAYER, u32::from(desc.z), rect)
                    .with_cursor(CursorKind::Default),
            ),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        OverlayEntry::popover(node, anchor_node, rect),
    )
}

/// The icon diameter of an empty state.
pub const EMPTY_ICON: f32 = 48.0;
/// The action button size.
pub const EMPTY_ACTION_SIZE: (f32, f32) = (120.0, 32.0);

/// A centred empty-list placeholder.
#[derive(Clone, Debug, PartialEq)]
pub struct EmptyStateDesc {
    /// The region the placeholder centres in.
    pub rect: Rect,
    /// The headline.
    pub title: String,
    /// The supporting copy.
    pub message: String,
    /// The optional call to action.
    pub action_label: Option<String>,
    /// The draw order key.
    pub z: u16,
}

impl EmptyStateDesc {
    /// An empty state with a title.
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            message: String::new(),
            action_label: None,
            z: 520,
        }
    }

    /// The same state with supporting copy.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// The same state with a call to action.
    #[must_use]
    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action_label = Some(label.into());
        self
    }

    /// The block height used to centre the content.
    fn content_height(&self) -> f32 {
        let mut height = EMPTY_ICON + 12.0 + 24.0;
        if !self.message.is_empty() {
            height += 6.0 + 20.0;
        }
        if self.action_label.is_some() {
            height += 16.0 + EMPTY_ACTION_SIZE.1;
        }
        height
    }

    /// The top of the centred block.
    fn content_top(&self) -> f32 {
        self.rect.origin.y + (self.rect.size.y - self.content_height()).max(0.0) * 0.5
    }

    /// The icon square.
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        Rect::new(
            self.rect.center().x - EMPTY_ICON * 0.5,
            self.content_top(),
            EMPTY_ICON,
            EMPTY_ICON,
        )
    }

    /// The headline.
    #[must_use]
    pub fn title_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.icon_rect().max().y + 12.0,
            self.rect.size.x,
            24.0,
        )
    }

    /// The supporting copy.
    #[must_use]
    pub fn message_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.title_rect().max().y + 6.0,
            self.rect.size.x,
            20.0,
        )
    }

    /// The call to action, when there is one.
    #[must_use]
    pub fn action_rect(&self) -> Option<Rect> {
        self.action_label.as_ref().map(|_| {
            let top = if self.message.is_empty() {
                self.title_rect().max().y + 16.0
            } else {
                self.message_rect().max().y + 16.0
            };
            Rect::new(
                self.rect.center().x - EMPTY_ACTION_SIZE.0 * 0.5,
                top,
                EMPTY_ACTION_SIZE.0,
                EMPTY_ACTION_SIZE.1,
            )
        })
    }
}

/// What one empty state emits.
#[derive(Clone, Debug)]
pub struct EmptyStateOutput {
    /// The icon, glyphs and action surface.
    pub output: WidgetOutput,
    /// The action button, when there is one.
    pub action: Option<HitRegion>,
}

/// Emit a centred empty state.
#[must_use]
pub fn emit_empty_state(
    node: u64,
    desc: &EmptyStateDesc,
    empty_glyphs: &[GlyphQuad],
    control: ControlState,
) -> EmptyStateOutput {
    let icon = desc.icon_rect();
    let mut primitives = Vec::with_capacity(5 + empty_glyphs.len());
    primitives.push(surface(
        icon,
        icon.size.x * 0.5,
        Token::SURFACE_SUNKEN,
        desc.z,
    ));
    primitives.push(surface(
        Rect::new(
            icon.origin.x + 12.0,
            icon.origin.y + 16.0,
            icon.size.x - 24.0,
            icon.size.y - 28.0,
        ),
        3.0,
        Token::PRIMARY_SOFT,
        desc.z + 1,
    ));
    let mut payloads = Vec::with_capacity(empty_glyphs.len());
    crate::primitive::glyphs(
        empty_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );

    let action = desc.action_rect();
    if let Some(action) = action {
        primitives.push(surface(action, 8.0, Token::PRIMARY_SOFT, desc.z + 1));
        primitives.push(crate::primitive::focus_ring(
            action,
            8.0,
            Token::FOCUS_RING,
            2.0,
            desc.z + 2,
            control.focused,
        ));
    }

    let mut a11y = A11yNode::new(Role::Group, desc.title.clone(), desc.rect);
    a11y.description = desc.message.clone();

    EmptyStateOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        action: action.map(|action| {
            HitRegion::new(node, PORTAL_LAYER, u32::from(desc.z), action)
                .with_cursor(CursorKind::Pointer)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    fn card() -> HoverCardDesc {
        HoverCardDesc::new(
            Rect::new(200.0, 200.0, 120.0, 32.0),
            viewport(),
            Vec2::new(240.0, 160.0),
        )
    }

    #[test]
    fn the_card_waits_for_the_delay_and_shows_once() {
        let desc = card();
        let mut state = HoverCardState::new();
        assert!(!state.update(&desc, Some(7), false, 499.0));
        assert!(!state.visible());
        assert!(state.update(&desc, Some(7), false, 2.0));
        assert!(state.visible());
        assert!(!state.update(&desc, Some(7), false, 16.0));
    }

    #[test]
    fn leaving_the_trigger_before_the_delay_cancels_it() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 300.0);
        state.update(&desc, None, false, 16.0);
        assert!(!state.update(&desc, None, false, 500.0));
        assert!(!state.visible());
    }

    #[test]
    fn moving_into_the_card_keeps_it_open() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 500.0);
        assert!(state.visible());
        // The pointer leaves the trigger and enters the card.
        state.update(&desc, None, true, 16.0);
        assert!(!state.update(&desc, None, true, 500.0));
        assert!(state.visible());
    }

    #[test]
    fn leaving_both_waits_for_the_grace_period() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 500.0);
        state.update(&desc, None, false, 100.0);
        assert!(state.visible());
        assert!(state.update(&desc, None, false, 60.0));
        assert!(!state.visible());
    }

    #[test]
    fn the_card_stays_inside_the_viewport() {
        let mut desc = card();
        desc.anchor = Rect::new(1200.0, 690.0, 60.0, 24.0);
        let rect = desc.rect();
        assert!(viewport().contains_rect(&rect));
    }

    #[test]
    fn the_card_is_interactive_unlike_a_tooltip() {
        let desc = card();
        let (output, entry) = emit_hover_card(9, 4, &desc, &[], ControlState::default());
        let hit = output.hit.expect("interactive hit region");
        assert_eq!(hit.node, 9);
        assert_eq!(hit.cursor, Some(CursorKind::Default));
        assert_eq!(entry.anchor, Some(4));
        assert_eq!(output.primitives.len(), 3);
    }

    fn empty() -> EmptyStateDesc {
        EmptyStateDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0), "Nothing here yet")
            .with_message("Create your first item to begin.")
            .with_action("Create")
    }

    #[test]
    fn the_empty_state_is_vertically_centred() {
        let desc = empty();
        let icon = desc.icon_rect();
        assert_eq!(icon.center().x, desc.rect.center().x);
        let top_gap = icon.origin.y - desc.rect.origin.y;
        let bottom_gap = desc.rect.max().y - desc.action_rect().unwrap().max().y;
        assert!(
            (top_gap - bottom_gap).abs() < 0.5,
            "{top_gap} vs {bottom_gap}"
        );
    }

    #[test]
    fn the_empty_state_without_an_action_has_no_hit_region() {
        let desc = EmptyStateDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0), "Empty");
        let output = emit_empty_state(3, &desc, &[], ControlState::default());
        assert!(output.action.is_none());
        assert!(output.output.hit.is_none());
        assert_eq!(output.output.a11y.map(|node| node.role), Some(Role::Group));
    }

    #[test]
    fn the_empty_state_action_is_clickable() {
        let desc = empty();
        let output = emit_empty_state(3, &desc, &[], ControlState::default());
        let action = output.action.expect("action");
        assert_eq!(action.bounds, desc.action_rect().unwrap());
        assert_eq!(action.cursor, Some(CursorKind::Pointer));
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = empty();
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_empty_state(3, &desc, &[], control);
                        counts.push(output.output.primitives.len());
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

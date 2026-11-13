//! The split pane: two regions separated by a draggable gutter.
//!
//! The widget owns the *split*, not the panes. A caller lays its own content
//! out inside [`SplitLayout::first`] and [`SplitLayout::second`]; the pane
//! only guarantees that the two rectangles never overlap, that the gutter is
//! hit-testable with the resize cursor, and that the minimum sizes survive a
//! drag.
//!
//! Ratio lives in the state, geometry lives in the descriptor: moving the
//! gutter is a pure `state.ratio = …` update, so an animation can drive the
//! same field without touching the layout tree.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{instance, outlined, surface};
use crate::scroll::ScrollAxis;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default gutter thickness, in logical pixels.
pub const SPLIT_HANDLE: f32 = 8.0;

/// The minimum length of a pane when the caller does not set one.
pub const SPLIT_MIN_PANE: f32 = 48.0;

/// A keyboard nudge as a fraction of the split region.
const KEY_STEP: f32 = 0.02;

/// Which of the two panes a collapse applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSide {
    /// The first pane (left or top).
    First,
    /// The second pane (right or bottom).
    Second,
}

/// The three rectangles a split produces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    /// The first pane.
    pub first: Rect,
    /// The draggable gutter.
    pub gutter: Rect,
    /// The second pane.
    pub second: Rect,
}

/// The pure data descriptor of a split pane.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitPaneDesc {
    /// The whole split area.
    pub rect: Rect,
    /// `Horizontal` puts the panes side by side, `Vertical` stacks them.
    ///
    /// The variant names the direction the panes are laid out in, so
    /// `Horizontal` yields a vertical gutter with the east-west resize
    /// cursor. `Both` falls back to `Horizontal`.
    pub axis: ScrollAxis,
    /// The smallest allowed length of the first pane, in pixels.
    pub min_first: f32,
    /// The smallest allowed length of the second pane, in pixels.
    pub min_second: f32,
    /// The gutter thickness, in pixels.
    pub handle_size: f32,
    /// The draw order key.
    pub z: u16,
}

impl SplitPaneDesc {
    /// A split pane with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, axis: ScrollAxis) -> Self {
        Self {
            rect,
            axis,
            min_first: SPLIT_MIN_PANE,
            min_second: SPLIT_MIN_PANE,
            handle_size: SPLIT_HANDLE,
            z: 400,
        }
    }

    /// A side-by-side split.
    #[must_use]
    pub fn horizontal(rect: Rect) -> Self {
        Self::new(rect, ScrollAxis::Horizontal)
    }

    /// A stacked split.
    #[must_use]
    pub fn vertical(rect: Rect) -> Self {
        Self::new(rect, ScrollAxis::Vertical)
    }

    /// Replace the minimum pane lengths.
    #[must_use]
    pub fn with_minimums(mut self, first: f32, second: f32) -> Self {
        self.min_first = first.max(0.0);
        self.min_second = second.max(0.0);
        self
    }

    /// Replace the gutter thickness.
    #[must_use]
    pub fn with_handle(mut self, handle: f32) -> Self {
        self.handle_size = handle.max(1.0);
        self
    }

    /// Whether the panes are laid out left-to-right.
    #[must_use]
    pub fn is_horizontal(&self) -> bool {
        !matches!(self.axis, ScrollAxis::Vertical)
    }

    /// The gutter cursor this split uses.
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        if self.is_horizontal() {
            CursorKind::ResizeEastWest
        } else {
            CursorKind::ResizeNorthSouth
        }
    }

    /// The length available to the panes, gutter excluded.
    #[must_use]
    pub fn split_length(&self) -> f32 {
        let total = if self.is_horizontal() {
            self.rect.size.x
        } else {
            self.rect.size.y
        };
        (total - self.handle_size).max(0.0)
    }

    /// The ratio range the minimum pane sizes allow.
    ///
    /// When the minima cannot both fit, the range collapses to the midpoint
    /// and the panes shrink evenly: refusing to split would be worse than
    /// violating a constraint that is already impossible.
    #[must_use]
    pub fn ratio_bounds(&self) -> (f32, f32) {
        let length = self.split_length();
        if length <= 0.0 {
            return (0.0, 0.0);
        }
        let low = (self.min_first / length).clamp(0.0, 1.0);
        let high = (1.0 - self.min_second / length).clamp(0.0, 1.0);
        if low > high {
            let mid = 0.5;
            (mid, mid)
        } else {
            (low, high)
        }
    }

    /// Clamp a ratio into the legal range.
    #[must_use]
    pub fn clamp_ratio(&self, ratio: f32) -> f32 {
        let (low, high) = self.ratio_bounds();
        ratio.clamp(low, high)
    }

    /// Turn a pointer position into a ratio along the split axis.
    #[must_use]
    pub fn ratio_at(&self, pointer: Vec2) -> f32 {
        let length = self.split_length();
        if length <= 0.0 {
            return 0.0;
        }
        let offset = if self.is_horizontal() {
            pointer.x - self.rect.origin.x
        } else {
            pointer.y - self.rect.origin.y
        };
        self.clamp_ratio(offset / length)
    }
}

/// The live state of a split pane.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitPaneState {
    /// The share of the split region owned by the first pane, `0.0..=1.0`.
    pub ratio: f32,
    /// Whether the gutter is currently being dragged.
    pub dragging: bool,
    /// The pane collapsed to zero, if any.
    pub collapsed: Option<SplitSide>,
    drag_origin: Option<f32>,
}

impl Default for SplitPaneState {
    fn default() -> Self {
        Self::new(0.5)
    }
}

impl SplitPaneState {
    /// A split at the given ratio.
    #[must_use]
    pub fn new(ratio: f32) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            dragging: false,
            collapsed: None,
            drag_origin: None,
        }
    }

    /// The ratio the panes currently use, after clamping and collapse.
    #[must_use]
    pub fn effective_ratio(&self, desc: &SplitPaneDesc) -> f32 {
        match self.collapsed {
            Some(SplitSide::First) => 0.0,
            Some(SplitSide::Second) => 1.0,
            None => desc.clamp_ratio(self.ratio),
        }
    }

    /// The three rectangles this state produces.
    #[must_use]
    pub fn layout(&self, desc: &SplitPaneDesc) -> SplitLayout {
        let length = desc.split_length();
        let first_len = length * self.effective_ratio(desc);
        let handle = desc.handle_size;
        if desc.is_horizontal() {
            let x = desc.rect.origin.x;
            let y = desc.rect.origin.y;
            let h = desc.rect.size.y;
            SplitLayout {
                first: Rect::new(x, y, first_len, h),
                gutter: Rect::new(x + first_len, y, handle, h),
                second: Rect::new(x + first_len + handle, y, (length - first_len).max(0.0), h),
            }
        } else {
            let x = desc.rect.origin.x;
            let y = desc.rect.origin.y;
            let w = desc.rect.size.x;
            SplitLayout {
                first: Rect::new(x, y, w, first_len),
                gutter: Rect::new(x, y + first_len, w, handle),
                second: Rect::new(x, y + first_len + handle, w, (length - first_len).max(0.0)),
            }
        }
    }

    /// Capture the pointer at press time.
    pub fn begin_drag(&mut self, desc: &SplitPaneDesc, pointer: Vec2) {
        self.dragging = true;
        self.drag_origin = Some(if desc.is_horizontal() {
            pointer.x
        } else {
            pointer.y
        });
        // Dragging a collapsed split implies expanding it: a gutter that
        // refuses to move reads as broken, not as "locked".
        self.collapsed = None;
    }

    /// Update the ratio from a pointer position.
    ///
    /// The ratio follows the pointer directly rather than accumulating a
    /// delta, so a dropped or coalesced move never leaves the gutter behind
    /// the cursor.
    pub fn drag_to(&mut self, desc: &SplitPaneDesc, pointer: Vec2) {
        if !self.dragging {
            return;
        }
        self.ratio = desc.ratio_at(pointer);
    }

    /// Release the pointer.
    pub fn end_drag(&mut self) {
        self.dragging = false;
        self.drag_origin = None;
    }

    /// Collapse the given side, or expand when it is already collapsed.
    pub fn toggle_collapsed(&mut self, side: SplitSide) {
        self.collapsed = if self.collapsed == Some(side) {
            None
        } else {
            Some(side)
        };
        self.dragging = false;
        self.drag_origin = None;
    }

    /// Expand both panes.
    pub fn expand(&mut self) {
        self.collapsed = None;
    }

    /// Nudge the ratio by a number of pixels.
    pub fn nudge(&mut self, desc: &SplitPaneDesc, pixels: f32) {
        let length = desc.split_length();
        if length <= 0.0 {
            return;
        }
        self.collapsed = None;
        self.ratio = desc.clamp_ratio(self.ratio + pixels / length);
    }

    /// The ratio change one arrow-key press produces.
    #[must_use]
    pub fn key_step() -> f32 {
        KEY_STEP
    }

    /// The pointer offset recorded at press time, if a drag is active.
    #[must_use]
    pub fn drag_origin(&self) -> Option<f32> {
        self.drag_origin
    }
}

/// The keys a focused split pane understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitKey {
    /// Move the gutter towards the start.
    Back,
    /// Move the gutter towards the end.
    Forward,
    /// Collapse the second pane.
    Collapse,
    /// Restore a collapsed pane.
    Expand,
}

/// Reduce one key press into a state update.
///
/// Returns whether the state changed, which is what the caller feeds into its
/// dirty flags.
pub fn reduce_split_key(desc: &SplitPaneDesc, state: &mut SplitPaneState, key: SplitKey) -> bool {
    match key {
        SplitKey::Back => {
            let before = (state.ratio, state.collapsed);
            state.nudge(desc, -desc.split_length() * KEY_STEP);
            before != (state.ratio, state.collapsed)
        }
        SplitKey::Forward => {
            let before = (state.ratio, state.collapsed);
            state.nudge(desc, desc.split_length() * KEY_STEP);
            before != (state.ratio, state.collapsed)
        }
        SplitKey::Collapse => {
            if state.collapsed == Some(SplitSide::Second) {
                false
            } else {
                state.toggle_collapsed(SplitSide::Second);
                true
            }
        }
        SplitKey::Expand => {
            if state.collapsed.is_some() {
                state.expand();
                true
            } else {
                false
            }
        }
    }
}

/// The colour slots a split pane paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitPaneTokens {
    /// The gutter track.
    pub track: Token,
    /// The gutter track while hovered or dragged.
    pub track_active: Token,
    /// The grabber.
    pub grip: Token,
    /// The grabber while hovered or dragged.
    pub grip_active: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl SplitPaneTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::SURFACE_SUNKEN,
            track_active: Token::CONTROL_HOVER,
            grip: Token::OUTLINE,
            grip_active: Token::PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit the gutter: its track, its grabber and the resize hit region.
///
/// The two panes are not painted here; the caller paints its own content into
/// [`SplitLayout::first`] and [`SplitLayout::second`].
#[must_use]
pub fn emit_split_pane(
    node: u64,
    desc: &SplitPaneDesc,
    state: &SplitPaneState,
    control: ControlState,
    tokens: SplitPaneTokens,
) -> WidgetOutput {
    let layout = state.layout(desc);
    let disabled = control.disabled;
    let active =
        matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed) || state.dragging;

    let track = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if active {
        tokens.track_active
    } else {
        tokens.track
    };
    let grip_color = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if active {
        tokens.grip_active
    } else {
        tokens.grip
    };

    let mut primitives = Vec::with_capacity(5);
    primitives.push(surface(
        layout.gutter,
        desc.handle_size * 0.5,
        track,
        desc.z,
    ));

    // Three dots along the gutter make the drag affordance visible without a
    // glyph, so a font without "⋮" still renders a usable handle.
    let (cx, cy) = (
        layout.gutter.origin.x + layout.gutter.size.x * 0.5,
        layout.gutter.origin.y + layout.gutter.size.y * 0.5,
    );
    let dot = (desc.handle_size * 0.22).clamp(1.0, 2.5);
    for offset in [-1.0f32, 0.0, 1.0] {
        let centre = if desc.is_horizontal() {
            Vec2::new(cx, cy + offset * dot * 3.0)
        } else {
            Vec2::new(cx + offset * dot * 3.0, cy)
        };
        if desc.is_horizontal() && layout.gutter.size.y < dot * 9.0 {
            continue;
        }
        if !desc.is_horizontal() && layout.gutter.size.x < dot * 9.0 {
            continue;
        }
        primitives.push(dot_primitive(centre, dot, grip_color, desc.z));
    }

    // The ring is always emitted; focus only changes its opacity.
    primitives.push(crate::primitive::focus_ring(
        layout.gutter.inflate(1.0),
        3.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), layout.gutter)
            .with_cursor(desc.cursor())
    });
    let percent = (state.effective_ratio(desc) * 100.0).round();
    let mut a11y = A11yNode::new(Role::Splitter, "", layout.gutter)
        .clickable()
        .disabled(disabled);
    a11y.value = format!("{percent:.0}%");

    WidgetOutput {
        primitives,
        payloads: Vec::<GpuPayload>::new(),
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
    }
}

/// A small circular grabber dot.
fn dot_primitive(centre: Vec2, radius: f32, color: Token, z: u16) -> GpuPrimitiveInstance {
    let dot = Rect::new(
        centre.x - radius,
        centre.y - radius,
        radius * 2.0,
        radius * 2.0,
    );
    let mut primitive = instance(
        dot,
        [radius; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Content,
        z,
    );
    primitive.border_token_or_tex = Token::NONE.raw() as u32;
    primitive
}

/// Emit the two pane backdrops, if the caller wants the split to paint them.
///
/// Many callers paint their own panels; this helper exists so a bare split
/// still shows both sides instead of one transparent rectangle.
#[must_use]
pub fn emit_split_pane_panes(
    desc: &SplitPaneDesc,
    state: &SplitPaneState,
    fill: Token,
    surface_token: Token,
) -> WidgetOutput {
    let layout = state.layout(desc);
    let mut primitives = Vec::with_capacity(2);
    if layout.first.size.x > 0.0 && layout.first.size.y > 0.0 {
        primitives.push(outlined(
            layout.first,
            0.0,
            fill,
            surface_token,
            1.0,
            0.0,
            desc.z,
        ));
    }
    if layout.second.size.x > 0.0 && layout.second.size.y > 0.0 {
        primitives.push(outlined(
            layout.second,
            0.0,
            fill,
            surface_token,
            1.0,
            0.0,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SplitPaneDesc {
        SplitPaneDesc::horizontal(Rect::new(0.0, 0.0, 500.0, 300.0))
    }

    #[test]
    fn the_gutter_separates_two_disjoint_panes() {
        let desc = desc();
        let layout = SplitPaneState::new(0.5).layout(&desc);
        assert_eq!(layout.first.origin.x, 0.0);
        assert_eq!(layout.first.size.x, 246.0);
        assert_eq!(layout.gutter.origin.x, 246.0);
        assert_eq!(layout.gutter.size.x, 8.0);
        assert_eq!(layout.second.origin.x, 254.0);
        assert_eq!(layout.second.size.x, 246.0);
    }

    #[test]
    fn a_vertical_split_stacks_along_y() {
        let desc = SplitPaneDesc::vertical(Rect::new(0.0, 0.0, 300.0, 500.0));
        let layout = SplitPaneState::new(0.25).layout(&desc);
        assert_eq!(layout.first.size.y, 123.0);
        assert_eq!(layout.gutter.origin.y, 123.0);
        assert_eq!(layout.gutter.size.y, 8.0);
        assert_eq!(layout.second.origin.y, 131.0);
        assert_eq!(layout.second.size.y, 369.0);
    }

    #[test]
    fn the_minimum_sizes_clamp_the_ratio() {
        let desc = desc().with_minimums(100.0, 100.0);
        let mut state = SplitPaneState::new(0.5);
        assert_eq!(state.effective_ratio(&desc), 0.5);

        state.drag_to(&desc, Vec2::new(0.0, 0.0));
        assert_eq!(state.effective_ratio(&desc), 0.5, "drag_to needs a begin");

        state.begin_drag(&desc, Vec2::new(250.0, 150.0));
        state.drag_to(&desc, Vec2::new(0.0, 150.0));
        assert!((state.effective_ratio(&desc) - 100.0 / 492.0).abs() < 1e-4);

        state.drag_to(&desc, Vec2::new(500.0, 150.0));
        assert!((state.effective_ratio(&desc) - (1.0 - 100.0 / 492.0)).abs() < 1e-4);
    }

    #[test]
    fn dragging_follows_the_pointer_and_ends_on_release() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.begin_drag(&desc, Vec2::new(246.0, 150.0));
        assert!(state.dragging);
        state.drag_to(&desc, Vec2::new(123.0, 150.0));
        assert!((state.ratio - 0.25).abs() < 1e-4);
        assert!((state.layout(&desc).first.size.x - 123.0).abs() < 1e-4);
        state.end_drag();
        assert!(!state.dragging);
        state.drag_to(&desc, Vec2::new(400.0, 150.0));
        assert!((state.ratio - 0.25).abs() < 1e-4, "released drag is inert");
    }

    #[test]
    fn double_click_collapses_and_restores_the_second_pane() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::Second);
        assert_eq!(state.collapsed, Some(SplitSide::Second));
        let layout = state.layout(&desc);
        assert_eq!(layout.second.size.x, 0.0);
        assert_eq!(layout.first.size.x, 492.0);
        assert_eq!(layout.gutter.origin.x, 492.0);

        state.toggle_collapsed(SplitSide::Second);
        assert_eq!(state.collapsed, None);
        assert!((state.layout(&desc).first.size.x - 246.0).abs() < 1e-4);
    }

    #[test]
    fn collapsing_the_first_pane_moves_the_gutter_to_the_origin() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::First);
        let layout = state.layout(&desc);
        assert_eq!(layout.first.size.x, 0.0);
        assert_eq!(layout.gutter.origin.x, 0.0);
        assert_eq!(layout.second.origin.x, 8.0);
        assert_eq!(layout.second.size.x, 492.0);
    }

    #[test]
    fn dragging_a_collapsed_split_expands_it() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::Second);
        state.begin_drag(&desc, Vec2::new(492.0, 150.0));
        assert_eq!(state.collapsed, None);
        assert!(state.dragging);
    }

    #[test]
    fn the_gutter_is_pickable_with_the_resize_cursor() {
        let desc = desc();
        let tokens = SplitPaneTokens::standard();
        let output = emit_split_pane(
            7,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState::default(),
            tokens,
        );
        let hit = output.hit.expect("the gutter is always pickable");
        assert_eq!(hit.node, 7);
        assert_eq!(hit.cursor, Some(CursorKind::ResizeEastWest));
        assert!(output.focusable);
        let a11y = output.a11y.expect("a splitter is reachable");
        assert_eq!(a11y.role, Role::Splitter);
        assert_eq!(a11y.value, "50%");
    }

    #[test]
    fn a_vertical_split_uses_the_north_south_cursor() {
        let desc = SplitPaneDesc::vertical(Rect::new(0.0, 0.0, 200.0, 400.0));
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState::default(),
            SplitPaneTokens::standard(),
        );
        assert_eq!(
            output.hit.expect("pickable").cursor,
            Some(CursorKind::ResizeNorthSouth)
        );
    }

    #[test]
    fn a_hovered_gutter_paints_the_active_tokens() {
        let desc = desc();
        let tokens = SplitPaneTokens::standard();
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track_active.raw() as u32
        );
    }

    #[test]
    fn a_disabled_split_is_not_pickable() {
        let desc = desc();
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            SplitPaneTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }

    #[test]
    fn arrow_keys_nudge_and_saturate_at_the_minimums() {
        let desc = desc().with_minimums(50.0, 50.0);
        let mut state = SplitPaneState::new(0.5);
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Forward));
        assert!((state.ratio - 0.52).abs() < 1e-4);
        for _ in 0..200 {
            reduce_split_key(&desc, &mut state, SplitKey::Forward);
        }
        assert!((state.effective_ratio(&desc) - (1.0 - 50.0 / 492.0)).abs() < 1e-4);
    }

    #[test]
    fn collapse_and_expand_keys_are_idempotent() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Collapse));
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Collapse));
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Expand));
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Expand));
        state.expand();
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Expand));
    }

    #[test]
    fn impossible_minimums_collapse_to_the_midpoint() {
        let desc =
            SplitPaneDesc::horizontal(Rect::new(0.0, 0.0, 100.0, 50.0)).with_minimums(200.0, 200.0);
        let (low, high) = desc.ratio_bounds();
        assert_eq!(low, high);
        assert_eq!(desc.clamp_ratio(0.1), 0.5);
    }
}

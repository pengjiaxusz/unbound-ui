//! Scroll containers and virtual-list playback.
//!
//! A [`ScrollViewDesc`] describes a viewport; [`ScrollState`] owns the scroll
//! offset and its spring. Neither holds child widgets: children are emitted by
//! the caller, clipped by [`ScrollViewOutput::clip`] and translated by
//! [`ScrollViewOutput::transform`]. That is what makes sub-pixel scrolling
//! free — a scroll changes one `GpuTransform`, never a single child primitive.
//!
//! Virtualised lists hook in through [`update_virtual_list`]: the window is
//! recomputed once per frame, and when only the sub-row offset moved the
//! function reports `WindowUpdate::offset_only` with **zero** rebinds.

use unbound_anim::{Spring2, SpringPreset};
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance, GpuTransform, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_world::virtualize::VirtualList;

use crate::primitive::surface;
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The UI layer a scroll view lives in.
pub const SCROLL_LAYER: u8 = 1;
/// The width of a scrollbar, in logical pixels.
pub const SCROLLBAR_WIDTH: f32 = 8.0;
/// The smallest scrollbar thumb, in logical pixels.
pub const MIN_THUMB: f32 = 24.0;

/// Which axes a scroll view accepts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ScrollAxis {
    #[default]
    Vertical,
    Horizontal,
    Both,
}

impl ScrollAxis {
    fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

/// The pure descriptor of a scroll viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollViewDesc {
    /// The viewport rectangle.
    pub rect: Rect,
    /// The full content size.
    pub content_size: Vec2,
    /// The accepted axes.
    pub axis: ScrollAxis,
    /// The draw order key.
    pub z: u16,
    /// The corner radius of the viewport clip.
    pub radius: f32,
    /// The scrollbar width.
    pub scrollbar_width: f32,
}

impl ScrollViewDesc {
    #[must_use]
    pub fn new(rect: Rect, content_size: Vec2) -> Self {
        Self {
            rect,
            content_size,
            axis: ScrollAxis::Vertical,
            z: 700,
            radius: 0.0,
            scrollbar_width: SCROLLBAR_WIDTH,
        }
    }

    /// The largest offset that still keeps the content covering the viewport.
    #[must_use]
    pub fn max_offset(&self) -> Vec2 {
        Vec2::new(
            (self.content_size.x - self.rect.width()).max(0.0),
            (self.content_size.y - self.rect.height()).max(0.0),
        )
    }

    /// The clamped offset for a proposed value.
    #[must_use]
    pub fn clamp_offset(&self, offset: Vec2) -> Vec2 {
        let max = self.max_offset();
        Vec2::new(
            if self.axis.horizontal() {
                offset.x.clamp(0.0, max.x)
            } else {
                0.0
            },
            if self.axis.vertical() {
                offset.y.clamp(0.0, max.y)
            } else {
                0.0
            },
        )
    }

    /// The scrollbar track rectangle on one axis.
    #[must_use]
    pub fn track_rect(&self, horizontal: bool) -> Rect {
        if horizontal {
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - self.scrollbar_width - 2.0,
                self.rect.width() - self.scrollbar_width - 4.0,
                self.scrollbar_width,
            )
        } else {
            Rect::new(
                self.rect.max().x - self.scrollbar_width - 2.0,
                self.rect.origin.y,
                self.scrollbar_width,
                self.rect.height() - self.scrollbar_width - 4.0,
            )
        }
    }

    /// The scrollbar thumb rectangle for an offset.
    #[must_use]
    pub fn thumb_rect(&self, offset: Vec2, horizontal: bool) -> Rect {
        let track = self.track_rect(horizontal);
        let (viewport, content, position) = if horizontal {
            (self.rect.width(), self.content_size.x, offset.x)
        } else {
            (self.rect.height(), self.content_size.y, offset.y)
        };
        let track_length = if horizontal {
            track.width()
        } else {
            track.height()
        };
        if content <= viewport || track_length <= 0.0 {
            return track;
        }
        let ratio = (viewport / content).clamp(0.0, 1.0);
        let thumb = (track_length * ratio).max(MIN_THUMB).min(track_length);
        let max_scroll = (content - viewport).max(0.0);
        let progress = if max_scroll > 0.0 {
            (position / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let travel = track_length - thumb;
        if horizontal {
            Rect::new(
                track.origin.x + travel * progress,
                track.origin.y,
                thumb,
                track.height(),
            )
        } else {
            Rect::new(
                track.origin.x,
                track.origin.y + travel * progress,
                track.width(),
                thumb,
            )
        }
    }
}

/// Which scrollbar is being dragged.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

impl ScrollbarAxis {
    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal)
    }
}

/// One scrollbar drag.
#[derive(Copy, Clone, Debug)]
struct Drag {
    axis: ScrollbarAxis,
    start_pointer: f32,
    start_offset: Vec2,
}

/// The live scroll offset and its spring.
#[derive(Clone, Debug)]
pub struct ScrollState {
    spring: Spring2,
    offset: Vec2,
    drag: Option<Drag>,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spring: Spring2::new(SpringPreset::Precise, Vec2::ZERO),
            offset: Vec2::ZERO,
            drag: None,
        }
    }

    /// The current visual offset.
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// The offset the spring is heading to.
    #[must_use]
    pub fn target(&self) -> Vec2 {
        Vec2::new(self.spring.x.target, self.spring.y.target)
    }

    /// Whether the spring is still moving.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.spring.is_settled()
    }

    /// Jump to an offset with no animation.
    pub fn jump_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let offset = desc.clamp_offset(offset);
        self.spring.snap_to(offset);
        self.offset = offset;
    }

    /// Add a delta (a wheel notch, a drag) and clamp it.
    pub fn scroll_by(&mut self, desc: &ScrollViewDesc, delta: Vec2) {
        let target = desc.clamp_offset(self.target() + delta);
        self.spring.set_target(target);
    }

    /// Scroll to an absolute offset, clamped.
    pub fn scroll_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let target = desc.clamp_offset(offset);
        self.spring.set_target(target);
    }

    /// Advance the spring. Returns `true` while it is moving.
    ///
    /// The visual offset is clamped every step: a spring may overshoot by a
    /// pixel or two on a hard fling, and the content must never show a gap
    /// above the first row.
    pub fn step(&mut self, desc: &ScrollViewDesc, dt: f32) -> bool {
        let moving = self.spring.step(dt);
        self.offset = desc.clamp_offset(self.spring.value());
        moving
    }

    /// Start dragging a scrollbar thumb.
    pub fn begin_drag(&mut self, axis: ScrollbarAxis, pointer: f32) {
        self.drag = Some(Drag {
            axis,
            start_pointer: pointer,
            start_offset: self.offset,
        });
    }

    /// Update a scrollbar drag.
    ///
    /// The thumb moves in track space; the content moves by the same ratio in
    /// content space, which keeps the thumb glued to the pointer.
    pub fn drag_to(&mut self, desc: &ScrollViewDesc, pointer: f32) {
        let Some(drag) = self.drag else {
            return;
        };
        let horizontal = drag.axis.horizontal();
        let track = desc.track_rect(horizontal);
        let track_length = if horizontal {
            track.width()
        } else {
            track.height()
        };
        let content = if horizontal {
            desc.content_size.x
        } else {
            desc.content_size.y
        };
        let viewport = if horizontal {
            desc.rect.width()
        } else {
            desc.rect.height()
        };
        let max_scroll = (content - viewport).max(0.0);
        if track_length <= 0.0 || max_scroll <= 0.0 {
            return;
        }
        let ratio = max_scroll / track_length;
        let delta = (pointer - drag.start_pointer) * ratio;
        let mut offset = drag.start_offset;
        if horizontal {
            offset.x += delta;
        } else {
            offset.y += delta;
        }
        self.jump_to(desc, offset);
    }

    /// End a scrollbar drag.
    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// Whether a scrollbar drag is in progress.
    #[must_use]
    pub fn dragging(&self) -> Option<ScrollbarAxis> {
        self.drag.map(|drag| drag.axis)
    }

    /// The offset a scrollbar press implies (clicking the track jumps).
    pub fn jump_from_track(&mut self, desc: &ScrollViewDesc, pointer: Vec2) {
        let horizontal = desc.axis.horizontal()
            && pointer.x >= desc.rect.origin.x
            && pointer.x <= desc.rect.max().x;
        let vertical = desc.axis.vertical()
            && pointer.y >= desc.rect.origin.y
            && pointer.y <= desc.rect.max().y;
        if horizontal {
            let track = desc.track_rect(true);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.x - desc.rect.width()).max(0.0);
                let progress =
                    ((pointer.x - track.origin.x) / track.width().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(max_scroll * progress, self.target().y));
                return;
            }
        }
        if vertical {
            let track = desc.track_rect(false);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.y - desc.rect.height()).max(0.0);
                let progress =
                    ((pointer.y - track.origin.y) / track.height().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(self.target().x, max_scroll * progress));
            }
        }
    }
}

/// What one scroll view emits.
///
/// The clip and transform are separate from [`WidgetOutput`] because they are
/// applied to the **children**; the scroll view itself only paints its
/// scrollbars.
#[derive(Clone, Debug)]
pub struct ScrollViewOutput {
    /// The scrollbars and the wheel hit region.
    pub output: WidgetOutput,
    /// The clip every child primitive must reference.
    pub clip: GpuClipNode,
    /// The translation every child primitive must reference.
    pub transform: GpuTransform,
    /// The scrollbar thumb hit region, if a scrollbar is visible.
    pub thumb: Option<HitRegion>,
}

/// A scroll view descriptor bound to an identity.
#[derive(Clone, Debug)]
pub struct ScrollViewWidget {
    /// The widget identity.
    pub node: u64,
    /// The descriptor.
    pub desc: ScrollViewDesc,
    /// The live offset.
    pub state: ScrollState,
}

impl WidgetDesc for ScrollViewWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_scroll_view(self.node, &self.desc, &self.state, state).output
    }
}

/// Emit the scrollbars, the clip and the child transform.
#[must_use]
pub fn emit_scroll_view(
    node: u64,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    state: ControlState,
) -> ScrollViewOutput {
    let rect = desc.rect;
    let offset = scroll.offset();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::with_capacity(4);

    let track_token = if state.disabled {
        Token::DISABLED
    } else {
        Token::CONTROL_TRACK
    };
    let thumb_token = if state.phase == crate::PointerPhase::Hovered || scroll.dragging().is_some()
    {
        Token::CONTROL_THUMB
    } else {
        Token::OUTLINE_STRONG
    };

    let mut thumb_region = None;
    if desc.axis.vertical() && desc.content_size.y > rect.height() {
        let track = desc.track_rect(false);
        primitives.push(surface(
            track,
            desc.scrollbar_width * 0.5,
            track_token,
            desc.z,
        ));
        let thumb = desc.thumb_rect(offset, false);
        primitives.push(surface(
            thumb,
            desc.scrollbar_width * 0.5,
            thumb_token,
            desc.z,
        ));
        thumb_region = Some(
            HitRegion::new(node ^ (1 << 63), SCROLL_LAYER, u32::from(desc.z) + 1, thumb)
                .with_cursor(CursorKind::Pointer),
        );
    }
    if desc.axis.horizontal() && desc.content_size.x > rect.width() {
        let track = desc.track_rect(true);
        primitives.push(surface(
            track,
            desc.scrollbar_width * 0.5,
            track_token,
            desc.z,
        ));
        let thumb = desc.thumb_rect(offset, true);
        primitives.push(surface(
            thumb,
            desc.scrollbar_width * 0.5,
            thumb_token,
            desc.z,
        ));
    }

    let hit = Some(HitRegion::new(node, SCROLL_LAYER, u32::from(desc.z), rect));
    let mut a11y =
        unbound_a11y::A11yNode::new(accesskit::Role::ScrollView, "", rect).disabled(state.disabled);
    a11y.actions
        .insert(unbound_a11y::A11yAction::ScrollIntoView);
    a11y.actions.insert(unbound_a11y::A11yAction::Focus);

    let clip = GpuClipNode::from_aabb([rect.min().x, rect.min().y, rect.max().x, rect.max().y]);
    let mut transform = GpuTransform::IDENTITY;
    transform.translation = [-offset.x, -offset.y];
    if offset != Vec2::ZERO {
        transform.flags = unbound_gpu::data::TransformFlags::ANIMATED.bits();
    }

    ScrollViewOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit,
            a11y: Some(a11y),
            focusable: false,
        },
        clip,
        transform,
        thumb: thumb_region,
    }
}

/// The state of a virtualised list's window after one frame.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VirtualWindow {
    /// The slot / bind accounting from [`VirtualList::update`].
    pub update: unbound_world::virtualize::WindowUpdate,
    /// The transform for the **whole** list, including the sub-row offset.
    pub transform: GpuTransform,
    /// The first item currently instantiated.
    pub first_item: usize,
    /// How many slots are live.
    pub slots: usize,
}

/// Update a virtual list from a scroll view.
///
/// Sub-row scrolling only changes `transform`: `update.rebound == 0` and no
/// data is re-bound, which is the "100 000 rows at 120 FPS" path.
#[must_use]
pub fn update_virtual_list(
    list: &mut VirtualList,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
) -> VirtualWindow {
    let offset = scroll.offset();
    let update = list.update(offset.y, desc.rect.height(), list_total(list, desc));
    let mut transform = GpuTransform::IDENTITY;
    // `VirtualList` reports the sub-row remainder as `offset`, but the
    // caller also needs the whole item offset for slot positions.
    transform.translation = [-offset.x, -offset.y];
    VirtualWindow {
        update,
        transform,
        first_item: list.first_item(),
        slots: list.slot_count(),
    }
}

/// The total row count a list currently reports.
///
/// `VirtualList` keeps its own `total`; the scroll descriptor carries the
/// content height. Converting back through the row height keeps a single
/// source of truth (the descriptor) without duplicating the item count.
fn list_total(list: &VirtualList, desc: &ScrollViewDesc) -> usize {
    let rows = (desc.content_size.y / list.row_height()).round();
    if rows.is_finite() && rows >= 0.0 {
        rows as usize
    } else {
        0
    }
}

/// One row's position for a virtual list, in viewport space.
#[must_use]
pub fn virtual_row_rect(
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    row_index: usize,
    row_height: f32,
) -> Rect {
    Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y + row_index as f32 * row_height - scroll.offset().y,
        desc.rect.width(),
        row_height,
    )
}

/// Paint a scrollbar thumb as a standalone primitive (for tests and tools).
#[must_use]
pub fn scrollbar_thumb_primitive(
    desc: &ScrollViewDesc,
    offset: Vec2,
    horizontal: bool,
) -> GpuPrimitiveInstance {
    let mut primitive = surface(
        desc.thumb_rect(offset, horizontal),
        desc.scrollbar_width * 0.5,
        Token::CONTROL_THUMB,
        desc.z,
    );
    primitive.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Background as u16,
    );
    primitive
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ScrollViewDesc {
        ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 3000.0))
    }

    #[test]
    fn wheel_scrolling_is_clamped_to_the_content() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(0.0, 120.0));
        state.step(&desc, 1.0 / 60.0);
        assert!(state.offset().y > 0.0);

        state.scroll_by(&desc, Vec2::new(0.0, 1_000_000.0));
        assert_eq!(state.target().y, 2700.0);
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert!(
            (state.offset().y - 2700.0).abs() < 0.5,
            "{}",
            state.offset().y
        );

        state.scroll_by(&desc, Vec2::new(0.0, -1_000_000.0));
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn a_vertical_only_view_never_scrolls_horizontally() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 3000.0));
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(200.0, 100.0));
        state.step(&desc, 1.0 / 60.0);
        assert_eq!(state.offset().x, 0.0);
        assert!(state.offset().y > 0.0);
    }

    #[test]
    fn the_thumb_ratio_tracks_the_viewport() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 600.0));
        let track = desc.track_rect(false);
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        assert!((thumb.height() - track.height() * 0.5).abs() < 1e-3);

        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).max().y,
            track.max().y
        );
        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).height(),
            thumb.height()
        );
    }

    #[test]
    fn the_thumb_is_never_smaller_than_the_minimum() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            Vec2::new(400.0, 100_000.0),
        );
        assert!(desc.thumb_rect(Vec2::ZERO, false).height() >= MIN_THUMB);
    }

    #[test]
    fn dragging_the_thumb_moves_the_content_by_the_track_ratio() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 10.0);
        assert_eq!(state.dragging(), Some(ScrollbarAxis::Vertical));
        state.drag_to(&desc, 110.0);
        assert!(state.offset().y > 0.0);
        state.end_drag();
        assert!(state.dragging().is_none());
    }

    #[test]
    fn a_thumb_drag_is_clamped_at_both_ends() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 100.0);
        state.drag_to(&desc, 100_000.0);
        assert_eq!(state.offset().y, 2700.0);
        state.drag_to(&desc, -100_000.0);
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn clicking_the_track_jumps_towards_the_click() {
        let desc = desc();
        let mut state = ScrollState::new();
        let track = desc.track_rect(false);
        state.jump_from_track(
            &desc,
            Vec2::new(track.center().x, track.origin.y + track.height() * 0.5),
        );
        assert!(state.target().y > 1000.0);
    }

    #[test]
    fn the_clip_covers_the_viewport_and_the_transform_owns_the_offset() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.jump_to(&desc, Vec2::new(0.0, 120.0));
        let output = emit_scroll_view(7, &desc, &state, ControlState::default());
        assert_eq!(output.clip.aabb_clip, [0.0, 0.0, 400.0, 300.0]);
        assert_eq!(output.transform.translation, [0.0, -120.0]);
        assert!(output
            .output
            .hit
            .as_ref()
            .is_some_and(|hit| hit.bounds.contains_rect(&desc.rect)));
    }

    #[test]
    fn a_sub_row_scroll_rebinds_nothing() {
        let desc = desc();
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let first = update_virtual_list(&mut list, &desc, &state);
        assert!(first.slots > 0);

        state.jump_to(&desc, Vec2::new(0.0, 7.5));
        let second = update_virtual_list(&mut list, &desc, &state);
        assert!(second.update.offset_only, "{:?}", second.update);
        assert_eq!(second.update.rebound, 0);
        assert_eq!(second.update.offset, -7.5);
        assert_eq!(second.transform.translation, [0.0, -7.5]);
    }

    #[test]
    fn a_virtual_list_keeps_its_entity_budget_across_a_long_scroll() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 800.0),
            Vec2::new(400.0, 28.0 * 100_000.0),
        );
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let _ = update_virtual_list(&mut list, &desc, &state);
        let budget = list.slot_count();
        for offset in [28.0, 2_800.0, 280_000.0, 2_800_000.0] {
            state.jump_to(&desc, Vec2::new(0.0, offset));
            let _ = update_virtual_list(&mut list, &desc, &state);
            assert_eq!(list.slot_count(), budget);
        }
        assert!((28..=120).contains(&budget), "{budget}");
    }
}

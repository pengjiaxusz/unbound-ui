//! TimelineTrack / DopeSheet multi-track keyframe & playhead scrubber.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_TRACKS: usize = 3;
const MAX_KEYFRAMES: usize = 6;

/// A single keyframe marker `(track_index, time_seconds)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// Track index `0..3`.
    pub track: usize,
    /// Time in seconds.
    pub time: f32,
}

/// Interactive state for [`TimelineTrackDesc`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineTrackState {
    /// Playhead position in seconds.
    pub playhead: f32,
    /// Total duration in seconds.
    pub duration: f32,
    /// Keyframes.
    pub keyframes: Vec<Keyframe>,
}

impl TimelineTrackState {
    /// Scrub playhead to `t` clamped to `0.0..=duration`.
    pub fn scrub(&mut self, t: f32) {
        self.playhead = t.clamp(0.0, self.duration.max(0.01));
    }
}

/// Descriptor for a [`TimelineTrack`].
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrackDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Playhead time in seconds.
    pub playhead: f32,
    /// Total duration in seconds.
    pub duration: f32,
    /// Keyframe markers.
    pub keyframes: Vec<Keyframe>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl TimelineTrackDesc {
    /// Create a timeline track descriptor.
    #[must_use]
    pub fn new(rect: Rect, playhead: f32, duration: f32, keyframes: Vec<Keyframe>) -> Self {
        Self {
            rect,
            playhead,
            duration: duration.max(0.01),
            keyframes,
            label: "Timeline Track".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape dope sheet (1 bg + 3 track lanes + 6 keyframe diamonds + 1 playhead + 1 focus ring = 12 primitives).
#[must_use]
pub fn emit_timeline_track(
    node: u64,
    desc: &TimelineTrackDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(12 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pad = 6.0;
    let lane_h = ((desc.rect.height() - pad * 4.0) / MAX_TRACKS as f32).max(14.0);
    let lane_w = (desc.rect.width() - pad * 2.0).max(20.0);
    for t in 0..MAX_TRACKS {
        let lane_r = Rect::new(
            desc.rect.origin.x + pad,
            desc.rect.origin.y + pad + t as f32 * (lane_h + pad),
            lane_w,
            lane_h,
        );
        primitives.push(outlined(
            lane_r,
            4.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    for k in 0..MAX_KEYFRAMES {
        let kf = desc.keyframes.get(k);
        let (track, norm_t) = kf
            .map(|f| {
                (
                    f.track.min(MAX_TRACKS - 1),
                    (f.time / desc.duration).clamp(0.0, 1.0),
                )
            })
            .unwrap_or((0, 0.0));
        let kx = desc.rect.origin.x + pad + norm_t * (lane_w - 8.0);
        let ky = desc.rect.origin.y + pad + track as f32 * (lane_h + pad) + (lane_h - 8.0) * 0.5;
        let mut marker = surface(Rect::new(kx, ky, 8.0, 8.0), 2.0, Token::WARNING, desc.z + 2);
        if kf.is_none() {
            marker.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(marker.depth_u16(), 0.0);
        }
        primitives.push(marker);
    }

    let ph_t = (desc.playhead / desc.duration).clamp(0.0, 1.0);
    let ph_x = desc.rect.origin.x + pad + ph_t * (lane_w - 2.0);
    primitives.push(surface(
        Rect::new(
            ph_x,
            desc.rect.origin.y + 2.0,
            2.0,
            (desc.rect.height() - 4.0).max(8.0),
        ),
        1.0,
        Token::PRIMARY,
        desc.z + 3,
    ));

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 4,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 5,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Slider, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

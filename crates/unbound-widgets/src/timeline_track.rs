//! TimelineTrack / DopeSheet 多轨道关键帧与播放头拖拽标尺。

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

/// 单个关键帧标记 `(轨道索引, 时间秒数)`。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// 轨道索引 `0..3`。
    pub track: usize,
    /// 时间（秒）。
    pub time: f32,
}

/// [`TimelineTrackDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineTrackState {
    /// 播放头位置（秒）。
    pub playhead: f32,
    /// 总时长（秒）。
    pub duration: f32,
    /// 关键帧列表。
    pub keyframes: Vec<Keyframe>,
}

impl TimelineTrackState {
    /// 将播放头拖拽到 `t`（夹紧至 `0.0..=duration`）。
    pub fn scrub(&mut self, t: f32) {
        self.playhead = t.clamp(0.0, self.duration.max(0.01));
    }
}

/// [`TimelineTrack`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrackDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 播放头时间（秒）。
    pub playhead: f32,
    /// 总时长（秒）。
    pub duration: f32,
    /// 关键帧标记列表。
    pub keyframes: Vec<Keyframe>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl TimelineTrackDesc {
    /// 创建时间轴轨道描述符。
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

/// 时间轴关键帧标记的确定性子节点 ID（`0..6`）。
#[must_use]
pub const fn timeline_keyframe_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xF00 + index as u64)
}

/// 时间轴播放头拖拽手柄的确定性子节点 ID。
#[must_use]
pub const fn timeline_playhead_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xF10
}

/// 输出结构稳定的摄影表（1 个背景 + 3 条轨道 + 6 个关键帧菱形 + 1 个播放头 + 1 个焦点环 = 12 个图元）。
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

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (k, f) in desc.keyframes.iter().take(MAX_KEYFRAMES).enumerate() {
            let track = f.track.min(MAX_TRACKS - 1);
            let norm_t = (f.time / desc.duration).clamp(0.0, 1.0);
            let kx = desc.rect.origin.x + pad + norm_t * (lane_w - 8.0);
            let ky =
                desc.rect.origin.y + pad + track as f32 * (lane_h + pad) + (lane_h - 8.0) * 0.5;
            let kf_r = Rect::new(kx - 2.0, ky - 2.0, 12.0, 12.0);
            let key = timeline_keyframe_node(node, k);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 2, kf_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, format!("Keyframe {}", k + 1), kf_r).clickable(),
                true,
            );
        }
        let ph_r = Rect::new(
            ph_x - 4.0,
            desc.rect.origin.y + 2.0,
            10.0,
            (desc.rect.height() - 4.0).max(8.0),
        );
        let ph_key = timeline_playhead_node(node);
        out.with_sub_control(
            ph_key,
            HitRegion::new(ph_key, CONTROL_LAYER, u32::from(desc.z) + 3, ph_r)
                .with_cursor(CursorKind::ResizeEastWest),
            A11yNode::new(Role::Slider, "Playhead", ph_r).clickable(),
            true,
        );
    }

    out
}

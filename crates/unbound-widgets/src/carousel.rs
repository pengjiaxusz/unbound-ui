//! 轮播组件（Carousel / Slider）：支持弹簧吸附分页与指示圆点。

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

const MAX_DOTS: usize = 5;

/// 轮播指示圆点的确定性子节点 ID。
#[must_use]
pub const fn carousel_dot_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x400 + index as u64)
}

/// [`CarouselDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CarouselState {
    /// 当前激活的幻灯片索引。
    pub active: usize,
    /// 用于弹簧吸附的归一化幻灯片偏移量。
    pub offset: f32,
}

impl CarouselState {
    /// 向前或向后步进，并在 `count` 范围内循环。
    pub fn step(&mut self, delta: isize, count: usize) {
        if count == 0 {
            return;
        }
        self.active = (self.active as isize + delta).rem_euclid(count as isize) as usize;
        self.offset = self.active as f32;
    }
}

/// [`Carousel`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CarouselDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 幻灯片总数。
    pub slide_count: usize,
    /// 当前激活的幻灯片索引。
    pub active: usize,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl CarouselDesc {
    /// 创建轮播描述符。
    #[must_use]
    pub fn new(rect: Rect, slide_count: usize, active: usize) -> Self {
        let count = slide_count.max(1);
        Self {
            rect,
            slide_count: count,
            active: active % count,
            label: "Carousel".to_owned(),
            z: 600,
        }
    }
}

/// 发射固定形状的轮播组件（1 舞台 + 5 指示圆点 + 1 焦点环 = 7 个图元）。
#[must_use]
pub fn emit_carousel(
    node: u64,
    desc: &CarouselDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let dots = desc.slide_count.min(MAX_DOTS);
    let dot_sz = 8.0;
    let gap = 6.0;
    let total_w = dots as f32 * dot_sz + (dots.saturating_sub(1)) as f32 * gap;
    let start_x = desc.rect.origin.x + (desc.rect.width() - total_w) * 0.5;
    let dot_y = desc.rect.max().y - dot_sz - 8.0;
    for i in 0..MAX_DOTS {
        let visible = i < dots;
        let is_active = i == desc.active.min(dots.saturating_sub(1));
        let mut dot = surface(
            Rect::new(start_x + i as f32 * (dot_sz + gap), dot_y, dot_sz, dot_sz),
            dot_sz * 0.5,
            if is_active {
                Token::PRIMARY
            } else {
                Token::CARD_BORDER
            },
            desc.z + 1,
        );
        if !visible {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for i in 0..dots {
            let dot_r = Rect::new(start_x + i as f32 * (dot_sz + gap), dot_y, dot_sz, dot_sz);
            let key = carousel_dot_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, dot_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, format!("Slide {}", i + 1), dot_r).clickable(),
                true,
            );
        }
    }

    out
}

//! Tour / Spotlight 引导式漫游浮层，带目标镂空与步骤气泡。

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

/// [`TourDesc`] 中的单个引导步骤。
#[derive(Clone, Debug, PartialEq)]
pub struct TourStep {
    /// 需要聚光灯高亮的目标矩形。
    pub target: Rect,
    /// 步骤标题。
    pub title: String,
    /// 步骤说明。
    pub body: String,
}

/// [`TourDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TourState {
    /// 引导漫游是否处于激活状态。
    pub active: bool,
    /// 当前步骤索引。
    pub step: usize,
}

impl TourState {
    /// 前进到下一步；若已在最后一步则关闭引导。
    pub fn next(&mut self, total: usize) {
        if self.step + 1 < total {
            self.step += 1;
        } else {
            self.active = false;
        }
    }
}

/// [`Tour`] 聚光灯浮层的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TourDesc {
    /// 视口边界。
    pub rect: Rect,
    /// 聚光灯目标矩形。
    pub target: Rect,
    /// 步骤标题。
    pub title: String,
    /// 当前步骤序号（从 1 开始）。
    pub step: usize,
    /// 总步骤数。
    pub total_steps: usize,
    /// 基础 z-index。
    pub z: u16,
}

impl TourDesc {
    /// 创建引导漫游描述符。
    #[must_use]
    pub fn new(rect: Rect, target: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            target,
            title: title.into(),
            step: 1,
            total_steps: 3,
            z: 900,
        }
    }
}

/// “上一步”按钮的确定性子节点键。
#[must_use]
pub const fn tour_prev_node(base: u64) -> u64 {
    base ^ 0x544F_5552_0001
}

/// “下一步”按钮的确定性子节点键。
#[must_use]
pub const fn tour_next_node(base: u64) -> u64 {
    base ^ 0x544F_5552_0002
}

/// “跳过”按钮的确定性子节点键。
#[must_use]
pub const fn tour_skip_node(base: u64) -> u64 {
    base ^ 0x544F_5552_0003
}

/// 输出结构稳定的引导聚光灯，四周由 4 块遮罩（上、下、左、右）围合，
/// 使 `desc.target` 保持透明且不被遮挡，并附带独立的上一步/下一步/跳过子命中区域。
#[must_use]
pub fn emit_tour(
    node: u64,
    desc: &TourDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(10 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let r = desc.rect;
    let t = desc.target;
    let scrim_rects = [
        // 顶部遮罩
        Rect::new(
            r.origin.x,
            r.origin.y,
            r.width(),
            (t.origin.y - r.origin.y).max(0.0),
        ),
        // 底部遮罩
        Rect::new(
            r.origin.x,
            t.max().y,
            r.width(),
            (r.max().y - t.max().y).max(0.0),
        ),
        // 左侧遮罩
        Rect::new(
            r.origin.x,
            t.origin.y,
            (t.origin.x - r.origin.x).max(0.0),
            t.height(),
        ),
        // 右侧遮罩
        Rect::new(
            t.max().x,
            t.origin.y,
            (r.max().x - t.max().x).max(0.0),
            t.height(),
        ),
    ];
    for sr in scrim_rects {
        let mut scrim = surface(sr, 0.0, Token::SURFACE_SUNKEN, desc.z);
        scrim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(scrim.depth_u16(), 0.45);
        primitives.push(scrim);
    }

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.target,
        6.0,
        Token::NONE,
        if hovered {
            Token::PRIMARY
        } else {
            Token::FOCUS_RING
        },
        2.0,
        0.0,
        desc.z + 1,
    ));

    let bubble_w = 180.0_f32.min(desc.rect.width() - 16.0).max(80.0);
    let bubble_h = 48.0_f32.min(desc.rect.height() * 0.45).max(28.0);
    let bubble_r = Rect::new(
        (desc.target.origin.x)
            .min(desc.rect.max().x - bubble_w - 8.0)
            .max(desc.rect.origin.x + 8.0),
        (desc.target.max().y + 6.0).min(desc.rect.max().y - bubble_h - 6.0),
        bubble_w,
        bubble_h,
    );
    primitives.push(outlined(
        bubble_r,
        8.0,
        Token::SURFACE_RAISED,
        Token::CARD_BORDER,
        1.0,
        6.0,
        desc.z + 2,
    ));

    let skip_r = Rect::new(bubble_r.origin.x + 8.0, bubble_r.max().y - 20.0, 32.0, 14.0);
    let prev_r = Rect::new(bubble_r.max().x - 80.0, bubble_r.max().y - 20.0, 34.0, 14.0);
    let next_r = Rect::new(bubble_r.max().x - 42.0, bubble_r.max().y - 20.0, 34.0, 14.0);
    primitives.push(surface(skip_r, 4.0, Token::SURFACE_SUNKEN, desc.z + 3));
    primitives.push(surface(prev_r, 4.0, Token::SURFACE_SUNKEN, desc.z + 3));
    primitives.push(surface(next_r, 4.0, Token::PRIMARY, desc.z + 3));

    primitives.push(focus_ring(
        bubble_r,
        8.0,
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
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z + 2), bubble_r)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Dialog, desc.title.clone(), bubble_r).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (sub_key, r_btn, label_str) in [
            (tour_skip_node(node), skip_r, "Skip"),
            (tour_prev_node(node), prev_r, "Prev"),
            (tour_next_node(node), next_r, "Next"),
        ] {
            out.with_sub_control(
                sub_key,
                HitRegion::new(sub_key, CONTROL_LAYER, u32::from(desc.z) + 3, r_btn)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, label_str, r_btn).clickable(),
                true,
            );
        }
    }

    out
}

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

/// 气泡圆角。对齐 WinUI `OverlayCornerRadius = 8`（TeachingTip / Flyout 浮层）。
const TOUR_BUBBLE_RADIUS: f32 = 8.0;
/// 气泡内容内边距。WinUI `TeachingTipContentMargin` 为 12，此处统一到 Fluent 2 浮层的 16px 档。
const TOUR_BUBBLE_PADDING: f32 = 16.0;
/// 主按钮高度。对齐 WinUI 按钮的 `MinHeight = 32`（`DefaultButtonStyle`）。
const TOUR_BUTTON_HEIGHT: f32 = 32.0;
/// 气泡内按钮宽度。
const TOUR_BUTTON_WIDTH: f32 = 56.0;
/// 相邻按钮之间的间距。
const TOUR_BUTTON_GAP: f32 = 8.0;
/// 按钮圆角。对齐 WinUI `ControlCornerRadius = 4`。
const TOUR_BUTTON_RADIUS: f32 = 4.0;
/// 气泡高度：32 高按钮行 + 底部 16 内边距 + 顶部标题行余量。
const TOUR_BUBBLE_HEIGHT: f32 = 72.0;
/// 气泡最小宽度：左右各 16 内边距 + 三枚 56 宽按钮 + 两处 8 间距（216），留少量余量。
const TOUR_BUBBLE_MIN_WIDTH: f32 = 240.0;
/// 气泡与目标之间的间距。
const TOUR_BUBBLE_GAP: f32 = 8.0;

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
    // 目标高亮环取 `ControlCornerRadius = 4` 的圆角档。
    primitives.push(outlined(
        desc.target,
        4.0,
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

    let bubble_w = TOUR_BUBBLE_MIN_WIDTH
        .min(desc.rect.width() - TOUR_BUBBLE_GAP * 2.0)
        .max(216.0);
    let bubble_h = TOUR_BUBBLE_HEIGHT.min((desc.rect.height() - TOUR_BUBBLE_GAP * 2.0).max(48.0));
    let bubble_x = (desc.target.origin.x)
        .min(desc.rect.max().x - bubble_w - TOUR_BUBBLE_GAP)
        .max(desc.rect.origin.x + TOUR_BUBBLE_GAP);
    // 优先放在目标下方；下方放不下就翻到上方；两侧都放不下时退回下方并允许少量溢出，
    // 以免气泡遮住需要保持可见的聚光灯目标。
    let below_y = desc.target.max().y + TOUR_BUBBLE_GAP;
    let above_y = desc.rect.origin.y + TOUR_BUBBLE_GAP;
    let bubble_y = if below_y + bubble_h <= desc.rect.max().y - TOUR_BUBBLE_GAP {
        below_y
    } else if above_y + bubble_h <= desc.target.origin.y - TOUR_BUBBLE_GAP {
        above_y
    } else {
        below_y
    };
    let bubble_r = Rect::new(bubble_x, bubble_y, bubble_w, bubble_h);
    // 气泡圆角 `OverlayCornerRadius = 8`，1px 描边沿用卡片边框档。
    primitives.push(outlined(
        bubble_r,
        TOUR_BUBBLE_RADIUS,
        Token::SURFACE_RAISED,
        Token::CARD_BORDER,
        1.0,
        6.0,
        desc.z + 2,
    ));

    let button_y = bubble_r.max().y - TOUR_BUBBLE_PADDING - TOUR_BUTTON_HEIGHT;
    let skip_r = Rect::new(
        bubble_r.origin.x + TOUR_BUBBLE_PADDING,
        button_y,
        TOUR_BUTTON_WIDTH,
        TOUR_BUTTON_HEIGHT,
    );
    let next_r = Rect::new(
        bubble_r.max().x - TOUR_BUBBLE_PADDING - TOUR_BUTTON_WIDTH,
        button_y,
        TOUR_BUTTON_WIDTH,
        TOUR_BUTTON_HEIGHT,
    );
    let prev_r = Rect::new(
        next_r.origin.x - TOUR_BUTTON_GAP - TOUR_BUTTON_WIDTH,
        button_y,
        TOUR_BUTTON_WIDTH,
        TOUR_BUTTON_HEIGHT,
    );
    // 次级按钮：`ControlFillColorDefault` 填充 + `ControlStrokeColorDefault` 1px 描边。
    primitives.push(outlined(
        skip_r,
        TOUR_BUTTON_RADIUS,
        Token::CONTROL_FILL,
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        desc.z + 3,
    ));
    primitives.push(outlined(
        prev_r,
        TOUR_BUTTON_RADIUS,
        Token::CONTROL_FILL,
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        desc.z + 3,
    ));
    // 主按钮：强调色填充，高度 32。
    primitives.push(surface(
        next_r,
        TOUR_BUTTON_RADIUS,
        Token::PRIMARY,
        desc.z + 3,
    ));

    primitives.push(focus_ring(
        bubble_r,
        TOUR_BUBBLE_RADIUS,
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

//! Popconfirm 锚定式轻量确认气泡。

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

/// 气泡圆角半径。对齐 WinUI `OverlayCornerRadius = 8`（`FlyoutPresenter` 的 `CornerRadius`）。
const POPCONFIRM_RADIUS: f32 = 8.0;
/// 气泡阴影模糊半径，与菜单/Toast 的屏幕级浮层阴影同档。
const POPCONFIRM_SHADOW: f32 = 18.0;
/// 按钮圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
const POPCONFIRM_BUTTON_RADIUS: f32 = 4.0;
/// 按钮高度上限。对齐 WinUI 标准 `Button` 的 `MinHeight = 32`。
const POPCONFIRM_BUTTON_HEIGHT: f32 = 32.0;
/// 两个按钮之间的间距。对齐 WinUI 对话框按钮的 8px 间距。
const POPCONFIRM_BUTTON_GAP: f32 = 8.0;

/// [`Popconfirm`] 锚定气泡的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PopconfirmDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 确认提示标题。
    pub title: String,
    /// 确认按钮文本。
    pub confirm_label: String,
    /// 取消按钮文本。
    pub cancel_label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl PopconfirmDesc {
    /// 创建确认气泡描述符。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            confirm_label: "Confirm".to_owned(),
            cancel_label: "Cancel".to_owned(),
            z: 800,
        }
    }
}

/// 确认气泡取消按钮的确定性子节点 ID。
#[must_use]
pub const fn popconfirm_cancel_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x101
}

/// 确认气泡确认按钮的确定性子节点 ID。
#[must_use]
pub const fn popconfirm_confirm_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x102
}

/// 输出结构稳定的确认气泡（1 张卡片 + 1 个悬停层 + 取消按钮 + 确认按钮 + 1 个焦点环 = 5 个图元）。
#[must_use]
pub fn emit_popconfirm(
    node: u64,
    desc: &PopconfirmDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    // 卡片：`FlyoutPresenter` 的 8px 圆角、1px 浮层描边与屏幕级阴影。
    primitives.push(outlined(
        desc.rect,
        POPCONFIRM_RADIUS,
        Token::SURFACE_RAISED,
        Token::CONTROL_STROKE,
        1.0,
        POPCONFIRM_SHADOW,
        desc.z,
    ));

    // 悬停层：`SubtleFillColorSecondary`；未悬停时靠不透明度隐藏，保持图元数量恒定。
    let mut hover_layer = surface(desc.rect, POPCONFIRM_RADIUS, Token::HOVER, desc.z);
    if !hovered {
        hover_layer.z_and_opacity =
            GpuPrimitiveInstance::pack_z_opacity(hover_layer.depth_u16(), 0.0);
    }
    primitives.push(hover_layer);

    let btn_w = 56.0_f32.min((desc.rect.width() - 24.0) * 0.45);
    let btn_h = POPCONFIRM_BUTTON_HEIGHT.min(desc.rect.height() * 0.4);
    let btn_y = desc.rect.max().y - btn_h - 8.0;
    let confirm_r = Rect::new(desc.rect.max().x - btn_w - 8.0, btn_y, btn_w, btn_h);
    let cancel_r = Rect::new(
        confirm_r.origin.x - btn_w - POPCONFIRM_BUTTON_GAP,
        btn_y,
        btn_w,
        btn_h,
    );

    // 次按钮：`ButtonBackground` = `ControlFillColorDefault`，与控件表面同档。
    primitives.push(outlined(
        cancel_r,
        POPCONFIRM_BUTTON_RADIUS,
        Token::CONTROL_FILL,
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        desc.z + 1,
    ));
    primitives.push(outlined(
        confirm_r,
        POPCONFIRM_BUTTON_RADIUS,
        Token::ERROR,
        Token::ERROR,
        1.0,
        0.0,
        desc.z + 1,
    ));

    primitives.push(focus_ring(
        desc.rect,
        POPCONFIRM_RADIUS,
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
    let a11y = A11yNode::new(Role::Dialog, desc.title.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let cancel_key = popconfirm_cancel_node(node);
        out.with_sub_control(
            cancel_key,
            HitRegion::new(cancel_key, CONTROL_LAYER, u32::from(desc.z) + 1, cancel_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, desc.cancel_label.clone(), cancel_r).clickable(),
            true,
        );
        let confirm_key = popconfirm_confirm_node(node);
        out.with_sub_control(
            confirm_key,
            HitRegion::new(confirm_key, CONTROL_LAYER, u32::from(desc.z) + 1, confirm_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, desc.confirm_label.clone(), confirm_r).clickable(),
            true,
        );
    }

    out
}

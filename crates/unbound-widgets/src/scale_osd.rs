//! 界面缩放浮层胶囊与步进控制器（`ScaleOsd`）。
//!
//! 提供通用的缩放比例状态机（[`ScaleOsdState`]）与基于 [`crate::overlay::OverlayStack`]
//! 挂载的浮层胶囊发射器（[`emit_scale_osd`]），用于响应 `Ctrl+滚轮`、`Ctrl+±/0` 等
//! 界面缩放交互，并在无障碍树中以 `Live::Polite` 播报当前缩放百分比。

use accesskit::{Live, Role};
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::{Token, CANONICAL_SCALE_STEPS};

use crate::overlay::{OverlayEntry, OverlayStack};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认自动隐藏倒计时（毫秒）。
pub const SCALE_OSD_DEFAULT_TIMEOUT_MS: u32 = 1800;

/// 缩放浮层胶囊默认逻辑宽度（像素）。
pub const SCALE_OSD_WIDTH: f32 = 176.0;

/// 缩放浮层胶囊默认逻辑高度（像素）。
pub const SCALE_OSD_HEIGHT: f32 = 36.0;

/// 缩放浮层胶囊距视口边缘的默认安全间距（像素）。
pub const SCALE_OSD_MARGIN: f32 = 16.0;

/// 缩放浮层胶囊内部按钮的确定性子节点 ID（缩小按钮）。
#[must_use]
pub const fn scale_osd_zoom_out_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA101
}

/// 缩放浮层胶囊内部按钮的确定性子节点 ID（重置 100% 按钮）。
#[must_use]
pub const fn scale_osd_reset_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA102
}

/// 缩放浮层胶囊内部按钮的确定性子节点 ID（放大按钮）。
#[must_use]
pub const fn scale_osd_zoom_in_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA103
}

/// 缩放浮层在视口内的锚点位置。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScaleOsdAnchor {
    /// 底部居中（默认）。
    #[default]
    BottomCenter,
    /// 右上角。
    TopRight,
    /// 顶部居中。
    TopCenter,
    /// 右下角。
    BottomRight,
}

/// 界面缩放浮层状态机。
///
/// 维护规范缩放档位、当前缩放比例、可见性倒计时与悬停暂停状态。
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleOsdState {
    scale: f32,
    steps: Vec<f32>,
    visible: bool,
    remaining_ms: u32,
    timeout_ms: u32,
    hovered: bool,
    window_focused: bool,
}

impl Default for ScaleOsdState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScaleOsdState {
    /// 使用规范缩放档位（[`CANONICAL_SCALE_STEPS`]）创建默认状态（100%，初始隐藏）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            scale: 1.0,
            steps: CANONICAL_SCALE_STEPS.to_vec(),
            visible: false,
            remaining_ms: 0,
            timeout_ms: SCALE_OSD_DEFAULT_TIMEOUT_MS,
            hovered: false,
            window_focused: true,
        }
    }

    /// 替换为自定义缩放档位表（自动过滤非正值并升序去重）。
    #[must_use]
    pub fn with_steps(mut self, steps: Vec<f32>) -> Self {
        let mut clean: Vec<f32> = steps
            .into_iter()
            .filter(|v| v.is_finite() && *v > 0.0)
            .collect();
        clean.sort_by(|a, b| a.total_cmp(b));
        clean.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        if !clean.is_empty() {
            self.steps = clean;
            self.scale = self.snap_to_step(self.scale);
        }
        self
    }

    /// 配置自动隐藏超时时间（毫秒）。
    #[must_use]
    pub fn with_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms.max(100);
        self
    }

    /// 当前逻辑缩放比例（例如 `1.0` 表示 100%）。
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// 当前缩放百分比整数（例如 `125` 表示 125%）。
    #[must_use]
    pub fn percentage(&self) -> u32 {
        (self.scale * 100.0).round().max(1.0) as u32
    }

    /// 格式化为人类可读的百分比文本（例如 `"100%"`）。
    #[must_use]
    pub fn formatted_percentage(&self) -> String {
        format!("{}%", self.percentage())
    }

    /// 返回当前生效的缩放档位表。
    #[must_use]
    pub fn steps(&self) -> &[f32] {
        &self.steps
    }

    /// 浮层当前是否可见。
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// 指针是否正悬停在浮层上（悬停期间暂停倒计时）。
    #[must_use]
    pub const fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// 当前剩余的自动隐藏倒计时（毫秒）。
    #[must_use]
    pub const fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }

    /// 是否还能继续放大。
    #[must_use]
    pub fn can_zoom_in(&self) -> bool {
        self.current_step_index() + 1 < self.steps.len()
    }

    /// 是否还能继续缩小。
    #[must_use]
    pub fn can_zoom_out(&self) -> bool {
        self.current_step_index() > 0
    }

    /// 向上步进一档；无论是否到达边界都会唤醒并刷新浮层倒计时。
    ///
    /// 当缩放比例实际发生变化时返回 `true`。
    pub fn zoom_in(&mut self) -> bool {
        let idx = self.current_step_index();
        let next_idx = (idx + 1).min(self.steps.len().saturating_sub(1));
        let next = self.steps[next_idx];
        let changed = (next - self.scale).abs() > 1e-4;
        self.scale = next;
        self.show();
        changed
    }

    /// 向下步进一档；无论是否到达边界都会唤醒并刷新浮层倒计时。
    ///
    /// 当缩放比例实际发生变化时返回 `true`。
    pub fn zoom_out(&mut self) -> bool {
        let idx = self.current_step_index();
        let prev_idx = idx.saturating_sub(1);
        let prev = self.steps[prev_idx];
        let changed = (prev - self.scale).abs() > 1e-4;
        self.scale = prev;
        self.show();
        changed
    }

    /// 重置缩放比例至 100%（若档位表不含 `1.0`，则吸附到最接近 `1.0` 的档位）。
    pub fn reset(&mut self) -> bool {
        self.set_scale(1.0)
    }

    /// 将缩放比例设置为指定值（自动吸附至最近档位），并唤醒浮层倒计时。
    pub fn set_scale(&mut self, target: f32) -> bool {
        if !target.is_finite() || target <= 0.0 {
            return false;
        }
        let snapped = self.snap_to_step(target);
        let changed = (snapped - self.scale).abs() > 1e-4;
        self.scale = snapped;
        self.show();
        changed
    }

    /// 处理 `Ctrl + 滚轮` 缩放手势（`delta_y > 0` 放大，`delta_y < 0` 缩小）。
    pub fn handle_wheel(&mut self, delta_y: f32, ctrl: bool) -> bool {
        if !ctrl || delta_y.abs() < f32::EPSILON {
            return false;
        }
        if delta_y > 0.0 {
            self.zoom_in()
        } else {
            self.zoom_out()
        }
    }

    /// 更新指针悬停状态：悬停时暂停自动隐藏倒计时，移出后恢复倒计时。
    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    /// 窗口失焦时取消悬停挂起并恢复倒计时，避免浮层永久滞留。
    pub fn on_window_blur(&mut self) {
        self.window_focused = false;
        self.hovered = false;
        if self.visible && self.remaining_ms == 0 {
            self.remaining_ms = self.timeout_ms;
        }
    }

    /// 窗口重新获得焦点。
    pub fn on_window_focus(&mut self) {
        self.window_focused = true;
    }

    /// 推进可见性倒计时 `dt_ms` 毫秒；若本次推进导致浮层由可见变为隐藏则返回 `true`。
    pub fn tick(&mut self, dt_ms: u32) -> bool {
        if !self.visible {
            return false;
        }
        if self.hovered && self.window_focused {
            return false;
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(dt_ms);
        if self.remaining_ms == 0 {
            self.visible = false;
            return true;
        }
        false
    }

    /// 立即隐藏浮层并清零倒计时。
    pub fn hide(&mut self) {
        self.visible = false;
        self.remaining_ms = 0;
        self.hovered = false;
    }

    fn show(&mut self) {
        self.visible = true;
        self.remaining_ms = self.timeout_ms;
    }

    fn current_step_index(&self) -> usize {
        self.steps
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - self.scale).abs().total_cmp(&(*b - self.scale).abs()))
            .map_or(0, |(idx, _)| idx)
    }

    fn snap_to_step(&self, value: f32) -> f32 {
        self.steps
            .iter()
            .copied()
            .min_by(|a, b| (*a - value).abs().total_cmp(&(*b - value).abs()))
            .unwrap_or(1.0)
    }
}

/// 界面缩放浮层胶囊描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleOsdDesc {
    /// 宿主视口矩形。
    pub viewport: Rect,
    /// 当前逻辑缩放比例。
    pub scale: f32,
    /// 浮层当前是否可见（不可见时仍保持恒定图元数，但透明且不产生命中区域）。
    pub visible: bool,
    /// 浮层锚点位置。
    pub anchor: ScaleOsdAnchor,
    /// 浮层与视口边缘的间距。
    pub margin: f32,
    /// 胶囊目标宽度（当 `ignore_ui_scale` 为 `true` 时保持物理可读尺寸稳定）。
    pub width: f32,
    /// 胶囊目标高度。
    pub height: f32,
    /// 是否忽略全局 UI 缩放（默认 `true`，避免在高/低缩放下自身过大或过小）。
    pub ignore_ui_scale: bool,
    /// 浮层绘制层级。
    pub z: u16,
}

impl ScaleOsdDesc {
    /// 创建默认位于底部居中的缩放浮层描述符。
    #[must_use]
    pub fn new(viewport: Rect, scale: f32) -> Self {
        Self {
            viewport,
            scale,
            visible: true,
            anchor: ScaleOsdAnchor::BottomCenter,
            margin: SCALE_OSD_MARGIN,
            width: SCALE_OSD_WIDTH,
            height: SCALE_OSD_HEIGHT,
            ignore_ui_scale: true,
            z: 910,
        }
    }

    /// 直接从 [`ScaleOsdState`] 构造描述符。
    #[must_use]
    pub fn from_state(viewport: Rect, state: &ScaleOsdState) -> Self {
        Self::new(viewport, state.scale()).with_visible(state.is_visible())
    }

    /// 设置锚点位置。
    #[must_use]
    pub const fn with_anchor(mut self, anchor: ScaleOsdAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// 设置可见性。
    #[must_use]
    pub const fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// 设置是否忽略 UI 缩放。
    #[must_use]
    pub const fn with_ignore_ui_scale(mut self, ignore: bool) -> Self {
        self.ignore_ui_scale = ignore;
        self
    }

    /// 设置视口边距。
    #[must_use]
    pub const fn with_margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    /// 设置胶囊尺寸。
    #[must_use]
    pub const fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// 设置绘制 z-index。
    #[must_use]
    pub const fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 计算胶囊在视口内的实际矩形（自动钳制至视口安全区域内）。
    #[must_use]
    pub fn pill_rect(&self) -> Rect {
        let factor = if self.ignore_ui_scale && self.scale > 0.0 {
            1.0 / self.scale
        } else {
            1.0
        };
        let max_w = (self.viewport.width() - self.margin * 2.0).max(24.0);
        let max_h = (self.viewport.height() - self.margin * 2.0).max(24.0);
        let w = (self.width * factor).clamp(96.0_f32.min(max_w), max_w);
        let h = (self.height * factor).clamp(28.0_f32.min(max_h), max_h);
        let x = match self.anchor {
            ScaleOsdAnchor::BottomCenter | ScaleOsdAnchor::TopCenter => {
                self.viewport.origin.x + (self.viewport.width() - w) * 0.5
            }
            ScaleOsdAnchor::TopRight | ScaleOsdAnchor::BottomRight => {
                self.viewport.max().x - self.margin - w
            }
        };
        let y = match self.anchor {
            ScaleOsdAnchor::BottomCenter | ScaleOsdAnchor::BottomRight => {
                self.viewport.max().y - self.margin - h
            }
            ScaleOsdAnchor::TopCenter | ScaleOsdAnchor::TopRight => {
                self.viewport.origin.y + self.margin
            }
        };
        Rect::new(x, y, w, h)
    }

    /// 缩小（`-`）按钮矩形。
    #[must_use]
    pub fn zoom_out_rect(&self) -> Rect {
        let r = self.pill_rect();
        let btn = (r.height() - 8.0).max(20.0);
        Rect::new(r.origin.x + 4.0, r.origin.y + (r.height() - btn) * 0.5, btn, btn)
    }

    /// 放大（`+`）按钮矩形。
    #[must_use]
    pub fn zoom_in_rect(&self) -> Rect {
        let r = self.pill_rect();
        let btn = (r.height() - 8.0).max(20.0);
        Rect::new(
            r.max().x - btn - 4.0,
            r.origin.y + (r.height() - btn) * 0.5,
            btn,
            btn,
        )
    }

    /// 中央百分比重置按钮矩形。
    #[must_use]
    pub fn reset_rect(&self) -> Rect {
        let r = self.pill_rect();
        let left = self.zoom_out_rect().max().x + 4.0;
        let right = self.zoom_in_rect().origin.x - 4.0;
        let h = (r.height() - 8.0).max(20.0);
        Rect::new(left, r.origin.y + (r.height() - h) * 0.5, (right - left).max(24.0), h)
    }

    /// 将当前缩放浮层挂载到 [`OverlayStack`]（不可见时自动卸载并返回 `None`）。
    pub fn mount_in_overlay(
        &self,
        stack: &mut OverlayStack,
        node: u64,
        anchor: u64,
    ) -> Option<u64> {
        if !self.visible {
            stack.close(node);
            return None;
        }
        let entry = OverlayEntry::tooltip(node, anchor, self.pill_rect());
        Some(stack.push(entry))
    }

    /// 从 [`OverlayStack`] 中卸载指定缩放浮层节点。
    pub fn unmount_from_overlay(stack: &mut OverlayStack, node: u64) -> bool {
        stack.close(node)
    }
}

/// 发射缩放浮层胶囊（保持 24 态恒定图元数：胶囊底板 + 缩小按钮 + 重置区 + 放大按钮 + 焦点环 = 5 个固定图元）。
#[must_use]
pub fn emit_scale_osd(
    node: u64,
    desc: &ScaleOsdDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rect = desc.pill_rect();
    let z = desc.z;
    let alpha = if desc.visible { 1.0 } else { 0.0 };
    let pill_radius = rect.height() * 0.5;

    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let border = if !state.disabled && state.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };

    let mut bg = outlined(rect, pill_radius, Token::OVERLAY_BG, border, 1.0, 12.0, z);
    bg.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, alpha);
    primitives.push(bg);

    let btn_fill = if state.disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.phase == PointerPhase::Pressed {
        Token::PRESSED
    } else if state.phase == PointerPhase::Hovered {
        Token::HOVER
    } else {
        Token::CONTROL_FILL
    };

    let out_r = desc.zoom_out_rect();
    let mut out_btn = outlined(out_r, 4.0, btn_fill, Token::CONTROL_STROKE, 1.0, 0.0, z + 1);
    out_btn.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, alpha);
    primitives.push(out_btn);

    let reset_r = desc.reset_rect();
    let mut reset_btn = surface(reset_r, 4.0, Token::SURFACE_RAISED, z + 1);
    reset_btn.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, alpha);
    primitives.push(reset_btn);

    let in_r = desc.zoom_in_rect();
    let mut in_btn = outlined(in_r, 4.0, btn_fill, Token::CONTROL_STROKE, 1.0, 0.0, z + 1);
    in_btn.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, alpha);
    primitives.push(in_btn);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        pill_radius + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        state.focused && !state.disabled && desc.visible,
    ));

    if desc.visible && !label_glyphs.is_empty() {
        let text_token = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        emit_glyphs(label_glyphs, text_token, z + 3, &mut primitives, &mut payloads);
    }

    let pct_text = format!("{}%", (desc.scale * 100.0).round().max(1.0) as u32);
    let a11y = A11yNode::new(Role::Status, "Interface Scale", rect)
        .with_value(pct_text)
        .with_live(Live::Polite)
        .disabled(state.disabled);

    let hit = (desc.visible && !state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [pill_radius; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: desc.visible && !state.disabled,
        ..Default::default()
    };

    if desc.visible && !state.disabled {
        let out_id = scale_osd_zoom_out_node(node);
        out.with_sub_control(
            out_id,
            HitRegion::new(out_id, CONTROL_LAYER, u32::from(z) + 1, out_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Zoom Out", out_r).clickable(),
            true,
        );
        let reset_id = scale_osd_reset_node(node);
        out.with_sub_control(
            reset_id,
            HitRegion::new(reset_id, CONTROL_LAYER, u32::from(z) + 1, reset_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Reset Zoom", reset_r).clickable(),
            true,
        );
        let in_id = scale_osd_zoom_in_node(node);
        out.with_sub_control(
            in_id,
            HitRegion::new(in_id, CONTROL_LAYER, u32::from(z) + 1, in_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Zoom In", in_r).clickable(),
            true,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn scale_osd_steps_boundaries_and_reset() {
        let mut state = ScaleOsdState::new();
        assert_eq!(state.scale(), 1.0);
        assert_eq!(state.formatted_percentage(), "100%");
        assert!(!state.is_visible());

        assert!(state.zoom_in());
        assert!((state.scale() - 1.125).abs() < 1e-4);
        assert!(state.is_visible());
        assert_eq!(state.remaining_ms(), SCALE_OSD_DEFAULT_TIMEOUT_MS);

        // 连续放大直至上限（2.0）。
        while state.can_zoom_in() {
            assert!(state.zoom_in());
        }
        assert!((state.scale() - 2.0).abs() < 1e-4);
        assert!(!state.zoom_in());
        assert!(state.is_visible());

        // 连续缩小直至下限（0.75）。
        while state.can_zoom_out() {
            assert!(state.zoom_out());
        }
        assert!((state.scale() - 0.75).abs() < 1e-4);
        assert!(!state.zoom_out());

        // 重置回 100%。
        assert!(state.reset());
        assert!((state.scale() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn scale_osd_auto_hide_hover_pause_and_blur_resume() {
        let mut state = ScaleOsdState::new().with_timeout_ms(1800);
        state.zoom_in();
        assert!(state.is_visible());

        // 推进 1000ms，尚未隐藏。
        assert!(!state.tick(1000));
        assert!(state.is_visible());
        assert_eq!(state.remaining_ms(), 800);

        // 悬停时暂停倒计时。
        state.set_hovered(true);
        assert!(!state.tick(5000));
        assert!(state.is_visible());
        assert_eq!(state.remaining_ms(), 800);

        // 窗口失焦时取消悬停挂起并恢复倒计时。
        state.on_window_blur();
        assert!(!state.is_hovered());
        assert!(!state.tick(500));
        assert_eq!(state.remaining_ms(), 300);
        assert!(state.tick(300));
        assert!(!state.is_visible());
    }

    #[test]
    fn scale_osd_overlay_mount_and_a11y_live_polite() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut state = ScaleOsdState::new();
        state.set_scale(1.25);

        let desc = ScaleOsdDesc::from_state(viewport, &state).with_anchor(ScaleOsdAnchor::TopRight);
        let mut stack = OverlayStack::new();
        assert!(desc.mount_in_overlay(&mut stack, 42, 1).is_some());
        assert!(stack.contains(42));

        let out = emit_scale_osd(42, &desc, ControlState::default(), &[]);
        let a11y = out.a11y.as_ref().expect("ScaleOsd must emit A11yNode");
        assert_eq!(a11y.role, Role::Status);
        assert_eq!(a11y.value, "125%");
        assert_eq!(a11y.live, Some(Live::Polite));
        let ak_node = a11y.to_accesskit(&[]);
        assert_eq!(ak_node.live(), Some(Live::Polite));

        assert!(ScaleOsdDesc::unmount_from_overlay(&mut stack, 42));
        assert!(!stack.contains(42));
    }

    #[test]
    fn scale_osd_stable_shape_and_headless_geometry() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let desc = ScaleOsdDesc::new(viewport, 1.0);
        let base_len = emit_scale_osd(1, &desc, ControlState::default(), &[])
            .primitives
            .len();
        for phase in [PointerPhase::Idle, PointerPhase::Hovered, PointerPhase::Pressed] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let st = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        assert_eq!(
                            emit_scale_osd(1, &desc, st, &[]).primitives.len(),
                            base_len
                        );
                    }
                }
            }
        }

        // 无头几何健康检查。
        let pill = desc.pill_rect();
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            pill,
            "ScaleOsd",
        );
        frame.close();
        frame.close();
        let report = Harness::assert_clean(&frame.build());
        assert!(report.is_clean(), "ScaleOsd must pass headless geometry checks");
    }
}

//! 一键复制按钮（`CopyButton`）与只读可选中复制输入框（`ReadOnlyInput`）。
//!
//! 覆盖路径、哈希、命令行、诊断信息等高频只读复制场景：
//! - [`CopyButtonState`] / [`CopyButtonDesc`] / [`emit_copy_button`]：点击后触发复制并进入
//!   默认 1500ms 的瞬态成功反馈（切换勾选图标与 `"Copied"` 提示，到期自动复原）。
//! - [`ReadOnlyInputState`] / [`ReadOnlyInputDesc`] / [`emit_read_only_input`]：明确区分
//!   `read_only` 与 `disabled`，只读态依然支持聚焦、全选/区间选中与一键复制，支持等宽字体变体、
//!   尾部内嵌复制按钮插槽以及配合 [`crate::elided_text::ElideMode`] 的长文本省略展示。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::elided_text::{elide_text_with_measure, ElideMode, ElideResult};
use crate::tooltip::TooltipDesc;
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{TextEditState, TEXT_INPUT_RADIUS};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 复制成功瞬态反馈的默认持续时长（毫秒）。
pub const COPY_FEEDBACK_DURATION_MS: u32 = 1500;

/// 控件标准圆角半径（对齐 WinUI `ControlCornerRadius = 4`）。
const COPY_CONTROL_RADIUS: f32 = 4.0;

/// 只读输入框尾部内嵌复制按钮的确定性子节点 ID。
#[must_use]
pub const fn read_only_copy_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA201
}

/// 一键复制按钮状态机。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyButtonState {
    /// 是否正处于“已复制”瞬态成功反馈期。
    copied: bool,
    /// 剩余反馈倒计时（毫秒）。
    remaining_ms: u32,
    /// 反馈总时长（毫秒）。
    feedback_duration_ms: u32,
    /// 累计成功触发复制的次数。
    copy_count: u64,
}

impl Default for CopyButtonState {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyButtonState {
    /// 创建默认复制按钮状态（反馈时长 1500ms）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            copied: false,
            remaining_ms: 0,
            feedback_duration_ms: COPY_FEEDBACK_DURATION_MS,
            copy_count: 0,
        }
    }

    /// 配置复制成功反馈的持续时长（毫秒）。
    #[must_use]
    pub fn with_feedback_duration_ms(mut self, duration_ms: u32) -> Self {
        self.feedback_duration_ms = duration_ms.max(100);
        self
    }

    /// 当前是否处于瞬态已复制状态。
    #[must_use]
    pub const fn is_copied(&self) -> bool {
        self.copied
    }

    /// 当前剩余的反馈倒计时（毫秒）。
    #[must_use]
    pub const fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }

    /// 累计复制次数。
    #[must_use]
    pub const fn copy_count(&self) -> u64 {
        self.copy_count
    }

    /// 触发复制并调用剪贴板写入回调，进入瞬态成功态。
    pub fn trigger_copy<F>(&mut self, payload: &str, on_copy: F)
    where
        F: FnOnce(&str),
    {
        on_copy(payload);
        self.mark_copied();
    }

    /// 直接标记为已复制并重置倒计时。
    pub fn mark_copied(&mut self) {
        self.copied = true;
        self.remaining_ms = self.feedback_duration_ms;
        self.copy_count += 1;
    }

    /// 推进瞬态倒计时 `dt_ms` 毫秒；若本次推进使状态从已复制复原为常态则返回 `true`。
    pub fn tick(&mut self, dt_ms: u32) -> bool {
        if !self.copied {
            return false;
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(dt_ms);
        if self.remaining_ms == 0 {
            self.copied = false;
            return true;
        }
        false
    }

    /// 立即复原为未复制常态。
    pub fn reset(&mut self) {
        self.copied = false;
        self.remaining_ms = 0;
    }

    /// 返回当前按钮的英文提示文案（`"Copied"` 或 `"Copy"`）。
    #[must_use]
    pub const fn status_label(&self) -> &'static str {
        if self.copied {
            "Copied"
        } else {
            "Copy"
        }
    }
}

/// 一键复制按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CopyButtonDesc {
    /// 按钮边界矩形。
    pub rect: Rect,
    /// 当前是否处于已复制反馈态。
    pub copied: bool,
    /// 常态无障碍与提示文案（默认 `"Copy"`）。
    pub label: String,
    /// 成功态无障碍与提示文案（默认 `"Copied"`）。
    pub copied_label: String,
    /// 绘制 z-index。
    pub z: u16,
}

impl CopyButtonDesc {
    /// 创建复制按钮描述符。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            copied: false,
            label: "Copy".to_owned(),
            copied_label: "Copied".to_owned(),
            z: 20,
        }
    }

    /// 从状态机同步构造描述符。
    #[must_use]
    pub fn from_state(rect: Rect, state: &CopyButtonState) -> Self {
        Self::new(rect).with_copied(state.is_copied())
    }

    /// 设置已复制反馈态。
    #[must_use]
    pub const fn with_copied(mut self, copied: bool) -> Self {
        self.copied = copied;
        self
    }

    /// 设置常态与成功态标签。
    #[must_use]
    pub fn with_labels(
        mut self,
        label: impl Into<String>,
        copied_label: impl Into<String>,
    ) -> Self {
        self.label = label.into();
        self.copied_label = copied_label.into();
        self
    }
}

/// 发射一键复制按钮（24 态恒定图元数：背景 + 2 条图标线段 + 焦点环 = 4 个固定图元）。
#[must_use]
pub fn emit_copy_button(
    node: u64,
    desc: &CopyButtonDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.rect;
    let z = desc.z;
    let mut primitives = Vec::with_capacity(4 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(2 + label_glyphs.len());

    let (fill, border, icon_color) = if state.disabled {
        (
            Token::CONTROL_DISABLED_BG,
            Token::CONTROL_DISABLED_BORDER,
            Token::TEXT_DISABLED,
        )
    } else if desc.copied {
        (Token::SUCCESS_SOFT, Token::SUCCESS, Token::SUCCESS)
    } else if state.phase == PointerPhase::Pressed {
        (Token::PRESSED, Token::CONTROL_STROKE, Token::TEXT_PRIMARY)
    } else if state.phase == PointerPhase::Hovered {
        (Token::HOVER, Token::INPUT_BORDER_HOVER, Token::TEXT_PRIMARY)
    } else {
        (Token::CONTROL_FILL, Token::CONTROL_STROKE, Token::TEXT_PRIMARY)
    };

    primitives.push(outlined(
        r,
        COPY_CONTROL_RADIUS,
        fill,
        border,
        1.0,
        0.0,
        z,
    ));

    // 图标区域：无论是否已复制，均固定发射 2 条线段图元，确保 24 态与反馈态图元数恒定。
    let cx = if label_glyphs.is_empty() {
        r.origin.x + r.width() * 0.5
    } else {
        r.origin.x + 14.0
    };
    let cy = r.origin.y + r.height() * 0.5;
    if desc.copied {
        // 勾选图标（两段折线）。
        curve_stroke(
            [cx - 4.0, cy],
            [cx - 1.0, cy + 3.0],
            1.6,
            icon_color,
            z + 1,
            &mut primitives,
            &mut payloads,
        );
        curve_stroke(
            [cx - 1.0, cy + 3.0],
            [cx + 4.5, cy - 3.0],
            1.6,
            icon_color,
            z + 1,
            &mut primitives,
            &mut payloads,
        );
    } else {
        // 剪贴板/双页图标简化双线段（与勾选态保持相同的 2 个 Curve 图元计数）。
        curve_stroke(
            [cx - 3.5, cy - 3.5],
            [cx + 3.5, cy - 3.5],
            1.5,
            icon_color,
            z + 1,
            &mut primitives,
            &mut payloads,
        );
        curve_stroke(
            [cx - 3.5, cy + 2.5],
            [cx + 3.5, cy + 2.5],
            1.5,
            icon_color,
            z + 1,
            &mut primitives,
            &mut payloads,
        );
    }

    primitives.push(focus_ring(
        r.inflate(3.0),
        COPY_CONTROL_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        state.focused && !state.disabled,
    ));

    if !label_glyphs.is_empty() {
        emit_glyphs(label_glyphs, icon_color, z + 3, &mut primitives, &mut payloads);
    }

    let active_label = if desc.copied {
        desc.copied_label.clone()
    } else {
        desc.label.clone()
    };
    let a11y = A11yNode::new(Role::Button, active_label, r)
        .clickable()
        .disabled(state.disabled);
    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [COPY_CONTROL_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

/// 只读可选中复制输入框状态机（支持选区复制与内嵌复制按钮反馈，禁止内容写入）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReadOnlyInputState {
    edit: TextEditState,
    copy_button: CopyButtonState,
}

impl ReadOnlyInputState {
    /// 创建带只读文本内容的状态机。
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            edit: TextEditState::with_text(value),
            copy_button: CopyButtonState::new(),
        }
    }

    /// 返回当前只读文本值。
    #[must_use]
    pub fn value(&self) -> &str {
        self.edit.text()
    }

    /// 更新只读展示值（来自外部数据源，非用户输入）。
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.edit.set_text(value);
    }

    /// 全选只读文本以便复制。
    pub fn select_all(&mut self) {
        self.edit.select_all();
    }

    /// 设置光标/选区字节位置（支持拖拽或 Shift+方向键选择）。
    pub fn set_selection(&mut self, start: usize, end: usize) {
        self.edit.set_cursor(start, false);
        self.edit.set_cursor(end, true);
    }

    /// 返回当前选中的文本片段（若无选区则返回 `None`）。
    #[must_use]
    pub fn selected_text(&self) -> Option<&str> {
        self.edit.selected_text()
    }

    /// 复制当前选区（若无选区则复制全文）并触发内嵌复制按钮的瞬态反馈。
    pub fn copy_selection_or_all<F>(&mut self, on_copy: F) -> String
    where
        F: FnOnce(&str),
    {
        let payload = self
            .edit
            .selected_text()
            .unwrap_or_else(|| self.edit.text())
            .to_owned();
        self.copy_button.trigger_copy(&payload, on_copy);
        payload
    }

    /// 访问内嵌复制按钮状态。
    #[must_use]
    pub const fn copy_button(&self) -> &CopyButtonState {
        &self.copy_button
    }

    /// 推进复制反馈倒计时。
    pub fn tick(&mut self, dt_ms: u32) -> bool {
        self.copy_button.tick(dt_ms)
    }
}

/// 只读可选中复制输入框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ReadOnlyInputDesc {
    /// 控件边界矩形。
    pub rect: Rect,
    /// 只读文本内容。
    pub value: String,
    /// 无障碍标签。
    pub label: String,
    /// 是否使用等宽字体样式变体（如哈希、路径、命令行）。
    pub monospace: bool,
    /// 是否在尾部内嵌一键复制按钮插槽。
    pub show_copy_button: bool,
    /// 内嵌复制按钮是否处于已复制反馈态。
    pub copied: bool,
    /// 可选的省略模式（与 [`crate::elided_text`] 配合处理超长路径/哈希）。
    pub elide_mode: Option<ElideMode>,
    /// 绘制 z-index。
    pub z: u16,
}

impl ReadOnlyInputDesc {
    /// 创建默认带内嵌复制按钮的只读输入框描述符。
    #[must_use]
    pub fn new(rect: Rect, value: impl Into<String>) -> Self {
        Self {
            rect,
            value: value.into(),
            label: "Read-only Value".to_owned(),
            monospace: false,
            show_copy_button: true,
            copied: false,
            elide_mode: None,
            z: 20,
        }
    }

    /// 从状态机同步构造描述符。
    #[must_use]
    pub fn from_state(rect: Rect, state: &ReadOnlyInputState) -> Self {
        Self::new(rect, state.value()).with_copied(state.copy_button().is_copied())
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 启用等宽字体变体。
    #[must_use]
    pub const fn with_monospace(mut self, monospace: bool) -> Self {
        self.monospace = monospace;
        self
    }

    /// 设置是否显示尾部复制按钮。
    #[must_use]
    pub const fn with_copy_button(mut self, show: bool) -> Self {
        self.show_copy_button = show;
        self
    }

    /// 设置已复制反馈态。
    #[must_use]
    pub const fn with_copied(mut self, copied: bool) -> Self {
        self.copied = copied;
        self
    }

    /// 配置长文本省略模式。
    #[must_use]
    pub const fn with_elide_mode(mut self, mode: Option<ElideMode>) -> Self {
        self.elide_mode = mode;
        self
    }

    /// 结合 `elide_mode` 与可用文本宽度计算显示文本（复用 `elided_text` 省略引擎）。
    #[must_use]
    pub fn resolved_display_text<F>(&self, measure: F) -> ElideResult
    where
        F: FnMut(&str) -> f32,
    {
        match self.elide_mode {
            Some(mode) => elide_text_with_measure(&self.value, self.text_rect().width(), mode, measure),
            None => ElideResult {
                display: self.value.clone(),
                was_elided: false,
            },
        }
    }

    /// 当长文本实际发生省略时生成条件溢出 Tooltip。
    #[must_use]
    pub fn overflow_tooltip(&self, viewport: Rect, was_elided: bool) -> Option<TooltipDesc> {
        if !was_elided || self.value.is_empty() {
            return None;
        }
        Some(TooltipDesc::labeled(self.rect, viewport, self.value.clone()))
    }

    /// 返回尾部内嵌复制按钮矩形。
    #[must_use]
    pub fn copy_button_rect(&self) -> Rect {
        let size = (self.rect.height() - 6.0).clamp(20.0, 28.0);
        Rect::new(
            self.rect.max().x - size - 3.0,
            self.rect.origin.y + (self.rect.height() - size) * 0.5,
            size,
            size,
        )
    }

    /// 返回文本内容可用区域矩形（已扣除内边距与右侧复制按钮槽位）。
    #[must_use]
    pub fn text_rect(&self) -> Rect {
        let right_pad = if self.show_copy_button {
            self.copy_button_rect().width() + 10.0
        } else {
            10.0
        };
        Rect::new(
            self.rect.origin.x + 10.0,
            self.rect.origin.y + 4.0,
            (self.rect.width() - 10.0 - right_pad).max(16.0),
            (self.rect.height() - 8.0).max(14.0),
        )
    }
}

/// 发射只读可选中复制输入框（24 态恒定图元数：背景 + 内嵌复制按钮底板 + 底部聚焦线 + 焦点环 = 4 个固定图元）。
#[must_use]
pub fn emit_read_only_input(
    node: u64,
    desc: &ReadOnlyInputDesc,
    state: ControlState,
    text_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.rect;
    let z = desc.z;
    let mut primitives = Vec::with_capacity(6 + text_glyphs.len());
    let mut payloads = Vec::with_capacity(2 + text_glyphs.len());

    // 只读态与禁用态明确区分：只读态使用沉降/代码底板但保持可聚焦与清晰文本对比度。
    let (bg_fill, border) = if state.disabled {
        (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
    } else {
        let fill = if desc.monospace {
            Token::CODE_BG
        } else {
            Token::SURFACE_SUNKEN
        };
        let bdr = if state.focused {
            Token::PRIMARY
        } else if state.phase == PointerPhase::Hovered {
            Token::INPUT_BORDER_HOVER
        } else {
            Token::CONTROL_STROKE
        };
        (fill, bdr)
    };
    primitives.push(outlined(r, TEXT_INPUT_RADIUS, bg_fill, border, 1.0, 0.0, z));

    // 尾部内嵌复制按钮底板（隐藏时透明以维持 24 态恒定图元数）。
    let copy_r = desc.copy_button_rect();
    let copy_fill = if state.disabled {
        Token::CONTROL_DISABLED_BG
    } else if desc.copied {
        Token::SUCCESS_SOFT
    } else if state.phase == PointerPhase::Hovered {
        Token::HOVER
    } else {
        Token::CONTROL_FILL
    };
    let copy_border = if desc.copied {
        Token::SUCCESS
    } else {
        Token::CONTROL_STROKE
    };
    let mut copy_prim = outlined(copy_r, COPY_CONTROL_RADIUS, copy_fill, copy_border, 1.0, 0.0, z + 1);
    let copy_alpha = if desc.show_copy_button { 1.0 } else { 0.0 };
    copy_prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, copy_alpha);
    primitives.push(copy_prim);

    // 内嵌复制按钮图标（固定 2 条线段：常态剪贴板图标 / 成功态勾选图标）。
    let copy_icon_tok = if state.disabled {
        Token::TEXT_DISABLED
    } else if desc.copied {
        Token::SUCCESS
    } else {
        Token::TEXT_PRIMARY
    };
    let icx = copy_r.origin.x + copy_r.width() * 0.5;
    let icy = copy_r.origin.y + copy_r.height() * 0.5;
    let icon_start = primitives.len();
    if desc.copied {
        curve_stroke([icx - 3.5, icy], [icx - 1.0, icy + 2.5], 1.5, copy_icon_tok, z + 2, &mut primitives, &mut payloads);
        curve_stroke([icx - 1.0, icy + 2.5], [icx + 4.0, icy - 2.5], 1.5, copy_icon_tok, z + 2, &mut primitives, &mut payloads);
    } else {
        curve_stroke([icx - 3.0, icy - 2.5], [icx + 3.0, icy - 2.5], 1.4, copy_icon_tok, z + 2, &mut primitives, &mut payloads);
        curve_stroke([icx - 3.0, icy + 2.5], [icx + 3.0, icy + 2.5], 1.4, copy_icon_tok, z + 2, &mut primitives, &mut payloads);
    }
    if !desc.show_copy_button {
        for p in &mut primitives[icon_start..] {
            p.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 2, 0.0);
        }
    }

    // 聚焦底部强调线（2px PRIMARY，未聚焦时透明保持固定形状）。
    let mut underline = surface(
        Rect::new(r.origin.x + 1.0, r.max().y - 2.0, (r.width() - 2.0).max(0.0), 2.0),
        0.0,
        Token::PRIMARY,
        z + 1,
    );
    underline.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z + 1,
        if state.focused && !state.disabled { 1.0 } else { 0.0 },
    );
    primitives.push(underline);

    primitives.push(focus_ring(
        r.inflate(3.0),
        TEXT_INPUT_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        state.focused && !state.disabled,
    ));

    if !text_glyphs.is_empty() {
        let text_tok = if state.disabled {
            Token::TEXT_DISABLED
        } else if desc.monospace {
            Token::CODE_TEXT
        } else {
            Token::TEXT_PRIMARY
        };
        emit_glyphs(text_glyphs, text_tok, z + 3, &mut primitives, &mut payloads);
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [TEXT_INPUT_RADIUS; 4],
            })
            .with_cursor(CursorKind::Text)
    });

    // 明确设置 `read_only(true)` 且在未禁用时保持 `focusable = true`。
    let a11y = A11yNode::new(Role::TextInput, desc.label.clone(), r)
        .with_value(desc.value.clone())
        .read_only(true)
        .disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if desc.show_copy_button && !state.disabled {
        let copy_id = read_only_copy_node(node);
        let btn_label = if desc.copied { "Copied" } else { "Copy" };
        out.with_sub_control(
            copy_id,
            HitRegion::new(copy_id, CONTROL_LAYER, u32::from(z) + 1, copy_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, btn_label, copy_r).clickable(),
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
    fn copy_button_transient_feedback_and_reset() {
        let mut st = CopyButtonState::new();
        assert!(!st.is_copied());
        assert_eq!(st.status_label(), "Copy");

        let mut clipboard = String::new();
        st.trigger_copy("sha256:abcdef", |text| clipboard = text.to_owned());
        assert_eq!(clipboard, "sha256:abcdef");
        assert!(st.is_copied());
        assert_eq!(st.status_label(), "Copied");
        assert_eq!(st.copy_count(), 1);

        assert!(!st.tick(1000));
        assert!(st.is_copied());
        assert!(st.tick(500));
        assert!(!st.is_copied());
        assert_eq!(st.status_label(), "Copy");
    }

    #[test]
    fn read_only_input_selection_copy_and_a11y_distinction() {
        let mut st = ReadOnlyInputState::new("vulkan-capture --frame=42");
        st.set_selection(0, 14);
        assert_eq!(st.selected_text(), Some("vulkan-capture"));

        let mut copied = String::new();
        let res = st.copy_selection_or_all(|s| copied = s.to_owned());
        assert_eq!(res, "vulkan-capture");
        assert_eq!(copied, "vulkan-capture");
        assert!(st.copy_button().is_copied());

        let rect = Rect::new(24.0, 24.0, 320.0, 32.0);
        let desc = ReadOnlyInputDesc::from_state(rect, &st).with_monospace(true);
        let out = emit_read_only_input(10, &desc, ControlState::default(), &[]);
        // 只读态仍可聚焦，与禁用态严格区分。
        assert!(out.focusable);
        let a11y = out.a11y.as_ref().expect("ReadOnlyInput must emit A11yNode");
        assert!(a11y.read_only);
        assert!(!a11y.disabled);
        assert!(a11y.to_accesskit(&[]).is_read_only());

        // 无头几何健康检查。
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "ReadOnlyInput",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }

    #[test]
    fn copy_button_and_read_only_input_stable_shape() {
        let btn_desc = CopyButtonDesc::new(Rect::new(10.0, 10.0, 80.0, 32.0));
        let btn_base = emit_copy_button(1, &btn_desc, ControlState::default(), &[])
            .primitives
            .len();
        let btn_copied = emit_copy_button(
            1,
            &btn_desc.clone().with_copied(true),
            ControlState::default(),
            &[],
        )
        .primitives
        .len();
        assert_eq!(btn_base, btn_copied);

        let ro_desc = ReadOnlyInputDesc::new(Rect::new(10.0, 50.0, 280.0, 32.0), "0xdeadbeef");
        let ro_base = emit_read_only_input(2, &ro_desc, ControlState::default(), &[])
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
                            emit_copy_button(1, &btn_desc, st, &[]).primitives.len(),
                            btn_base
                        );
                        assert_eq!(
                            emit_read_only_input(2, &ro_desc, st, &[]).primitives.len(),
                            ro_base
                        );
                    }
                }
            }
        }
    }
}

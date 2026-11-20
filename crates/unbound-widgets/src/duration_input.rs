//! 时间时长输入框控件（`DurationInput`）。
//!
//! 支持人类可读时间字符串（`ms` / `s` / `m` / `h` 及纯数字按默认单位解释）的解析与规范化格式化、
//! 键盘上下步进（`ArrowUp` / `ArrowDown` 与 `Shift+Arrow` 大步长）、`min_ms..=max_ms` 范围钳制，
//! 并向无障碍树暴露 `Role::SpinButton` 语义与当前格式化值。

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yActions, A11yNode};
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::number_input::{spin_down_rect, spin_up_rect, NUMBER_RADIUS};
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 时长解析与显示的默认单位。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum DurationUnit {
    /// 毫秒（`ms`）。
    #[default]
    Ms,
    /// 秒（`s`）。
    Sec,
    /// 分钟（`m`）。
    Min,
    /// 小时（`h`）。
    Hour,
}

impl DurationUnit {
    /// 单个单位对应的毫秒数。
    #[must_use]
    pub const fn multiplier_ms(self) -> f64 {
        match self {
            Self::Ms => 1.0,
            Self::Sec => 1_000.0,
            Self::Min => 60_000.0,
            Self::Hour => 3_600_000.0,
        }
    }

    /// 单位标准后缀字符串。
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::Sec => "s",
            Self::Min => "m",
            Self::Hour => "h",
        }
    }
}

/// 将人类可读时长字符串（如 `"500ms"`、`"1.5s"`、`"2m"`、`"1h"` 或纯数字）解析为毫秒整数。
///
/// 不允许负数、NaN、无穷大或未知单位后缀。
pub fn parse_duration_ms(input: &str, default_unit: DurationUnit) -> Result<u64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Duration cannot be empty".to_owned());
    }

    let lower = trimmed.to_ascii_lowercase();
    let (num_str, unit) = if let Some(s) = lower.strip_suffix("ms") {
        (s.trim(), DurationUnit::Ms)
    } else if let Some(s) = lower.strip_suffix("sec") {
        (s.trim(), DurationUnit::Sec)
    } else if let Some(s) = lower.strip_suffix('s') {
        (s.trim(), DurationUnit::Sec)
    } else if let Some(s) = lower.strip_suffix("min") {
        (s.trim(), DurationUnit::Min)
    } else if let Some(s) = lower.strip_suffix('m') {
        (s.trim(), DurationUnit::Min)
    } else if let Some(s) = lower.strip_suffix("hr") {
        (s.trim(), DurationUnit::Hour)
    } else if let Some(s) = lower.strip_suffix('h') {
        (s.trim(), DurationUnit::Hour)
    } else {
        (lower.as_str(), default_unit)
    };

    let value: f64 = num_str
        .parse()
        .map_err(|_| format!("Invalid duration value: {trimmed}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err("Duration must be a non-negative finite number".to_owned());
    }

    let ms = (value * unit.multiplier_ms()).round();
    if !ms.is_finite() || ms < 0.0 || ms > u64::MAX as f64 {
        return Err("Duration overflows supported millisecond range".to_owned());
    }
    Ok(ms as u64)
}

/// 将毫秒整数格式化为简洁的人类可读时长字符串（如 `500ms`、`1.5s`、`2m`、`1h`）。
#[must_use]
pub fn format_duration_ms(ms: u64) -> String {
    if ms == 0 {
        return "0ms".to_owned();
    }
    if ms % 3_600_000 == 0 {
        return format!("{}h", ms / 3_600_000);
    }
    if ms % 60_000 == 0 {
        return format!("{}m", ms / 60_000);
    }
    if ms >= 1_000 {
        if ms % 1_000 == 0 {
            return format!("{}s", ms / 1_000);
        }
        if ms % 100 == 0 {
            let tenths = ms / 100;
            return format!("{}.{}s", tenths / 10, tenths % 10);
        }
        if ms % 10 == 0 && ms < 60_000 {
            let hundredths = ms / 10;
            return format!("{}.{:02}s", hundredths / 100, hundredths % 100);
        }
    }
    format!("{ms}ms")
}

/// 时长输入框状态机。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurationInputState {
    value_ms: u64,
    min_ms: u64,
    max_ms: u64,
    step_ms: u64,
    large_step_ms: u64,
    default_unit: DurationUnit,
    draft: String,
    editing: bool,
    error: Option<String>,
}

impl Default for DurationInputState {
    fn default() -> Self {
        Self::new(1000, 0, 3_600_000)
    }
}

impl DurationInputState {
    /// 创建时长输入框状态（自动钳制初始值到 `min_ms..=max_ms`）。
    #[must_use]
    pub fn new(value_ms: u64, min_ms: u64, max_ms: u64) -> Self {
        let max_ms = max_ms.max(min_ms);
        let clamped = value_ms.clamp(min_ms, max_ms);
        Self {
            value_ms: clamped,
            min_ms,
            max_ms,
            step_ms: 100,
            large_step_ms: 1000,
            default_unit: DurationUnit::Ms,
            draft: format_duration_ms(clamped),
            editing: false,
            error: None,
        }
    }

    /// 设置步进大小（毫秒），大步长默认同步为 `step_ms * 10`。
    #[must_use]
    pub fn with_step_ms(mut self, step_ms: u64) -> Self {
        let step = step_ms.max(1);
        self.step_ms = step;
        self.large_step_ms = step.saturating_mul(10);
        self
    }

    /// 显式设置 `Shift+Arrow` 大步长（毫秒）。
    #[must_use]
    pub fn with_large_step_ms(mut self, large_step_ms: u64) -> Self {
        self.large_step_ms = large_step_ms.max(1);
        self
    }

    /// 设置纯数字输入时采用的默认单位。
    #[must_use]
    pub const fn with_default_unit(mut self, unit: DurationUnit) -> Self {
        self.default_unit = unit;
        self
    }

    /// 当前规范化毫秒值。
    #[must_use]
    pub const fn value_ms(&self) -> u64 {
        self.value_ms
    }

    /// 最小值限制（毫秒）。
    #[must_use]
    pub const fn min_ms(&self) -> u64 {
        self.min_ms
    }

    /// 最大值限制（毫秒）。
    #[must_use]
    pub const fn max_ms(&self) -> u64 {
        self.max_ms
    }

    /// 是否处于文本编辑态。
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        self.editing
    }

    /// 当前校验错误提示（若上次提交解析失败）。
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 当前应显示的文本（编辑态返回草稿，否则返回规范化格式化文本）。
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.editing {
            self.draft.clone()
        } else {
            format_duration_ms(self.value_ms)
        }
    }

    /// 直接设置毫秒值（自动钳制并刷新草稿与错误态）。
    pub fn set_value_ms(&mut self, value_ms: u64) -> bool {
        let clamped = value_ms.clamp(self.min_ms, self.max_ms);
        let changed = clamped != self.value_ms;
        self.value_ms = clamped;
        self.draft = format_duration_ms(clamped);
        self.error = None;
        changed
    }

    /// 进入文本编辑态并写入草稿文本。
    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.editing = true;
        self.draft = draft.into();
        self.error = None;
    }

    /// 解析并提交当前草稿；合法时规范化并钳制，非法时回滚至旧值并记录错误提示。
    pub fn commit(&mut self) -> Result<u64, String> {
        let raw = self.draft.clone();
        match parse_duration_ms(&raw, self.default_unit) {
            Ok(ms) => {
                let clamped = ms.clamp(self.min_ms, self.max_ms);
                self.value_ms = clamped;
                self.draft = format_duration_ms(clamped);
                self.editing = false;
                self.error = None;
                Ok(clamped)
            }
            Err(err) => {
                // 非法输入回滚显示值并暴露错误态。
                self.draft = format_duration_ms(self.value_ms);
                self.editing = false;
                self.error = Some(err.clone());
                Err(err)
            }
        }
    }

    /// 失焦时自动规范化提交当前草稿（非法输入回滚并保留错误态）。
    pub fn on_blur(&mut self) -> Result<u64, String> {
        if self.editing {
            self.commit()
        } else {
            Ok(self.value_ms)
        }
    }

    /// 取消编辑并回滚草稿。
    pub fn cancel(&mut self) {
        self.editing = false;
        self.draft = format_duration_ms(self.value_ms);
        self.error = None;
    }

    /// 按步长调整时长（`delta_steps > 0` 增加，`< 0` 减少；`large == true` 使用大步长）。
    pub fn step(&mut self, delta_steps: i32, large: bool) -> bool {
        if delta_steps == 0 {
            return false;
        }
        let step = if large { self.large_step_ms } else { self.step_ms };
        let magnitude = step.saturating_mul(delta_steps.unsigned_abs() as u64);
        let next = if delta_steps > 0 {
            self.value_ms.saturating_add(magnitude)
        } else {
            self.value_ms.saturating_sub(magnitude)
        };
        self.editing = false;
        self.set_value_ms(next)
    }

    /// 响应键盘命名按键（`ArrowUp` / `ArrowDown` 步进，`Enter` 提交，`Escape` 取消）。
    pub fn handle_named_key(&mut self, key: &str, shift: bool) -> bool {
        match key {
            "ArrowUp" => self.step(1, shift),
            "ArrowDown" => self.step(-1, shift),
            "Enter" => self.commit().is_ok(),
            "Escape" => {
                self.cancel();
                true
            }
            _ => false,
        }
    }
}

/// 时长输入框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DurationInputDesc {
    /// 控件边界矩形。
    pub rect: Rect,
    /// 当前毫秒值。
    pub value_ms: u64,
    /// 当前展示文本。
    pub display_text: String,
    /// 无障碍标签。
    pub label: String,
    /// 是否处于校验错误态。
    pub has_error: bool,
    /// 可选错误说明文案。
    pub error_message: Option<String>,
    /// 绘制 z-index。
    pub z: u16,
}

impl DurationInputDesc {
    /// 创建时长输入框描述符。
    #[must_use]
    pub fn new(rect: Rect, value_ms: u64) -> Self {
        Self {
            rect,
            value_ms,
            display_text: format_duration_ms(value_ms),
            label: "Duration".to_owned(),
            has_error: false,
            error_message: None,
            z: 20,
        }
    }

    /// 从状态机同步构造描述符。
    #[must_use]
    pub fn from_state(rect: Rect, state: &DurationInputState) -> Self {
        Self {
            rect,
            value_ms: state.value_ms(),
            display_text: state.display_text(),
            label: "Duration".to_owned(),
            has_error: state.error().is_some(),
            error_message: state.error().map(ToOwned::to_owned),
            z: 20,
        }
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置错误态文案。
    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.has_error = true;
        self.error_message = Some(error.into());
        self
    }

    /// 返回校验错误提示槽位矩形（位于输入框下方 4px）。
    #[must_use]
    pub fn error_message_rect(&self) -> Rect {
        Rect::new(self.rect.origin.x, self.rect.max().y + 4.0, self.rect.width(), 18.0)
    }
}

/// 递增微调按钮的确定性子节点 ID。
#[must_use]
pub const fn duration_spin_up_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA301
}

/// 递减微调按钮的确定性子节点 ID。
#[must_use]
pub const fn duration_spin_down_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA302
}

/// 发射时长输入框（24 态恒定图元数：背景 + 底部强调线 + 上/下微调底板 + 4 条箭头线段 + 焦点环 = 9 个固定图元）。
#[must_use]
pub fn emit_duration_input(
    node: u64,
    desc: &DurationInputDesc,
    state: ControlState,
    text_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.rect;
    let z = desc.z;
    let mut primitives = Vec::with_capacity(10 + text_glyphs.len());
    let mut payloads = Vec::with_capacity(4 + text_glyphs.len());

    let (fill, border) = if state.disabled {
        (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
    } else if desc.has_error {
        (Token::INPUT_BG, Token::INPUT_BORDER_ERROR)
    } else if state.phase == PointerPhase::Hovered {
        (Token::INPUT_BG, Token::INPUT_BORDER_HOVER)
    } else {
        (Token::INPUT_BG, Token::CONTROL_STROKE)
    };
    primitives.push(outlined(r, NUMBER_RADIUS, fill, border, 1.0, 0.0, z));

    // 底部强调线（聚焦或错误态显示 2px 强调线，否则透明保持固定形状）。
    let accent_tok = if desc.has_error {
        Token::INPUT_BORDER_ERROR
    } else {
        Token::PRIMARY
    };
    let mut accent = surface(
        Rect::new(r.origin.x + 1.0, r.max().y - 2.0, (r.width() - 2.0).max(0.0), 2.0),
        0.0,
        accent_tok,
        z + 1,
    );
    accent.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z + 1,
        if !state.disabled && (state.focused || desc.has_error) {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(accent);

    // 右侧上/下步进按钮。
    let up_r = spin_up_rect(r);
    let down_r = spin_down_rect(r);
    let spin_fill = if state.disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.phase == PointerPhase::Pressed {
        Token::PRESSED
    } else if state.phase == PointerPhase::Hovered {
        Token::HOVER
    } else {
        Token::CONTROL_FILL
    };
    primitives.push(surface(up_r, 0.0, spin_fill, z + 1));
    primitives.push(surface(down_r, 0.0, spin_fill, z + 1));

    let arrow_tok = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    let ux = up_r.origin.x + up_r.width() * 0.5;
    let uy = up_r.origin.y + up_r.height() * 0.5;
    curve_stroke([ux - 3.0, uy + 1.5], [ux, uy - 1.5], 1.4, arrow_tok, z + 2, &mut primitives, &mut payloads);
    curve_stroke([ux, uy - 1.5], [ux + 3.0, uy + 1.5], 1.4, arrow_tok, z + 2, &mut primitives, &mut payloads);

    let dx = down_r.origin.x + down_r.width() * 0.5;
    let dy = down_r.origin.y + down_r.height() * 0.5;
    curve_stroke([dx - 3.0, dy - 1.5], [dx, dy + 1.5], 1.4, arrow_tok, z + 2, &mut primitives, &mut payloads);
    curve_stroke([dx, dy + 1.5], [dx + 3.0, dy - 1.5], 1.4, arrow_tok, z + 2, &mut primitives, &mut payloads);

    // 校验错误提示槽位（无错误时透明以维持 24 态恒定图元数）。
    let mut err_bg = outlined(
        desc.error_message_rect(),
        4.0,
        Token::ERROR_SOFT,
        Token::INPUT_BORDER_ERROR,
        1.0,
        0.0,
        z + 2,
    );
    err_bg.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z + 2,
        if desc.has_error && desc.error_message.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(err_bg);

    primitives.push(focus_ring(
        r.inflate(3.0),
        NUMBER_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        state.focused && !state.disabled,
    ));

    if !text_glyphs.is_empty() {
        let text_tok = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        emit_glyphs(text_glyphs, text_tok, z + 4, &mut primitives, &mut payloads);
    }

    let mut actions = A11yActions::NONE;
    actions.insert(A11yAction::Focus);
    actions.insert(A11yAction::SetValue);
    actions.insert(A11yAction::Increment);
    actions.insert(A11yAction::Decrement);
    let mut a11y = A11yNode::new(Role::SpinButton, desc.label.clone(), r)
        .with_value(desc.display_text.clone())
        .with_actions(actions)
        .disabled(state.disabled);
    if let Some(err) = &desc.error_message {
        a11y = a11y.with_description(err.clone());
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [NUMBER_RADIUS; 4],
            })
            .with_cursor(CursorKind::Text)
    });

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let up_id = duration_spin_up_node(node);
        out.with_sub_control(
            up_id,
            HitRegion::new(up_id, CONTROL_LAYER, u32::from(z) + 1, up_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Increase Duration", up_r).clickable(),
            false,
        );
        let down_id = duration_spin_down_node(node);
        out.with_sub_control(
            down_id,
            HitRegion::new(down_id, CONTROL_LAYER, u32::from(z) + 1, down_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Decrease Duration", down_r).clickable(),
            false,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn duration_parser_and_formatter_handle_units_and_invalid_input() {
        assert_eq!(parse_duration_ms("500ms", DurationUnit::Ms), Ok(500));
        assert_eq!(parse_duration_ms("1.5s", DurationUnit::Ms), Ok(1500));
        assert_eq!(parse_duration_ms("2m", DurationUnit::Ms), Ok(120_000));
        assert_eq!(parse_duration_ms("1h", DurationUnit::Ms), Ok(3_600_000));
        assert_eq!(parse_duration_ms("25", DurationUnit::Sec), Ok(25_000));

        assert!(parse_duration_ms("", DurationUnit::Ms).is_err());
        assert!(parse_duration_ms("-5s", DurationUnit::Ms).is_err());
        assert!(parse_duration_ms("abc", DurationUnit::Ms).is_err());

        assert_eq!(format_duration_ms(0), "0ms");
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(120_000), "2m");
        assert_eq!(format_duration_ms(3_600_000), "1h");
    }

    #[test]
    fn duration_input_keyboard_stepping_clamping_and_rollback() {
        let mut st = DurationInputState::new(1000, 200, 5000).with_step_ms(100);
        assert!(st.handle_named_key("ArrowUp", false));
        assert_eq!(st.value_ms(), 1100);
        assert!(st.handle_named_key("ArrowUp", true));
        assert_eq!(st.value_ms(), 2100);

        // 超过上限时严格钳制。
        st.set_draft("30s");
        assert_eq!(st.commit(), Ok(5000));
        assert_eq!(st.display_text(), "5s");

        // 非法输入回滚并保留错误信息。
        st.set_draft("not-a-duration");
        assert!(st.commit().is_err());
        assert_eq!(st.value_ms(), 5000);
        assert_eq!(st.display_text(), "5s");
        assert!(st.error().is_some());

        // 下限钳制。
        assert!(st.step(-100, true));
        assert_eq!(st.value_ms(), 200);
    }

    #[test]
    fn duration_input_a11y_spinbutton_stable_shape_and_headless() {
        let rect = Rect::new(20.0, 20.0, 180.0, 32.0);
        let desc = DurationInputDesc::new(rect, 1500);
        let out = emit_duration_input(1, &desc, ControlState::default(), &[]);
        let a11y = out.a11y.as_ref().expect("DurationInput must emit A11yNode");
        assert_eq!(a11y.role, Role::SpinButton);
        assert_eq!(a11y.value, "1.5s");

        let base = out.primitives.len();
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
                            emit_duration_input(1, &desc, st, &[]).primitives.len(),
                            base
                        );
                    }
                }
            }
        }

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "DurationInput",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

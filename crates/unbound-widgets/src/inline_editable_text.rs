//! 原地编辑文本控件（`InlineEditableText`）。
//!
//! 在展示态文本与编辑态 [`crate::text_input::TextEditState`] 之间无缝原地切换，
//! 支持单击或双击触发、进入编辑自动全选、`Enter` 提交、`Esc` 取消回滚、
//! 失焦提交/取消策略、空值拦截与自定义校验错误态反馈。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{TextEditState, ValidationState, TEXT_INPUT_RADIUS};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 进入编辑态的触发手势策略。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditTrigger {
    /// 单击即进入编辑态。
    SingleClick,
    /// 双击进入编辑态（默认）。
    #[default]
    DoubleClick,
}

/// 失焦时的提交策略。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlurBehavior {
    /// 失焦时自动尝试提交（默认）。
    #[default]
    Commit,
    /// 失焦时放弃修改并回滚至原始文本。
    Cancel,
}

/// 原地编辑交互结果事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineEditEvent {
    /// 已进入编辑态。
    EnteredEdit,
    /// 成功提交新文本。
    Committed(String),
    /// 取消编辑并回滚到原文本。
    Cancelled,
    /// 校验失败并停留在编辑态，携带英文错误提示。
    ValidationFailed(String),
    /// 无状态变更。
    None,
}

/// 原地编辑文本控件状态机。
#[derive(Clone, Debug, PartialEq)]
pub struct InlineEditableTextState {
    /// 当前已提交的持久化文本值。
    value: String,
    /// 处于编辑态时的文本缓冲与光标/选区状态。
    draft: TextEditState,
    /// 是否正处于编辑态。
    editing: bool,
    /// 是否允许提交空字符串（去除首尾空白后）。
    allow_empty: bool,
    /// 触发进入编辑态的方式。
    trigger: EditTrigger,
    /// 失焦时的默认行为。
    blur_behavior: BlurBehavior,
    /// 当前校验错误信息（`None` 表示无错误）。
    validation_error: Option<String>,
}

impl Default for InlineEditableTextState {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl InlineEditableTextState {
    /// 以初始值创建处于展示态的原地编辑状态机。
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let draft = TextEditState::with_text(value.clone());
        Self {
            value,
            draft,
            editing: false,
            allow_empty: false,
            trigger: EditTrigger::DoubleClick,
            blur_behavior: BlurBehavior::Commit,
            validation_error: None,
        }
    }

    /// 配置是否允许提交空文本（默认 `false`）。
    #[must_use]
    pub const fn with_allow_empty(mut self, allow: bool) -> Self {
        self.allow_empty = allow;
        self
    }

    /// 配置触发进入编辑态的手势（单击或双击）。
    #[must_use]
    pub const fn with_trigger(mut self, trigger: EditTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    /// 配置失焦时的行为（提交或取消）。
    #[must_use]
    pub const fn with_blur_behavior(mut self, behavior: BlurBehavior) -> Self {
        self.blur_behavior = behavior;
        self
    }

    /// 当前已提交文本值。
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// 是否正处于编辑态。
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        self.editing
    }

    /// 返回编辑态草稿状态（用于光标/选区渲染与文本编辑）。
    #[must_use]
    pub fn draft(&self) -> &TextEditState {
        &self.draft
    }

    /// 返回可变编辑态草稿状态。
    pub fn draft_mut(&mut self) -> &mut TextEditState {
        &mut self.draft
    }

    /// 当前校验错误文案（若存在）。
    #[must_use]
    pub fn validation_error(&self) -> Option<&str> {
        self.validation_error.as_deref()
    }

    /// 当前展示文本（编辑态返回草稿文本，展示态返回已提交值）。
    #[must_use]
    pub fn display_text(&self) -> &str {
        if self.editing {
            self.draft.text()
        } else {
            &self.value
        }
    }

    /// 外部强制同步已提交值（若当前未编辑则同时刷新草稿）。
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        if !self.editing {
            self.draft.set_text(self.value.clone());
            self.validation_error = None;
        }
    }

    /// 主动进入编辑态并自动全选当前文本。
    pub fn begin_edit(&mut self) -> InlineEditEvent {
        if self.editing {
            return InlineEditEvent::None;
        }
        self.editing = true;
        self.validation_error = None;
        self.draft.set_text(self.value.clone());
        self.draft.select_all();
        InlineEditEvent::EnteredEdit
    }

    /// 响应指针点击（`click_count`: `1` 为单击，`2` 为双击）。
    pub fn on_click(&mut self, click_count: u8) -> InlineEditEvent {
        if self.editing {
            return InlineEditEvent::None;
        }
        let should_enter = match self.trigger {
            EditTrigger::SingleClick => click_count >= 1,
            EditTrigger::DoubleClick => click_count >= 2,
        };
        if should_enter {
            self.begin_edit()
        } else {
            InlineEditEvent::None
        }
    }

    /// 在编辑态下输入文本并清除旧校验错误。
    pub fn set_draft_text(&mut self, text: impl Into<String>) {
        if self.editing {
            self.draft.set_text(text);
            self.validation_error = None;
        }
    }

    /// 提交当前草稿（使用默认空值校验规则）。
    pub fn commit(&mut self) -> InlineEditEvent {
        self.commit_with_validator(|_| Ok(()))
    }

    /// 提交当前草稿并执行自定义校验器（返回 `Err(msg)` 时保留编辑态并展示错误）。
    pub fn commit_with_validator<F>(&mut self, validator: F) -> InlineEditEvent
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        if !self.editing {
            return InlineEditEvent::None;
        }
        let candidate = self.draft.text().trim().to_owned();
        if !self.allow_empty && candidate.is_empty() {
            let msg = "Value cannot be empty".to_owned();
            self.validation_error = Some(msg.clone());
            return InlineEditEvent::ValidationFailed(msg);
        }
        if let Err(err) = validator(&candidate) {
            self.validation_error = Some(err.clone());
            return InlineEditEvent::ValidationFailed(err);
        }
        self.value = candidate.clone();
        self.editing = false;
        self.validation_error = None;
        InlineEditEvent::Committed(candidate)
    }

    /// 取消编辑并回滚至原始值。
    pub fn cancel(&mut self) -> InlineEditEvent {
        if !self.editing {
            return InlineEditEvent::None;
        }
        self.editing = false;
        self.validation_error = None;
        self.draft.set_text(self.value.clone());
        InlineEditEvent::Cancelled
    }

    /// 响应键盘确认/取消快捷键（`"Enter"` 提交，`"Escape"` 取消，展示态 `"Enter"`/`"F2"` 进入编辑）。
    pub fn handle_named_key(&mut self, key: &str) -> InlineEditEvent {
        self.handle_named_key_with_validator(key, |_| Ok(()))
    }

    /// 带自定义校验器的键盘响应。
    pub fn handle_named_key_with_validator<F>(&mut self, key: &str, validator: F) -> InlineEditEvent
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        if self.editing {
            match key {
                "Enter" => self.commit_with_validator(validator),
                "Escape" => self.cancel(),
                _ => InlineEditEvent::None,
            }
        } else if key == "Enter" || key == "F2" {
            self.begin_edit()
        } else {
            InlineEditEvent::None
        }
    }

    /// 响应失焦事件（依据 `blur_behavior` 提交或取消）。
    pub fn on_blur(&mut self) -> InlineEditEvent {
        self.on_blur_with_validator(|_| Ok(()))
    }

    /// 带自定义校验器的失焦响应。
    pub fn on_blur_with_validator<F>(&mut self, validator: F) -> InlineEditEvent
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        if !self.editing {
            return InlineEditEvent::None;
        }
        match self.blur_behavior {
            BlurBehavior::Commit => self.commit_with_validator(validator),
            BlurBehavior::Cancel => self.cancel(),
        }
    }
}

/// 原地编辑文本控件描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct InlineEditableTextDesc {
    /// 控件边界矩形。
    pub rect: Rect,
    /// 当前显示文本（展示态为原值，编辑态为草稿）。
    pub text: String,
    /// 占位提示文本（当文本为空时使用）。
    pub placeholder: String,
    /// 无障碍标签。
    pub label: String,
    /// 当前是否处于编辑态。
    pub editing: bool,
    /// 校验状态（错误时高亮边框并暴露错误描述）。
    pub validation: ValidationState,
    /// 校验错误提示文案。
    pub error_message: Option<String>,
    /// 展示态悬停时是否显示铅笔编辑图标暗示。
    pub show_edit_hint: bool,
    /// 绘制 z-index。
    pub z: u16,
}

impl InlineEditableTextDesc {
    /// 创建默认展示态描述符。
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            placeholder: "Click to edit".to_owned(),
            label: "Editable Text".to_owned(),
            editing: false,
            validation: ValidationState::None,
            error_message: None,
            show_edit_hint: true,
            z: 20,
        }
    }

    /// 从状态机同步构造描述符。
    #[must_use]
    pub fn from_state(rect: Rect, state: &InlineEditableTextState) -> Self {
        let mut desc = Self::new(rect, state.display_text());
        desc.editing = state.is_editing();
        if let Some(err) = state.validation_error() {
            desc.validation = ValidationState::Error;
            desc.error_message = Some(err.to_owned());
        }
        desc
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置占位提示文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 设置编辑态标志。
    #[must_use]
    pub const fn with_editing(mut self, editing: bool) -> Self {
        self.editing = editing;
        self
    }

    /// 设置校验错误提示。
    #[must_use]
    pub fn with_error(mut self, message: impl Into<String>) -> Self {
        self.validation = ValidationState::Error;
        self.error_message = Some(message.into());
        self
    }

    /// 返回校验错误提示文案的布局矩形（位于输入框下方 4px）。
    #[must_use]
    pub fn error_message_rect(&self) -> Rect {
        Rect::new(self.rect.origin.x, self.rect.max().y + 4.0, self.rect.width(), 18.0)
    }

    /// 返回尾部编辑铅笔图标区域矩形。
    #[must_use]
    pub fn edit_icon_rect(&self) -> Rect {
        let size = 14.0_f32.min((self.rect.height() - 8.0).max(8.0));
        Rect::new(
            self.rect.max().x - size - 8.0,
            self.rect.origin.y + (self.rect.height() - size) * 0.5,
            size,
            size,
        )
    }
}

/// 发射原地编辑文本控件（24 态恒定图元数：背景 + 底部强调线 + 铅笔图标线段 + 焦点环 = 4 个固定图元）。
#[must_use]
pub fn emit_inline_editable_text(
    node: u64,
    desc: &InlineEditableTextDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.rect;
    let z = desc.z;
    let hovered = !state.disabled && state.phase == PointerPhase::Hovered;
    let pressed = !state.disabled && state.phase == PointerPhase::Pressed;
    let is_error = desc.validation == ValidationState::Error;

    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(1 + label_glyphs.len());

    // 1. 背景表面：编辑态沿用 TextBox 表面；展示态仅在悬停/按下时浮现微弱底板。
    let (bg_fill, bg_border) = if state.disabled {
        (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
    } else if desc.editing {
        let border = if is_error {
            Token::INPUT_BORDER_ERROR
        } else if hovered {
            Token::INPUT_BORDER_HOVER
        } else {
            Token::CONTROL_STROKE
        };
        (Token::INPUT_BG, border)
    } else if pressed {
        (Token::PRESSED, Token::CONTROL_STROKE)
    } else if hovered {
        (Token::HOVER, Token::CONTROL_STROKE)
    } else {
        (Token::NONE, Token::NONE)
    };
    primitives.push(outlined(
        r,
        TEXT_INPUT_RADIUS,
        bg_fill,
        bg_border,
        1.0,
        0.0,
        z,
    ));

    // 2. 底部强调线（遵循 WinUI TextBox 规律：编辑聚焦时 2px PRIMARY/ERROR，否则透明以保持固定形状）。
    let accent_h = if desc.editing { 2.0 } else { 1.0 };
    let accent_token = if is_error {
        Token::INPUT_BORDER_ERROR
    } else {
        Token::PRIMARY
    };
    let mut accent = surface(
        Rect::new(r.origin.x + 1.0, r.max().y - accent_h, (r.width() - 2.0).max(0.0), accent_h),
        0.0,
        accent_token,
        z + 1,
    );
    let accent_alpha = if desc.editing && !state.disabled { 1.0 } else { 0.0 };
    accent.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, accent_alpha);
    primitives.push(accent);

    // 3. 展示态悬停编辑铅笔图标暗示（始终发射单个线段图元，不可见时透明保持固定形状）。
    let icon_r = desc.edit_icon_rect();
    let show_icon = desc.show_edit_hint && !desc.editing && (hovered || state.focused) && !state.disabled;
    let icon_idx = primitives.len();
    curve_stroke(
        [icon_r.origin.x + 2.0, icon_r.max().y - 2.0],
        [icon_r.max().x - 2.0, icon_r.origin.y + 2.0],
        1.5,
        Token::TEXT_SECONDARY,
        z + 1,
        &mut primitives,
        &mut payloads,
    );
    if !show_icon {
        primitives[icon_idx].z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, 0.0);
    }

    // 4. 校验错误提示槽位底板（无错误时透明以维持 24 态恒定图元数）。
    let err_r = desc.error_message_rect();
    let mut err_bg = outlined(
        err_r,
        4.0,
        Token::ERROR_SOFT,
        Token::INPUT_BORDER_ERROR,
        1.0,
        0.0,
        z + 1,
    );
    err_bg.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z + 1,
        if is_error && desc.error_message.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(err_bg);

    // 5. 键盘焦点环（按 WinUI Fluent 规范外扩 3px）。
    primitives.push(focus_ring(
        r.inflate(3.0),
        TEXT_INPUT_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        state.focused && !state.disabled,
    ));

    // 5. 文本字形。
    if !label_glyphs.is_empty() {
        let text_tok = if state.disabled {
            Token::TEXT_DISABLED
        } else if desc.text.is_empty() {
            Token::TEXT_SECONDARY
        } else {
            Token::TEXT_PRIMARY
        };
        emit_glyphs(label_glyphs, text_tok, z + 3, &mut primitives, &mut payloads);
    }

    let cursor = if desc.editing {
        CursorKind::Text
    } else {
        CursorKind::Pointer
    };
    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [TEXT_INPUT_RADIUS; 4],
            })
            .with_cursor(cursor)
    });

    // 展示态暴露可激活语义（Button），编辑态无缝切换为 TextInput 语义。
    let mut a11y = if desc.editing {
        A11yNode::new(Role::TextInput, desc.label.clone(), r)
            .with_value(desc.text.clone())
            .disabled(state.disabled)
    } else {
        A11yNode::new(Role::Button, desc.label.clone(), r)
            .with_value(desc.text.clone())
            .clickable()
            .disabled(state.disabled)
    };
    if let Some(err) = &desc.error_message {
        a11y = a11y.with_description(err.clone());
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn inline_editable_text_commit_cancel_and_blur_flows() {
        let mut st = InlineEditableTextState::new("Preset Alpha")
            .with_trigger(EditTrigger::DoubleClick)
            .with_blur_behavior(BlurBehavior::Commit);

        // 单击不触发双击模式。
        assert_eq!(st.on_click(1), InlineEditEvent::None);
        assert!(!st.is_editing());

        // 双击进入编辑态并自动全选。
        assert_eq!(st.on_click(2), InlineEditEvent::EnteredEdit);
        assert!(st.is_editing());
        assert_eq!(st.draft().selected_text(), Some("Preset Alpha"));

        // Esc 取消并回滚。
        st.set_draft_text("Temporary");
        assert_eq!(st.handle_named_key("Escape"), InlineEditEvent::Cancelled);
        assert!(!st.is_editing());
        assert_eq!(st.value(), "Preset Alpha");

        // 再次进入并修改，Enter 提交。
        st.begin_edit();
        st.set_draft_text("  Preset Beta  ");
        assert_eq!(
            st.handle_named_key("Enter"),
            InlineEditEvent::Committed("Preset Beta".to_owned())
        );
        assert!(!st.is_editing());
        assert_eq!(st.value(), "Preset Beta");

        // 失焦默认提交。
        st.begin_edit();
        st.set_draft_text("Preset Gamma");
        assert_eq!(
            st.on_blur(),
            InlineEditEvent::Committed("Preset Gamma".to_owned())
        );
        assert_eq!(st.value(), "Preset Gamma");
    }

    #[test]
    fn inline_editable_text_validation_rejects_empty_and_custom_error() {
        let mut st = InlineEditableTextState::new("Scene A");
        st.begin_edit();
        st.set_draft_text("   ");
        assert_eq!(
            st.commit(),
            InlineEditEvent::ValidationFailed("Value cannot be empty".to_owned())
        );
        assert!(st.is_editing());
        assert!(st.validation_error().is_some());

        st.set_draft_text("bad/name");
        let ev = st.commit_with_validator(|v| {
            if v.contains('/') {
                Err("Name cannot contain '/'".to_owned())
            } else {
                Ok(())
            }
        });
        assert_eq!(
            ev,
            InlineEditEvent::ValidationFailed("Name cannot contain '/'".to_owned())
        );
        assert!(st.is_editing());
        assert_eq!(st.value(), "Scene A");
    }

    #[test]
    fn inline_editable_text_a11y_role_transition_and_stable_shape() {
        let rect = Rect::new(20.0, 20.0, 220.0, 32.0);
        let display_desc = InlineEditableTextDesc::new(rect, "Title");
        let edit_desc = InlineEditableTextDesc::new(rect, "Title").with_editing(true);

        let display_out = emit_inline_editable_text(1, &display_desc, ControlState::default(), &[]);
        let edit_out = emit_inline_editable_text(1, &edit_desc, ControlState::default(), &[]);
        assert_eq!(display_out.a11y.as_ref().unwrap().role, Role::Button);
        assert_eq!(edit_out.a11y.as_ref().unwrap().role, Role::TextInput);
        assert_eq!(display_out.primitives.len(), edit_out.primitives.len());

        // 24 态恒定图元数。
        let base = display_out.primitives.len();
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
                            emit_inline_editable_text(1, &display_desc, st, &[]).primitives.len(),
                            base
                        );
                    }
                }
            }
        }

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
            "InlineEditableText",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

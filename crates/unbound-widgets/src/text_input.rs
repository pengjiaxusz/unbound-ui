//! 单行文本输入框：描述符、编辑状态、归约器与输出器。
//!
//! 该组件按其余控件相同的边界拆分：
//!
//! * [`TextInputDesc`] 为纯数据——仅含几何与策略，不含任何可变状态。
//! * [`TextEditState`] 持有 UTF-8 缓冲区、光标、选区与撤销/重做栈。完全不依赖字体或 GPU。
//!
//! * [`reduce_text_input`] 是针对 [`TextInputEvent`] 的纯状态演化函数。
//!   所有 UTF-8 边界规则均驻留于此：光标只能落在 `char` 边界上，
//!   因此退格删除中日韩汉字或 Emoji 时会移除整个标量值，而不会因拆分码点而 panic。
//!
//! * [`emit_text_input`] 将 `(描述符, 状态, 编辑状态, 已测量文本布局)` 转换为图元、命中区域与语义节点。
//!
//!
//! 无论是否存在选区或焦点，输出器始终写入相同的图元槽位——背景、边框、选区、光标、焦点环。
//! 缺席时选区与光标使用 `0.0` 不透明度，因此输入单个字符只需重绘一个槽位而无需重建整个图元池。
//!
//!
//!
//! 已测量的光标/选区矩形由调用方提供：文本引擎了解塑形步进宽度，而本 crate 故意不耦合这些细节。
//!

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement, CLIP_NONE};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{f16, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 文本输入框命中区域所在的界面层。
pub const TEXT_INPUT_LAYER: u8 = 1;
/// 默认字号。
pub const TEXT_INPUT_FONT_SIZE: f32 = 14.0;
/// 默认圆角半径。
pub const TEXT_INPUT_RADIUS: f32 = 8.0;
/// 边框与文本之间的水平内边距。
pub const TEXT_INPUT_PADDING: f32 = 10.0;
/// 焦点环向输入框外侧延伸的距离。
const FOCUS_RING_GAP: f32 = 3.0;

/// 文本输入框可识别的用户操作。
///
/// 复制与剪切不由控件直接执行：它们在 [`TextInputOutcome`] 中返回待复制文本，
/// 由应用程序与平台剪贴板交互。粘贴则相反：由应用程序读取剪贴板后发送 [`TextInputEvent::Paste`]。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// 在光标处插入文本（键入字符、输入法提交、粘贴）。
    Insert(String),
    /// 删除选区，或光标前的一个码点。
    Backspace,
    /// 删除选区，或光标后的一个码点。
    Delete,
    /// 向左移动一个码点。
    MoveLeft { extend: bool },
    /// 向右移动一个码点。
    MoveRight { extend: bool },
    /// 向左移动一个单词（`Ctrl+Left`）。
    MoveWordLeft { extend: bool },
    /// 向右移动一个单词（`Ctrl+Right`）。
    MoveWordRight { extend: bool },
    /// 删除光标前的单词（`Ctrl+Backspace`）。
    DeleteWordBack,
    /// 删除光标后的单词（`Ctrl+Delete`）。
    DeleteWordForward,
    /// 移动到文本开头。
    Home { extend: bool },
    /// 移动到文本末尾。
    End { extend: bool },
    /// `Ctrl+A` 全选。
    SelectAll,
    /// `Ctrl+C` 复制。
    Copy,
    /// `Ctrl+X` 剪切。
    Cut,
    /// `Ctrl+V` 粘贴（应用程序读取剪贴板后发送）。
    Paste(String),
    /// `Ctrl+Z` 撤销。
    Undo,
    /// `Ctrl+Shift+Z` / `Ctrl+Y` 重做。
    Redo,
    /// 指针在某字节偏移处按下。
    PointerDown { offset: usize },
    /// 按住指针移动，扩展选区。
    PointerDrag { offset: usize },
    /// 指针释放。
    PointerUp,
    /// 双击选中字节偏移周围的单词。
    DoubleClick { offset: usize },
    /// 三击选中整行（此处为整个缓冲区）。
    TripleClick,
    /// 输入法预编辑文本占用 `start..end`，以便提交时替换。
    ImePreedit { start: usize, end: usize },
    /// 组合输入结束；`text` 替换预编辑区间。
    ImeCommit { text: String },
}

/// 单次归约的输出结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInputOutcome {
    /// 编辑缓冲区或选区是否发生变化。
    pub changed: bool,
    /// 应用程序必须写入剪贴板的文本（`Ctrl+C`）。
    pub copied: Option<String>,
    /// 应用程序必须写入剪贴板并从缓冲区移除的文本（`Ctrl+X`）。
    pub cut: Option<String>,
    /// 应用程序应读取剪贴板并发送 [`TextInputEvent::Paste`]。
    pub paste_requested: bool,
}

impl TextInputOutcome {
    fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }
}

/// 输入框内容的校验状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValidationState {
    /// 无校验消息。
    #[default]
    None,
    /// 内容无效。
    Error,
    /// 内容需警告但仍可接受。
    Warning,
}

/// 单行文本输入框的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputDesc {
    /// 输入框矩形。
    pub rect: Rect,
    /// 无障碍标签（为空时回退到占位文本）。
    pub label: String,
    /// 缓冲区为空时显示的占位文本。
    pub placeholder: String,
    /// 禁用交互与聚焦。
    pub disabled: bool,
    /// 文本可选中与复制，但不可编辑。
    pub read_only: bool,
    /// 掩码隐藏字符，且拒绝复制/剪切。
    pub password: bool,
    /// 校验状态，驱动边框颜色。
    pub validation: ValidationState,
    /// 非空时显示清除按钮。
    pub clearable: bool,
    /// 缓冲区最大字节长度。
    pub max_bytes: usize,
    /// 绘制顺序键。
    pub z: u16,
    /// 圆角半径。
    pub radius: f32,
    /// 字号（供调用方塑形时使用）。
    pub font_size: f32,
}

impl TextInputDesc {
    /// 使用默认尺寸创建的输入框。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            read_only: false,
            password: false,
            validation: ValidationState::None,
            clearable: false,
            max_bytes: 1 << 20,
            z: 720,
            radius: TEXT_INPUT_RADIUS,
            font_size: TEXT_INPUT_FONT_SIZE,
        }
    }

    /// 设置占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置为只读状态。
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// 设置为密码掩码模式。
    #[must_use]
    pub fn password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    /// 设置校验状态。
    #[must_use]
    pub fn with_validation(mut self, validation: ValidationState) -> Self {
        self.validation = validation;
        self
    }

    /// 启用清除按钮。
    #[must_use]
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }

    /// 设置最大字节预算。
    #[must_use]
    pub fn max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// 待塑形与绘制的文本。
    ///
    /// 密码框会将每个标量值替换为 `•`；调用方应塑形此返回值而非 [`TextEditState::text`]。
    #[must_use]
    pub fn display_text(&self, text: &str) -> String {
        if self.password {
            "\u{2022}".repeat(text.chars().count())
        } else {
            text.to_owned()
        }
    }

    /// 位于尾部内边距内居中的清除按钮矩形。
    #[must_use]
    pub fn clear_rect(&self) -> Rect {
        const SIZE: f32 = 16.0;
        Rect::new(
            self.rect.max().x - TEXT_INPUT_PADDING - SIZE,
            self.rect.origin.y + (self.rect.size.y - SIZE) * 0.5,
            SIZE,
            SIZE,
        )
    }

    fn editable(&self) -> bool {
        !self.disabled && !self.read_only
    }

    fn accessible_label(&self) -> String {
        if self.label.is_empty() {
            self.placeholder.clone()
        } else {
            self.label.clone()
        }
    }
}

/// 单个撤销检查点。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// 单个文本输入框的可编辑状态。
///
/// 光标与锚点均是进入 [`Self::text`] 的**字节偏移**，且始终位于 UTF-8 边界上。
/// 二元组 `(anchor, cursor)` 表示选区：两者相等表示“无选区”。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextEditState {
    text: String,
    cursor: usize,
    anchor: usize,
    scroll_x: f32,
    preedit: Option<(usize, usize)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// 连续键入合并为单次撤销步骤。
    coalescing: bool,
    /// 指针拖拽扩展选区所基于的锚点。
    drag_anchor: Option<usize>,
}

impl TextEditState {
    /// 创建空输入框状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建预填文本的输入框状态，光标位于末尾。
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: cursor,
            ..Self::default()
        }
    }

    /// 文本缓冲区引用。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标字节偏移量。
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// 选区锚点字节偏移量。
    #[must_use]
    pub const fn anchor(&self) -> usize {
        self.anchor
    }

    /// 水平滚动偏移量（逻辑像素）。
    #[must_use]
    pub const fn scroll_x(&self) -> f32 {
        self.scroll_x
    }

    /// 若输入法组合输入激活，返回预编辑字节区间。
    #[must_use]
    pub const fn preedit(&self) -> Option<(usize, usize)> {
        self.preedit
    }

    /// 输入框是否存在非空选区。
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    /// 排序后的 `(start, end)` 选区字节区间。
    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        if !self.has_selection() {
            return None;
        }
        Some((self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
    }

    /// 当前选中的文本。
    #[must_use]
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        self.text.get(start..end)
    }

    /// 替换整个缓冲区并将光标置于末尾。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.normalize();
    }

    /// 设置水平滚动偏移量，夹紧至非负值。
    pub fn set_scroll_x(&mut self, scroll_x: f32) {
        self.scroll_x = scroll_x.max(0.0);
    }

    /// 滚动最小距离使光标落入可见窗口内。
    ///
    /// `caret_x` 为光标在文本自身（未滚动）坐标空间中的位置；
    /// 光标将被保持在 `[scroll_x, scroll_x + viewport_width]` 范围内。
    pub fn ensure_caret_visible(&mut self, caret_x: f32, viewport_width: f32) {
        let viewport_width = viewport_width.max(0.0);
        if caret_x < self.scroll_x {
            self.scroll_x = caret_x.max(0.0);
        } else if caret_x > self.scroll_x + viewport_width {
            self.scroll_x = (caret_x - viewport_width).max(0.0);
        }
    }

    /// 将光标移动到指定字节偏移并夹紧到字符边界。
    ///
    /// `extend` 为 true 时保留锚点以扩展或收缩选区。
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        let cursor = floor_boundary(&self.text, offset);
        self.cursor = cursor;
        if !extend {
            self.anchor = cursor;
        }
    }

    /// 全选文本。
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.coalescing = false;
    }

    /// 使用 `shaped` 通过点击 `local_x`（相对于文本原点）定位光标。
    pub fn pointer_down_at(
        &mut self,
        shaped: &unbound_text::ShapedText,
        local_x: f32,
        extend: bool,
    ) {
        let offset = shaped.hit_byte_index(&self.text, Vec2::new(local_x + self.scroll_x, 0.0));
        self.set_cursor(offset, extend);
    }

    /// 拖拽到 `local_x`（相对于文本原点）时扩展选区。
    pub fn drag_to(&mut self, shaped: &unbound_text::ShapedText, local_x: f32) {
        let offset = shaped.hit_byte_index(&self.text, Vec2::new(local_x + self.scroll_x, 0.0));
        self.set_cursor(offset, true);
    }

    /// 选中 `local_x` 处的单词（双击）。
    pub fn double_click_at(&mut self, shaped: &unbound_text::ShapedText, local_x: f32) {
        let offset = shaped.hit_byte_index(&self.text, Vec2::new(local_x + self.scroll_x, 0.0));
        if let Some((start, end)) = word_range(&self.text, offset) {
            self.anchor = start;
            self.cursor = end;
            self.coalescing = false;
        }
    }

    /// 选中全部文本（三击）。
    pub fn triple_click(&mut self) {
        self.select_all();
    }

    /// 在光标处插入文本并替换当前选区。
    ///
    /// 缓冲区会夹紧至 `max_bytes`，且不会拆分码点。
    pub fn insert(&mut self, text: &str, max_bytes: usize) {
        self.insert_with(text, max_bytes, true);
    }

    fn insert_with(&mut self, text: &str, max_bytes: usize, coalesce: bool) {
        if text.is_empty() {
            return;
        }
        self.push_undo(coalesce);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let mut next = String::with_capacity(self.text.len() + text.len());
        next.push_str(&self.text[..start]);
        next.push_str(text);
        next.push_str(&self.text[end..]);
        if next.len() > max_bytes {
            // 尽量取最多能放下的完整码点。
            let room = max_bytes.saturating_sub(self.text.len() - (end - start));
            let mut accepted = 0usize;
            for (index, character) in text.char_indices() {
                let after = index + character.len_utf8();
                if after > room {
                    break;
                }
                accepted = after;
            }
            next.clear();
            next.push_str(&self.text[..start]);
            next.push_str(&text[..accepted]);
            next.push_str(&self.text[end..]);
        }
        self.text = next;
        self.cursor = floor_boundary(&self.text, start + text.len());
        self.anchor = self.cursor;
        self.normalize();
    }

    /// 删除选区，或光标前的一个码点。
    pub fn backspace(&mut self) {
        let (start, end) = self.selection().unwrap_or_else(|| {
            let start = prev_boundary(&self.text, self.cursor);
            (start, self.cursor)
        });
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// 删除选区，或光标后的一个码点。
    pub fn delete_forward(&mut self) {
        let (start, end) = self.selection().unwrap_or_else(|| {
            let end = next_boundary(&self.text, self.cursor);
            (self.cursor, end)
        });
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// 删除选区，或光标前的单词。
    pub fn delete_word_backward(&mut self) {
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (prev_word_boundary(&self.text, self.cursor), self.cursor));
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// 删除选区，或光标后的单词。
    pub fn delete_word_forward(&mut self) {
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (self.cursor, next_word_boundary(&self.text, self.cursor)));
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// 撤销一步，返回是否实际执行了撤销。
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        self.coalescing = false;
        true
    }

    /// 重做一步，返回是否实际执行了重做。
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        self.coalescing = false;
        true
    }

    /// 丢弃撤销历史（例如加载了其他文档之后）。
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.coalescing = false;
    }

    /// 用提交的文本替换预编辑区间（输入法提交）。
    fn commit_preedit(&mut self, text: &str, max_bytes: usize) {
        if let Some((start, end)) = self.preedit.take() {
            let start = floor_boundary(&self.text, start);
            let end = floor_boundary(&self.text, end).max(start);
            self.cursor = start;
            self.anchor = end;
        }
        self.insert_with(text, max_bytes, false);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.normalize();
    }

    /// 将连续键入合并为单个撤销步骤。
    fn push_undo(&mut self, coalesce: bool) {
        if !(coalesce && self.coalescing) {
            let snapshot = self.snapshot();
            self.undo.push(snapshot);
            // 长时间编辑会话的历史记录不得无限制增长。
            if self.undo.len() > 256 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.coalescing = coalesce;
    }

    /// 确保光标保持在字符边界及缓冲区范围内。
    fn normalize(&mut self) {
        self.cursor = floor_boundary(&self.text, self.cursor);
        self.anchor = floor_boundary(&self.text, self.anchor);
    }
}

/// 响应单个事件推进文本输入框状态。
pub fn reduce_text_input(
    desc: &TextInputDesc,
    edit: &mut TextEditState,
    event: &TextInputEvent,
) -> TextInputOutcome {
    if desc.disabled {
        return TextInputOutcome::default();
    }
    match event {
        TextInputEvent::Insert(text) => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.insert(text, desc.max_bytes);
            TextInputOutcome::changed()
        }
        TextInputEvent::Backspace => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.backspace();
            TextInputOutcome::changed()
        }
        TextInputEvent::Delete => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_forward();
            TextInputOutcome::changed()
        }
        TextInputEvent::DeleteWordBack => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_word_backward();
            TextInputOutcome::changed()
        }
        TextInputEvent::DeleteWordForward => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_word_forward();
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveLeft { extend } => {
            let cursor = prev_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveRight { extend } => {
            let cursor = next_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveWordLeft { extend } => {
            let cursor = prev_word_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveWordRight { extend } => {
            let cursor = next_word_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::Home { extend } => {
            edit.set_cursor(0, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::End { extend } => {
            let end = edit.text().len();
            edit.set_cursor(end, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::SelectAll => {
            edit.select_all();
            TextInputOutcome::changed()
        }
        TextInputEvent::Copy => {
            if desc.password {
                return TextInputOutcome::default();
            }
            TextInputOutcome {
                copied: edit.selected_text().map(str::to_owned),
                ..TextInputOutcome::default()
            }
        }
        TextInputEvent::Cut => {
            if !desc.editable() || desc.password {
                return TextInputOutcome::default();
            }
            let Some(text) = edit.selected_text().map(str::to_owned) else {
                return TextInputOutcome::default();
            };
            edit.backspace();
            TextInputOutcome {
                changed: true,
                cut: Some(text),
                ..TextInputOutcome::default()
            }
        }
        TextInputEvent::Paste(text) => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.insert_with(text, desc.max_bytes, false);
            TextInputOutcome::changed()
        }
        TextInputEvent::Undo => TextInputOutcome {
            changed: edit.undo(),
            ..TextInputOutcome::default()
        },
        TextInputEvent::Redo => TextInputOutcome {
            changed: edit.redo(),
            ..TextInputOutcome::default()
        },
        TextInputEvent::PointerDown { offset } => {
            edit.drag_anchor = Some(*offset);
            edit.set_cursor(*offset, false);
            TextInputOutcome::changed()
        }
        TextInputEvent::PointerDrag { offset } => {
            let anchor = edit.drag_anchor.unwrap_or(edit.anchor());
            edit.anchor = floor_boundary(edit.text(), anchor);
            edit.set_cursor(*offset, true);
            TextInputOutcome::changed()
        }
        TextInputEvent::PointerUp => {
            edit.drag_anchor = None;
            TextInputOutcome::default()
        }
        TextInputEvent::DoubleClick { offset } => {
            if let Some((start, end)) = word_range(edit.text(), *offset) {
                edit.anchor = start;
                edit.cursor = end;
            } else {
                edit.set_cursor(*offset, false);
            }
            TextInputOutcome::changed()
        }
        TextInputEvent::TripleClick => {
            edit.select_all();
            TextInputOutcome::changed()
        }
        TextInputEvent::ImePreedit { start, end } => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            let start = floor_boundary(edit.text(), *start);
            let end = floor_boundary(edit.text(), *end).max(start);
            edit.preedit = Some((start, end));
            edit.cursor = end;
            edit.anchor = end;
            TextInputOutcome::changed()
        }
        TextInputEvent::ImeCommit { text } => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.commit_preedit(text, desc.max_bytes);
            TextInputOutcome::changed()
        }
    }
}

/// 输出器绘制所需的已测量文本几何信息。
///
/// 调用方负责塑形文本（其持有字体、图集与缩放），
/// 并在与 [`TextInputDesc::rect`] 相同的坐标空间中传回光标与选区矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextInputVisual {
    /// 光标矩形。
    pub caret: Rect,
    /// 选区矩形（若选中了任何文本）。
    pub selection: Option<Rect>,
    /// 已测量的输入法预编辑矩形（若组合输入激活）。
    pub preedit: Option<Rect>,
    /// 光标是否处于闪烁可见阶段。
    pub caret_visible: bool,
}

impl Default for TextInputVisual {
    fn default() -> Self {
        Self {
            caret: Rect::ZERO,
            selection: None,
            preedit: None,
            caret_visible: true,
        }
    }
}

/// 绑定了节点标识、编辑状态与已测量布局的文本输入框。
#[derive(Clone, Debug)]
pub struct TextInputWidget {
    /// 控件标识。
    pub node: u64,
    /// 描述符。
    pub desc: TextInputDesc,
    /// 编辑状态。
    pub edit: TextEditState,
    /// 已测量的光标/选区几何信息。
    pub visual: TextInputVisual,
}

impl WidgetDesc for TextInputWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn disabled(&self) -> bool {
        self.desc.disabled
    }

    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput {
        emit_text_input(
            self.node,
            &self.desc,
            state,
            &self.edit,
            &self.visual,
            glyphs,
        )
    }
}

/// 纯输出器。
///
/// 对于固定字形串，图元数量保持恒定：选区与光标槽位始终写入，
/// 仅通过不透明度指示其是否可见。
#[must_use]
pub fn emit_text_input(
    node: u64,
    desc: &TextInputDesc,
    state: ControlState,
    edit: &TextEditState,
    visual: &TextInputVisual,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let z = desc.z;
    let mut primitives = Vec::with_capacity(5 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if state.disabled {
        Token::DISABLED
    } else if state.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_RAISED
    };
    primitives.push(surface(rect, desc.radius, fill, z));

    let border = match desc.validation {
        ValidationState::Error => Token::INPUT_BORDER_ERROR,
        ValidationState::Warning => Token::INPUT_BORDER_WARNING,
        ValidationState::None => {
            if state.focused {
                Token::OUTLINE_STRONG
            } else {
                Token::OUTLINE
            }
        }
    };
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z,
    ));

    // 选区槽位始终存在；通过不透明度控制可见性。
    let mut selection = outlined(
        visual.selection.unwrap_or(Rect::ZERO),
        2.0,
        Token::NONE,
        Token::NONE,
        0.0,
        0.0,
        z,
    );
    selection.fill_token_or_uv = u32::from(Token::SELECTION.raw());
    selection.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::InnerShadow as u16,
    );
    set_opacity(
        &mut selection,
        if visual.selection.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(selection);

    // 光标槽位：1 像素宽，未聚焦时隐藏。
    let mut caret = surface(visual.caret, 0.0, Token::PRIMARY, z);
    caret.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    set_opacity(
        &mut caret,
        if state.focused && !state.disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    // 输入法预编辑下划线：与光标一样始终存在于图元池中，不透明度指示组合输入是否激活。
    //
    let mut preedit = surface(
        visual.preedit.unwrap_or(Rect::ZERO),
        0.0,
        Token::PREEDIT_UNDERLINE,
        z,
    );
    preedit.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    set_opacity(
        &mut preedit,
        if edit.preedit().is_some() && visual.preedit.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(preedit);

    // 调用方定位好的字形。
    let ink = if state.disabled {
        Token::TEXT_DISABLED
    } else if edit.text().is_empty() && !desc.placeholder.is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    for quad in glyphs.iter() {
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
        let mut glyph = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
            size: [quad.rect.size.x, quad.rect.size.y],
            corner_radii: f16::pack_radii([0.0; 4]),
            border_width: f16::f32_to_f16(0.0),
            shadow_blur: f16::f32_to_f16(0.0),
            transform_idx: 0,
            clip_idx: CLIP_NONE,
            fill_token_or_uv: u32::from(ink.raw()),
            border_token_or_tex: quad.layer,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                Default::default(),
                SubElement::Content as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: payload_idx,
        };
        if quad.msdf {
            glyph.prim_type_and_flags |= u32::from(unbound_gpu::data::PrimFlags::MSDF.bits()) << 16;
        }
        primitives.push(glyph);
    }

    primitives.push(focus_ring(
        rect.inflate(FOCUS_RING_GAP),
        desc.radius + FOCUS_RING_GAP,
        Token::FOCUS_RING,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, TEXT_INPUT_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::TextInput, desc.accessible_label(), rect)
        .clickable()
        .disabled(state.disabled);
    a11y.value = desc.display_text(edit.text());
    a11y.actions.insert(A11yAction::SetValue);
    a11y.actions.insert(A11yAction::Focus);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };
    if desc.clearable && !edit.text().is_empty() && !state.disabled && !desc.read_only {
        let clear_key = clear_button_node(node);
        let clear_rect = desc.clear_rect();
        out.with_sub_control(
            clear_key,
            HitRegion::new(clear_key, TEXT_INPUT_LAYER, u32::from(z) + 1, clear_rect)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Clear", clear_rect).clickable(),
            true,
        );
    }
    out
}

/// [`TextInputDesc`] 清除按钮的确定性子节点键。
#[must_use]
pub const fn clear_button_node(node: u64) -> u64 {
    node ^ 0x434C_4541_5200
}

fn set_opacity(primitive: &mut GpuPrimitiveInstance, opacity: f32) {
    primitive.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(primitive.depth_u16(), opacity);
}

/// `offset` 处或之前的最近字符边界。
#[must_use]
pub fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// 上一个字符边界。
#[must_use]
pub fn prev_boundary(text: &str, offset: usize) -> usize {
    let offset = floor_boundary(text, offset);
    if offset == 0 {
        return 0;
    }
    let mut index = offset - 1;
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// 下一个字符边界。
#[must_use]
pub fn next_boundary(text: &str, offset: usize) -> usize {
    let offset = floor_boundary(text, offset);
    if offset >= text.len() {
        return text.len();
    }
    let mut index = offset + 1;
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index.min(text.len())
}

/// 上一个单词边界。
///
/// 本 crate 不依赖 `unicode_segmentation`，因此“单词”定义为连续的 `char::is_alphanumeric`（或 `_`）标量，
/// 其余字符（空格与标点）均被跳过。这与 [`word_range`] 用于双击选词的规则一致。
#[must_use]
pub fn prev_word_boundary(text: &str, offset: usize) -> usize {
    let mut index = floor_boundary(text, offset);
    while index > 0 {
        let previous = prev_boundary(text, index);
        let Some(character) = text[previous..index].chars().next() else {
            break;
        };
        if is_word_char(character) {
            break;
        }
        index = previous;
    }
    while index > 0 {
        let previous = prev_boundary(text, index);
        let Some(character) = text[previous..index].chars().next() else {
            break;
        };
        if !is_word_char(character) {
            break;
        }
        index = previous;
    }
    index
}

/// 下一个单词边界（先跳过分隔符）。
#[must_use]
pub fn next_word_boundary(text: &str, offset: usize) -> usize {
    let mut index = floor_boundary(text, offset);
    while index < text.len() {
        let next = next_boundary(text, index);
        let Some(character) = text[index..next].chars().next() else {
            break;
        };
        if is_word_char(character) {
            break;
        }
        index = next;
    }
    while index < text.len() {
        let next = next_boundary(text, index);
        let Some(character) = text[index..next].chars().next() else {
            break;
        };
        if !is_word_char(character) {
            break;
        }
        index = next;
    }
    index
}

/// 字节偏移周围的单词区间。
///
/// 单词字符为 Unicode 字母数字与 `_`；标点与空白符用于分隔单词。
/// 从偏移处向外扫描（而非依赖分词器）可将连续的中日韩汉字保持在一起，符合双击选词的预期。
#[must_use]
pub fn word_range(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = floor_boundary(text, offset);
    let mut start = offset;
    while start > 0 {
        let previous = prev_boundary(text, start);
        let character = text[previous..start].chars().next()?;
        if !is_word_char(character) {
            break;
        }
        start = previous;
    }
    let mut end = offset;
    while end < text.len() {
        let next = next_boundary(text, end);
        let character = text[end..next].chars().next()?;
        if !is_word_char(character) {
            break;
        }
        end = next;
    }
    if start == end {
        return None;
    }
    Some((start, end))
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WidgetHarness;

    fn desc() -> TextInputDesc {
        TextInputDesc::new(Rect::new(20.0, 20.0, 240.0, 36.0)).with_placeholder("type here")
    }

    fn reduce(edit: &mut TextEditState, event: TextInputEvent) -> TextInputOutcome {
        reduce_text_input(&desc(), edit, &event)
    }

    #[test]
    fn typing_and_backspacing_respect_char_boundaries() {
        let mut edit = TextEditState::new();
        reduce(&mut edit, TextInputEvent::Insert("你好".into()));
        assert_eq!(edit.text(), "你好");
        assert_eq!(edit.cursor(), 6);

        // 一次退格删除一个完整汉字，而非单个字节。
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "你");
        assert!(edit.text().is_char_boundary(edit.cursor()));

        reduce(&mut edit, TextInputEvent::Insert("a".into()));
        assert_eq!(edit.text(), "你a");
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "你");
    }

    #[test]
    fn an_emoji_is_deleted_as_one_scalar_value() {
        let mut edit = TextEditState::with_text("a😀b");
        edit.set_cursor(5, false);
        assert!(edit.text().is_char_boundary(5));
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "ab");
        assert_eq!(edit.cursor(), 1);
    }

    #[test]
    fn the_cursor_never_lands_inside_a_multibyte_character() {
        let mut edit = TextEditState::with_text("你好");
        edit.set_cursor(1, false);
        assert_eq!(edit.cursor(), 0);
        edit.set_cursor(34, false);
        assert_eq!(edit.cursor(), 6);
    }

    #[test]
    fn shift_arrows_extend_the_selection_and_typing_replaces_it() {
        let mut edit = TextEditState::with_text("hello world");
        reduce(&mut edit, TextInputEvent::Home { extend: false });
        for _ in 0..5 {
            reduce(&mut edit, TextInputEvent::MoveRight { extend: true });
        }
        assert_eq!(edit.selected_text(), Some("hello"));
        reduce(&mut edit, TextInputEvent::Insert(",".into()));
        assert_eq!(edit.text(), ", world");
        assert_eq!(edit.cursor(), 1);
    }

    #[test]
    fn double_click_selects_a_word_including_cjk() {
        let mut edit = TextEditState::with_text("hello 世界 world");
        reduce(&mut edit, TextInputEvent::DoubleClick { offset: 8 });
        assert_eq!(edit.selected_text(), Some("世界"));

        let mut edit = TextEditState::with_text("hello 世界 world");
        reduce(&mut edit, TextInputEvent::DoubleClick { offset: 1 });
        assert_eq!(edit.selected_text(), Some("hello"));
    }

    #[test]
    fn dragging_selects_between_two_offsets() {
        let mut edit = TextEditState::with_text("abcdef");
        reduce(&mut edit, TextInputEvent::PointerDown { offset: 1 });
        reduce(&mut edit, TextInputEvent::PointerDrag { offset: 4 });
        assert_eq!(edit.selected_text(), Some("bcd"));
        reduce(&mut edit, TextInputEvent::PointerUp);
        assert!(edit.drag_anchor.is_none());
    }

    #[test]
    fn copy_and_cut_report_the_clipboard_payload() {
        let mut edit = TextEditState::with_text("abcdef");
        edit.anchor = 0;
        edit.cursor = 3;

        let outcome = reduce(&mut edit, TextInputEvent::Copy);
        assert_eq!(outcome.copied.as_deref(), Some("abc"));
        assert!(!outcome.changed, "copy must not edit");

        let outcome = reduce(&mut edit, TextInputEvent::Cut);
        assert_eq!(outcome.cut.as_deref(), Some("abc"));
        assert_eq!(edit.text(), "def");
    }

    #[test]
    fn undo_merges_a_typing_run_but_not_paste() {
        let mut edit = TextEditState::new();
        for character in ["a", "b", "c"] {
            reduce(&mut edit, TextInputEvent::Insert(character.into()));
        }
        assert_eq!(edit.text(), "abc");
        assert!(edit.undo());
        assert_eq!(edit.text(), "", "the whole typing run is one step");

        reduce(&mut edit, TextInputEvent::Paste("xyz".into()));
        reduce(&mut edit, TextInputEvent::Insert("!".into()));
        assert!(edit.undo());
        assert_eq!(edit.text(), "xyz");
        assert!(edit.undo());
        assert_eq!(edit.text(), "");
    }

    #[test]
    fn redo_restores_an_undone_edit() {
        let mut edit = TextEditState::new();
        reduce(&mut edit, TextInputEvent::Insert("a".into()));
        assert!(edit.undo());
        assert_eq!(edit.text(), "");
        assert!(edit.redo());
        assert_eq!(edit.text(), "a");
    }

    #[test]
    fn ime_preedit_is_replaced_by_the_commit() {
        let mut edit = TextEditState::with_text("ab");
        reduce(&mut edit, TextInputEvent::ImePreedit { start: 2, end: 2 });
        assert_eq!(edit.preedit(), Some((2, 2)));

        reduce(&mut edit, TextInputEvent::ImeCommit { text: "你".into() });
        assert_eq!(edit.text(), "ab你");
        assert_eq!(edit.preedit(), None);
    }

    #[test]
    fn the_byte_budget_never_splits_a_character() {
        let desc = desc().max_bytes(5);
        let mut edit = TextEditState::new();
        reduce_text_input(&desc, &mut edit, &TextInputEvent::Insert("你好".into()));
        assert_eq!(edit.text(), "你", "only the three-byte character fits");
    }

    #[test]
    fn a_disabled_or_read_only_field_rejects_edits_but_allows_navigation() {
        let mut edit = TextEditState::with_text("locked");
        let mut disabled = desc();
        disabled.disabled = true;
        assert!(
            !reduce_text_input(&disabled, &mut edit, &TextInputEvent::Insert("x".into())).changed
        );
        assert_eq!(edit.text(), "locked");

        let mut read_only = desc();
        read_only.read_only = true;
        assert!(!reduce_text_input(&read_only, &mut edit, &TextInputEvent::Backspace).changed);
        assert!(reduce_text_input(&read_only, &mut edit, &TextInputEvent::SelectAll).changed);
        assert_eq!(edit.selected_text(), Some("locked"));
    }

    #[test]
    fn the_emitter_reports_a_text_input_role_and_value() {
        let mut edit = TextEditState::with_text("abc");
        edit.anchor = 1;
        let widget = TextInputWidget {
            node: 5,
            desc: desc(),
            edit,
            visual: TextInputVisual::default(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        harness.focus();
        let node = harness.output.a11y.as_ref().expect("a11y node");
        assert_eq!(node.role, Role::TextInput);
        assert_eq!(node.value, "abc");
        assert!(node.actions.contains(A11yAction::SetValue));
        assert_eq!(
            harness.output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let edit = TextEditState::with_text("abc");
        let glyphs: Vec<GlyphQuad> = Vec::new();
        let mut counts = Vec::new();
        for caret_visible in [false, true] {
            let visual = TextInputVisual {
                caret: Rect::new(30.0, 24.0, 1.0, 20.0),
                selection: Some(Rect::new(24.0, 24.0, 20.0, 20.0)),
                caret_visible,
                ..TextInputVisual::default()
            };
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let state = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_text_input(
                                9,
                                &desc(),
                                state,
                                &edit,
                                &visual,
                                &mut glyphs.clone(),
                            );
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

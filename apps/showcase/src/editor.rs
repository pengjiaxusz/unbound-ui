//! 展厅各单行文本编辑器共用的测量辅助。
//!
//! 编辑状态本身来自 `unbound-widgets`，这里只补齐“字节偏移 ↔ 像素位置”的换算，
//! 让搜索框、演示输入框和 FPS 输入框用同一套逻辑绘制选区、插入符与组合下划线。

use unbound_widgets::text_input::TextEditState;

use crate::gallery::Labels;

/// 光标在文本自身坐标空间中的水平位置。
#[must_use]
pub fn caret_x(labels: &mut Labels, text: &str, cursor: usize, font_size: f32) -> f32 {
    let cursor = cursor.min(text.len());
    labels.measure(&text[..cursor], font_size).x
}

/// 将局部横坐标换算为最近的字符边界字节偏移。
#[must_use]
pub fn byte_at_x(
    labels: &mut Labels,
    text: &str,
    font_size: f32,
    local_x: f32,
    scroll_x: f32,
) -> usize {
    let target = (local_x + scroll_x).max(0.0);
    let mut previous_offset = 0usize;
    let mut previous_width = 0.0f32;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let width = labels.measure(&text[..next], font_size).x;
        if width > target {
            return if (target - previous_width) <= (width - target) {
                previous_offset
            } else {
                next
            };
        }
        previous_offset = next;
        previous_width = width;
    }
    text.len()
}

/// 选区在文本自身坐标空间中的横向范围。
#[must_use]
pub fn selection_span(
    labels: &mut Labels,
    edit: &TextEditState,
    font_size: f32,
) -> Option<(f32, f32)> {
    let (start, end) = edit.selection()?;
    let text = edit.text();
    let x0 = caret_x(labels, text, start, font_size);
    let x1 = caret_x(labels, text, end, font_size);
    Some((x0, x1))
}

/// IME 组合区间在文本自身坐标空间中的横向范围。
#[must_use]
pub fn preedit_span(
    labels: &mut Labels,
    edit: &TextEditState,
    font_size: f32,
) -> Option<(f32, f32)> {
    let (start, end) = edit.preedit()?;
    if start == end {
        return None;
    }
    let text = edit.text();
    let x0 = caret_x(labels, text, start, font_size);
    let x1 = caret_x(labels, text, end, font_size);
    Some((x0, x1))
}

/// 指针局部横坐标对应的字节偏移（已计入水平滚动）。
#[must_use]
pub fn pointer_offset(
    labels: &mut Labels,
    edit: &TextEditState,
    font_size: f32,
    local_x: f32,
) -> usize {
    byte_at_x(labels, edit.text(), font_size, local_x, edit.scroll_x())
}

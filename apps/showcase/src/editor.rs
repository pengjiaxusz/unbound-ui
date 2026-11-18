//! 展厅各单行文本编辑器共用的测量辅助。
//!
//! 编辑状态本身来自 `unbound-widgets`，这里只补齐“字节偏移 ↔ 像素位置”的换算，
//! 让搜索框、演示输入框和 FPS 输入框用同一套逻辑绘制选区、插入符与组合下划线。

use unbound_math::Rect;
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

/// 组合下划线的绘制矩形（文本自身坐标空间）。
///
/// 纵向按**基线**定位，而非字形墨水框：拼音字母与 CJK 的墨水底边就贴在基线上，
/// 以墨水下沿为基准会让下划线压进字形最后一两个像素，看起来像被文字吞掉。
///
/// `text_rect` 必须是传给 [`crate::canvas::Canvas::layout_text`] 的同一个矩形
/// （横向已扣除滚动偏移），这样下划线才能与字形共享同一条基线。
#[must_use]
pub fn preedit_underline(
    labels: &mut Labels,
    edit: &TextEditState,
    font_size: f32,
    text_rect: Rect,
) -> Option<Rect> {
    let (from, to) = preedit_span(labels, edit, font_size)?;
    let metrics = labels.metrics(edit.text(), font_size);
    let baseline =
        text_rect.origin.y + (text_rect.size.y - metrics.line_height()) * 0.5 + metrics.ascent;
    Some(Rect::new(
        text_rect.origin.x + from,
        baseline + 1.0,
        (to - from).max(1.0),
        1.0,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_widgets::text_input::{
        reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    };

    /// 组合下划线必须落在基线下方，而不是压进拼音字母的墨水底边。
    ///
    #[test]
    fn the_composition_underline_sits_below_the_baseline() {
        let mut labels = crate::gallery::Labels::new();
        let mut edit = TextEditState::new();
        let desc = TextInputDesc::new(Rect::new(0.0, 0.0, 1.0, 1.0));
        reduce_text_input(
            &desc,
            &mut edit,
            &TextInputEvent::ImePreedit {
                text: "nihao".into(),
                cursor: None,
            },
        );

        let text_rect = Rect::new(10.0, 4.0, 200.0, 28.0);
        let underline = preedit_underline(&mut labels, &edit, 11.0, text_rect)
            .expect("an active composition must produce an underline");

        let metrics = labels.metrics(edit.text(), 11.0);
        let baseline =
            text_rect.origin.y + (text_rect.size.y - metrics.line_height()) * 0.5 + metrics.ascent;
        assert!(
            underline.origin.y > baseline,
            "underline y {} must sit below baseline {baseline}",
            underline.origin.y
        );
        assert!((underline.origin.x - text_rect.origin.x).abs() < 0.01);
        assert!(underline.size.x > 0.0);
        assert_eq!(underline.size.y, 1.0);
    }
}

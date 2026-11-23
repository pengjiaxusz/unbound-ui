//! 多行文本编辑器：跨折行的垂直光标移动。
//!
//! [`TextEditState`] 持有文本缓冲区与所有 UTF-8 规则；本模块仅补充*多行*输入框所需的功能：
//!
//! * 换行符插入归约器；
//! * 基于调用方传入视觉行列表的垂直移动（带目标列记忆功能），
//!   使光标穿越段落时不会向左漂移；以及
//! * 垂直滚动弹簧。
//!
//! 调用方仍负责文本塑形。它以字节区间（[`AreaLine`]）与合并后的选区槽位形式传入折行结果，
//! 既使本模块免于字体依赖，又能生成与文本栈其余部分一致的圆角选区几何形状。

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::selection::RoundedSlot;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::scroll::{
    emit_scrollbar, ScrollBarDesc, ScrollPointerEvent, ScrollState, ScrollViewDesc,
};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{
    floor_boundary, focus_underline, reduce_text_input, TextEditState, TextInputDesc,
    TextInputEvent,
};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 边框与文本之间的水平内边距（左）。
pub const AREA_PADDING: f32 = 10.0;
/// 默认圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const AREA_RADIUS: f32 = 4.0;
/// 右侧内边距。对齐 WinUI `TextControlThemePadding = 10,5,6,6` 的右值 6。
const AREA_PADDING_RIGHT: f32 = 6.0;
/// 顶部内边距。对齐 WinUI `TextControlThemePadding` 的上值 5。
const AREA_PADDING_TOP: f32 = 5.0;
/// 底部内边距。对齐 WinUI `TextControlThemePadding` 的下值 6。
const AREA_PADDING_BOTTOM: f32 = 6.0;

/// 以字节区间表示的单条视觉行（已完成折行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaLine {
    /// 该行的起始字节偏移。
    pub start: usize,
    /// 该行末尾字节之后的偏移（开区间）。
    pub end: usize,
}

impl AreaLine {
    /// 创建覆盖 `start..end` 的行。
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// 按 `\n` 拆分 `text`（忽略 `\r`）。
///
/// 结果至少包含一行，因此 `"a\n"` 会以一个空行结尾——光标可以停留在那里，
/// 符合编辑器的常规行为。
#[must_use]
pub fn logical_lines(text: &str) -> Vec<AreaLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            let end = if index > start && text.as_bytes()[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push(AreaLine::new(start, end));
            start = index + 1;
        }
    }
    lines.push(AreaLine::new(start, text.len()));
    lines
}

/// 多行文本输入框的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TextAreaDesc {
    /// 输入框矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 缓冲区为空时显示的占位文本。
    pub placeholder: String,
    /// 是否禁用交互。
    pub disabled: bool,
    /// 文本可选中但不可编辑。
    pub read_only: bool,
    /// 缓冲区最大字节长度。
    pub max_bytes: usize,
    /// 绘制顺序键。
    pub z: u16,
    /// 圆角半径。
    pub radius: f32,
    /// 调用方塑形所用的行高。
    pub line_height: f32,
}

impl TextAreaDesc {
    /// 使用默认尺寸创建的输入框。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            read_only: false,
            max_bytes: 1 << 20,
            z: 725,
            radius: AREA_RADIUS,
            line_height: 20.0,
        }
    }

    /// 文本绘制的目标矩形。
    ///
    /// 内边距对齐 WinUI `TextControlThemePadding = 10,5,6,6`。
    #[must_use]
    pub fn text_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + AREA_PADDING,
            self.rect.origin.y + AREA_PADDING_TOP,
            (self.rect.size.x - AREA_PADDING - AREA_PADDING_RIGHT).max(0.0),
            (self.rect.size.y - AREA_PADDING_TOP - AREA_PADDING_BOTTOM).max(0.0),
        )
    }

    fn editable(&self) -> bool {
        !self.disabled && !self.read_only
    }
}

/// 多行文本输入框的实时状态。
#[derive(Clone, Debug)]
pub struct TextAreaState {
    /// 共用的文本缓冲区、光标与选区状态。
    pub edit: TextEditState,
    /// 垂直移动时尝试保持的目标列。
    preferred_column: Option<usize>,
    /// 垂直滚动偏移量。
    pub scroll: ScrollState,
    /// 最近一次同步的完整排版内容高度（支持软折行）。
    content_height: Option<f32>,
    /// 指针是否悬停在按需叠加滚动条热区上。
    scrollbar_hovered: bool,
    /// 滚动条滑块拖拽起始快照 `(start_pointer_y, start_offset_y)`。
    scrollbar_drag: Option<(f32, f32)>,
}

impl TextAreaState {
    /// 创建空输入框状态。
    #[must_use]
    pub fn new() -> Self {
        Self {
            edit: TextEditState::new(),
            preferred_column: None,
            scroll: ScrollState::new(),
            content_height: None,
            scrollbar_hovered: false,
            scrollbar_drag: None,
        }
    }

    /// 创建持有初始文本 `text` 的输入框状态。
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            edit: TextEditState::with_text(text),
            preferred_column: None,
            scroll: ScrollState::new(),
            content_height: None,
            scrollbar_hovered: false,
            scrollbar_drag: None,
        }
    }

    /// 文本缓冲区引用。
    #[must_use]
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// 若正在进行垂直移动，返回记忆的目标列。
    #[must_use]
    pub fn preferred_column(&self) -> Option<usize> {
        self.preferred_column
    }

    /// 清除记忆的目标列；水平编辑会移动锚点。
    pub fn clear_preferred(&mut self) {
        self.preferred_column = None;
    }

    /// 通过点击 `local_pos`（相对于 text_rect 原点）定位光标。
    pub fn pointer_down_at(
        &mut self,
        shaped: &unbound_text::ShapedText,
        local_pos: Vec2,
        extend: bool,
    ) {
        self.preferred_column = None;
        let offset = shaped.hit_byte_index(self.edit.text(), local_pos + self.scroll.offset());
        self.edit.set_cursor(offset, extend);
    }

    /// 拖拽到 `local_pos`（相对于 text_rect 原点）时扩展选区。
    pub fn drag_to(&mut self, shaped: &unbound_text::ShapedText, local_pos: Vec2) {
        self.preferred_column = None;
        let offset = shaped.hit_byte_index(self.edit.text(), local_pos + self.scroll.offset());
        self.edit.set_cursor(offset, true);
    }

    /// 在光标处插入换行符。
    pub fn insert_newline(&mut self, desc: &TextAreaDesc) -> bool {
        if !desc.editable() {
            return false;
        }
        self.preferred_column = None;
        self.edit.insert("\n", desc.max_bytes);
        true
    }

    /// 将文本事件路由至缓冲区，并在任何水平移动光标的事件后清除目标列。
    pub fn handle_event(
        &mut self,
        desc: &TextAreaDesc,
        event: &TextInputEvent,
    ) -> crate::text_input::TextInputOutcome {
        if matches!(
            event,
            TextInputEvent::MoveLeft { .. }
                | TextInputEvent::MoveRight { .. }
                | TextInputEvent::MoveWordLeft { .. }
                | TextInputEvent::MoveWordRight { .. }
                | TextInputEvent::Home { .. }
                | TextInputEvent::End { .. }
        ) {
            self.preferred_column = None;
        }
        let mut editor = TextInputDesc::new(desc.rect);
        editor.label = desc.label.clone();
        editor.disabled = desc.disabled;
        editor.read_only = desc.read_only;
        editor.max_bytes = desc.max_bytes;
        reduce_text_input(&editor, &mut self.edit, event)
    }

    /// 包含 `cursor` 的行索引。
    #[must_use]
    pub fn line_index(lines: &[AreaLine], cursor: usize) -> Option<usize> {
        if lines.is_empty() {
            return None;
        }
        lines
            .iter()
            .position(|line| cursor >= line.start && cursor <= line.end)
            .or(Some(lines.len() - 1))
    }

    /// 光标字节偏移量。
    #[must_use]
    pub fn caret(&self) -> usize {
        self.edit.cursor()
    }

    /// 将光标向上或向下移动一个视觉行。
    ///
    /// 字节列会被记忆（`preferred_column`），因此光标从长行移入短行再移回时，
    /// 会落回起始列而非停留在短行末尾。
    pub fn move_vertical(&mut self, lines: &[AreaLine], delta: isize, extend: bool) -> bool {
        if lines.is_empty() {
            return false;
        }
        let cursor = self.caret();
        let Some(current) = Self::line_index(lines, cursor) else {
            return false;
        };
        let line = lines[current];
        let column = self
            .preferred_column
            .unwrap_or_else(|| cursor.saturating_sub(line.start));
        self.preferred_column = Some(column);
        let target = (current as isize + delta).clamp(0, lines.len() as isize - 1) as usize;
        if target == current {
            return false;
        }
        let target_line = lines[target];
        let offset = floor_boundary(
            self.edit.text(),
            (target_line.start + column).min(target_line.end),
        );
        self.edit.set_cursor(offset, extend);
        true
    }

    /// 按整段逻辑段落移动（`Ctrl+Up` / `Ctrl+Down`）。
    pub fn move_paragraph(&mut self, delta: isize, extend: bool) -> bool {
        let lines = logical_lines(self.edit.text());
        self.move_vertical(&lines, delta, extend)
    }

    /// 显式同步外部排版测量得到的内容总高度（用于软折行场景）。
    pub fn set_content_height(&mut self, height: f32) {
        self.content_height = Some(height.max(0.0));
    }

    /// 结合逻辑行数与外部排版高度估算当前内容总高度。
    #[must_use]
    pub fn effective_content_height(&self, desc: &TextAreaDesc) -> f32 {
        let logical_height = logical_lines(self.edit.text()).len() as f32 * desc.line_height;
        self.content_height
            .map_or(logical_height, |h| h.max(logical_height))
    }

    /// 更新垂直叠加滚动条热区悬停状态。
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) {
        self.scrollbar_hovered = hovered;
    }

    /// 当前是否正在拖拽垂直叠加滚动条滑块。
    #[must_use]
    pub fn is_dragging_scrollbar(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    /// 构造宿主在 `TextAreaDesc::text_rect()` 内的垂直按需叠加滚动条描述符。
    #[must_use]
    pub fn scrollbar_desc(&self, desc: &TextAreaDesc) -> ScrollBarDesc {
        let mut bar = ScrollBarDesc::vertical(
            desc.text_rect(),
            self.effective_content_height(desc),
            self.scroll.offset().y,
        );
        bar.z = desc.z.saturating_add(1);
        bar.layer = CONTROL_LAYER;
        bar.hovered = self.scrollbar_hovered;
        bar.dragging = self.scrollbar_drag.is_some();
        bar
    }

    /// 归约作用于多行文本区垂直叠加滚动条的指针手势（滑块拖拽与点击轨道翻页）。
    pub fn reduce_scrollbar_pointer(
        &mut self,
        desc: &TextAreaDesc,
        content_height: f32,
        event: ScrollPointerEvent,
    ) -> bool {
        self.content_height = Some(content_height.max(0.0));
        let viewport = desc.text_rect();
        let scroll_desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
        match event {
            ScrollPointerEvent::Press(pos) => {
                let bar = self.scrollbar_desc(desc);
                if !bar.is_overflowing() {
                    return false;
                }
                if bar.thumb_rect().contains_point(pos) {
                    self.scrollbar_drag = Some((pos.y, self.scroll.offset().y));
                    self.scroll.reveal_scrollbar();
                    return true;
                }
                if let Some(next_y) = bar.page_offset_for_track_click(pos) {
                    self.scroll.scroll_to(&scroll_desc, Vec2::new(0.0, next_y));
                    return true;
                }
                false
            }
            ScrollPointerEvent::Drag(pos) => {
                let Some((start_pointer, start_offset)) = self.scrollbar_drag else {
                    return false;
                };
                let bar = self.scrollbar_desc(desc);
                if !bar.is_overflowing() {
                    return false;
                }
                let next_y = bar.offset_for_drag(start_offset, start_pointer, pos.y);
                self.scroll.jump_to(&scroll_desc, Vec2::new(0.0, next_y));
                true
            }
            ScrollPointerEvent::Release => self.scrollbar_drag.take().is_some(),
        }
    }

    /// 按像素增量滚动并夹紧到内容范围。
    pub fn scroll_by(&mut self, desc: &TextAreaDesc, dy: f32, content_height: f32) -> bool {
        self.content_height = Some(content_height.max(0.0));
        let viewport = desc.text_rect();
        let scroll_desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
        let before = self.scroll.target().y;
        self.scroll.scroll_by(&scroll_desc, Vec2::new(0.0, dy));
        (self.scroll.target().y - before).abs() > f32::EPSILON
    }

    /// 推进滚动弹簧动画。
    pub fn step(&mut self, desc: &TextAreaDesc, dt: f32, content_height: f32) -> bool {
        self.content_height = Some(content_height.max(0.0));
        let viewport = desc.text_rect();
        let scroll_desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
        self.scroll.step(&scroll_desc, dt)
    }
}

impl Default for TextAreaState {
    fn default() -> Self {
        Self::new()
    }
}

/// 输出器绘制所需的已测量几何信息。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextAreaVisual {
    /// 光标矩形。
    pub caret: Rect,
    /// 按绘制顺序排列的合并选区槽位。
    pub selection: Vec<RoundedSlot>,
    /// 光标是否处于闪烁可见阶段。
    pub caret_visible: bool,
}

/// 输出输入框外观、选区、光标与语义节点。
#[must_use]
pub fn emit_text_area(
    node: u64,
    desc: &TextAreaDesc,
    state: &TextAreaState,
    control: ControlState,
    visual: &TextAreaVisual,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(6 + visual.selection.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    // 填充槽位对齐 WinUI：静息 `ControlFillColorDefault`、悬停
    // `ControlFillColorSecondary`、禁用 `ControlFillColorDisabled`。
    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::CONTROL_FILL
    };
    primitives.push(surface(rect, desc.radius, fill, desc.z));
    // 描边 `ControlStrokeColorDefault` 为 1px（`TextControlBorderThemeThickness = 1`）。
    let border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    // 聚焦态底部 2px 强调条（`TextControlBorderThemeThicknessFocused = 1,1,1,2`）。
    // 始终写入图元池，仅调整不透明度。
    primitives.push(focus_underline(
        rect,
        desc.radius,
        desc.z,
        if control.focused && !control.disabled {
            1.0
        } else {
            0.0
        },
    ));

    for slot in &visual.selection {
        primitives.push(instance(
            slot.rect,
            slot.radii,
            Token::SELECTION,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::InnerShadow,
            desc.z,
        ));
    }

    // 光标槽位始终存在；仅通过不透明度控制可见性（与 `emit_text_input` 一致），
    // 因此聚焦/闪烁绝不会改变图元池结构。WinUI 的插入符跟随正文色。
    let mut caret = surface(visual.caret, 0.0, Token::TEXT_PRIMARY, desc.z);
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        caret.depth_u16(),
        if control.focused && !control.disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else if state.edit.text().is_empty() && !desc.placeholder.is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        desc.radius + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    // 按需叠加垂直滚动条：宿主在 `text_rect()` 右缘，未溢出零图元。
    let mut bar = state.scrollbar_desc(desc);
    bar.hovered = bar.hovered
        || (!control.disabled
            && matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed));
    bar.disabled = control.disabled;
    let bar_out = emit_scrollbar(node, &bar);
    primitives.extend(bar_out.output.primitives);

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::MultilineTextInput, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.edit.text().to_owned();
    a11y.description = desc.placeholder.clone();
    a11y.actions.insert(A11yAction::SetValue);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    };
    if !control.disabled {
        for sub_hit in bar_out.hits {
            out.sub_hits.push(sub_hit);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TextAreaDesc {
        TextAreaDesc::new(Rect::new(10.0, 10.0, 260.0, 120.0))
    }

    #[test]
    fn logical_lines_end_with_an_empty_line_after_a_newline() {
        let lines = logical_lines("ab\ncd\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], AreaLine::new(3, 5));
        assert_eq!(lines[2], AreaLine::new(6, 6));
        assert_eq!(logical_lines("").len(), 1);
    }

    #[test]
    fn inserting_a_newline_splits_the_buffer() {
        let desc = desc();
        let mut state = TextAreaState::with_text("ab");
        state.edit.set_cursor(1, false);
        assert!(state.insert_newline(&desc));
        assert_eq!(state.text(), "a\nb");
        assert_eq!(state.caret(), 2);
    }

    #[test]
    fn vertical_movement_keeps_the_byte_column() {
        let mut state = TextAreaState::with_text("abcdef\nxy\nabcdef");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(5, false);
        assert!(state.move_vertical(&lines, 1, false));
        assert_eq!(state.caret(), 9, "the short line clamps to its end");
        assert!(state.move_vertical(&lines, 1, false));
        assert_eq!(state.caret(), 15, "the column is restored");
        assert!(state.move_vertical(&lines, -1, false));
        assert_eq!(state.caret(), 9);
        assert!(state.move_vertical(&lines, -3, false));
        assert_eq!(state.caret(), 5);
    }

    #[test]
    fn shifting_extends_the_selection_vertically() {
        let mut state = TextAreaState::with_text("aa\nbb\ncc");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(1, false);
        assert!(state.move_vertical(&lines, 1, true));
        assert_eq!(state.edit.selected_text(), Some("a\nb"));
    }

    #[test]
    fn horizontal_events_clear_the_target_column() {
        let desc = desc();
        let mut state = TextAreaState::with_text("aa\nbb");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(1, false);
        state.move_vertical(&lines, 1, false);
        assert!(state.preferred_column().is_some());
        state.handle_event(&desc, &TextInputEvent::MoveLeft { extend: false });
        assert!(state.preferred_column().is_none());
    }

    #[test]
    fn a_read_only_field_rejects_newlines() {
        let mut desc = desc();
        desc.read_only = true;
        let mut state = TextAreaState::with_text("a");
        assert!(!state.insert_newline(&desc));
        assert_eq!(state.text(), "a");
    }

    #[test]
    fn scrolling_clamps_to_the_content_height() {
        let desc = desc();
        let mut state = TextAreaState::new();
        let content = 1000.0;
        assert!(state.scroll_by(&desc, 5000.0, content));
        assert!(state.scroll.target().y > 0.0);
        assert!(state.scroll.target().y <= content);
        for _ in 0..120 {
            state.step(&desc, 1.0 / 60.0, content);
        }
        assert!(state.scroll.offset().y <= content);
        assert!(state.scroll_by(&desc, -5000.0, content));
        assert_eq!(state.scroll.target().y, 0.0);
    }

    #[test]
    fn the_emitter_reports_a_multiline_role_and_value() {
        let state = TextAreaState::with_text("one\ntwo");
        let visual = TextAreaVisual {
            caret: Rect::new(20.0, 20.0, 1.0, 18.0),
            selection: vec![RoundedSlot {
                rect: Rect::new(20.0, 20.0, 40.0, 18.0),
                radii: [3.0; 4],
            }],
            caret_visible: true,
        };
        let output = emit_text_area(
            5,
            &desc(),
            &state,
            ControlState::default(),
            &visual,
            &mut [],
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::MultilineTextInput);
        assert_eq!(a11y.value, "one\ntwo");
        // 表面、边框、聚焦强调条、焦点环以及始终存在的光标槽位。
        assert_eq!(output.primitives.len(), 5 + visual.selection.len());
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = TextAreaState::with_text("one\ntwo");
        let mut counts = Vec::new();
        for caret_visible in [false, true] {
            let visual = TextAreaVisual {
                caret: Rect::new(20.0, 20.0, 1.0, 18.0),
                selection: vec![RoundedSlot {
                    rect: Rect::new(20.0, 20.0, 40.0, 18.0),
                    radii: [3.0; 4],
                }],
                caret_visible,
            };
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output =
                                emit_text_area(5, &desc, &state, control, &visual, &mut []);
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

    #[test]
    fn text_area_overlay_scrollbar_emits_only_on_overflow_and_supports_drag_and_paging() {
        use crate::scroll::{
            scrollbar_thumb_node, scrollbar_track_node, ScrollbarAxis, SCROLLBAR_THUMB_IDLE_WIDTH,
            SCROLLBAR_THUMB_WIDTH,
        };

        let desc = desc();
        let node = 5u64;
        let thumb_node = scrollbar_thumb_node(node, ScrollbarAxis::Vertical);
        let track_node = scrollbar_track_node(node, ScrollbarAxis::Vertical);
        let visual = TextAreaVisual::default();

        // 未溢出：2 行（40px <= text_rect 109px）不发射滚动条图元与命中区。
        let short_state = TextAreaState::with_text("one\ntwo");
        let short_out = emit_text_area(
            node,
            &desc,
            &short_state,
            ControlState::default(),
            &visual,
            &mut [],
        );
        assert_eq!(short_out.primitives.len(), 5);
        assert!(short_out.sub_hits.is_empty());

        // 溢出：15 行（300px > text_rect 109px）发射 2 个稳定槽位与轨道/滑块命中区。
        let long_text = (0..15)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut tall_state = TextAreaState::with_text(long_text);
        let idle_out = emit_text_area(
            node,
            &desc,
            &tall_state,
            ControlState::default(),
            &visual,
            &mut [],
        );
        assert_eq!(idle_out.primitives.len(), 7);
        let track_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == track_node)
            .expect("text area track hit");
        let thumb_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == thumb_node)
            .expect("text area thumb hit");
        assert!(desc.text_rect().contains_rect(&track_hit.bounds));
        assert!(desc.text_rect().contains_rect(&thumb_hit.bounds));
        assert_eq!(idle_out.primitives[5].opacity(), 0.0);
        assert!((idle_out.primitives[6].size[0] - SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01);

        // 悬停展开为 6px 粗滑块与半透明轨道胶囊，图元总数恒定。
        tall_state.set_scrollbar_hovered(true);
        let hovered_out = emit_text_area(
            node,
            &desc,
            &tall_state,
            ControlState::default(),
            &visual,
            &mut [],
        );
        assert_eq!(hovered_out.primitives.len(), 7);
        assert!(hovered_out.primitives[5].opacity() > 0.0);
        assert!((hovered_out.primitives[6].size[0] - SCROLLBAR_THUMB_WIDTH).abs() < 0.01);

        // 点击轨道翻页与拖拽滑块。
        let content_h = tall_state.effective_content_height(&desc);
        let track_click = Vec2::new(track_hit.bounds.center().x, track_hit.bounds.max().y - 4.0);
        assert!(tall_state.reduce_scrollbar_pointer(
            &desc,
            content_h,
            ScrollPointerEvent::Press(track_click)
        ));
        assert!(tall_state.scroll.target().y > 0.0);

        let drag_start = thumb_hit.bounds.center();
        assert!(tall_state.reduce_scrollbar_pointer(
            &desc,
            content_h,
            ScrollPointerEvent::Press(drag_start)
        ));
        assert!(tall_state.is_dragging_scrollbar());
        assert!(tall_state.reduce_scrollbar_pointer(
            &desc,
            content_h,
            ScrollPointerEvent::Drag(Vec2::new(drag_start.x, drag_start.y + 30.0))
        ));
        assert!(tall_state.scroll.offset().y > 0.0);
        assert!(tall_state.reduce_scrollbar_pointer(&desc, content_h, ScrollPointerEvent::Release));
        assert!(!tall_state.is_dragging_scrollbar());
    }
}

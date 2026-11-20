//! 可编辑代码编辑器（CodeEditor），支持行号槽、当前行高亮、选区、括号匹配、
//! 只读/可编辑模式边界、键盘光标移动、选区复制、行内查找高亮与撤回/重做栈。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::code_block::{highlight_line, CodeSpan, CODE_LINE_HEIGHT};
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 编辑器容器圆角，对齐 WinUI `ControlCornerRadius`（4）。
const EDITOR_RADIUS: f32 = 4.0;
/// 编辑器描边宽度，对齐 WinUI `TextControlBorderThemeThickness`（1）。
const EDITOR_BORDER: f32 = 1.0;

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditorSnapshot {
    source: String,
    cursor_line: usize,
    cursor_col: usize,
}

/// [`CodeEditorDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorState {
    /// 源码缓冲区。
    pub source: String,
    /// 0-based 当前光标所在行。
    pub cursor_line: usize,
    /// 0-based 当前光标所在列（按字符索引）。
    pub cursor_col: usize,
    /// 可选选区 `(start_line, start_col)..(end_line, end_col)`。
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// 只读模式标志（为 `true` 时允许移动光标、选区复制与查找，禁止文本写入与撤回/重做）。
    pub read_only: bool,
    /// 当前查找关键词。
    pub search_query: String,
    /// 查找命中列表 `(line, start_col, end_col)`。
    pub search_matches: Vec<(usize, usize, usize)>,
    /// 当前激活的查找命中索引。
    pub active_match: Option<usize>,
    undo_stack: Vec<EditorSnapshot>,
    redo_stack: Vec<EditorSnapshot>,
}

impl CodeEditorState {
    /// 使用源码文本创建可编辑状态。
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            cursor_line: 0,
            cursor_col: 0,
            selection: None,
            read_only: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            active_match: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// 切换只读/可编辑模式。
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// 按行切分源码（空源码返回单空行）。
    fn split_lines(&self) -> Vec<&str> {
        if self.source.is_empty() {
            vec![""]
        } else {
            self.source.split('\n').collect()
        }
    }

    fn line_char_len(line: &str) -> usize {
        line.chars().count()
    }

    fn line_col_to_byte(&self, target_line: usize, target_col: usize) -> usize {
        let mut offset = 0usize;
        for (idx, line) in self.source.split('\n').enumerate() {
            if idx == target_line {
                let col_byte = line
                    .char_indices()
                    .nth(target_col)
                    .map_or(line.len(), |(b, _)| b);
                return offset + col_byte;
            }
            offset += line.len() + 1;
        }
        self.source.len()
    }

    fn push_undo_snapshot(&mut self) {
        self.undo_stack.push(EditorSnapshot {
            source: self.source.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
        });
        self.redo_stack.clear();
    }

    fn update_selection(&mut self, prev: (usize, usize), extend: bool) {
        if extend {
            let anchor = self.selection.map_or(prev, |(start, _)| start);
            let head = (self.cursor_line, self.cursor_col);
            self.selection = if anchor == head {
                None
            } else {
                Some((anchor, head))
            };
        } else {
            self.selection = None;
        }
    }

    /// 光标向左移动一列（行首自动回退至上一行末尾）。
    pub fn move_left(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        // 先取出回绕目标行的长度，结束对 `self.source` 的不可变借用，再修改光标。
        let wrap_len = if self.cursor_col == 0 && self.cursor_line > 0 {
            let target = self.cursor_line - 1;
            let lines = self.split_lines();
            Some(Self::line_char_len(lines[target]))
        } else {
            None
        };
        if let Some(len) = wrap_len {
            self.cursor_line -= 1;
            self.cursor_col = len;
        } else if self.cursor_col > 0 {
            self.cursor_col -= 1;
        }
        self.update_selection(prev, extend_selection);
    }

    /// 光标向右移动一列（行尾自动步入下一行行首）。
    pub fn move_right(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        let (line_len, line_count) = {
            let lines = self.split_lines();
            let len = lines
                .get(self.cursor_line)
                .map_or(0, |l| Self::line_char_len(l));
            (len, lines.len())
        };
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < line_count {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.update_selection(prev, extend_selection);
    }

    /// 光标向上移动一行（列号自动钳制至目标行长度）。
    pub fn move_up(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        if self.cursor_line > 0 {
            let target = self.cursor_line - 1;
            let len = {
                let lines = self.split_lines();
                Self::line_char_len(lines[target])
            };
            self.cursor_line = target;
            self.cursor_col = self.cursor_col.min(len);
        }
        self.update_selection(prev, extend_selection);
    }

    /// 光标向下移动一行（列号自动钳制至目标行长度）。
    pub fn move_down(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        let len = {
            let lines = self.split_lines();
            (self.cursor_line + 1 < lines.len())
                .then(|| Self::line_char_len(lines[self.cursor_line + 1]))
        };
        if let Some(len) = len {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(len);
        }
        self.update_selection(prev, extend_selection);
    }

    /// 移动至当前行首。
    pub fn move_line_start(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        self.cursor_col = 0;
        self.update_selection(prev, extend_selection);
    }

    /// 移动至当前行尾。
    pub fn move_line_end(&mut self, extend_selection: bool) {
        let prev = (self.cursor_line, self.cursor_col);
        let lines = self.split_lines();
        self.cursor_col = lines
            .get(self.cursor_line)
            .map_or(0, |l| Self::line_char_len(l));
        self.update_selection(prev, extend_selection);
    }

    /// 提取当前选区内的文本（支持单行与跨行选区，用于复制到剪贴板）。
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let start_byte = self.line_col_to_byte(start.0, start.1);
        let end_byte = self.line_col_to_byte(end.0, end.1);
        if start_byte >= end_byte || end_byte > self.source.len() {
            return None;
        }
        Some(self.source[start_byte..end_byte].to_owned())
    }

    /// 在当前光标处插入文本（只读模式下拒绝修改并返回 `false`）。
    pub fn insert_text(&mut self, text: &str) -> bool {
        if self.read_only || text.is_empty() {
            return false;
        }
        self.push_undo_snapshot();
        let insert_at = if let Some((a, b)) = self.selection.take() {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            let sb = self.line_col_to_byte(start.0, start.1);
            let eb = self.line_col_to_byte(end.0, end.1);
            self.source.replace_range(sb..eb, "");
            self.cursor_line = start.0;
            self.cursor_col = start.1;
            sb
        } else {
            self.line_col_to_byte(self.cursor_line, self.cursor_col)
        };

        self.source.insert_str(insert_at, text);
        let added_lines: Vec<&str> = text.split('\n').collect();
        if added_lines.len() == 1 {
            self.cursor_col += added_lines[0].chars().count();
        } else {
            self.cursor_line += added_lines.len() - 1;
            self.cursor_col = added_lines.last().map_or(0, |l| l.chars().count());
        }
        true
    }

    /// 删除选区或光标前一个字符（只读模式下拒绝修改并返回 `false`）。
    pub fn backspace(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        if let Some((a, b)) = self.selection {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            self.push_undo_snapshot();
            let sb = self.line_col_to_byte(start.0, start.1);
            let eb = self.line_col_to_byte(end.0, end.1);
            self.source.replace_range(sb..eb, "");
            self.cursor_line = start.0;
            self.cursor_col = start.1;
            self.selection = None;
            return true;
        }
        let end_byte = self.line_col_to_byte(self.cursor_line, self.cursor_col);
        if end_byte == 0 {
            return false;
        }
        self.push_undo_snapshot();
        let prev_char_byte = self.source[..end_byte]
            .char_indices()
            .next_back()
            .map_or(0, |(b, _)| b);
        let removed = self.source[prev_char_byte..end_byte].to_owned();
        self.source.replace_range(prev_char_byte..end_byte, "");
        if removed == "\n" {
            self.cursor_line = self.cursor_line.saturating_sub(1);
            let line_prefix = self.source[..prev_char_byte]
                .rsplit('\n')
                .next()
                .unwrap_or("");
            self.cursor_col = line_prefix.chars().count();
        } else {
            self.cursor_col = self.cursor_col.saturating_sub(1);
        }
        true
    }

    /// 撤回上一步编辑操作（只读模式下不执行）。
    pub fn undo(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(EditorSnapshot {
            source: self.source.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
        });
        self.source = prev.source;
        self.cursor_line = prev.cursor_line;
        self.cursor_col = prev.cursor_col;
        self.selection = None;
        true
    }

    /// 重做最近一次被撤回的操作（只读模式下不执行）。
    pub fn redo(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(EditorSnapshot {
            source: self.source.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
        });
        self.source = next.source;
        self.cursor_line = next.cursor_line;
        self.cursor_col = next.cursor_col;
        self.selection = None;
        true
    }

    /// 在源码中执行行内查找（大小写不敏感），高亮并跳转至首个命中项，返回总命中数。
    pub fn find(&mut self, query: &str) -> usize {
        self.search_query = query.to_owned();
        self.search_matches.clear();
        self.active_match = None;
        if query.is_empty() {
            return 0;
        }
        let needle = query.to_ascii_lowercase();
        let needle_chars = query.chars().count();
        for (line_idx, line) in self.source.split('\n').enumerate() {
            let lower_line = line.to_ascii_lowercase();
            let mut search_from = 0usize;
            while let Some(rel) = lower_line[search_from..].find(&needle) {
                let match_byte = search_from + rel;
                let start_col = lower_line[..match_byte].chars().count();
                let end_col = start_col + needle_chars;
                self.search_matches.push((line_idx, start_col, end_col));
                search_from = match_byte + needle.len().max(1);
            }
        }
        if !self.search_matches.is_empty() {
            self.focus_match(0);
        }
        self.search_matches.len()
    }

    /// 跳转到下一处查找命中项（循环）。
    pub fn find_next(&mut self) -> Option<(usize, usize, usize)> {
        if self.search_matches.is_empty() {
            return None;
        }
        let next = self
            .active_match
            .map_or(0, |idx| (idx + 1) % self.search_matches.len());
        self.focus_match(next);
        self.search_matches.get(next).copied()
    }

    /// 跳转到上一处查找命中项（循环）。
    pub fn find_prev(&mut self) -> Option<(usize, usize, usize)> {
        if self.search_matches.is_empty() {
            return None;
        }
        let len = self.search_matches.len();
        let prev = self
            .active_match
            .map_or(len - 1, |idx| if idx == 0 { len - 1 } else { idx - 1 });
        self.focus_match(prev);
        self.search_matches.get(prev).copied()
    }

    /// 清空查找高亮状态。
    pub fn clear_find(&mut self) {
        self.search_query.clear();
        self.search_matches.clear();
        self.active_match = None;
    }

    fn focus_match(&mut self, index: usize) {
        if let Some(&(line, start_col, end_col)) = self.search_matches.get(index) {
            self.active_match = Some(index);
            self.cursor_line = line;
            self.cursor_col = end_col;
            self.selection = Some(((line, start_col), (line, end_col)));
        }
    }

    /// 使用 [`highlight_line`] 高亮每一行。
    #[must_use]
    pub fn highlight_spans(&self) -> Vec<Vec<CodeSpan>> {
        self.source.lines().map(highlight_line).collect()
    }

    /// 若光标处为括号，则在 `(cursor_line, cursor_col)` 周围查找匹配的 `()`、`[]` 或 `{}`。
    #[must_use]
    pub fn find_matching_bracket(&self) -> Option<(usize, usize)> {
        let lines: Vec<&str> = self.source.lines().collect();
        let line = lines.get(self.cursor_line)?;
        let ch = line.as_bytes().get(self.cursor_col).copied()?;
        let (open, close, forward) = match ch {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return None,
        };
        let mut depth = 0i32;
        if forward {
            for (li, l) in lines.iter().enumerate().skip(self.cursor_line) {
                let start_col = if li == self.cursor_line {
                    self.cursor_col
                } else {
                    0
                };
                for (ci, &b) in l.as_bytes().iter().enumerate().skip(start_col) {
                    if b == open {
                        depth += 1;
                    } else if b == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((li, ci));
                        }
                    }
                }
            }
        } else {
            for li in (0..=self.cursor_line).rev() {
                let l = lines[li];
                let end_col = if li == self.cursor_line {
                    self.cursor_col + 1
                } else {
                    l.len()
                };
                for ci in (0..end_col.min(l.len())).rev() {
                    let b = l.as_bytes()[ci];
                    if b == close {
                        depth += 1;
                    } else if b == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((li, ci));
                        }
                    }
                }
            }
        }
        None
    }

    /// 从当前交互状态构造渲染描述符。
    #[must_use]
    pub fn to_desc(&self, rect: Rect) -> CodeEditorDesc {
        let lines: Vec<String> = self.split_lines().into_iter().map(ToOwned::to_owned).collect();
        let single_line_sel = self.selection.and_then(|(a, b)| {
            let (s, e) = if a <= b { (a, b) } else { (b, a) };
            if s.0 == self.cursor_line && e.0 == self.cursor_line {
                Some((s.1, e.1))
            } else {
                None
            }
        });
        let mut desc = CodeEditorDesc::new(rect, lines.len(), self.cursor_line)
            .with_lines(lines)
            .with_cursor_col(self.cursor_col)
            .with_selection(single_line_sel)
            .with_matching_bracket(self.find_matching_bracket());
        desc.read_only = self.read_only;
        desc.search_match_count = self.search_matches.len();
        desc
    }
}

/// [`CodeEditor`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CodeEditorDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 总行数。
    pub line_count: usize,
    /// 0-based 当前光标行。
    pub cursor_line: usize,
    /// 0-based 当前光标列。
    pub cursor_col: usize,
    /// 用于语法显示的可选行切片。
    pub lines: Vec<String>,
    /// `cursor_line` 上可选的单行选区字节范围 `(start_col, end_col)`。
    pub selection: Option<(usize, usize)>,
    /// 可选的待高亮匹配括号坐标 `(line, col)`。
    pub matching_bracket: Option<(usize, usize)>,
    /// 是否处于只读模式。
    pub read_only: bool,
    /// 当前查找命中总数。
    pub search_match_count: usize,
    /// 垂直滚动偏移量。
    pub scroll: Vec2,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl CodeEditorDesc {
    /// 创建代码编辑器描述符。
    #[must_use]
    pub fn new(rect: Rect, line_count: usize, cursor_line: usize) -> Self {
        Self {
            rect,
            line_count: line_count.max(1),
            cursor_line,
            cursor_col: 0,
            lines: Vec::new(),
            selection: None,
            matching_bracket: None,
            read_only: false,
            search_match_count: 0,
            scroll: Vec2::ZERO,
            label: "Code Editor".to_owned(),
            z: 600,
        }
    }

    /// 附加源码行到描述符。
    #[must_use]
    pub fn with_lines(mut self, lines: Vec<String>) -> Self {
        self.line_count = lines.len().max(1);
        self.lines = lines;
        self
    }

    /// 附加光标列位置。
    #[must_use]
    pub fn with_cursor_col(mut self, col: usize) -> Self {
        self.cursor_col = col;
        self
    }

    /// 附加活动选区范围。
    #[must_use]
    pub fn with_selection(mut self, selection: Option<(usize, usize)>) -> Self {
        self.selection = selection;
        self
    }

    /// 附加匹配括号坐标。
    #[must_use]
    pub fn with_matching_bracket(mut self, bracket: Option<(usize, usize)>) -> Self {
        self.matching_bracket = bracket;
        self
    }
}

/// 发射固定形状的代码编辑器（背景 + 行号槽 + 当前行 + 选区 + 括号 + 光标 + 焦点环 = 7 个固定图元）。
#[must_use]
pub fn emit_code_editor(
    node: u64,
    desc: &CodeEditorDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    // 容器与只读代码块统一：CODE_BG 底、4px 圆角、1px `ControlStrokeColorDefault` 描边；
    // 悬停换成 `TextControlBorderBrushPointerOver`，聚焦改由外扩焦点环表达。
    let border = if state.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if state.phase == PointerPhase::Hovered {
        Token::INPUT_BORDER_HOVER
    } else {
        Token::CONTROL_STROKE
    };
    primitives.push(outlined(
        desc.rect,
        EDITOR_RADIUS,
        Token::CODE_BG,
        border,
        EDITOR_BORDER,
        0.0,
        desc.z,
    ));

    // 行号槽与只读代码块同色；向内缩进 2px，圆角取容器圆角减缩进（4 − 2 = 2）。
    let gutter_w = 34.0_f32.min(desc.rect.width() * 0.25);
    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + 2.0,
            desc.rect.origin.y + 2.0,
            gutter_w,
            (desc.rect.height() - 4.0).max(8.0),
        ),
        2.0,
        Token::SURFACE_RAISED,
        desc.z + 1,
    ));

    // 行高与只读代码块共用 `CODE_LINE_HEIGHT`（20px，匹配 14px 正文字号）。
    let line_h = CODE_LINE_HEIGHT.min(desc.rect.height() * 0.35);
    let visible_line = desc
        .cursor_line
        .saturating_sub((desc.scroll.y / line_h) as usize);
    let line_y = (desc.rect.origin.y + 6.0 + (visible_line % 16) as f32 * line_h)
        .min(desc.rect.max().y - line_h - 4.0);
    // 当前行高亮沿用 Win11 `SubtleFillColorSecondary`（HOVER 令牌），
    // 比选区更轻；显式选区仍用 SELECTION。
    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + gutter_w + 4.0,
            line_y,
            (desc.rect.width() - gutter_w - 8.0).max(12.0),
            line_h,
        ),
        2.0,
        Token::HOVER,
        desc.z + 1,
    ));

    // 选区条带（无选区时保持透明以维持固定形状）。
    let (sel_x, sel_w, sel_alpha) = if let Some((s, e)) = desc.selection {
        let lo = s.min(e) as f32 * 8.0;
        let hi = s.max(e) as f32 * 8.0;
        (
            desc.rect.origin.x + gutter_w + 10.0 + lo,
            (hi - lo).max(2.0),
            if e != s { 1.0 } else { 0.0 },
        )
    } else {
        (desc.rect.origin.x + gutter_w + 10.0, 0.0, 0.0)
    };
    let mut sel_prim = surface(
        Rect::new(sel_x, line_y + 1.0, sel_w, (line_h - 2.0).max(6.0)),
        2.0,
        Token::SELECTION,
        desc.z + 1,
    );
    sel_prim.z_and_opacity =
        unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(desc.z + 1, sel_alpha);
    primitives.push(sel_prim);

    // 匹配括号高亮（无匹配时保持透明以维持固定形状）。
    let (brk_rect, brk_alpha) = if let Some((bl, bc)) = desc.matching_bracket {
        let by = (desc.rect.origin.y + 6.0 + (bl % 16) as f32 * line_h)
            .min(desc.rect.max().y - line_h - 4.0);
        let bx = desc.rect.origin.x + gutter_w + 10.0 + bc as f32 * 8.0;
        (Rect::new(bx, by + 1.0, 8.0, (line_h - 2.0).max(6.0)), 1.0)
    } else {
        (
            Rect::new(desc.rect.origin.x, desc.rect.origin.y, 0.0, 0.0),
            0.0,
        )
    };
    let mut brk_prim = outlined(
        brk_rect,
        2.0,
        Token::NONE,
        Token::ACCENT,
        1.0,
        0.0,
        desc.z + 2,
    );
    brk_prim.z_and_opacity =
        unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(desc.z + 2, brk_alpha);
    primitives.push(brk_prim);

    let caret_x = (desc.rect.origin.x + gutter_w + 10.0 + desc.cursor_col as f32 * 8.0
        - desc.scroll.x)
        .clamp(desc.rect.origin.x + gutter_w + 6.0, desc.rect.max().x - 6.0);
    // 插入符跟随正文色（WinUI 的 Caret 使用文本前景色）。
    primitives.push(surface(
        Rect::new(caret_x, line_y + 2.0, 2.0, (line_h - 4.0).max(8.0)),
        1.0,
        Token::CODE_TEXT,
        desc.z + 2,
    ));

    primitives.push(focus_ring(
        desc.rect,
        EDITOR_RADIUS,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::CODE_TEXT,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect)
        .read_only(desc.read_only)
        .disabled(state.disabled);
    if !desc.lines.is_empty() {
        a11y.value = desc.lines.join("\n");
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
    fn code_editor_cursor_selection_find_undo_redo_and_read_only() {
        let mut ed = CodeEditorState::new("fn main() {\n    let x = foo(1);\n    let y = foo(2);\n}");

        // 括号匹配。
        ed.cursor_line = 0;
        ed.cursor_col = 10; // '{'
        assert_eq!(ed.find_matching_bracket(), Some((3, 0)));

        // 光标移动与选区复制。
        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.move_right(true);
        ed.move_right(true);
        assert_eq!(ed.selected_text().as_deref(), Some("fn"));

        // 行内查找与循环跳转。
        assert_eq!(ed.find("foo"), 2);
        assert_eq!(ed.active_match, Some(0));
        assert_eq!(ed.selected_text().as_deref(), Some("foo"));
        assert_eq!(ed.find_next(), Some((2, 12, 15)));
        assert_eq!(ed.find_prev(), Some((1, 12, 15)));

        // 编辑、撤回与重做。
        assert!(ed.insert_text("bar"));
        assert!(ed.source.contains("bar(1)"));
        assert!(ed.undo());
        assert!(ed.source.contains("foo(1)"));
        assert!(ed.redo());
        assert!(ed.source.contains("bar(1)"));

        // 只读模式下允许导航和查找，但拒绝编辑与撤回。
        ed.set_read_only(true);
        assert!(!ed.insert_text("nope"));
        assert!(!ed.backspace());
        assert!(!ed.undo());
        assert_eq!(ed.find("bar"), 1);
    }

    #[test]
    fn code_editor_headless_geometry() {
        let rect = Rect::new(20.0, 20.0, 480.0, 220.0);
        let ed = CodeEditorState::new("let alpha = 1;\nlet beta = 2;");
        let desc = ed.to_desc(rect);
        let out = emit_code_editor(1, &desc, ControlState::default(), &[]);
        assert_eq!(out.primitives.len(), 7);

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "CodeEditor",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

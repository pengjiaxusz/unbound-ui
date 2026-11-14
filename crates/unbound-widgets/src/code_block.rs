//! 语法高亮代码块（CodeBlock）。
//!
//! 该组件是只读查看器而非编辑器：仅持有几何布局、粗粒度词法分类器、
//! 高亮行背景条与复制控件。分类器刻意保持精简——仅识别关键字、数字、字符串、
//! 注释与调用点。完整语法分析属于语言服务而非 UI 组件；此处关注的是用正确的
//! 语义令牌绘制代码行并保持行号槽对齐。
//!
//! 复制操作是一个*请求*：[`CodeBlockState::mark_copied`] 记录上次确认复制的时间，
//! 实际写入剪贴板由应用程序执行。

use std::ops::Range;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 代码块的行高节奏。
pub const CODE_LINE_HEIGHT: f32 = 20.0;
/// 行号槽宽度。
pub const CODE_GUTTER: f32 = 44.0;
/// 复制确认提示的持续时长。
pub const COPY_CONFIRM_SECONDS: f32 = 1.4;

/// 粗粒度语法分类。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyntaxToken {
    /// 标识符、标点符号与空白字符。
    #[default]
    Plain,
    /// 语言关键字。
    Keyword,
    /// 类型名称（首字母大写的标识符或内置类型）。
    Type,
    /// 引号包裹的字符串。
    String,
    /// 单行注释。
    Comment,
    /// 数字字面量。
    Number,
    /// 后跟 `(` 的标识符（调用点）。
    Function,
}

impl SyntaxToken {
    /// 该语法类别绘制使用的颜色令牌。
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Plain => Token::CODE_TEXT,
            Self::Keyword => Token::PRIMARY,
            Self::Type => Token::WARNING,
            Self::String => Token::SUCCESS,
            Self::Comment => Token::TEXT_SECONDARY,
            Self::Number => Token::ACCENT,
            Self::Function => Token::ERROR,
        }
    }
}

/// 单行内的字节范围及其语法类别。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeSpan {
    /// 行内字节范围。
    pub range: Range<usize>,
    /// 语法类别。
    pub token: SyntaxToken,
}

/// 对单行代码进行词法分类。
#[must_use]
pub fn highlight_line(line: &str) -> Vec<CodeSpan> {
    const KEYWORDS: [&str; 24] = [
        "fn", "let", "mut", "if", "else", "match", "for", "while", "loop", "return", "struct",
        "enum", "impl", "trait", "pub", "use", "mod", "const", "static", "async", "await", "self",
        "Self", "where",
    ];
    let mut spans = Vec::new();
    let mut index = 0usize;
    let bytes = line.as_bytes();
    while index < bytes.len() {
        let rest = &line[index..];
        if rest.starts_with("//") {
            spans.push(CodeSpan {
                range: index..line.len(),
                token: SyntaxToken::Comment,
            });
            break;
        }
        if rest.starts_with('"') || rest.starts_with('\'') {
            let quote = rest.as_bytes()[0] as char;
            let end = rest[1..]
                .find(quote)
                .map_or(line.len(), |close| index + 1 + close + 1);
            spans.push(CodeSpan {
                range: index..end,
                token: SyntaxToken::String,
            });
            index = end;
            continue;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        if ch.is_ascii_digit() {
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '_')
                .map_or(line.len(), |close| index + close);
            spans.push(CodeSpan {
                range: index..end,
                token: SyntaxToken::Number,
            });
            index = end;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map_or(line.len(), |close| index + close);
            let word = &line[index..end];
            let after = line[end..].trim_start();
            let token = if KEYWORDS.contains(&word) {
                SyntaxToken::Keyword
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                SyntaxToken::Type
            } else if after.starts_with('(') {
                SyntaxToken::Function
            } else {
                SyntaxToken::Plain
            };
            spans.push(CodeSpan {
                range: index..end,
                token,
            });
            index = end;
            continue;
        }
        spans.push(CodeSpan {
            range: index..index + ch.len_utf8(),
            token: SyntaxToken::Plain,
        });
        index += ch.len_utf8();
    }
    spans
}

/// 代码块的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlockDesc {
    /// 整体代码块矩形。
    pub rect: Rect,
    /// 源码文本。
    pub text: String,
    /// 头部显示的语言标签。
    pub language: String,
    /// 起始可见行号。
    pub start_line: usize,
    /// 需要高亮条带标注的 1-based 行号列表。
    pub highlight_lines: Vec<usize>,
    /// 行号槽是否显示行号。
    pub show_numbers: bool,
    /// 是否显示复制控件。
    pub copyable: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl CodeBlockDesc {
    /// 创建带语言标签的代码块。
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            language: language.into(),
            start_line: 1,
            highlight_lines: Vec::new(),
            show_numbers: true,
            copyable: true,
            z: 520,
        }
    }

    /// 代码总行数。
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.text.lines().count()
    }

    /// 行号槽宽度（隐藏行号时为 `0`）。
    #[must_use]
    pub fn gutter_width(&self) -> f32 {
        if self.show_numbers {
            CODE_GUTTER
        } else {
            0.0
        }
    }

    /// 排除行号槽后的源码区域。
    #[must_use]
    pub fn code_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gutter_width(),
            self.rect.origin.y,
            (self.rect.size.x - self.gutter_width()).max(0.0),
            self.rect.size.y,
        )
    }

    /// 单个可见行的矩形区域。
    #[must_use]
    pub fn line_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + index as f32 * CODE_LINE_HEIGHT,
            self.rect.size.x,
            CODE_LINE_HEIGHT,
        )
    }

    /// 位于右上角的复制按钮。
    #[must_use]
    pub fn copy_rect(&self) -> Option<Rect> {
        self.copyable.then_some(Rect::new(
            self.rect.max().x - 32.0,
            self.rect.origin.y + 6.0,
            26.0,
            22.0,
        ))
    }

    /// 判断指定 1-based 行号是否处于高亮条带中。
    #[must_use]
    pub fn is_highlighted(&self, line: usize) -> bool {
        self.highlight_lines.contains(&line)
    }
}

/// 代码块的可变状态。
#[derive(Clone, Debug, Default)]
pub struct CodeBlockState {
    copied_at: Option<f32>,
}

impl CodeBlockState {
    /// 未处于复制确认状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录复制按钮在 `now` 秒时被按下。
    pub fn mark_copied(&mut self, now: f32) {
        self.copied_at = Some(now);
    }

    /// 判断复制确认提示是否仍可见。
    #[must_use]
    pub fn copied(&self, now: f32) -> bool {
        self.copied_at
            .is_some_and(|at| now - at < COPY_CONFIRM_SECONDS)
    }
}

/// 发射行号槽、高亮行条带、字形与复制按钮。
#[must_use]
pub fn emit_code_block(
    node: u64,
    desc: &CodeBlockDesc,
    state: &CodeBlockState,
    code_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + desc.highlight_lines.len() + code_glyphs.len());
    let mut payloads = Vec::with_capacity(code_glyphs.len());

    primitives.push(surface(desc.rect, 8.0, Token::CODE_BG, desc.z));
    if desc.show_numbers {
        primitives.push(surface(
            Rect::new(
                desc.rect.origin.x,
                desc.rect.origin.y,
                CODE_GUTTER,
                desc.rect.size.y,
            ),
            0.0,
            Token::SURFACE_SUNKEN,
            desc.z + 1,
        ));
        primitives.push(line(
            Rect::new(
                desc.rect.origin.x + CODE_GUTTER - 1.0,
                desc.rect.origin.y,
                1.0,
                desc.rect.size.y,
            ),
            Token::DIVIDER,
            desc.z + 2,
        ));
    }
    for index in 0..desc.text.lines().count() {
        if !desc.is_highlighted(desc.start_line + index) {
            continue;
        }
        primitives.push(surface(
            desc.line_rect(index),
            0.0,
            Token::SELECTION,
            desc.z + 1,
        ));
    }
    glyphs(
        code_glyphs,
        Token::CODE_TEXT,
        desc.z + 3,
        &mut primitives,
        &mut payloads,
    );

    if let Some(button) = desc.copy_rect() {
        let fill = if state.copied_at.is_some() {
            Token::SUCCESS_SOFT
        } else {
            Token::SURFACE_RAISED
        };
        primitives.push(surface(button, 5.0, fill, desc.z + 4));
        // 悬停底色始终保留在图元池中；仅改变其不透明度，因此悬停绝不会重排图元序列。
        let mut hover = surface(button, 5.0, Token::CONTROL_HOVER, desc.z + 5);
        hover.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
            hover.depth_u16(),
            if control.phase == crate::PointerPhase::Hovered {
                1.0
            } else {
                0.0
            },
        );
        primitives.push(hover);
    }
    primitives.push(focus_ring(
        desc.rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 6,
        control.focused,
    ));

    let mut a11y =
        A11yNode::new(Role::Code, format!("{} 代码块", desc.language), desc.rect).clickable();
    a11y.value = desc.text.clone();
    a11y.description = format!("{} 行", desc.line_count());
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rounded { radii: [8.0; 4] })
                .with_cursor(CursorKind::Text),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 实心线段图元。
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CodeBlockDesc {
        CodeBlockDesc::new(
            Rect::new(0.0, 0.0, 400.0, 120.0),
            "fn main() {\n    let x = 42;\n}",
            "rust",
        )
    }

    #[test]
    fn keywords_numbers_and_calls_are_classified() {
        let spans = highlight_line("let answer = compute(42);");
        let classes: Vec<SyntaxToken> = spans.iter().map(|span| span.token).collect();
        assert!(classes.contains(&SyntaxToken::Keyword));
        assert!(classes.contains(&SyntaxToken::Function));
        assert!(classes.contains(&SyntaxToken::Number));
    }

    #[test]
    fn comments_swallow_the_rest_of_the_line() {
        let spans = highlight_line("let x = 1; // note");
        let last = spans.last().expect("comment");
        assert_eq!(last.token, SyntaxToken::Comment);
        assert_eq!(&"let x = 1; // note"[last.range.clone()], "// note");
    }

    #[test]
    fn strings_keep_their_quotes() {
        let spans = highlight_line("let s = \"hi\";");
        let string = spans
            .iter()
            .find(|span| span.token == SyntaxToken::String)
            .expect("string");
        assert_eq!(&"let s = \"hi\";"[string.range.clone()], "\"hi\"");
    }

    #[test]
    fn the_gutter_can_be_hidden() {
        let mut desc = desc();
        assert_eq!(desc.gutter_width(), CODE_GUTTER);
        desc.show_numbers = false;
        assert_eq!(desc.gutter_width(), 0.0);
        assert_eq!(desc.code_rect().size.x, desc.rect.size.x);
    }

    #[test]
    fn line_rects_use_the_fixed_rhythm() {
        let desc = desc();
        assert_eq!(desc.line_count(), 3);
        assert_eq!(desc.line_rect(2).origin.y, CODE_LINE_HEIGHT * 2.0);
    }

    #[test]
    fn a_highlighted_line_is_matched_by_number() {
        let mut desc = desc();
        desc.start_line = 10;
        desc.highlight_lines = vec![11];
        assert!(desc.is_highlighted(11));
        assert!(!desc.is_highlighted(10));
    }

    #[test]
    fn the_copy_confirmation_expires() {
        let mut state = CodeBlockState::new();
        assert!(!state.copied(0.0));
        state.mark_copied(10.0);
        assert!(state.copied(10.5));
        assert!(!state.copied(12.0));
    }

    #[test]
    fn the_emitter_reports_a_code_region_with_the_source() {
        let desc = desc();
        let output = emit_code_block(
            2,
            &desc,
            &CodeBlockState::new(),
            &[],
            ControlState::default(),
        );
        let a11y = output.a11y.expect("code");
        assert_eq!(a11y.role, Role::Code);
        assert_eq!(a11y.value, desc.text);
        assert_eq!(a11y.description, "3 行");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = CodeBlockState::new();
        let mut counts = Vec::new();
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
                        let output = emit_code_block(2, &desc, &state, &[], control);
                        counts.push(output.primitives.len());
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

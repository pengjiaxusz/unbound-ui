//! Syntax-highlighted code block.
//!
//! The block is a viewer, not an editor: it owns geometry, a coarse token
//! classifier, the highlighted-line bands and the copy affordance. The
//! classifier is intentionally small — keywords, numbers, strings, comments
//! and call sites. A full grammar belongs in a language service, not in a
//! widget; what matters here is that a line can be painted with the right
//! semantic tokens and that the gutter stays aligned.
//!
//! Copy is a *request*: [`CodeBlockState::mark_copied`] remembers when the
//! affordance last confirmed, and the application performs the clipboard
//! write.

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

/// The line rhythm of a code block.
pub const CODE_LINE_HEIGHT: f32 = 20.0;
/// The width of the line-number gutter.
pub const CODE_GUTTER: f32 = 44.0;
/// How long the copy affordance confirms for.
pub const COPY_CONFIRM_SECONDS: f32 = 1.4;

/// One coarse syntax class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyntaxToken {
    /// Identifiers, punctuation and whitespace.
    #[default]
    Plain,
    /// A language keyword.
    Keyword,
    /// A type name (capitalised identifier or built-in).
    Type,
    /// A quoted string.
    String,
    /// A line comment.
    Comment,
    /// A numeric literal.
    Number,
    /// An identifier followed by `(`.
    Function,
}

impl SyntaxToken {
    /// The colour token the class paints with.
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

/// A byte range of one line and its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeSpan {
    /// Byte range inside the line.
    pub range: Range<usize>,
    /// The syntax class.
    pub token: SyntaxToken,
}

/// Classify one line.
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

/// The pure descriptor of a code block.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlockDesc {
    /// The whole block rectangle.
    pub rect: Rect,
    /// The source text.
    pub text: String,
    /// The language tag shown in the header.
    pub language: String,
    /// The first visible line number.
    pub start_line: usize,
    /// One-based line indices to band.
    pub highlight_lines: Vec<usize>,
    /// Whether the gutter shows line numbers.
    pub show_numbers: bool,
    /// Whether the copy affordance is visible.
    pub copyable: bool,
    /// The draw order key.
    pub z: u16,
}

impl CodeBlockDesc {
    /// A code block with a language tag.
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

    /// The number of lines.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.text.lines().count()
    }

    /// The line-number gutter width (`0` when numbers are hidden).
    #[must_use]
    pub fn gutter_width(&self) -> f32 {
        if self.show_numbers {
            CODE_GUTTER
        } else {
            0.0
        }
    }

    /// The source area, excluding the gutter.
    #[must_use]
    pub fn code_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gutter_width(),
            self.rect.origin.y,
            (self.rect.size.x - self.gutter_width()).max(0.0),
            self.rect.size.y,
        )
    }

    /// The rectangle of one visible line.
    #[must_use]
    pub fn line_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + index as f32 * CODE_LINE_HEIGHT,
            self.rect.size.x,
            CODE_LINE_HEIGHT,
        )
    }

    /// The copy button in the top-trailing corner.
    #[must_use]
    pub fn copy_rect(&self) -> Option<Rect> {
        self.copyable.then_some(Rect::new(
            self.rect.max().x - 32.0,
            self.rect.origin.y + 6.0,
            26.0,
            22.0,
        ))
    }

    /// Whether a one-based line number is banded.
    #[must_use]
    pub fn is_highlighted(&self, line: usize) -> bool {
        self.highlight_lines.contains(&line)
    }
}

/// The mutable state of a code block.
#[derive(Clone, Debug, Default)]
pub struct CodeBlockState {
    copied_at: Option<f32>,
}

impl CodeBlockState {
    /// No copy confirmation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember that the copy button was pressed at `now` seconds.
    pub fn mark_copied(&mut self, now: f32) {
        self.copied_at = Some(now);
    }

    /// Whether the copy confirmation is still visible.
    #[must_use]
    pub fn copied(&self, now: f32) -> bool {
        self.copied_at
            .is_some_and(|at| now - at < COPY_CONFIRM_SECONDS)
    }
}

/// Emit the gutter, highlight bands, glyphs and the copy button.
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
        if control.phase == crate::PointerPhase::Hovered {
            primitives.push(surface(button, 5.0, Token::CONTROL_HOVER, desc.z + 5));
        }
    }
    if control.focused {
        primitives.push(focus_ring(
            desc.rect,
            8.0,
            Token::FOCUS_RING,
            2.0,
            desc.z + 6,
            true,
        ));
    }

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
    }
}

/// A solid line primitive.
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
}

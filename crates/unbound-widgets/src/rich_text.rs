//! 富文本与 Markdown 块渲染。
//!
//! Markdown 被解析为多个 [`RichBlock`]，每个块携带行内 [`TextSpan`]——
//! 包括粗体、行内代码与链接——由文本引擎一次性排版。
//! 控件自身仅绘制块级表面背景；调用方传入塑形后的字形，与其他文本控件一致。
//!
//!
//! 链接在此不生成独立命中区域：单个段落通常包含多个链接，而 [`WidgetOutput`] 仅携带一个区域。
//! 相反，[`RichBlock::link_at`] 用于回答“该字节偏移处是哪个链接？”，
//! 再由应用程序将其转换为光标样式切换与点击动作。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::{GlyphQuad, TextSpan, TextStyle};
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 已解析 Markdown 的默认正文字号。
pub const RICH_TEXT_SIZE: f32 = 14.0;
/// 应用于标题层级的缩放比例。
pub const HEADING_SCALE: [f32; 6] = [1.9, 1.6, 1.4, 1.25, 1.15, 1.05];

/// Markdown 块的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// ATX 标题，`1..=6`。
    Heading(u8),
    /// 段落行。
    Paragraph,
    /// 无序列表项。
    Bullet,
    /// 引用块。
    Quote,
    /// 围栏代码块。
    Code,
}

impl BlockKind {
    /// 块的行高倍率。
    #[must_use]
    pub fn line_height(self) -> f32 {
        match self {
            Self::Heading(level) => {
                RICH_TEXT_SIZE * HEADING_SCALE[(level.clamp(1, 6) - 1) as usize] * 1.25
            }
            Self::Code => RICH_TEXT_SIZE * 1.45,
            _ => RICH_TEXT_SIZE * 1.5,
        }
    }

    /// 该块是否在文本背后绘制表面背景。
    #[must_use]
    pub fn background(self) -> Option<Token> {
        match self {
            Self::Code => Some(Token::CODE_BG),
            Self::Quote => Some(Token::SURFACE_SUNKEN),
            _ => None,
        }
    }
}

/// 单个已解析的块。
#[derive(Clone, Debug, PartialEq)]
pub struct RichBlock {
    /// 块类型。
    pub kind: BlockKind,
    /// 去除 Markdown 标记后的源文本。
    pub text: String,
    /// 覆盖 `text` 的行内片段列表。
    pub spans: Vec<TextSpan>,
}

impl RichBlock {
    /// 获取字节偏移处的链接。
    #[must_use]
    pub fn link_at(&self, offset: usize) -> Option<u32> {
        self.spans.iter().find_map(|span| {
            (span.link_id.is_some() && span.range.contains(&offset)).then_some(span.link_id?)
        })
    }

    /// 应用行内覆盖之前的块基础样式。
    #[must_use]
    pub fn base_style(&self) -> TextStyle {
        match self.kind {
            BlockKind::Heading(level) => {
                let mut style = TextStyle::sized(
                    RICH_TEXT_SIZE * HEADING_SCALE[(level.clamp(1, 6) - 1) as usize],
                );
                style.weight = 700;
                style
            }
            _ => TextStyle::sized(RICH_TEXT_SIZE),
        }
    }
}

/// 已解析的 Markdown 文档。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichDocument {
    /// 按源顺序排列的块列表。
    pub blocks: Vec<RichBlock>,
}

impl RichDocument {
    /// 链接总数。
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter(|span| span.link_id.is_some())
            .count()
    }
}

/// 解析 Markdown 子集：标题、段落、无序列表、引用块与围栏代码块。
/// 行内片段支持粗体、行内代码与链接。
#[must_use]
pub fn parse_markdown(markdown: &str) -> RichDocument {
    let mut blocks = Vec::new();
    let mut code: Option<String> = None;
    let mut link_id = 0u32;
    for line in markdown.lines() {
        if let Some(body) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                let mut text = std::mem::take(body);
                while text.ends_with('\n') {
                    text.pop();
                }
                let spans = parse_inline(&text, &TextStyle::sized(RICH_TEXT_SIZE), &mut link_id);
                blocks.push(RichBlock {
                    kind: BlockKind::Code,
                    text,
                    spans,
                });
                code = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            code = Some(String::new());
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let (kind, text) = classify(trimmed);
        let base = RichBlock {
            kind,
            text: text.to_owned(),
            spans: Vec::new(),
        }
        .base_style();
        let spans = parse_inline(text, &base, &mut link_id);
        blocks.push(RichBlock {
            kind,
            text: text.to_owned(),
            spans,
        });
    }
    if let Some(mut text) = code {
        while text.ends_with('\n') {
            text.pop();
        }
        let spans = parse_inline(&text, &TextStyle::sized(RICH_TEXT_SIZE), &mut link_id);
        blocks.push(RichBlock {
            kind: BlockKind::Code,
            text,
            spans,
        });
    }
    RichDocument { blocks }
}

fn classify(line: &str) -> (BlockKind, &str) {
    if let Some((hashes, rest)) = split_heading(line) {
        return (BlockKind::Heading(hashes), rest);
    }
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return (BlockKind::Bullet, rest);
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return (BlockKind::Quote, rest);
    }
    (BlockKind::Paragraph, line)
}

fn split_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line.get(hashes..)?;
    let rest = rest.strip_prefix(' ')?;
    Some((u8::try_from(hashes).ok()?, rest))
}

/// 将单行拆分为普通文本、粗体、行内代码与链接片段。
fn parse_inline(text: &str, base: &TextStyle, link_id: &mut u32) -> Vec<TextSpan> {
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut plain_start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if text[index..].starts_with("**") {
            if let Some(close) = text[index + 2..].find("**") {
                let end = index + 2 + close;
                push_plain(&mut spans, plain_start, index, base);
                let mut style = base.clone();
                style.weight = 700;
                spans.push(TextSpan {
                    range: index + 2..end,
                    style,
                    color: Token::TEXT_PRIMARY,
                    underline: false,
                    background: None,
                    link_id: None,
                });
                index = end + 2;
                plain_start = index;
                continue;
            }
        }
        if let Some(rest) = text[index..].strip_prefix('`') {
            if let Some(close) = rest.find('`') {
                let end = index + 1 + close;
                push_plain(&mut spans, plain_start, index, base);
                spans.push(TextSpan {
                    range: index + 1..end,
                    style: base.clone(),
                    color: Token::CODE_TEXT,
                    underline: false,
                    background: Some(Token::CODE_BG),
                    link_id: None,
                });
                index = end + 1;
                plain_start = index;
                continue;
            }
        }
        if let Some(rest) = text[index..].strip_prefix('[') {
            if let Some(close) = rest.find(']') {
                let label_end = index + 1 + close;
                if text[label_end..].starts_with("](") {
                    if let Some(paren) = text[label_end + 2..].find(')') {
                        let end = label_end + 2 + paren;
                        push_plain(&mut spans, plain_start, index, base);
                        *link_id += 1;
                        spans.push(TextSpan {
                            range: index + 1..label_end,
                            style: base.clone(),
                            color: Token::PRIMARY,
                            underline: true,
                            background: None,
                            link_id: Some(*link_id),
                        });
                        index = end + 1;
                        plain_start = index;
                        continue;
                    }
                }
            }
        }
        index += text[index..].chars().next().map_or(1, char::len_utf8);
    }
    push_plain(&mut spans, plain_start, text.len(), base);
    spans
}

fn push_plain(spans: &mut Vec<TextSpan>, start: usize, end: usize, base: &TextStyle) {
    if end > start {
        spans.push(TextSpan {
            range: start..end,
            style: base.clone(),
            color: Token::TEXT_PRIMARY,
            underline: false,
            background: None,
            link_id: None,
        });
    }
}

/// 富文本块堆栈的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RichTextDesc {
    /// 内容区域矩形。
    pub rect: Rect,
    /// 无障碍名称。
    pub label: String,
    /// 块与块之间的间距。
    pub gap: f32,
    /// 列表项与引用块的前导缩进。
    pub indent: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl RichTextDesc {
    /// 创建块间距为 8 px 的富文本描述符。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            gap: 8.0,
            indent: 18.0,
            z: 500,
        }
    }

    /// 设置无障碍名称。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 根据已测量的各块高度进行布局。
    #[must_use]
    pub fn layout(&self, document: &RichDocument, heights: &[f32]) -> Vec<Rect> {
        let mut out = Vec::with_capacity(document.blocks.len());
        let mut y = self.rect.origin.y;
        for (block, height) in document.blocks.iter().zip(heights) {
            if y >= self.rect.max().y {
                break;
            }
            let indent = match block.kind {
                BlockKind::Bullet | BlockKind::Quote => self.indent,
                _ => 0.0,
            };
            out.push(Rect::new(
                self.rect.origin.x + indent,
                y,
                (self.rect.size.x - indent).max(0.0),
                *height,
            ));
            y += height + self.gap;
        }
        out
    }
}

/// 输出块表面与字形。
#[must_use]
pub fn emit_rich_text(
    node: u64,
    desc: &RichTextDesc,
    document: &RichDocument,
    layout: &[Rect],
    rich_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(layout.len() * 2 + rich_glyphs.len());
    let mut payloads = Vec::with_capacity(rich_glyphs.len());
    for (block, rect) in document.blocks.iter().zip(layout) {
        if let Some(fill) = block.kind.background() {
            primitives.push(surface(*rect, 6.0, fill, desc.z));
        }
        if block.kind == BlockKind::Quote {
            primitives.push(surface(
                Rect::new(rect.origin.x - 4.0, rect.origin.y, 3.0, rect.size.y),
                1.5,
                Token::PRIMARY,
                desc.z + 1,
            ));
        }
    }
    glyphs(
        rich_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(Role::Document, desc.label.clone(), desc.rect).clickable();
    a11y.description = format!(
        "{} 个段落, {} 个链接",
        document.blocks.len(),
        document.link_count()
    );
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Text),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_capture_their_level() {
        let document = parse_markdown("# Title\n\n### Detail");
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(document.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(document.blocks[0].text, "Title");
        assert_eq!(document.blocks[1].kind, BlockKind::Heading(3));
    }

    #[test]
    fn bullets_and_quotes_lose_their_markers() {
        let document = parse_markdown("- one\n> quoted");
        assert_eq!(document.blocks[0].kind, BlockKind::Bullet);
        assert_eq!(document.blocks[0].text, "one");
        assert_eq!(document.blocks[1].kind, BlockKind::Quote);
        assert_eq!(document.blocks[1].text, "quoted");
    }

    #[test]
    fn fenced_code_keeps_newlines_and_the_language() {
        let document = parse_markdown("```rust\nlet x = 1;\nlet y = 2;\n```");
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.blocks[0].kind, BlockKind::Code);
        assert_eq!(document.blocks[0].text, "let x = 1;\nlet y = 2;");
    }

    #[test]
    fn inline_spans_cover_bold_code_and_links() {
        let document = parse_markdown("a **bold** `code` [link](https://x)");
        let block = &document.blocks[0];
        assert_eq!(block.text, "a **bold** `code` [link](https://x)");
        assert!(block.spans.iter().any(|span| span.style.weight == 700));
        assert!(block
            .spans
            .iter()
            .any(|span| span.background == Some(Token::CODE_BG)));
        assert!(block.spans.iter().any(|span| span.underline));
        assert_eq!(document.link_count(), 1);
    }

    #[test]
    fn a_link_is_found_by_byte_offset() {
        let document = parse_markdown("see [docs](https://example.com) now");
        let block = &document.blocks[0];
        let link = block
            .spans
            .iter()
            .find(|span| span.link_id.is_some())
            .unwrap();
        assert_eq!(block.link_at(link.range.start), Some(1));
        assert_eq!(block.link_at(0), None);
    }

    #[test]
    fn spans_cover_the_text_but_not_the_markers() {
        let document = parse_markdown("plain **b** tail");
        let block = &document.blocks[0];
        let covered: usize = block.spans.iter().map(|span| span.range.len()).sum();
        assert_eq!(
            covered,
            block.text.len() - 4,
            "the four `*` markers are excluded"
        );
    }

    #[test]
    fn layout_indents_bullets_and_gaps_paragraphs() {
        let document = parse_markdown("# T\n- one\n- two");
        let desc = RichTextDesc::new(Rect::new(0.0, 0.0, 300.0, 400.0));
        let layout = desc.layout(&document, &[20.0, 20.0, 20.0]);
        assert_eq!(layout.len(), 3);
        assert_eq!(layout[0].origin.x, 0.0);
        assert_eq!(layout[1].origin.x, desc.indent);
        assert_eq!(layout[1].origin.y, 28.0);
    }

    #[test]
    fn the_emitter_reports_a_text_frame() {
        let document = parse_markdown("hello");
        let desc = RichTextDesc::new(Rect::new(0.0, 0.0, 300.0, 100.0)).with_label("Intro");
        let layout = desc.layout(&document, &[20.0]);
        let output = emit_rich_text(5, &desc, &document, &layout, &[], ControlState::default());
        let a11y = output.a11y.expect("text frame");
        assert_eq!(a11y.role, Role::Document);
        assert_eq!(a11y.label, "Intro");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }
}

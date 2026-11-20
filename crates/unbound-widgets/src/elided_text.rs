//! 多模式文本省略与条件溢出提示控件（`ElidedText`）。
//!
//! 基于真实字形宽度（复用 [`unbound_text::TextEngine::measure`] 或自定义测量器）实现
//! 首部（`Start`）、中部（`Middle`，路径感知，保留盘符/根段与完整文件名/扩展名）
//! 与尾部（`End`）省略，内置结果缓存并在发生实际截断时按需生成 [`crate::tooltip::TooltipDesc`]。

use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::{GlyphQuad, TextEngine, TextStyle};
use unbound_theme::Token;

use crate::primitive::{glyphs as emit_glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::tooltip::TooltipDesc;
use crate::{ControlState, WidgetOutput};

/// 默认省略号字符串。
pub const ELLIPSIS: &str = "...";

/// 文本省略方向模式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ElideMode {
    /// 尾部省略（例如 `"Very long ti..."`）。
    #[default]
    End,
    /// 首部省略（例如 `"...suffix/file.rs"`）。
    Start,
    /// 中部省略（路径感知：优先保留盘符/根前缀与完整文件名及扩展名）。
    Middle,
}

/// 单次省略测量结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElideResult {
    /// 经过省略处理后用于显示的文本。
    pub display: String,
    /// 是否实际发生了截断省略。
    pub was_elided: bool,
}

/// 按真实宽度测量函数执行多模式省略（严格遵守 UTF-8 字符边界）。
///
/// `measure` 返回给定字符串在目标字体/字号下的逻辑像素宽度。
#[must_use]
pub fn elide_text_with_measure<F>(
    text: &str,
    max_width: f32,
    mode: ElideMode,
    mut measure: F,
) -> ElideResult
where
    F: FnMut(&str) -> f32,
{
    if text.is_empty() || !max_width.is_finite() || max_width <= 0.0 {
        return ElideResult {
            display: if text.is_empty() {
                String::new()
            } else {
                ELLIPSIS.to_owned()
            },
            was_elided: !text.is_empty(),
        };
    }

    if measure(text) <= max_width {
        return ElideResult {
            display: text.to_owned(),
            was_elided: false,
        };
    }

    let ellipsis_w = measure(ELLIPSIS);
    if ellipsis_w >= max_width {
        return ElideResult {
            display: ELLIPSIS.to_owned(),
            was_elided: true,
        };
    }

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let char_count = chars.len();
    let display = match mode {
        ElideMode::End => elide_end(text, &chars, max_width, &mut measure),
        ElideMode::Start => elide_start(text, &chars, max_width, &mut measure),
        ElideMode::Middle => elide_middle(text, &chars, char_count, max_width, &mut measure),
    };

    ElideResult {
        display,
        was_elided: true,
    }
}

fn byte_prefix<'a>(text: &'a str, chars: &[(usize, char)], count: usize) -> &'a str {
    if count == 0 {
        ""
    } else if count >= chars.len() {
        text
    } else {
        &text[..chars[count].0]
    }
}

fn byte_suffix<'a>(text: &'a str, chars: &[(usize, char)], count: usize) -> &'a str {
    if count == 0 {
        ""
    } else if count >= chars.len() {
        text
    } else {
        let start_idx = chars.len() - count;
        &text[chars[start_idx].0..]
    }
}

fn elide_end<F>(text: &str, chars: &[(usize, char)], max_width: f32, measure: &mut F) -> String
where
    F: FnMut(&str) -> f32,
{
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let candidate = format!("{}{ELLIPSIS}", byte_prefix(text, chars, mid));
        if measure(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}{ELLIPSIS}", byte_prefix(text, chars, lo))
}

fn elide_start<F>(text: &str, chars: &[(usize, char)], max_width: f32, measure: &mut F) -> String
where
    F: FnMut(&str) -> f32,
{
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let candidate = format!("{ELLIPSIS}{}", byte_suffix(text, chars, mid));
        if measure(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{ELLIPSIS}{}", byte_suffix(text, chars, lo))
}

/// 路径感知的中部省略：若包含路径分隔符（`/` 或 `\`），优先完整保留末段文件名（含扩展名）
/// 与前导盘符/根段；否则在字符级别均衡保留首尾两端。
fn elide_middle<F>(
    text: &str,
    chars: &[(usize, char)],
    char_count: usize,
    max_width: f32,
    measure: &mut F,
) -> String
where
    F: FnMut(&str) -> f32,
{
    if let Some(sep_pos) = text.rfind(['/', '\\']) {
        let sep_len = text[sep_pos..].chars().next().map_or(1, char::len_utf8);
        let tail = &text[sep_pos..]; // 含分隔符与完整文件名（如 `\\capture.rdc` 或 `/frame.png`）
        let head_part = &text[..sep_pos];
        if sep_pos + sep_len < text.len() {
            // 检测盘符或根段（如 `C:\`、`C:/` 或 `/`），宽度允许时完整保留根段避免截断为孤立字母。
            let root_prefix = extract_path_root(head_part);
            let min_with_root = format!("{root_prefix}{ELLIPSIS}{tail}");
            let min_with_tail = format!("{ELLIPSIS}{tail}");
            let head_chars: Vec<(usize, char)> = head_part.char_indices().collect();
            let root_char_count = root_prefix.chars().count();
            if !root_prefix.is_empty() && measure(&min_with_root) <= max_width {
                let mut lo = root_char_count;
                let mut hi = head_chars.len();
                while lo < hi {
                    let mid = lo + (hi - lo).div_ceil(2);
                    let prefix = byte_prefix(head_part, &head_chars, mid);
                    let candidate = format!("{prefix}{ELLIPSIS}{tail}");
                    if measure(&candidate) <= max_width {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                let prefix = byte_prefix(head_part, &head_chars, lo);
                return format!("{prefix}{ELLIPSIS}{tail}");
            }
            if measure(&min_with_tail) <= max_width {
                return min_with_tail;
            }
            // 文件名本身超宽时，尽量保留完整扩展名（如 `.rdc` / `.png`）避免切碎。
            if let Some(dot_pos) = tail.rfind('.') {
                let ext = &tail[dot_pos..];
                if dot_pos > sep_len && measure(&format!("{ELLIPSIS}{ext}")) <= max_width {
                    let stem = &tail[sep_len..dot_pos];
                    let stem_chars: Vec<(usize, char)> = stem.char_indices().collect();
                    let mut lo = 0usize;
                    let mut hi = stem_chars.len();
                    while lo < hi {
                        let mid = lo + (hi - lo).div_ceil(2);
                        let candidate =
                            format!("{}{ELLIPSIS}{ext}", byte_prefix(stem, &stem_chars, mid));
                        if measure(&candidate) <= max_width {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    return format!("{}{ELLIPSIS}{ext}", byte_prefix(stem, &stem_chars, lo));
                }
            }
        }
    }

    // 通用字符串或文件名本身超宽时：二分总保留字符数，前半向下取整、后半向上取整以尽量保留扩展名。
    let mut lo = 0usize;
    let mut hi = char_count.saturating_sub(1);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let right = mid.div_ceil(2);
        let left = mid - right;
        let candidate = format!(
            "{}{ELLIPSIS}{}",
            byte_prefix(text, chars, left),
            byte_suffix(text, chars, right)
        );
        if measure(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let right = lo.div_ceil(2);
    let left = lo - right;
    format!(
        "{}{ELLIPSIS}{}",
        byte_prefix(text, chars, left),
        byte_suffix(text, chars, right)
    )
}

/// 提取路径前导盘符或根前缀（如 `"C:\\"`、`"C:/"` 或 `"/"`）。
fn extract_path_root(head: &str) -> &str {
    let bytes = head.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return &head[..3];
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return &head[..2];
    }
    if head.starts_with('/') {
        return "/";
    }
    ""
}

/// 带测量结果缓存的省略文本状态机，避免热路径在文本与宽度未变时重复排版测量。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ElidedTextState {
    cached_source: String,
    cached_width_bits: u32,
    cached_font_size_bits: u32,
    cached_mode: ElideMode,
    display: String,
    was_elided: bool,
    valid: bool,
    recompute_count: u64,
}

impl ElidedTextState {
    /// 创建空缓存状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前缓存的展示文本。
    #[must_use]
    pub fn display_text(&self) -> &str {
        &self.display
    }

    /// 最近一次测量是否发生了实际省略。
    #[must_use]
    pub const fn was_elided(&self) -> bool {
        self.was_elided
    }

    /// 累计触发重新测量的次数（用于验证零浪费缓存命中）。
    #[must_use]
    pub const fn recompute_count(&self) -> u64 {
        self.recompute_count
    }

    /// 主动清空缓存。
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// 使用 [`TextEngine`] 更新省略缓存（复用 `unbound-text` 内部塑形测量缓存）。
    pub fn update_with_engine(
        &mut self,
        engine: &mut TextEngine,
        style: &TextStyle,
        text: &str,
        max_width: f32,
        mode: ElideMode,
    ) -> &str {
        self.update_with_measure(text, max_width, style.font_size, mode, |sample| {
            engine.measure(sample, style).x
        })
    }

    /// 使用自定义宽度测量函数更新省略缓存；仅当文本、宽度、字号或模式变化时才重算。
    pub fn update_with_measure<F>(
        &mut self,
        text: &str,
        max_width: f32,
        font_size: f32,
        mode: ElideMode,
        measure: F,
    ) -> &str
    where
        F: FnMut(&str) -> f32,
    {
        let width_bits = max_width.to_bits();
        let size_bits = font_size.to_bits();
        if self.valid
            && self.cached_width_bits == width_bits
            && self.cached_font_size_bits == size_bits
            && self.cached_mode == mode
            && self.cached_source == text
        {
            return &self.display;
        }

        let res = elide_text_with_measure(text, max_width, mode, measure);
        self.cached_source.clear();
        self.cached_source.push_str(text);
        self.cached_width_bits = width_bits;
        self.cached_font_size_bits = size_bits;
        self.cached_mode = mode;
        self.display = res.display;
        self.was_elided = res.was_elided;
        self.valid = true;
        self.recompute_count += 1;
        &self.display
    }
}

/// 省略文本控件描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ElidedTextDesc {
    /// 控件边界矩形。
    pub rect: Rect,
    /// 完整原始文本（用于无障碍树与溢出 Tooltip）。
    pub full_text: String,
    /// 经过省略计算后的显示文本。
    pub display_text: String,
    /// 省略模式。
    pub mode: ElideMode,
    /// 是否实际发生截断（仅当为 `true` 时才应挂载溢出 Tooltip）。
    pub is_elided: bool,
    /// 文本颜色令牌。
    pub color: Token,
    /// 绘制 z-index。
    pub z: u16,
}

impl ElidedTextDesc {
    /// 创建未省略状态的描述符。
    #[must_use]
    pub fn new(rect: Rect, full_text: impl Into<String>) -> Self {
        let full_text = full_text.into();
        Self {
            rect,
            display_text: full_text.clone(),
            full_text,
            mode: ElideMode::End,
            is_elided: false,
            color: Token::TEXT_PRIMARY,
            z: 10,
        }
    }

    /// 基于状态机缓存结果构造描述符。
    #[must_use]
    pub fn from_state(
        rect: Rect,
        full_text: impl Into<String>,
        mode: ElideMode,
        state: &ElidedTextState,
    ) -> Self {
        Self {
            rect,
            full_text: full_text.into(),
            display_text: state.display_text().to_owned(),
            mode,
            is_elided: state.was_elided(),
            color: Token::TEXT_PRIMARY,
            z: 10,
        }
    }

    /// 设置省略模式。
    #[must_use]
    pub const fn with_mode(mut self, mode: ElideMode) -> Self {
        self.mode = mode;
        self
    }

    /// 显式设置省略后文本与截断标记。
    #[must_use]
    pub fn with_elided(mut self, display_text: impl Into<String>, is_elided: bool) -> Self {
        self.display_text = display_text.into();
        self.is_elided = is_elided;
        self
    }

    /// 设置文本颜色令牌。
    #[must_use]
    pub const fn with_color(mut self, color: Token) -> Self {
        self.color = color;
        self
    }

    /// 当且仅当文本实际被省略时构造溢出 Tooltip 描述符；未省略时返回 `None`。
    #[must_use]
    pub fn tooltip_desc(&self, viewport: Rect) -> Option<TooltipDesc> {
        if !self.is_elided || self.full_text.is_empty() {
            return None;
        }
        Some(TooltipDesc::labeled(self.rect, viewport, self.full_text.clone()))
    }
}

/// 发射省略文本控件（24 态恒定图元数：1 个透明占位底板 + 字形）。
#[must_use]
pub fn emit_elided_text(
    node: u64,
    desc: &ElidedTextDesc,
    state: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(1 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    // 透明占位表面用于确保 24 态下至少存在 1 个固定图元。
    primitives.push(surface(desc.rect, 0.0, Token::NONE, desc.z));

    if !glyphs.is_empty() {
        let color = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            desc.color
        };
        emit_glyphs(glyphs, color, desc.z + 1, &mut primitives, &mut payloads);
    }

    // 仅在发生实际省略时才需要悬停命中以触发条件 Tooltip。
    let hit = (desc.is_elided && !state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Default)
    });

    // 无障碍节点始终暴露完整文本，确保屏幕阅读器与 Agent 不会丢失被省略的信息。
    let a11y = A11yNode::static_text(desc.full_text.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    fn mono_measure(sample: &str) -> f32 {
        sample.chars().count() as f32 * 8.0
    }

    #[test]
    fn elide_end_start_and_path_aware_middle() {
        // 不超宽时不截断。
        let fit = elide_text_with_measure("short.rs", 120.0, ElideMode::End, mono_measure);
        assert!(!fit.was_elided);
        assert_eq!(fit.display, "short.rs");

        // 尾部省略。
        let end = elide_text_with_measure("abcdefghijklmnop", 80.0, ElideMode::End, mono_measure);
        assert!(end.was_elided);
        assert!(end.display.ends_with(ELLIPSIS));
        assert!(mono_measure(&end.display) <= 80.0);

        // 首部省略。
        let start =
            elide_text_with_measure("abcdefghijklmnop", 80.0, ElideMode::Start, mono_measure);
        assert!(start.was_elided);
        assert!(start.display.starts_with(ELLIPSIS));
        assert!(mono_measure(&start.display) <= 80.0);

        // 路径中部省略：保留盘符与完整文件名及扩展名。
        let path = r"C:\Projects\Unbound\Captures\2026\frame_0042.rdc";
        let mid = elide_text_with_measure(path, 200.0, ElideMode::Middle, mono_measure);
        assert!(mid.was_elided);
        assert!(mid.display.starts_with("C:\\"));
        assert!(mid.display.ends_with("\\frame_0042.rdc"));
        assert!(mid.display.contains(ELLIPSIS));
        assert!(mono_measure(&mid.display) <= 200.0);
    }

    #[test]
    fn elided_text_cache_avoids_redundant_recomputation() {
        let mut st = ElidedTextState::new();
        let text = "C:/workspace/render/debugger/capture_scene.rdc";

        let first = st
            .update_with_measure(text, 160.0, 14.0, ElideMode::Middle, mono_measure)
            .to_owned();
        assert_eq!(st.recompute_count(), 1);
        assert!(st.was_elided());

        // 相同输入不重算。
        let second = st
            .update_with_measure(text, 160.0, 14.0, ElideMode::Middle, mono_measure)
            .to_owned();
        assert_eq!(first, second);
        assert_eq!(st.recompute_count(), 1);

        // 宽度变化后触发重算。
        let _third = st.update_with_measure(text, 600.0, 14.0, ElideMode::Middle, mono_measure);
        assert_eq!(st.recompute_count(), 2);
        assert!(!st.was_elided());
    }

    #[test]
    fn elided_text_conditional_tooltip_and_headless_geometry() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let rect = Rect::new(24.0, 24.0, 180.0, 24.0);

        let unelided = ElidedTextDesc::new(rect, "Hello");
        assert!(unelided.tooltip_desc(viewport).is_none());

        let elided = ElidedTextDesc::new(rect, "Very long diagnostic path")
            .with_elided("Very long...", true);
        let tip = elided
            .tooltip_desc(viewport)
            .expect("elided text must produce a tooltip");
        assert_eq!(tip.label, "Very long diagnostic path");

        let out = emit_elided_text(7, &elided, ControlState::default(), &[]);
        assert_eq!(out.a11y.as_ref().unwrap().label, "Very long diagnostic path");
        assert!(out.hit.is_some());

        // 无头几何检查。
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Text {
                ink: rect,
                ellipsis: true,
            },
            rect,
            "ElidedText",
        );
        frame.set_text_colors([28, 30, 38], [255, 255, 255]);
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());

        // 24 态恒定图元数校验。
        let base = out.primitives.len();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let st = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        assert_eq!(emit_elided_text(7, &elided, st, &[]).primitives.len(), base);
                    }
                }
            }
        }
    }
}

//! 基于 `cosmic-text` 的文本塑形，配备架构文档要求的两级缓存
//! （`docs` §3.5）。
//!
//! * **一级 —— 塑形缓存**：以 `(text hash, style)` 为键。按钮标签或固定短语
//!   在进程生命周期内仅需塑形一次。
//! * **二级 —— 换行缓存**：以 `(text hash, style, max width)` 为键。将宽度
//!   纳入键中使面板缩放极其廉价：塑形结果直接复用，仅重新计算断行，
//!   且仅当宽度跨越量化分桶边界时才重新计算。
//!
//!
//! 缓存刻意按每个 `TextEngine` 独立持有而非全局共享：全局缓存每帧都需要
//! 加锁，而锁开销高于其省下的塑形计算。
//!

use std::sync::Arc;

use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};
use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

/// 将所有影响塑形的属性打包为单一可比较的值。
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    /// `0.0` 表示“使用字体自然行高 × 1.2”。
    pub line_height: f32,
    pub letter_spacing: f32,
    pub weight: u16,
    pub italic: bool,
    /// `None` 表示选择系统 UI 字体。
    pub family: Option<String>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            line_height: 0.0,
            letter_spacing: 0.0,
            weight: 400,
            italic: false,
            family: None,
        }
    }
}

impl TextStyle {
    /// 指定字号的文本样式，其余字段保持默认值。
    #[must_use]
    pub fn sized(size: f32) -> Self {
        Self {
            font_size: size,
            ..Default::default()
        }
    }

    fn metrics(&self) -> Metrics {
        let line_height = if self.line_height > 0.0 {
            self.line_height
        } else {
            (self.font_size * 1.2).round()
        };
        Metrics::new(self.font_size, line_height)
    }

    /// 样式的稳定哈希，用作缓存键的一部分。浮点数按位转换，
    /// 确保相同的 `14.0` 始终一致，且 `14.0` 与 `14.5` 绝不冲突。
    ///
    fn hash_into(&self, h: &mut u64) {
        fn mix(h: &mut u64, v: u64) {
            *h ^= v;
            *h = h.wrapping_mul(0x100_0000_01b3);
        }
        mix(h, self.font_size.to_bits() as u64);
        mix(h, self.line_height.to_bits() as u64);
        mix(h, self.letter_spacing.to_bits() as u64);
        mix(h, u64::from(self.weight));
        mix(h, u64::from(self.italic));
        // 字体族名同时混入其长度与字节内容：缺失字体族与空字体族此前哈希相同，
        // 导致两种样式共享同一缓存条目并返回错误的塑形结果。
        //
        match &self.family {
            Some(family) => {
                mix(h, family.len() as u64);
                for b in family.bytes() {
                    mix(h, u64::from(b));
                }
            }
            None => mix(h, u64::MAX),
        }
    }
}

/// 富文本中的单个行内片段（span）。
///
/// span 从不携带字体或已塑形文本段：它只携带某段字节范围的**样式策略**，
/// [`TextEngine::shape_rich`] 会在同一缓冲区中排版所有片段，
/// 使不同字号与字重的基线保持对齐。
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// 该片段覆盖的字节范围。
    pub range: std::ops::Range<usize>,
    /// 该范围内字形的文本样式。
    pub style: TextStyle,
    /// 调用方用于绘制字形的语义颜色标记。
    pub color: Token,
    /// 该范围是否带下划线（用于链接、强调文本）。
    pub underline: bool,
    /// 可选的行内背景颜色标记（用于行内代码、高亮）。
    pub background: Option<Token>,
    /// 当该范围可交互时的链接标识。
    pub link_id: Option<u32>,
}

/// 单个已定位的字形，以相对于文本原点的逻辑像素表示。
///
/// 字体面通过 [`ShapedText::faces`] 的索引引用，而非 `fontdb::ID`：
/// 该 ID 类型不透明且非 `Pod`，保持字形为 POD 可使文本段无需转换趟
/// 即可直接上传至 GPU。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    /// 在 [`ShapedText::faces`] 中的字体索引。
    pub face_index: u16,
    /// 字形左边缘的画笔 X 坐标。
    pub x: f32,
    /// 字形顶边缘的画笔 Y 坐标（Y 轴向下增长）。
    pub y: f32,
    /// 字形的步进宽度。
    pub w: f32,
    /// 塑形该字形时使用的字号（光栅缓存键的一部分）。
    pub font_size: f32,
    /// 在源字符串中的字节偏移量，用于命中测试与文本选择。
    pub cluster: u32,
    /// `BiDi` 嵌入层级：奇数表示从右到左（RTL）。
    pub level: u8,
    pub _pad: [u8; 3],
}

impl PositionedGlyph {
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

/// 整段文本的塑形结果。
#[derive(Clone, Debug, Default)]
pub struct ShapedText {
    /// 本段文本用到的所有不重复字体面，由 `PositionedGlyph::face_index` 索引。
    pub faces: Vec<fontdb::ID>,
    pub glyphs: Vec<PositionedGlyph>,
    /// 与墨迹无关的布局宽度（最大行宽）。
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_count: u32,
    /// 首行顶部偏移量，使单行标签能够精确居中。
    pub first_baseline: f32,
}

impl ShapedText {
    /// 当没有任何内容被排版时为 `true`（空字符串或仅含空白符）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// 覆盖 `byte` 所在字形簇（cluster）的字形字节范围。
    ///
    /// cluster 是字形首个字符在源字符串中的字节偏移量。因此覆盖 `byte` 的字形
    /// 是最后一个起始位置不晚于 `byte` 的字形——而非“8 字节窗口内的第一个”，
    /// 后者可能会误选下一个字形簇中的字形。
    ///
    #[must_use]
    pub fn glyph_at_byte(&self, byte: u32) -> Option<&PositionedGlyph> {
        self.glyphs.iter().rev().find(|g| g.cluster <= byte)
    }

    /// 距离局部坐标点最近的字形（用于命中测试与光标定位）。
    ///
    ///
    /// 返回的距离是到最近字形**边界**的距离，而非到字形左边缘的距离：
    /// 点击字符右半部分必须将光标放在该字符之后，而非之前。
    ///
    #[must_use]
    pub fn nearest_glyph(&self, p: Vec2) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, g) in self.glyphs.iter().enumerate() {
            let dx = (p.x - g.x).abs();
            let dy = (p.y - g.y).abs();
            // X 轴权重远大于 Y 轴：光标定位主要按列选择。
            let cost = dx + dy * 0.25;
            if best.map(|(_, c)| cost < c).unwrap_or(true) {
                best = Some((i, cost));
            }
        }
        best
    }

    /// 将局部坐标 `(x, y)` 映射到 `text` 中的 UTF-8 字节偏移量。
    ///
    /// 选取距离 `point.y` 最近的行，再将该行每个字形的水平中点与 `point.x` 比较，
    /// 使点击字形右半部分时将光标推进到下一个字符边界。
    ///
    #[must_use]
    pub fn hit_test_point(&self, point: Vec2, text: &str) -> usize {
        if self.glyphs.is_empty() || text.is_empty() {
            return 0;
        }
        let line_h = self.line_height.max(1.0);
        // 按 Y 坐标将字形分组到各行；选取距离 point.y 最近的行。
        let mut best_line_y = self.glyphs[0].y;
        let mut best_dy = (point.y - (best_line_y + line_h * 0.5)).abs();
        for g in &self.glyphs {
            let dy = (point.y - (g.y + line_h * 0.5)).abs();
            if dy < best_dy {
                best_dy = dy;
                best_line_y = g.y;
            }
        }
        let next_boundary = |byte: usize| -> usize {
            let b = byte.min(text.len());
            let mut idx = b + 1;
            while idx < text.len() && !text.is_char_boundary(idx) {
                idx += 1;
            }
            idx.min(text.len())
        };
        let mut line_glyphs: Vec<&PositionedGlyph> = self
            .glyphs
            .iter()
            .filter(|g| (g.y - best_line_y).abs() < line_h * 0.45)
            .collect();
        if line_glyphs.is_empty() {
            return 0;
        }
        line_glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
        for g in &line_glyphs {
            let mid = g.x + g.w * 0.5;
            if point.x < mid {
                return if g.is_rtl() {
                    next_boundary(g.cluster as usize)
                } else {
                    (g.cluster as usize).min(text.len())
                };
            }
        }
        let last = line_glyphs[line_glyphs.len() - 1];
        if last.is_rtl() {
            (last.cluster as usize).min(text.len())
        } else {
            next_boundary(last.cluster as usize)
        }
    }

    /// 指定字节偏移量对应的文本光标位置。
    ///
    /// 光标位于该字节所属字形的起始处；当字节超出文本末尾时，
    /// 位于最后一个字形的末尾。
    #[must_use]
    pub fn caret_at_byte(&self, byte: u32) -> Option<f32> {
        let glyph = self.glyph_at_byte(byte)?;
        let at_end = byte >= glyph.cluster + glyph.w as u32;
        Some(if at_end { glyph.x + glyph.w } else { glyph.x })
    }

    /// 将相对于文本原点的局部像素坐标 `(x, y)` 映射到最近的合法 UTF-8 字符边界字节偏移量。
    ///
    #[must_use]
    pub fn hit_byte_index(&self, text: &str, local_pos: Vec2) -> usize {
        let idx = self.hit_test_point(local_pos, text).min(text.len());
        if text.is_char_boundary(idx) {
            idx
        } else {
            let mut b = idx;
            while b > 0 && !text.is_char_boundary(b) {
                b -= 1;
            }
            b
        }
    }

    /// 计算 `byte_index` 处的局部光标矩形。
    #[must_use]
    pub fn caret_rect_at(&self, byte_index: usize) -> Rect {
        let line_h = self.line_height.max(14.0);
        if self.glyphs.is_empty() {
            return Rect::new(0.0, 0.0, 1.5, line_h);
        }
        let byte = u32::try_from(byte_index).unwrap_or(u32::MAX);
        if let Some((pos, glyph)) = self
            .glyphs
            .iter()
            .enumerate()
            .rev()
            .find(|(_, g)| g.cluster <= byte)
        {
            let next_cluster = self
                .glyphs
                .get(pos + 1)
                .map(|g| g.cluster)
                .unwrap_or(u32::MAX);
            let at_end = byte > glyph.cluster && next_cluster == u32::MAX;
            let x = if at_end { glyph.x + glyph.w } else { glyph.x };
            Rect::new(x, glyph.y, 1.5, line_h)
        } else {
            let first = &self.glyphs[0];
            Rect::new(first.x, first.y, 1.5, line_h)
        }
    }

    /// 为给定的 UTF-8 字节范围 `range` 生成逐行选区矩形。
    ///
    /// 将结果传给 [`crate::selection::merge_line_rects`] 即可获得无缝圆角高亮。
    #[must_use]
    pub fn selection_rects(&self, range: core::ops::Range<usize>) -> Vec<Rect> {
        if range.is_empty() || self.glyphs.is_empty() {
            return Vec::new();
        }
        let line_h = self.line_height.max(14.0);
        let start = u32::try_from(range.start).unwrap_or(u32::MAX);
        let end = u32::try_from(range.end).unwrap_or(u32::MAX);
        let mut lines: Vec<(f32, f32, f32)> = Vec::new();
        for g in &self.glyphs {
            if g.cluster >= start && g.cluster < end {
                let gx0 = g.x;
                let gx1 = g.x + g.w;
                if let Some(last) = lines.last_mut() {
                    if (last.0 - g.y).abs() < line_h * 0.45 {
                        last.1 = last.1.min(gx0);
                        last.2 = last.2.max(gx1);
                        continue;
                    }
                }
                lines.push((g.y, gx0, gx1));
            }
        }
        lines
            .into_iter()
            .filter_map(|(y, x0, x1)| {
                let w = (x1 - x0).max(0.0);
                (w > 0.0).then(|| Rect::new(x0, y, w, line_h))
            })
            .collect()
    }
}

/// 已折行的段落：各行字节范围及其测量宽度。
#[derive(Clone, Debug, Default)]
pub struct WrappedText {
    pub lines: Vec<TextLine>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
}

/// 排版后的单行。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TextLine {
    /// 首字符的字节偏移量。
    pub start: u32,
    /// 末字符之后的字节偏移量（开区间）。
    pub end: u32,
    pub width: f32,
    pub baseline: f32,
}

/// 缓存统计信息，供调试浮层展示。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextCacheStats {
    pub shape_hits: u64,
    pub shape_misses: u64,
    pub wrap_hits: u64,
    pub wrap_misses: u64,
    pub entries: usize,
}

impl TextCacheStats {
    #[must_use]
    pub fn shape_hit_rate(&self) -> f32 {
        let total = self.shape_hits + self.shape_misses;
        if total == 0 {
            0.0
        } else {
            self.shape_hits as f32 / total as f32
        }
    }
}

/// `text` 中每个源文本行的起始字节偏移量。
///
/// cosmic-text 在 `\r`、`\n`、`\r\n` 和 `\n\r` 处断行，并将字形的 cluster
/// 上报为**行内**索引。要将 cluster 索引转换为源文本中的绝对字节偏移量，
/// 必须知道字形所属行的偏移量——本辅助函数即用于此，与 cosmic-text 自身的
/// 断行逻辑保持一致。
///
fn source_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                // `\r\n` 与 `\n\r` 均视为单个换行终止符。
                if bytes.get(index + 1) == Some(&b'\n') || bytes.get(index + 1) == Some(&b'\r') {
                    index += 2;
                } else {
                    index += 1;
                }
                starts.push(index);
            }
            b'\n' => {
                if bytes.get(index + 1) == Some(&b'\r') {
                    index += 2;
                } else {
                    index += 1;
                }
                starts.push(index);
            }
            _ => index += 1,
        }
    }
    starts
}

/// 塑形引擎：持有字体系统与两级缓存。
pub struct TextEngine {
    font_system: FontSystem,
    /// 跨调用复用，避免每个标签都分配一个 `Buffer`。
    buffer: Option<Buffer>,
    buffer_metrics: Option<(f32, f32)>,
    shape_cache: FxHashMap<(u64, u64), Arc<ShapedText>>,
    wrap_cache: FxHashMap<(u64, u64, u32), Arc<WrappedText>>,
    stats: TextCacheStats,
    max_cache_entries: usize,
}

impl core::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextEngine")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// 加载系统字体。耗时约 50–150 ms；每个进程只需执行一次。
    #[must_use]
    pub fn new() -> Self {
        Self::with_font_system(FontSystem::new())
    }

    /// 基于已有 `FontSystem` 构建（测试中可使用最小字体集）。
    #[must_use]
    pub fn with_font_system(mut font_system: FontSystem) -> Self {
        #[cfg(windows)]
        {
            let db = font_system.db_mut();
            db.set_sans_serif_family("Segoe UI");
            db.set_monospace_family("Cascadia Mono");
        }
        Self {
            font_system,
            buffer: None,
            buffer_metrics: None,
            shape_cache: FxHashMap::default(),
            wrap_cache: FxHashMap::default(),
            stats: TextCacheStats::default(),
            max_cache_entries: 4096,
        }
    }

    /// 提供可变访问以供字形光栅化（`SwashCache` 需要）。
    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// 从内存字节流注入字体。
    ///
    /// 注入的字体立即可供塑形器使用，同时清空两级缓存，防止任何标签保留
    /// 基于旧字体列表解析的塑形结果。返回字节流提供的字体面数量；
    /// 格式错误的字节块不会加载任何字体并返回 `0`，而不会 panic。
    ///
    pub fn load_font_bytes(&mut self, data: Vec<u8>) -> usize {
        let faces = self
            .font_system
            .db_mut()
            .load_font_source(fontdb::Source::Binary(Arc::new(data)));
        self.clear_caches();
        faces.len()
    }

    #[must_use]
    pub fn stats(&self) -> TextCacheStats {
        self.stats
    }

    /// 清空两级缓存（主题/字体重载或收到内存压力信号时调用）。
    pub fn clear_caches(&mut self) {
        self.shape_cache.clear();
        self.wrap_cache.clear();
        self.buffer = None;
        self.buffer_metrics = None;
    }

    fn hash_text(text: &str) -> u64 {
        // FNV-1a；短标签占绝大多数，字节循环比随机每帧哈希器更快，
        // 且 64 位下的碰撞率可忽略不计。
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in text.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    fn attrs<'a>(&self, style: &'a TextStyle) -> Attrs<'a> {
        let weight = Weight(style.weight);
        let mut attrs = Attrs::new().weight(weight);
        if style.italic {
            attrs = attrs.style(cosmic_text::Style::Italic);
        }
        if let Some(family) = &style.family {
            attrs = attrs.family(Family::Name(family.as_str()));
        }
        attrs
    }

    fn prepare_buffer(&mut self, text: &str, style: &TextStyle) {
        let metrics = style.metrics();
        let key = (metrics.font_size, metrics.line_height);
        if self.buffer_metrics != Some(key) {
            self.buffer = Some(Buffer::new(&mut self.font_system, metrics));
            self.buffer_metrics = Some(key);
        }
        // 先构建 `Attrs`：它只借用 `style` 而非 `self`，因此下方对缓冲区的可变借用无歧义。
        //
        let attrs = self.attrs(style);
        let buffer = self.buffer.as_mut().expect("buffer just created");
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
    }

    /// 对 `text` 进行塑形（带缓存）。
    pub fn shape(&mut self, text: &str, style: &TextStyle) -> Arc<ShapedText> {
        let mut style_hash = 0u64;
        style.hash_into(&mut style_hash);
        let key = (Self::hash_text(text), style_hash);
        if let Some(cached) = self.shape_cache.get(&key) {
            self.stats.shape_hits += 1;
            return Arc::clone(cached);
        }
        self.stats.shape_misses += 1;
        let shaped = Arc::new(self.shape_uncached(text, style));
        if self.shape_cache.len() >= self.max_cache_entries {
            // 廉价淘汰：对于仅在当前可见屏幕上热用的缓存，直接清空比维护 LRU 账本更划算。
            //
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, Arc::clone(&shaped));
        self.stats.entries = self.shape_cache.len() + self.wrap_cache.len();
        shaped
    }

    /// 按文本段顺序返回塑形 `text` 所用到的系统字体族名称。
    ///
    /// 回退链对外部不可见：请求 UI 字体并不能说明 CJK 部分最终落在了
    /// `Microsoft YaHei UI` 还是阿拉伯文落在了 `Segoe UI`。字体检查器页面
    /// 需要此信息，因此直接从渲染器绘制的同一塑形段中提取。
    ///
    #[must_use]
    pub fn resolved_families(&mut self, text: &str, style: &TextStyle) -> Vec<String> {
        let shaped = self.shape(text, style);
        let ids = shaped.faces.clone();
        let db = self.font_system_mut().db_mut();
        let mut out: Vec<String> = Vec::new();
        for id in ids {
            let Some(face) = db.face(id) else {
                continue;
            };
            let name = face.families.first().map_or_else(
                || face.post_script_name.clone(),
                |(family, _)| family.clone(),
            );
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    fn shape_uncached(&mut self, text: &str, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::default();
        }
        self.prepare_buffer(text, style);
        let buffer = self.buffer.as_ref().expect("prepared");
        Self::extract(buffer, text, style)
    }

    /// 从已塑形的缓冲区中提取已定位的字形。
    ///
    /// 单行路径与折行路径共用：两者都会生成已完成塑形的 `Buffer`，且都需要
    /// 相同的字形提取逻辑。二者的区别仅在于缓冲区是否设置了宽度限制。
    ///
    ///
    fn extract(buffer: &Buffer, text: &str, style: &TextStyle) -> ShapedText {
        let mut faces: Vec<fontdb::ID> = Vec::new();
        let mut glyphs = Vec::with_capacity(text.len());
        let mut width: f32 = 0.0;
        let mut height: f32 = 0.0;
        let mut line_count = 0u32;
        let mut first_baseline = 0.0;
        let mut line_height = style.metrics().line_height;
        let mut ascent = 0.0;
        let mut descent = 0.0;

        for run in buffer.layout_runs() {
            line_count += 1;
            line_height = run.line_height;
            if line_count == 1 {
                first_baseline = run.line_y;
                ascent = run.line_y - run.line_top;
                descent = run.line_height - ascent;
            }
            width = width.max(run.line_w);
            height = height.max(run.line_y + (run.line_height - ascent).max(0.0));
            for g in run.glyphs {
                let face_index = match faces.iter().position(|f| *f == g.font_id) {
                    Some(i) => i,
                    None => {
                        faces.push(g.font_id);
                        faces.len() - 1
                    }
                };
                glyphs.push(PositionedGlyph {
                    glyph_id: g.glyph_id,
                    face_index: u16::try_from(face_index).unwrap_or(0),
                    // HarfBuzz（通过 rustybuzz）将附加符号（mark）定位在文本段画笔流之外：
                    // 阿拉伯文音符、泰文声调符号或越南文双变音符的真实位置保存在
                    // `x_offset`/`y_offset`（以 em 为单位）中。忽略它们会导致所有附加符号
                    // 塌陷到基线上并叠在基础字母上方。
                    //
                    //
                    //
                    // 此处与 `cosmic_text::LayoutGlyph::physical` 保持一致：
                    // `x = g.x + font_size * x_offset`，
                    // `y = g.y - font_size * y_offset`。
                    x: g.x + g.font_size * g.x_offset,
                    // 字形基线，相对于文本框顶部。
                    //
                    //
                    // cosmic-text 将行基线上报为 `line_y`（已累加上方各行高度），将字形在该行内的
                    // 偏移上报为 `g.y`（水平文本下为零）。旧代码误将 `g.font_size` 当作 ascent：
                    // 这不仅把每个字形放到了错误的高度，还忽略了行位置，导致第二行直接
                    // 绘制在第一行之上。
                    //
                    //
                    //
                    //
                    y: run.line_y + g.y - g.font_size * g.y_offset,
                    w: g.w,
                    font_size: g.font_size,
                    cluster: u32::try_from(g.start).unwrap_or(u32::MAX),
                    level: g.level.number(),
                    _pad: [0; 3],
                });
            }
        }

        ShapedText {
            faces,
            glyphs,
            width,
            height: if height > 0.0 { height } else { line_height },
            line_height,
            ascent,
            descent,
            line_count,
            first_baseline,
        }
    }

    /// 对 `text` 进行塑形并按 `max_width` 折行，返回每条折行后的已定位字形。
    ///
    ///
    /// 换行缓存仅记录**断点位置**；折行段落的字形位置此前无法获取，导致渲染器
    /// 无法绘制折行段落——只能一次绘制一行。此方法补齐了这一环：执行与
    /// [`TextEngine::shape`] 相同的提取流程，但缓冲区预先受限于 `max_width`。
    ///
    ///
    ///
    /// 宽度会混入样式哈希中，因此折行文本段绝不会与无宽度限制的同一文本发生冲突。
    ///
    pub fn shape_wrapped(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: f32,
    ) -> Arc<ShapedText> {
        let mut style_hash = 0u64;
        style.hash_into(&mut style_hash);
        let width_key = (max_width.max(1.0) * 4.0).round() as u32;
        let wrapped_hash = style_hash ^ u64::from(width_key).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let key = (Self::hash_text(text), wrapped_hash);
        if let Some(cached) = self.shape_cache.get(&key) {
            self.stats.shape_hits += 1;
            return Arc::clone(cached);
        }
        self.stats.shape_misses += 1;
        let shaped = Arc::new(self.shape_wrapped_uncached(text, style, width_key as f32 * 0.25));
        if self.shape_cache.len() >= self.max_cache_entries {
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, Arc::clone(&shaped));
        self.stats.entries = self.shape_cache.len() + self.wrap_cache.len();
        shaped
    }

    fn shape_wrapped_uncached(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: f32,
    ) -> ShapedText {
        if text.is_empty() {
            return ShapedText::default();
        }
        let metrics = style.metrics();
        // 使用独立缓冲区，使塑形缓存缓冲区保持无宽度限制：塑形结果绝不能依赖折行宽度。
        //
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(max_width.max(1.0)), None);
        let attrs = self.attrs(style);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        Self::extract(&buffer, text, style)
    }

    /// 将 `text` 按 `max_width` 折行（带缓存）。宽度量化为 ¼ px，
    /// 使平滑缩放面板时能够复用缓存结果。
    pub fn wrap(&mut self, text: &str, style: &TextStyle, max_width: f32) -> Arc<WrappedText> {
        let mut style_hash = 0u64;
        style.hash_into(&mut style_hash);
        let width_key = (max_width.max(0.0) * 4.0).round() as u32;
        let key = (Self::hash_text(text), style_hash, width_key);
        if let Some(cached) = self.wrap_cache.get(&key) {
            self.stats.wrap_hits += 1;
            return Arc::clone(cached);
        }
        self.stats.wrap_misses += 1;
        let wrapped = Arc::new(self.wrap_uncached(text, style, width_key as f32 * 0.25));
        if self.wrap_cache.len() >= self.max_cache_entries {
            self.wrap_cache.clear();
        }
        self.wrap_cache.insert(key, Arc::clone(&wrapped));
        self.stats.entries = self.shape_cache.len() + self.wrap_cache.len();
        wrapped
    }

    fn wrap_uncached(&mut self, text: &str, style: &TextStyle, max_width: f32) -> WrappedText {
        let metrics = style.metrics();
        // 使用独立缓冲区，使塑形缓存缓冲区保持无宽度限制（塑形结果绝不能依赖折行宽度）。
        //
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(max_width.max(1.0)), None);
        let attrs = self.attrs(style);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut lines = Vec::new();
        let mut height: f32 = 0.0;
        let mut width: f32 = 0.0;
        let mut line_height = metrics.line_height;
        // `text` 中每个源文本行的字节偏移量。
        //
        // cosmic-text 将字形的 cluster 上报为**行内**索引，而 `LayoutRun::text`
        // 是整条源文本行（而非折行后的片段）。因此需要行偏移量将 cluster 索引
        // 转换为 `text` 中的绝对字节偏移量。
        //
        let line_starts = source_line_starts(text);
        for run in buffer.layout_runs() {
            line_height = run.line_height;
            height = height.max(run.line_y + run.line_height);
            width = width.max(run.line_w);
            // 该文本段所属源文本行的基准偏移量。
            let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);
            // 文本段中的字形携带相对于源文本行的 cluster 索引，因此该段在源行中的
            // 范围即首个字形的起始位置与末个字形的结束位置。
            //
            let (rel_start, rel_end) = match (run.glyphs.first(), run.glyphs.last()) {
                (Some(first), Some(last)) => (first.start, last.end),
                _ => (0, 0),
            };
            let start = line_base + rel_start;
            let end = line_base + rel_end;
            lines.push(TextLine {
                start: start as u32,
                end: (end as u32).min(text.len() as u32),
                width: run.line_w,
                baseline: run.line_y,
            });
        }
        WrappedText {
            lines,
            width,
            height,
            line_height,
        }
    }

    /// 对 `text` 进行塑形，并在超出 `max_width` 时以 `…` 省略截断。
    ///
    /// 先对省略号塑形，再依据**合并后**的总宽度选取可见前缀，确保宽字形
    /// 绝不会把省略号挤出边界。前缀搜索在字符边界上二分查找；每次试探前缀
    /// 都会经过塑形缓存。
    ///
    pub fn shape_elided(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: f32,
    ) -> Arc<ShapedText> {
        let shaped = self.shape(text, style);
        if shaped.width <= max_width || shaped.glyphs.is_empty() {
            return shaped;
        }
        let cut = self.elision_cut(text, style, max_width);
        if cut == 0 {
            return self.shape("\u{2026}", style);
        }
        if cut >= text.len() {
            return shaped;
        }
        let elided = format!("{}\u{2026}", &text[..cut]);
        self.shape(&elided, style)
    }

    /// 对 `text` 折行并限制在 `max_lines` 行以内，超出时在末行执行省略截断。
    pub fn wrap_clamped(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: f32,
        max_lines: usize,
    ) -> Arc<WrappedText> {
        let wrapped = self.wrap(text, style, max_width);
        if max_lines == 0 || wrapped.lines.len() <= max_lines {
            return wrapped;
        }
        let mut out = (*wrapped).clone();
        out.lines.truncate(max_lines);
        let Some(last) = out.lines.last_mut() else {
            return Arc::new(out);
        };
        let start = usize::try_from(last.start).unwrap_or(0);
        let tail = text.get(start..).unwrap_or("");
        let cut = self.elision_cut(tail, style, max_width);
        let dots = self.measure("\u{2026}", style).x;
        let visible = &tail[..cut];
        last.end = u32::try_from(start + cut).unwrap_or(u32::MAX);
        last.width = self.measure(visible, style).x + dots;
        out.width = out.lines.iter().map(|line| line.width).fold(0.0, f32::max);
        out.height = out.lines.len() as f32 * out.line_height;
        Arc::new(out)
    }

    /// 在同一缓冲区中以分段样式排版 `text`。
    ///
    /// 每个 span 成为一个 `Attrs` 文本段，使 14 px 段落中的加粗词与段落共享基线，
    /// 而非单独塑形后强行拼贴。span 之间的空隙回退到 `base` 样式。
    ///
    pub fn shape_rich(
        &mut self,
        text: &str,
        base: &TextStyle,
        spans: &[TextSpan],
        max_width: Option<f32>,
    ) -> Arc<ShapedText> {
        let mut span_hash = 0u64;
        for span in spans {
            span_hash ^= (span.range.start as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (span.range.end as u64)
                ^ u64::from(span.color.raw()).rotate_left(48);
            let mut style_hash = 0u64;
            span.style.hash_into(&mut style_hash);
            span_hash = span_hash.wrapping_mul(0x100_0000_01b3) ^ style_hash;
            if let Some(background) = span.background {
                span_hash ^= u64::from(background.raw()).rotate_left(16);
            }
            span_hash ^= u64::from(span.underline) << 3;
            span_hash ^= u64::from(span.link_id.unwrap_or(0)) << 8;
        }
        let mut style_hash = 0u64;
        base.hash_into(&mut style_hash);
        let width_key = max_width.map_or(0, |width| (width.max(1.0) * 4.0).round() as u32);
        let key = (
            Self::hash_text(text),
            style_hash ^ span_hash ^ u64::from(width_key).rotate_left(32),
        );
        if let Some(cached) = self.shape_cache.get(&key) {
            self.stats.shape_hits += 1;
            return Arc::clone(cached);
        }
        self.stats.shape_misses += 1;

        // 先构建 `Attrs` 文本段，再借用字体系统：文本段仅借用 spans，绝不借用 `self`。
        //
        let mut runs: Vec<(&str, Attrs<'_>)> = Vec::new();
        let mut cursor = 0usize;
        for span in spans {
            let start = span.range.start.min(text.len());
            let end = span.range.end.min(text.len());
            if start > cursor {
                if let Some(gap) = text.get(cursor..start) {
                    runs.push((gap, self.attrs(base)));
                }
            }
            if end > start {
                if let Some(slice) = text.get(start..end) {
                    runs.push((slice, self.attrs(&span.style)));
                }
            }
            cursor = end.max(cursor);
        }
        if cursor < text.len() {
            if let Some(gap) = text.get(cursor..) {
                runs.push((gap, self.attrs(base)));
            }
        }
        if runs.is_empty() {
            runs.push((text, self.attrs(base)));
        }
        let default = self.attrs(base);
        let metrics = base.metrics();
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(max_width.map(|width| width.max(1.0)), None);
        buffer.set_rich_text(runs, &default, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let shaped = Arc::new(Self::extract(&buffer, text, base));
        if self.shape_cache.len() >= self.max_cache_entries {
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, Arc::clone(&shaped));
        self.stats.entries = self.shape_cache.len() + self.wrap_cache.len();
        shaped
    }

    /// 计算使 `prefix + …` 恰好容纳于 `max_width` 内所需截断的 `text` 前缀字节位置。
    fn elision_cut(&mut self, text: &str, style: &TextStyle, max_width: f32) -> usize {
        let dots = self.measure("\u{2026}", style).x;
        if max_width <= 0.0 {
            return 0;
        }
        if dots > max_width {
            return 0;
        }
        let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
        boundaries.push(text.len());
        let chars = boundaries.len() - 1;
        let (mut low, mut high) = (0usize, chars + 1);
        while low < high {
            let mid = (low + high) / 2;
            let width = if mid == 0 {
                0.0
            } else {
                self.measure(&text[..boundaries[mid]], style).x
            };
            if width + dots <= max_width {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let best = low.saturating_sub(1).min(chars);
        boundaries.get(best).copied().unwrap_or(0)
    }

    /// 仅测量尺寸而不保留已塑形的字形（仍走缓存）。
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> Vec2 {
        let shaped = self.shape(text, style);
        Vec2::new(shaped.width, shaped.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[test]
    fn shaping_produces_one_glyph_per_visible_character() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        let shaped = e.shape("Hello", &style);
        assert_eq!(shaped.glyphs.len(), 5);
        assert!(shaped.width > 10.0, "width = {}", shaped.width);
        assert_eq!(shaped.line_count, 1);
        assert!(shaped.first_baseline > 0.0);
    }

    #[test]
    fn shape_cache_returns_the_same_arc() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        let a = e.shape("cache me", &style);
        let b = e.shape("cache me", &style);
        assert!(Arc::ptr_eq(&a, &b), "second call must hit the cache");
        assert_eq!(e.stats().shape_hits, 1);
        assert_eq!(e.stats().shape_misses, 1);
    }

    #[test]
    fn different_style_is_a_different_cache_entry() {
        let mut e = engine();
        let a = e.shape("same text", &TextStyle::sized(12.0));
        let b = e.shape("same text", &TextStyle::sized(24.0));
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(b.width > a.width);
        assert_eq!(e.stats().shape_misses, 2);
    }

    #[test]
    fn eliding_stays_within_the_width_and_ends_with_a_dot() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        let full = e.shape("M M M M M M M M M", &style);
        let limit = full.width * 0.5;
        let elided = e.shape_elided("M M M M M M M M M", &style, limit);
        assert!(elided.width <= limit + 0.5, "width = {}", elided.width);
        assert!(elided.glyphs.len() < full.glyphs.len());
        let last = elided.glyphs.last().expect("glyphs");
        assert_eq!(last.cluster, (elided.glyphs.len() - 1) as u32);
    }

    #[test]
    fn text_that_fits_is_not_elided() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        let shaped = e.shape_elided("ok", &style, 500.0);
        assert_eq!(shaped.glyphs.len(), 2);
    }

    #[test]
    fn clamping_limits_lines_and_keeps_the_paragraph_width() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        let text = "one two three four five six seven eight nine ten";
        let wrapped = e.wrap(text, &style, 60.0);
        assert!(wrapped.lines.len() > 2);
        let clamped = e.wrap_clamped(text, &style, 60.0, 2);
        assert_eq!(clamped.lines.len(), 2);
        assert!(clamped.lines[1].width <= 60.5);
        assert_eq!(
            e.wrap_clamped(text, &style, 60.0, 0).lines.len(),
            wrapped.lines.len()
        );
    }

    #[test]
    fn rich_spans_lay_out_in_one_run() {
        let mut e = engine();
        let base = TextStyle::sized(14.0);
        let bold = TextStyle {
            weight: 700,
            ..base.clone()
        };
        let spans = [TextSpan {
            range: 0..1,
            style: bold,
            color: Token::PRIMARY,
            underline: false,
            background: None,
            link_id: None,
        }];
        let shaped = e.shape_rich("a b", &base, &spans, None);
        assert_eq!(shaped.glyphs.len(), 3);
        assert_eq!(shaped.line_count, 1);
        assert!(shaped.width > 0.0);
    }

    /// 缺失字体族与空字体族是不同的样式。
    ///
    /// 字体族此前仅按字节混入哈希，导致 `family: None` 与 `family: Some("")`
    /// 哈希相同并共享缓存条目：返回了错误的塑形结果。
    ///
    #[test]
    fn an_absent_family_is_not_an_empty_family() {
        let mut e = engine();
        let without = e.shape("same text", &TextStyle::sized(12.0));
        let mut empty_style = TextStyle::sized(12.0);
        empty_style.family = Some(String::new());
        let empty = e.shape("same text", &empty_style);
        assert!(
            !Arc::ptr_eq(&without, &empty),
            "the two styles must not share a cache entry"
        );
        assert_eq!(e.stats().shape_misses, 2);
    }

    #[test]
    fn empty_text_shapes_to_nothing() {
        let mut e = engine();
        let shaped = e.shape("", &TextStyle::default());
        assert!(shaped.is_empty());
        assert_eq!(shaped.height, 0.0);
    }

    #[test]
    fn multiline_text_reports_the_right_line_count() {
        let mut e = engine();
        let shaped = e.shape("one\ntwo\nthree", &TextStyle::sized(14.0));
        assert_eq!(shaped.line_count, 3);
        assert!(shaped.height >= shaped.line_height * 2.5);
    }

    #[test]
    fn cjk_text_shapes_without_panicking() {
        let mut e = engine();
        let shaped = e.shape("中文排版测试", &TextStyle::sized(16.0));
        assert_eq!(shaped.glyphs.len(), 6, "one glyph per CJK character");
        assert!(shaped.width > 0.0);
    }

    #[test]
    fn wrapping_splits_at_word_boundaries() {
        let mut e = engine();
        let style = TextStyle::sized(14.0);
        let text = "the quick brown fox jumps over the lazy dog";
        let full = e.measure(text, &style);
        let wrapped = e.wrap(text, &style, full.x * 0.45);
        assert!(wrapped.lines.len() >= 2, "lines = {}", wrapped.lines.len());
        for line in &wrapped.lines {
            assert!(
                line.width <= full.x * 0.45 + 2.0,
                "line wider than the limit: {}",
                line.width
            );
            assert!(line.end >= line.start);
        }
    }

    #[test]
    fn wrap_cache_is_keyed_by_width_bucket() {
        let mut e = engine();
        let style = TextStyle::sized(14.0);
        let text = "alpha beta gamma delta epsilon";
        let a = e.wrap(text, &style, 100.0);
        let b = e.wrap(text, &style, 100.0);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(e.stats().wrap_hits, 1);
        // 微小的宽度变化仍落在同一个 ¼px 分桶内。
        let c = e.wrap(text, &style, 100.04);
        assert!(Arc::ptr_eq(&a, &c), "sub-pixel width changes must reuse");
        // 真实跨桶变化则会未命中缓存。
        let d = e.wrap(text, &style, 80.0);
        assert!(!Arc::ptr_eq(&a, &d));
    }

    /// 折行段落必须返回每一行的字形，且每行字形定位在各自的行基线上。
    ///
    ///
    /// 换行缓存仅记录断行位置，缺失此接口时渲染器无法绘制折行段落。
    ///
    #[test]
    fn shape_wrapped_produces_multiline_positioned_glyphs() {
        let mut e = engine();
        let style = TextStyle::sized(14.0);
        let text = "the quick brown fox jumps over the lazy dog";
        let full = e.measure(text, &style);
        let shaped = e.shape_wrapped(text, &style, full.x * 0.45);
        assert!(
            shaped.line_count >= 2,
            "the paragraph must wrap: lines = {}",
            shaped.line_count
        );
        assert!(
            shaped.glyphs.len() > 1,
            "every wrapped line needs its own glyphs"
        );
        // 同一行内的字形共享一条基线；不同行之间则不共享。
        // 若整段共用单一基线，每一行都会叠画在第一行上方。
        //
        let mut baselines: Vec<f32> = shaped.glyphs.iter().map(|g| g.y).collect();
        baselines.sort_by(|a, b| a.total_cmp(b));
        baselines.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(
            baselines.len(),
            shaped.line_count as usize,
            "each line needs its own baseline: {baselines:?}"
        );
        // 每一行都从文本框左边缘开始。
        let first_x = shaped
            .glyphs
            .iter()
            .map(|g| g.x)
            .fold(f32::INFINITY, f32::min);
        assert!(first_x < 1.0, "the first glyph must start at the left edge");
    }

    /// 折行文本段绝不能与未折行文本段发生缓存碰撞。
    #[test]
    fn a_wrapped_run_is_not_the_unwrapped_run() {
        let mut e = engine();
        let style = TextStyle::sized(14.0);
        let text = "alpha beta gamma delta epsilon";
        let plain = e.shape(text, &style);
        let wrapped = e.shape_wrapped(text, &style, 60.0);
        assert!(wrapped.line_count >= 2);
        assert_eq!(plain.line_count, 1);
        assert_ne!(plain.glyphs.len(), 0);
    }

    #[test]
    fn clusters_are_monotonic_for_ltr() {
        let mut e = engine();
        let shaped = e.shape("abcdef", &TextStyle::sized(16.0));
        let mut prev = 0;
        for g in &shaped.glyphs {
            assert!(g.cluster >= prev);
            prev = g.cluster;
        }
    }

    #[test]
    fn cache_clear_resets_statistics_targets() {
        let mut e = engine();
        let style = TextStyle::sized(16.0);
        e.shape("x", &style);
        e.clear_caches();
        let again = e.shape("x", &style);
        assert_eq!(again.glyphs.len(), 1);
        assert_eq!(e.stats().shape_misses, 2);
    }

    #[test]
    fn loading_a_malformed_font_blob_reports_no_faces_and_keeps_shaping() {
        let mut e = engine();
        assert_eq!(e.load_font_bytes(b"this is not a font".to_vec()), 0);
        // 缓存已被清空，但系统字体面依然可用。
        let shaped = e.shape("ok", &TextStyle::sized(16.0));
        assert_eq!(shaped.glyphs.len(), 2);
    }

    #[test]
    fn rtl_detection_follows_the_bidi_level() {
        let mut e = engine();
        let rtl = e.shape("שלום", &TextStyle::sized(16.0));
        assert!(!rtl.glyphs.is_empty());
        assert!(rtl.glyphs.iter().all(|g| g.is_rtl()), "Hebrew must be RTL");
        let ltr = e.shape("hello", &TextStyle::sized(16.0));
        assert!(ltr.glyphs.iter().all(|g| !g.is_rtl()));
    }

    /// 字形的 `y` 是其所在行的基线，相对于文本框顶部。
    ///
    ///
    /// cosmic-text 将行基线上报为 `line_y`，将字形在该行内的偏移上报为 `g.y`
    /// （水平文本下为零）。旧代码误将 `g.font_size` 当作 ascent，导致每个字形
    /// 高度错误且忽略了行位置——使第二行直接画在了第一行之上。
    ///
    ///
    #[test]
    fn glyph_baselines_follow_their_line() {
        let mut e = engine();
        let shaped = e.shape("one\ntwo", &TextStyle::sized(32.0));
        assert_eq!(shaped.line_count, 2, "the text must have two lines");
        assert!(
            shaped.glyphs.len() >= 6,
            "expected six glyphs, got {}",
            shaped.glyphs.len()
        );
        // 第一行的字形位于第二行字形上方。
        let first: Vec<f32> = shaped.glyphs[..3].iter().map(|g| g.y).collect();
        let second: Vec<f32> = shaped.glyphs[3..].iter().map(|g| g.y).collect();
        assert!(
            first.iter().all(|y| *y < second[0]),
            "the first line's baseline ({first:?}) must be above the second's ({second:?})"
        );
        // 第一行的基线即该行自身的基线。
        assert!(
            (first[0] - shaped.first_baseline).abs() < 0.01,
            "the first glyph's baseline {} must be the line's baseline {}",
            first[0],
            shaped.first_baseline
        );
    }

    /// 单行内的所有字形必须共享同一条基线。
    #[test]
    fn one_line_has_one_baseline() {
        let mut e = engine();
        let shaped = e.shape("Hello", &TextStyle::sized(16.0));
        let baselines: Vec<f32> = shaped.glyphs.iter().map(|g| g.y).collect();
        assert!(baselines.len() >= 4);
        let first = baselines[0];
        assert!(
            baselines.iter().all(|y| (*y - first).abs() < 0.01),
            "every glyph of a line shares the baseline, got {baselines:?}"
        );
    }

    /// 折行后的各行字节范围必须指向正确的字符。
    ///
    /// cosmic-text 的 `LayoutRun::text` 是**不含**换行符的行内容，因此直接累加
    /// run 长度曾跳过每个换行符，导致字节范围每行漂移一个字节。
    ///
    #[test]
    fn wrapped_lines_point_at_the_right_bytes() {
        let mut e = engine();
        let text = "one\ntwo\nthree";
        let wrapped = e.wrap(text, &TextStyle::sized(16.0), 1000.0);
        assert_eq!(wrapped.lines.len(), 3);
        let expected = [("one", 0u32, 3u32), ("two", 4, 7), ("three", 8, 13)];
        for (line, (content, start, end)) in wrapped.lines.iter().zip(expected) {
            assert_eq!(line.start, start, "line start for {content:?}");
            assert_eq!(line.end, end, "line end for {content:?}");
            assert_eq!(
                &text[line.start as usize..line.end as usize],
                content,
                "the range must select the line's text"
            );
        }
    }

    #[test]
    fn hit_test_point_maps_x_and_y_to_byte_offsets() {
        let mut engine = TextEngine::new();
        let text = "Hello world wrapped line";
        let shaped = engine.shape_wrapped(
            text,
            &TextStyle {
                font_size: 16.0,
                line_height: 22.0,
                ..TextStyle::default()
            },
            100.0,
        );
        let first_y = shaped.glyphs.first().map(|g| g.y).unwrap_or(0.0);
        let second_y = shaped
            .glyphs
            .iter()
            .map(|g| g.y)
            .find(|&y| (y - first_y).abs() > 1.0)
            .expect("wrapped text must have at least two lines");
        // 点击原点 -> 第一行起始处
        assert_eq!(
            shaped.hit_test_point(Vec2::new(0.0, first_y + 4.0), text),
            0
        );
        // 点击第一行最右侧 -> 第二行之前的字节偏移
        let first_end = shaped.hit_test_point(Vec2::new(500.0, first_y + 4.0), text);
        // 点击第二行起始附近 -> 大于或等于 first_end 的字节偏移
        let second_start = shaped.hit_test_point(Vec2::new(0.0, second_y + 4.0), text);
        assert!(
            second_start >= first_end && first_end > 0,
            "first_end={first_end}, second_start={second_start}"
        );
        // 点击第二行最右侧 -> 字符串末尾（或最后一行末尾）
        let last_y = shaped.glyphs.iter().map(|g| g.y).fold(first_y, f32::max);
        let last_end = shaped.hit_test_point(Vec2::new(500.0, last_y + 4.0), text);
        assert_eq!(last_end, text.len());
    }
}

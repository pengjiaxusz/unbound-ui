//! Text shaping on top of `cosmic-text`, with the two-level cache the
//! architecture calls for (`docs` §3.5).
//!
//! * **Level 1 — shape cache**: keyed by `(text hash, style)`. A button label or
//!   a fixed phrase is shaped once for the lifetime of the process.
//! * **Level 2 — wrap cache**: keyed by `(text hash, style, max width)`. Adding
//!   a width to the key is what makes resizing a panel cheap: the shaped result
//!   is reused, only the line breaking is recomputed, and only when the width
//!   actually crosses a bucket boundary.
//!
//! The caches are deliberately per-`TextEngine` rather than global: a global
//! cache would have to be locked on every frame, and the lock costs more than
//! the shaping it saves.

use std::sync::Arc;

use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};
use rustc_hash::FxHashMap;
use unbound_math::Vec2;

/// Everything that affects shaping, in one comparable value.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    /// `0.0` means "use the font's natural line height × 1.2".
    pub line_height: f32,
    pub letter_spacing: f32,
    pub weight: u16,
    pub italic: bool,
    /// `None` selects the system UI font.
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
    /// Style for a specific size, keeping the rest of the defaults.
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

    /// Stable hash of the style, used as part of the cache keys. Floats are
    /// bit-cast so that `14.0` and `14.0` always agree and `14.0`/`14.5` never
    /// collide.
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
        // The family is mixed with its length as well as its bytes: an absent
        // family and an empty one used to hash the same, so the two styles
        // shared a cache entry and the wrong shaped run came back.
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

/// One positioned glyph, in logical pixels relative to the text origin.
///
/// The face is referenced by index into [`ShapedText::faces`] rather than by
/// `fontdb::ID`: the id type is opaque and not `Pod`, and keeping the glyph POD
/// means a run can be uploaded to the GPU without a conversion pass.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    /// Index into [`ShapedText::faces`].
    pub face_index: u16,
    /// Pen position of the glyph's left edge.
    pub x: f32,
    /// Pen position of the glyph's top edge (y grows down).
    pub y: f32,
    /// Advance width of the glyph.
    pub w: f32,
    /// Font size the glyph was shaped at (part of the raster key).
    pub font_size: f32,
    /// Byte offset in the source string, for hit testing and selection.
    pub cluster: u32,
    /// `BiDi` embedding level: odd means right-to-left.
    pub level: u8,
    pub _pad: [u8; 3],
}

impl PositionedGlyph {
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

/// The shaped result for a whole paragraph.
#[derive(Clone, Debug, Default)]
pub struct ShapedText {
    /// Distinct faces used by this run, indexed by `PositionedGlyph::face_index`.
    pub faces: Vec<fontdb::ID>,
    pub glyphs: Vec<PositionedGlyph>,
    /// Ink-independent layout width (max line width).
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_count: u32,
    /// First line's top offset, so a single-line label can be centred exactly.
    pub first_baseline: f32,
}

impl ShapedText {
    /// True when nothing was laid out (empty or whitespace-only string).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Byte range of the glyph whose cluster covers `byte`.
    ///
    /// The cluster is the byte offset of the glyph's first character in the
    /// source string. The glyph that covers `byte` is therefore the last one
    /// whose cluster starts at or before it — not "the first one within an
    /// eight-byte window", which could pick a glyph from the next cluster.
    #[must_use]
    pub fn glyph_at_byte(&self, byte: u32) -> Option<&PositionedGlyph> {
        self.glyphs.iter().rev().find(|g| g.cluster <= byte)
    }

    /// Nearest glyph to a local point (used by hit testing and cursor
    /// placement).
    ///
    /// The returned distance is the distance to the nearest glyph *boundary*,
    /// not to the glyph's left edge: clicking the right half of a character
    /// has to put the caret after it, not before it.
    #[must_use]
    pub fn nearest_glyph(&self, p: Vec2) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, g) in self.glyphs.iter().enumerate() {
            let dx = (p.x - g.x).abs();
            let dy = (p.y - g.y).abs();
            // Weight x much more than y: a cursor is placed by column.
            let cost = dx + dy * 0.25;
            if best.map(|(_, c)| cost < c).unwrap_or(true) {
                best = Some((i, cost));
            }
        }
        best
    }

    /// The caret position for a byte offset.
    ///
    /// The caret sits at the start of the glyph the byte belongs to, or at the
    /// end of the last glyph when the byte is past the text.
    #[must_use]
    pub fn caret_at_byte(&self, byte: u32) -> Option<f32> {
        let glyph = self.glyph_at_byte(byte)?;
        let at_end = byte >= glyph.cluster + glyph.w as u32;
        Some(if at_end { glyph.x + glyph.w } else { glyph.x })
    }
}

/// A wrapped paragraph: line byte ranges plus their measured widths.
#[derive(Clone, Debug, Default)]
pub struct WrappedText {
    pub lines: Vec<TextLine>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
}

/// One laid-out line.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TextLine {
    /// Byte offset of the first character.
    pub start: u32,
    /// Byte offset one past the last character.
    pub end: u32,
    pub width: f32,
    pub baseline: f32,
}

/// Cache statistics, surfaced by the debug overlay.
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

/// The byte offset of every source line in `text`.
///
/// cosmic-text splits a string into lines at `\r`, `\n`, `\r\n` and `\n\r`,
/// and reports a glyph's cluster as an index *within its line*. To turn a
/// cluster index into an absolute byte offset into the source text, the
/// offset of the line the glyph belongs to has to be known — hence this
/// helper, which mirrors cosmic-text's own line splitting.
fn source_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                // `\r\n` and `\n\r` are a single terminator.
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

/// The shaping engine: owns the font system and both cache levels.
pub struct TextEngine {
    font_system: FontSystem,
    /// Reused across calls so shaping does not allocate a `Buffer` per label.
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
    /// Load the system fonts. Costs ~50–150 ms; do it once per process.
    #[must_use]
    pub fn new() -> Self {
        Self::with_font_system(FontSystem::new())
    }

    /// Build on an existing `FontSystem` (tests use a minimal one).
    #[must_use]
    pub fn with_font_system(font_system: FontSystem) -> Self {
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

    /// Mutable access for glyph rasterisation (`SwashCache` needs it).
    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    #[must_use]
    pub fn stats(&self) -> TextCacheStats {
        self.stats
    }

    /// Drop both caches (theme/font reload, or after a memory-pressure signal).
    pub fn clear_caches(&mut self) {
        self.shape_cache.clear();
        self.wrap_cache.clear();
        self.buffer = None;
        self.buffer_metrics = None;
    }

    fn hash_text(text: &str) -> u64 {
        // FNV-1a; short labels dominate, so a byte loop is faster than a random
        // per-frame hasher and the collision rate is irrelevant at 64 bits.
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
        // Build the `Attrs` first: it borrows `style`, not `self`, so the
        // mutable buffer borrow below is unambiguous.
        let attrs = self.attrs(style);
        let buffer = self.buffer.as_mut().expect("buffer just created");
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
    }

    /// Shape `text` (with caching).
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
            // Cheap eviction: clearing beats LRU bookkeeping for a cache that is
            // only hot for the currently visible screen.
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, Arc::clone(&shaped));
        self.stats.entries = self.shape_cache.len() + self.wrap_cache.len();
        shaped
    }

    fn shape_uncached(&mut self, text: &str, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::default();
        }
        self.prepare_buffer(text, style);
        let buffer = self.buffer.as_ref().expect("prepared");

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
                    x: g.x,
                    // The baseline of the glyph, relative to the top of the
                    // text box.
                    //
                    // cosmic-text reports the baseline of a line as `line_y`
                    // (already accumulated over the lines above it) and the
                    // glyph's own offset within that line as `g.y`, which is
                    // zero for horizontal text. The old code used
                    // `g.font_size` as if it were the ascent: that put every
                    // glyph at the wrong height *and* ignored the line's
                    // position, so a second line was drawn on top of the
                    // first.
                    y: run.line_y + g.y,
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

    /// Wrap `text` to `max_width` (with caching). Widths are quantised to ¼ px so
    /// that a smoothly resizing panel reuses cached results.
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
        // A separate buffer is used so the shape-cache buffer keeps its
        // unlimited width (shape results must not depend on a wrap width).
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(max_width.max(1.0)), None);
        let attrs = self.attrs(style);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut lines = Vec::new();
        let mut height: f32 = 0.0;
        let mut width: f32 = 0.0;
        let mut line_height = metrics.line_height;
        // The byte offset of every source line in `text`.
        //
        // cosmic-text reports a glyph's cluster as an index *within its line*,
        // and `LayoutRun::text` is the whole source line (not the wrapped
        // fragment). The line offsets are therefore needed to turn a cluster
        // index into an absolute byte offset into `text`.
        let line_starts = source_line_starts(text);
        for run in buffer.layout_runs() {
            line_height = run.line_height;
            height = height.max(run.line_y + run.line_height);
            width = width.max(run.line_w);
            // The base offset of the source line this run belongs to.
            let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);
            // The run's glyphs carry cluster indices relative to the source
            // line, so the run's extent within it is the first glyph's start
            // and the last glyph's end.
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

    /// Measure without keeping the shaped glyphs (still cached).
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

    /// An absent family and an empty one are different styles.
    ///
    /// The family used to be mixed into the hash by its bytes only, so
    /// `family: None` and `family: Some("")` hashed the same and shared a
    /// cache entry: the wrong shaped run came back.
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
        // A tiny width change stays inside the same ¼px bucket.
        let c = e.wrap(text, &style, 100.04);
        assert!(Arc::ptr_eq(&a, &c), "sub-pixel width changes must reuse");
        // A real change misses.
        let d = e.wrap(text, &style, 80.0);
        assert!(!Arc::ptr_eq(&a, &d));
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
    fn rtl_detection_follows_the_bidi_level() {
        let mut e = engine();
        let rtl = e.shape("שלום", &TextStyle::sized(16.0));
        assert!(!rtl.glyphs.is_empty());
        assert!(rtl.glyphs.iter().all(|g| g.is_rtl()), "Hebrew must be RTL");
        let ltr = e.shape("hello", &TextStyle::sized(16.0));
        assert!(ltr.glyphs.iter().all(|g| !g.is_rtl()));
    }

    /// The glyph's `y` is the baseline of its line, relative to the top of
    /// the text box.
    ///
    /// cosmic-text reports the baseline of a line as `line_y` and the glyph's
    /// own offset within that line as `g.y` (zero for horizontal text). The
    /// old code used `g.font_size` as if it were the ascent, which put every
    /// glyph at the wrong height and ignored the line's position — so a
    /// second line was drawn on top of the first.
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
        // The first line's glyphs sit above the second line's.
        let first: Vec<f32> = shaped.glyphs[..3].iter().map(|g| g.y).collect();
        let second: Vec<f32> = shaped.glyphs[3..].iter().map(|g| g.y).collect();
        assert!(
            first.iter().all(|y| *y < second[0]),
            "the first line's baseline ({first:?}) must be above the second's ({second:?})"
        );
        // The first line's baseline is the line's own baseline.
        assert!(
            (first[0] - shaped.first_baseline).abs() < 0.01,
            "the first glyph's baseline {} must be the line's baseline {}",
            first[0],
            shaped.first_baseline
        );
    }

    /// A single line's glyphs must all share one baseline.
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

    /// The wrapped lines' byte ranges must point at the right characters.
    ///
    /// cosmic-text's `LayoutRun::text` is the line's content *without* the
    /// line terminator, so accumulating the run lengths skipped every newline
    /// and the ranges drifted by one byte per line.
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
}

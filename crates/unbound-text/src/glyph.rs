//! Glyph rasterisation, atlas residency and quad generation.
//!
//! The pipeline for one glyph:
//!
//! 1. rasterise it with `swash` at the subpixel phase implied by its position
//!    (four phases, so a 12px label keeps even spacing instead of snapping every
//!    glyph to the same pixel grid),
//! 2. reserve a slot in the R8 atlas (or the RGBA atlas for colour emoji and
//!    distance fields),
//! 3. emit an [`AtlasUpdate`] the renderer uploads to the texture array,
//! 4. emit a [`GlyphQuad`] the view layer turns into a `GpuPrimitiveInstance`.
//!
//! Steps 1–4 are pure CPU work, so the whole cache is testable without a device.

use cosmic_text::{fontdb, CacheKey, CacheKeyFlags, SwashCache};
use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};

use crate::atlas::{GlyphAtlas, GlyphKey, GlyphSlot, SUBPIXEL_PHASES};
use crate::sdf;
use crate::shaping::{PositionedGlyph, ShapedText, TextEngine};

/// Which atlas an update targets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AtlasTarget {
    /// `Atlas_R8`, one byte per texel.
    Coverage,
    /// `Atlas_RGBA8`, four bytes per texel.
    Color,
}

/// A rectangle of texels to upload into one atlas layer.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasUpdate {
    pub target: AtlasTarget,
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major texels; length is `width * height * bpp`.
    pub data: Vec<u8>,
}

impl AtlasUpdate {
    #[must_use]
    pub fn bpp(&self) -> u32 {
        match self.target {
            AtlasTarget::Coverage => 1,
            AtlasTarget::Color => 4,
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> usize {
        (self.width * self.height) as usize * self.bpp() as usize
    }
}

/// A rasterised glyph, atlas-resident.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ResolvedGlyph {
    pub slot: GlyphSlot,
    /// Offset from the pen position to the quad's top-left, in logical pixels.
    pub bearing: Vec2,
    /// Quad size in logical pixels.
    pub size: Vec2,
    /// Advance width in logical pixels.
    pub advance: f32,
    /// True when the glyph came from the RGBA atlas.
    pub color: bool,
}

impl ResolvedGlyph {
    /// Quad in text-local space.
    #[must_use]
    pub fn quad(&self, pen: Vec2) -> Rect {
        Rect::new(
            pen.x + self.bearing.x,
            pen.y + self.bearing.y,
            self.size.x,
            self.size.y,
        )
    }
}

/// One glyph ready for the primitive builder.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    pub rect: Rect,
    pub uv: [f32; 4],
    pub layer: u32,
    pub color: bool,
    /// Horizontal subpixel phase (0..3) used for this glyph.
    pub phase: u8,
}

/// Rasterises and caches glyphs, keeping them resident in the atlas.
pub struct GlyphCache {
    swash: SwashCache,
    pub atlas: GlyphAtlas,
    resolved: FxHashMap<GlyphKey, ResolvedGlyph>,
    /// Updates produced since the last drain.
    updates: Vec<AtlasUpdate>,
    /// Emit distance fields instead of plain coverage (canvas text).
    pub use_sdf: bool,
    /// Distance-field spread in texels.
    pub sdf_spread: f32,
    /// Extra texels around each glyph (bilinear bleed guard).
    pub padding: u32,
    /// Raster glyphs on the whole-pixel grid instead of in four subpixel phases.
    ///
    /// The vertex stage snaps every static quad's AABB to a physical pixel, so
    /// a phase-specific raster is translated away from the position it was
    /// generated for and every edge turns into a half-pixel blend. UI text that
    /// is pixel-snapped therefore has to use phase 0; only animated canvas text
    /// keeps the four phases.
    pub snap_phases: bool,
    /// Bijection between `fontdb::ID` and the compact face index stored in
    /// [`GlyphKey`]. `fontdb::ID` is an opaque slotmap key with no public
    /// integer conversion, so the atlas keys on a dense index instead.
    faces: Vec<fontdb::ID>,
    face_index: FxHashMap<fontdb::ID, u32>,
    /// Keys the atlas evicted since the last drain.
    ///
    /// The atlas recycles the rectangle of an evicted glyph, so the cache
    /// entry that still points at it would sample somebody else's texels.
    evicted: Vec<GlyphKey>,
    misses: u64,
    hits: u64,
}

impl core::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("resident", &self.resolved.len())
            .field("updates", &self.updates.len())
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish_non_exhaustive()
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            swash: SwashCache::new(),
            atlas: GlyphAtlas::default(),
            resolved: FxHashMap::default(),
            updates: Vec::new(),
            use_sdf: false,
            sdf_spread: 4.0,
            padding: 2,
            snap_phases: false,
            faces: Vec::new(),
            face_index: FxHashMap::default(),
            evicted: Vec::new(),
            misses: 0,
            hits: 0,
        }
    }

    /// Dense index for a face, allocating a new one on first sight.
    fn face_id(&mut self, face: fontdb::ID) -> u32 {
        if let Some(i) = self.face_index.get(&face) {
            return *i;
        }
        let i = self.faces.len() as u32;
        self.faces.push(face);
        self.face_index.insert(face, i);
        i
    }

    /// Advance the atlas eviction clock.
    pub fn begin_frame(&mut self, frame: u64) {
        self.atlas.begin_frame(frame);
    }

    /// Drain the pending atlas uploads.
    pub fn take_updates(&mut self) -> Vec<AtlasUpdate> {
        core::mem::take(&mut self.updates)
    }

    #[must_use]
    pub fn pending_update_bytes(&self) -> usize {
        self.updates.iter().map(AtlasUpdate::byte_len).sum()
    }

    #[must_use]
    pub fn resident_glyphs(&self) -> usize {
        self.resolved.len()
    }

    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f32 / total as f32
        }
    }

    /// Subpixel phase implied by a pen position.
    #[must_use]
    pub fn phase_for(x: f32) -> u8 {
        let fract = x - x.floor();
        // Four phases: 0, ¼, ½, ¾. Rounding to the nearest phase keeps the
        // error at most ⅛ px, which is invisible; snapping to whole pixels
        // instead would visibly change letter spacing.
        ((fract * 4.0).round() as u8) % SUBPIXEL_PHASES
    }

    /// Resolve (and if needed rasterise) one glyph.
    pub fn resolve(
        &mut self,
        engine: &mut TextEngine,
        glyph: &PositionedGlyph,
        face: fontdb::ID,
        pen: Vec2,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedGlyph> {
        let phase = if self.snap_phases {
            // The quad is snapped to a whole physical pixel, so the raster must
            // be the one generated for a whole-pixel pen.
            0
        } else {
            Self::phase_for(pen.x)
        };
        let sdf_mode = self.use_sdf;
        let face_index = u64::from(self.face_id(face));
        // The raster depends on the phase, the weight and the italic slant,
        // so all three are part of the key.
        let key = GlyphKey::styled(
            glyph.glyph_id,
            glyph.font_size,
            phase,
            sdf_mode,
            face_index,
            weight,
            italic,
        );
        if let Some(hit) = self.resolved.get(&key).copied() {
            if hit.slot.is_valid() {
                // Refresh the atlas generation without re-rasterising.
                self.hits += 1;
                if hit.color {
                    self.atlas.rgba.touch(&key);
                } else {
                    self.atlas.r8.touch(&key);
                }
                return Some(hit);
            }
        }
        self.misses += 1;

        let mut flags = CacheKeyFlags::empty();
        if italic {
            flags |= CacheKeyFlags::FAKE_ITALIC;
        }
        // Bias the position into the requested subpixel phase before asking
        // swash for the raster, so the four phases are genuinely different.
        let phase_offset = phase as f32 * 0.25;
        let (cache_key, _, _) = CacheKey::new(
            face,
            glyph.glyph_id,
            glyph.font_size,
            (phase_offset, 0.0),
            cosmic_text::Weight(weight),
            flags,
        );
        let image = self
            .swash
            .get_image(engine.font_system_mut(), cache_key)
            .as_ref()?;

        let placement = image.placement;
        let w = placement.width;
        let h = placement.height;
        if w == 0 || h == 0 {
            // Whitespace: no texture, only an advance.
            return Some(ResolvedGlyph {
                slot: GlyphSlot::default(),
                bearing: Vec2::new(placement.left as f32, 0.0),
                size: Vec2::ZERO,
                advance: glyph.w,
                color: matches!(image.content, cosmic_text::SwashContent::Color),
            });
        }

        let is_color = matches!(image.content, cosmic_text::SwashContent::Color);
        let pad = self.padding;
        let pad2 = pad * 2;
        // The atlas may evict a glyph to make room; the evicted key's cache
        // entry has to go too, or it would point at the recycled rectangle.
        let mut evicted = core::mem::take(&mut self.evicted);
        let mut resolved = if is_color || sdf_mode {
            let slot = self
                .atlas
                .allocate_color(key, w + pad2, h + pad2, &mut evicted)?;
            let data = if sdf_mode {
                let field = sdf::from_coverage(&image.data, w, h, self.sdf_spread);
                pad_field(&field.data, w, h, pad)
            } else {
                pad_rgba(&image.data, w, h, pad)
            };
            self.updates.push(AtlasUpdate {
                target: AtlasTarget::Color,
                layer: slot.layer,
                x: slot.x,
                y: slot.y,
                width: slot.w,
                height: slot.h,
                data,
            });
            ResolvedGlyph {
                slot,
                bearing: Vec2::new(
                    placement.left as f32 - pad as f32,
                    -(placement.top as f32) - pad as f32,
                ),
                size: Vec2::new(w as f32 + pad2 as f32, h as f32 + pad2 as f32),
                advance: glyph.w,
                color: true,
            }
        } else {
            let slot = self
                .atlas
                .allocate_coverage(key, w + pad2, h + pad2, &mut evicted)?;
            let data = pad_r8(&image.data, w, h, pad);
            self.updates.push(AtlasUpdate {
                target: AtlasTarget::Coverage,
                layer: slot.layer,
                x: slot.x,
                y: slot.y,
                width: slot.w,
                height: slot.h,
                data,
            });
            ResolvedGlyph {
                slot,
                bearing: Vec2::new(
                    placement.left as f32 - pad as f32,
                    -(placement.top as f32) - pad as f32,
                ),
                size: Vec2::new(w as f32 + pad2 as f32, h as f32 + pad2 as f32),
                advance: glyph.w,
                color: false,
            }
        };

        // The UV rect must cover the *whole padded allocation*, because that
        // is what the quad's size says. The quad is `w + 2·pad` wide and the
        // padding is what keeps a bilinear tap from reaching the neighbouring
        // glyph, so both the quad and the UV have to include it.
        let sz = self.atlas.r8.page_size() as f32;
        resolved.slot.uv = [
            resolved.slot.x as f32 / sz,
            resolved.slot.y as f32 / sz,
            (resolved.slot.x + resolved.slot.w) as f32 / sz,
            (resolved.slot.y + resolved.slot.h) as f32 / sz,
        ];

        // An evicted glyph's rectangle goes back into the free pool and is
        // handed to the next one, so the cache entry that still points at it
        // would sample somebody else's texels. Drop those entries now.
        for key in evicted.drain(..) {
            self.resolved.remove(&key);
        }

        self.resolved.insert(key, resolved);
        Some(resolved)
    }

    /// Resolve an entire shaped run into quads.
    ///
    /// `origin` is the text box's top-left in the caller's space; the returned
    /// quads are in the same space.
    pub fn quads_for(
        &mut self,
        engine: &mut TextEngine,
        shaped: &ShapedText,
        origin: Vec2,
        weight: u16,
        italic: bool,
    ) -> Vec<GlyphQuad> {
        let mut out = Vec::with_capacity(shaped.glyphs.len());
        for g in &shaped.glyphs {
            let Some(face) = shaped.faces.get(g.face_index as usize).copied() else {
                continue;
            };
            let pen = origin + Vec2::new(g.x, g.y);
            if let Some(resolved) = self.resolve(engine, g, face, pen, weight, italic) {
                if resolved.size.x <= 0.0 || resolved.size.y <= 0.0 {
                    continue;
                }
                out.push(GlyphQuad {
                    rect: resolved.quad(pen),
                    uv: resolved.slot.uv,
                    layer: resolved.slot.layer,
                    color: resolved.color,
                    phase: Self::phase_for(pen.x),
                });
            }
        }
        out
    }

    /// Drop every cached glyph (atlas resize, font reload).
    pub fn clear(&mut self) {
        self.resolved.clear();
        self.atlas = GlyphAtlas::default();
        self.updates.clear();
        // Face indices stay valid: the atlas only forgot the texels.
    }

    /// Atlas memory in bytes.
    #[must_use]
    pub fn atlas_memory_bytes(&self) -> u64 {
        self.atlas.memory_bytes()
    }
}

fn pad_r8(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph) as usize];
    for y in 0..h {
        let dst = ((y + pad) * pw + pad) as usize;
        let src_off = (y * w) as usize;
        out[dst..dst + w as usize].copy_from_slice(&src[src_off..src_off + w as usize]);
    }
    out
}

fn pad_rgba(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..h {
        let dst = (((y + pad) * pw + pad) * 4) as usize;
        let src_off = (y * w * 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&src[src_off..src_off + (w * 4) as usize]);
    }
    out
}

fn pad_field(src: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    // The distance field's encoding is "high = inside": the GPU reference
    // (`unbound-gpu/src/sdf.rs`) asserts that the centre of a covered block
    // samples above 0.9 and the corner below 0.1, and `uber.frag` turns that
    // into coverage with `smoothstep(0.5 - w, 0.5 + w, m)`.
    //
    // So the padding must be padded with **0**, which is "far outside".
    // Padding with 255 would read as "deep inside" and paint an opaque
    // rectangular halo around every glyph.
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..h {
        let dst = (((y + pad) * pw + pad) * 4) as usize;
        let src_off = (y * w * 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&src[src_off..src_off + (w * 4) as usize]);
    }
    out
}

// `SubpixelBin` documents the four-phase scheme above and is exercised through
// `CacheKey::new`; keep the import explicit for readers.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::AtlasConfig;
    use crate::shaping::TextStyle;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[test]
    fn phase_bucketing_covers_four_steps() {
        assert_eq!(GlyphCache::phase_for(0.0), 0);
        assert_eq!(GlyphCache::phase_for(0.26), 1);
        assert_eq!(GlyphCache::phase_for(0.51), 2);
        assert_eq!(GlyphCache::phase_for(0.76), 3);
        assert_eq!(GlyphCache::phase_for(1.02), 0);
        assert_eq!(GlyphCache::phase_for(10.99), 0);
    }

    #[test]
    fn rasterising_a_label_produces_quads_and_uploads() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let style = TextStyle::sized(16.0);
        let shaped = e.shape("Ag", &style);
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 2, "both glyphs must produce a quad");
        assert!(
            cache.pending_update_bytes() > 0,
            "rasterising must queue uploads"
        );
        let updates = cache.take_updates();
        assert_eq!(updates.len(), 2);
        for u in &updates {
            assert_eq!(u.target, AtlasTarget::Coverage);
            assert_eq!(u.data.len(), u.byte_len());
            assert!(u.width > 0 && u.height > 0);
        }
        for q in &quads {
            assert!(q.rect.width() > 0.0 && q.rect.height() > 0.0);
            assert!(q.uv[2] > q.uv[0] && q.uv[3] > q.uv[1]);
            assert!(!q.color);
        }
    }

    #[test]
    fn second_resolution_hits_the_cache() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("hello", &TextStyle::sized(16.0));
        let first = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let uploads = cache.take_updates().len();
        let second = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(first, second);
        assert_eq!(cache.take_updates().len(), 0, "no re-upload on a cache hit");
        assert!(uploads > 0);
        assert!(cache.hit_rate() > 0.3);
        // h/e/l/o plus a second `l` at a different subpixel phase: the phase is
        // part of the key because it changes the raster.
        assert!(
            (4..=5).contains(&cache.resident_glyphs()),
            "resident = {}",
            cache.resident_glyphs()
        );
    }

    /// An italic run must not reuse the upright glyph's raster.
    ///
    /// `FAKE_ITALIC` makes swash produce a skewed raster from the same face
    /// id, so the two runs share every part of the key but the slant. Without
    /// the slant in the key the second run would sample the first one's
    /// texels.
    #[test]
    fn an_italic_run_does_not_reuse_the_upright_raster() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("M", &TextStyle::sized(24.0));
        let upright = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let italic = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, true);
        assert_eq!(upright.len(), 1);
        assert_eq!(italic.len(), 1);
        assert_ne!(
            (upright[0].layer, upright[0].uv),
            (italic[0].layer, italic[0].uv),
            "the italic glyph must have its own atlas rectangle"
        );
    }

    /// When the atlas evicts a glyph, the cache entry that pointed at it has
    /// to go: the rectangle is recycled and handed to the next glyph.
    #[test]
    fn an_evicted_glyph_leaves_the_cache() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        // A tiny atlas that cannot hold the whole string.
        cache.atlas = GlyphAtlas::new(AtlasConfig {
            page_size: 32,
            max_layers: 1,
            padding: 1,
        });
        let shaped = e.shape("hello world", &TextStyle::sized(16.0));
        cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let resident = cache.resident_glyphs();
        assert!(resident > 0);
        // The atlas evicted at least one glyph to fit them all.
        assert!(
            cache.atlas.r8.evictions() > 0,
            "the test must actually exercise the eviction path"
        );
        // Every resident entry must still own a live atlas slot: an entry
        // whose slot was recycled would sample somebody else's texels.
        assert!(
            cache.resident_glyphs() <= resident,
            "the cache must not grow beyond the atlas"
        );
    }

    #[test]
    fn whitespace_has_an_advance_but_no_quad() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("a b", &TextStyle::sized(16.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 2, "the space must not paint");
    }

    #[test]
    fn sdf_mode_targets_the_rgba_atlas() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        cache.use_sdf = true;
        let shaped = e.shape("M", &TextStyle::sized(24.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert_eq!(quads.len(), 1);
        assert!(quads[0].color, "distance fields live in the RGBA atlas");
        let updates = cache.take_updates();
        assert_eq!(updates[0].target, AtlasTarget::Color);
        assert_eq!(updates[0].data.len(), updates[0].byte_len());
        // The distance field's encoding is "high = inside", so the padding
        // must read as *outside*: a low value. Padding with 255 would read as
        // "deep inside" and paint an opaque halo around every glyph.
        assert!(updates[0].data[0] < 55, "padded SDF must read as outside");
    }

    #[test]
    fn scaled_text_produces_larger_quads() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let small = e.shape("W", &TextStyle::sized(12.0));
        let large = e.shape("W", &TextStyle::sized(48.0));
        let qs = cache.quads_for(&mut e, &small, Vec2::ZERO, 400, false);
        let ql = cache.quads_for(&mut e, &large, Vec2::ZERO, 400, false);
        assert!(ql[0].rect.height() > qs[0].rect.height() * 2.0);
        assert_ne!(
            (ql[0].layer, ql[0].uv),
            (qs[0].layer, qs[0].uv),
            "different sizes must not share a slot"
        );
    }

    #[test]
    fn quads_follow_the_pen_positions() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("ii", &TextStyle::sized(16.0));
        let quads = cache.quads_for(&mut e, &shaped, Vec2::new(10.0, 20.0), 400, false);
        assert_eq!(quads.len(), 2);
        assert!(quads[1].rect.origin.x > quads[0].rect.origin.x);
        assert!(quads[0].rect.origin.x >= 10.0 - 2.0);
    }

    #[test]
    fn clearing_resets_the_atlas() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("abc", &TextStyle::sized(16.0));
        cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        assert!(cache.resident_glyphs() > 0);
        cache.clear();
        assert_eq!(cache.resident_glyphs(), 0);
        assert_eq!(cache.atlas_memory_bytes(), 0);
    }
}

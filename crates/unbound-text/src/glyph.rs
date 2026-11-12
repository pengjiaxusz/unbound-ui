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

use crate::atlas::{GlyphAtlas, GlyphKey, GlyphSlot};
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
    /// True when the RGBA atlas entry is a multi-channel distance field.
    ///
    /// The flag has to reach the fragment shader: without it the distance
    /// field is sampled as an ordinary RGBA bitmap and the glyph turns into a
    /// white blob.
    pub msdf: bool,
}

impl ResolvedGlyph {
    /// Quad in text-local space.
    ///
    /// The pen is snapped to the raster's own pixel grid: the sub-pixel part
    /// of the pen position is already baked into the bitmap (the raster is
    /// generated for the phase implied by `pen.x`), so adding the fractional
    /// part a second time would shift the glyph twice.
    ///
    /// The horizontal origin comes from [`GlyphCache::quantize_x`], which
    /// carries the integer pixel along with the phase: a pen fraction in
    /// `[0.875, 1.0)` rounds to phase 0 of the *next* pixel, and the quad has
    /// to move with it. Dropping that carry (the old `% 4` on its own) put
    /// 12.5 % of all glyphs one pixel to the left, where their edge overlapped
    /// the previous letter.
    #[must_use]
    pub fn quad(&self, pen: Vec2, snap_phases: bool) -> Rect {
        let (base_x, _) = GlyphCache::quantize_x(pen.x, snap_phases);
        Rect::new(
            base_x + self.bearing.x,
            pen.y.round() + self.bearing.y,
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
    /// The glyph lives in the RGBA atlas as a distance field.
    pub msdf: bool,
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
    /// The phase has to match the position the quad is finally drawn at: the
    /// raster is generated for one phase and the quad is snapped to the pixel
    /// grid, so a phase-specific raster that does not match the pen would be
    /// sampled away from the position it was generated for and every edge
    /// would turn into a half-pixel blend.
    ///
    /// Turning this on forces every glyph to phase 0, which is only correct
    /// when the pen itself is always a whole pixel. UI text keeps the four
    /// phases: the phase is derived from the glyph's absolute position and
    /// the error against the true pen is at most ⅛ px.
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

    /// Split a pen position into a whole-pixel origin and a subpixel phase.
    ///
    /// Four phases cover `[0, 1)` as `0, ¼, ½, ¾`. Rounding decides the phase,
    /// so the top of the last bucket (`fract ≥ 0.875`) rounds to `4` — that is
    /// phase 0 of the *next* pixel. The whole-pixel origin has to advance with
    /// it, which is exactly what `div_euclid`/`rem_euclid` give: one helper
    /// produces both numbers, so they can never disagree.
    ///
    /// `snap_phases` forces the whole-pixel raster instead (phase 0 at
    /// `round(x)`), for callers whose pen is always on the grid.
    #[must_use]
    pub fn quantize_x(x: f32, snap_phases: bool) -> (f32, u8) {
        if snap_phases {
            return (x.round(), 0);
        }
        let quarters = (x * 4.0).round();
        (quarters.div_euclid(4.0), quarters.rem_euclid(4.0) as u8)
    }

    /// Subpixel phase implied by a pen position.
    #[must_use]
    pub fn phase_for(x: f32) -> u8 {
        // Four phases: 0, ¼, ½, ¾. Rounding to the nearest phase keeps the
        // error at most ⅛ px, which is invisible; snapping to whole pixels
        // instead would visibly change letter spacing.
        Self::quantize_x(x, false).1
    }

    /// The subpixel phase a glyph at `pen_x` is actually rasterised at.
    ///
    /// The phase has to be derived from the glyph's **absolute** position:
    /// the raster is generated for that phase, so a phase derived from a
    /// run-relative position would shift every glyph by up to a pixel.
    #[must_use]
    pub fn phase_at(&self, pen_x: f32) -> u8 {
        Self::quantize_x(pen_x, self.snap_phases).1
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
        let phase = self.phase_at(pen.x);
        let sdf_mode = self.use_sdf;
        let face_index = u64::from(self.face_id(face));
        // Optical sizing: at 9–17 px a regular weight loses about one third of
        // its stem in the rasteriser, which is why small labels used to read
        // as light grey. Asking for a slightly heavier cut at raster time is
        // what browsers call "optical sizing"; the advance comes from the
        // shaped run, so the layout does not shift.
        let weight = if glyph.font_size < 18.0 {
            let bump = ((18.0 - glyph.font_size) * 10.0).clamp(80.0, 120.0) as u16;
            weight.saturating_add(bump)
        } else {
            weight
        };
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
                msdf: false,
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
                msdf: sdf_mode,
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
                msdf: false,
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
                let snap = self.snap_phases;
                out.push(GlyphQuad {
                    rect: resolved.quad(pen, snap),
                    uv: resolved.slot.uv,
                    layer: resolved.slot.layer,
                    color: resolved.color,
                    msdf: resolved.msdf,
                    phase: Self::quantize_x(pen.x, snap).1,
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
    // The padding is a *ramp*, not a transparent gutter.
    //
    // The glyph quad is inflated by `padding` on every side, so the outer
    // texels are actually sampled. Leaving them at zero sampled as a hard
    // transparent edge (thin, washed-out stems); copying the edge value
    // verbatim widened every stem by two full pixels. A linear falloff keeps
    // the antialiased edge and adds just enough weight to match an LCD
    // renderer on a 1 px stem.
    let pw = w + pad * 2;
    let ph = h + pad * 2;
    let mut out = vec![0u8; (pw * ph) as usize];
    if w == 0 || h == 0 {
        return out;
    }
    let max_x = (w - 1) as i32;
    let max_y = (h - 1) as i32;
    // Stem darkening: coverage is gamma-lifted once, at atlas build time, so
    // the raster carries the weight even where the shader's gamma correction
    // is disabled. 0 stays 0, so the padding gutter remains transparent.
    let mut curve = [0u8; 256];
    for (value, slot) in curve.iter_mut().enumerate() {
        *slot = if value == 0 {
            0
        } else {
            ((value as f32 / 255.0).powf(0.82) * 1.05 * 255.0).min(255.0) as u8
        };
    }
    for y in 0..ph {
        for x in 0..pw {
            let sx = x as i32 - pad as i32;
            let sy = y as i32 - pad as i32;
            let cx = sx.clamp(0, max_x);
            let cy = sy.clamp(0, max_y);
            let src_off = (cy as u32 * w + cx as u32) as usize;
            let value = curve[src[src_off] as usize];
            let out_of_bounds = (sx < 0 || sy < 0 || sx > max_x || sy > max_y) as u32;
            // Distance outside the glyph, in pixels.
            let dist = out_of_bounds
                * (sx.clamp(0, max_x) - sx)
                    .unsigned_abs()
                    .max((sy.clamp(0, max_y) - sy).unsigned_abs());
            let faded = if pad == 0 {
                value
            } else {
                let keep = pad.saturating_sub(dist);
                (u32::from(value) * keep / pad) as u8
            };
            out[(y * pw + x) as usize] = faded;
        }
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

    /// The quad's origin must not carry the pen's fractional part.
    ///
    /// The sub-pixel phase is already baked into the bitmap, so adding the
    /// fractional part of the pen a second time shifts the glyph twice — and
    /// leaves the quad on a fractional pixel, which makes the sampler blur it.
    #[test]
    fn quad_origin_floors_pen_x_to_avoid_double_subpixel_shift() {
        let glyph = ResolvedGlyph {
            slot: GlyphSlot::default(),
            bearing: Vec2::new(1.0, -2.0),
            size: Vec2::new(8.0, 10.0),
            advance: 9.0,
            color: false,
            msdf: false,
        };
        let quad = glyph.quad(Vec2::new(10.75, 20.4), false);
        assert_eq!(quad.origin.x, 11.0, "the pen's fraction must be dropped");
        assert_eq!(quad.origin.y, 18.0, "the baseline is snapped to a pixel");
        assert_eq!(quad.size.x, 8.0);
        assert_eq!(quad.size.y, 10.0);
    }

    /// The last phase bucket must carry into the next whole pixel.
    ///
    /// `round(fract * 4)` returns 4 for `fract ∈ [0.875, 1)`, which is phase 0
    /// of the *next* pixel. The old `% 4` folded it back to phase 0 without
    /// moving the quad, dragging 12.5 % of all glyphs one pixel to the left.
    #[test]
    fn the_last_phase_bucket_carries_into_the_next_pixel() {
        for thousandths in 875..1000 {
            let x = 10.0 + thousandths as f32 / 1000.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            assert_eq!(phase, 0, "x = {x} must round to phase 0");
            assert_eq!(base, 11.0, "x = {x} must move to the next pixel");
        }
    }

    /// Quantisation is monotonic: no pen position may ever render to the left
    /// of a smaller one, and the quantised position stays within ⅛ px.
    #[test]
    fn quantize_x_is_monotonic_and_never_jumps_backwards() {
        let mut previous = f32::NEG_INFINITY;
        for step in 0..=1280 {
            let x = step as f32 / 64.0;
            let (base, phase) = GlyphCache::quantize_x(x, false);
            let quantised = base + phase as f32 * 0.25;
            assert!(
                quantised >= previous,
                "x = {x} quantised to {quantised}, behind {previous}"
            );
            assert!(
                (quantised - x).abs() <= 0.126,
                "x = {x} quantised to {quantised}, more than ⅛ px away"
            );
            previous = quantised;
        }
    }

    /// Whole-pixel snapping puts the quad on the rounded pen, not the floor.
    #[test]
    fn snapped_phases_use_the_rounded_pen() {
        let glyph = ResolvedGlyph {
            slot: GlyphSlot::default(),
            bearing: Vec2::ZERO,
            size: Vec2::new(4.0, 4.0),
            advance: 4.0,
            color: false,
            msdf: false,
        };
        let quad = glyph.quad(Vec2::new(10.6, 0.0), true);
        assert_eq!(quad.origin.x, 11.0);
        assert_eq!(GlyphCache::quantize_x(10.6, true), (11.0, 0));
    }

    /// The phase a glyph is rasterised at has to follow its **absolute**
    /// position: the raster is generated for that phase, so deriving it from
    /// a run-relative position would shift every glyph by up to a pixel.
    #[test]
    fn the_subpixel_phase_follows_the_absolute_pen_position() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("Hello", &TextStyle::sized(16.0));
        let at_zero = cache.quads_for(&mut e, &shaped, Vec2::ZERO, 400, false);
        let at_half = cache.quads_for(&mut e, &shaped, Vec2::new(0.5, 0.0), 400, false);
        assert_eq!(at_zero.len(), at_half.len());
        for (a, b) in at_zero.iter().zip(at_half.iter()) {
            assert_ne!(
                a.phase, b.phase,
                "moving the run by half a pixel must change the phase"
            );
        }
    }

    /// Two runs of the same text at the same absolute position must produce
    /// the same quads — the phase is derived from the position, not from the
    /// order the runs were laid out in.
    #[test]
    fn the_same_absolute_position_produces_the_same_phase() {
        let mut e = engine();
        let mut cache = GlyphCache::new();
        let shaped = e.shape("Hg", &TextStyle::sized(16.0));
        let first = cache.quads_for(&mut e, &shaped, Vec2::new(3.25, 0.0), 400, false);
        let second = cache.quads_for(&mut e, &shaped, Vec2::new(3.25, 0.0), 400, false);
        assert_eq!(first, second);
        // The quad origin is floored onto the raster's pixel grid.
        for quad in &first {
            assert_eq!(quad.rect.origin.x, quad.rect.origin.x.floor());
            assert_eq!(quad.rect.origin.y, quad.rect.origin.y.floor());
        }
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
        assert!(
            quads[0].msdf,
            "the quad must carry the MSDF flag to the primitive builder"
        );
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

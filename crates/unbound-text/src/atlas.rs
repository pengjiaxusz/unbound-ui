//! Dual-format paged glyph atlas with frame-generation LRU eviction
//! (`docs` §3.5).
//!
//! Two independent `Texture2DArray`s:
//!
//! * `Atlas_R8` — one byte per pixel, four horizontal subpixel phases per glyph.
//!   Standard UI text only ever needs coverage, so spending four bytes per texel
//!   on it would quadruple the atlas for no visual gain.
//! * `Atlas_RGBA8` — colour emoji and multi-channel distance fields, where four
//!   channels are load-bearing.
//!
//! Allocation is shelf-based inside a page, with a free-rect list so that an
//! evicted glyph leaves a reusable hole instead of a permanent gap. Eviction is
//! by *frame generation*, not by wall clock: a glyph used last frame is never
//! evicted before one that has not been used for a thousand frames, regardless
//! of how long those frames took.

use rustc_hash::FxHashMap;

/// Identifies one rasterised glyph variant.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    /// Glyph id inside its face.
    pub glyph_id: u16,
    /// Horizontal subpixel phase (0..3), or 0 for non-subpixel atlases.
    pub phase: u8,
    /// 1 for a distance field, 0 for a plain coverage bitmap.
    pub sdf: u8,
    /// Font size rounded to ¼ px, so a 0.1px animation step does not thrash.
    pub size_q4: u16,
    /// Which face the glyph came from.
    pub face: u32,
    /// Weight the glyph was rasterised with.
    ///
    /// The raster depends on the weight, so it has to be part of the key: two
    /// weights of the same glyph are two different images.
    pub weight: u16,
    /// 1 when the glyph was rasterised with a synthetic italic slant.
    ///
    /// A slanted raster is a different image from the upright one even though
    /// the glyph id and the face are the same.
    pub italic: u8,
}

impl GlyphKey {
    #[must_use]
    pub fn new(glyph_id: u16, size: f32, phase: u8, sdf: bool, face: u64) -> Self {
        Self::styled(glyph_id, size, phase, sdf, face, 400, false)
    }

    /// The full key, including the weight and the italic slant.
    #[must_use]
    pub fn styled(
        glyph_id: u16,
        size: f32,
        phase: u8,
        sdf: bool,
        face: u64,
        weight: u16,
        italic: bool,
    ) -> Self {
        Self {
            glyph_id,
            phase: phase & 3,
            sdf: u8::from(sdf),
            size_q4: (size.max(0.0) * 4.0).round().clamp(0.0, 65535.0) as u16,
            face: (face & 0xffff_ffff) as u32,
            weight,
            italic: u8::from(italic),
        }
    }
}

/// Where a glyph lives inside the atlas.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GlyphSlot {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Normalised `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Last frame the slot was requested.
    pub last_used: u64,
}

impl GlyphSlot {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }
}

/// Atlas dimensions and capacity.
#[derive(Copy, Clone, Debug)]
pub struct AtlasConfig {
    pub page_size: u32,
    pub max_layers: u32,
    /// Transparent padding between glyphs, to stop bilinear taps bleeding
    /// neighbouring glyphs into each other.
    pub padding: u32,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            page_size: 2048,
            max_layers: 8,
            padding: 2,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    /// Next free x on this shelf.
    cursor: u32,
}

#[derive(Copy, Clone, Debug)]
struct FreeRect {
    layer: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Debug, Default)]
struct Page {
    shelves: Vec<Shelf>,
    /// Next free y (below the last shelf).
    next_y: u32,
}

/// One format's atlas: `max_layers` pages of `page_size²`.
#[derive(Debug)]
pub struct Atlas {
    config: AtlasConfig,
    pages: Vec<Page>,
    slots: FxHashMap<GlyphKey, GlyphSlot>,
    free: Vec<FreeRect>,
    clock: u64,
    evictions: u64,
    /// Bytes of texel storage currently allocated (pages × page_size² × bpp).
    pub bytes_per_texel: u32,
}

impl Atlas {
    #[must_use]
    pub fn new(config: AtlasConfig, bytes_per_texel: u32) -> Self {
        Self {
            config,
            pages: Vec::new(),
            slots: FxHashMap::default(),
            free: Vec::new(),
            clock: 0,
            evictions: 0,
            bytes_per_texel,
        }
    }

    /// Advance the frame clock and mark a slot as used.
    pub fn begin_frame(&mut self, frame: u64) {
        self.clock = frame;
    }

    #[must_use]
    pub fn page_size(&self) -> u32 {
        self.config.page_size
    }

    #[must_use]
    pub fn layer_count(&self) -> u32 {
        self.pages.len() as u32
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// Bytes of texel memory reserved so far.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        u64::from(self.pages.len() as u32)
            * u64::from(self.config.page_size)
            * u64::from(self.config.page_size)
            * u64::from(self.bytes_per_texel)
    }

    /// Look up an existing slot and refresh its generation.
    pub fn touch(&mut self, key: &GlyphKey) -> Option<GlyphSlot> {
        let clock = self.clock;
        let slot = self.slots.get_mut(key)?;
        slot.last_used = clock;
        Some(*slot)
    }

    /// Reserve `w × h` for `key`, evicting the coldest slot when full.
    ///
    /// Returns `None` only when the request cannot fit in a single page.
    ///
    /// The keys of evicted glyphs are appended to `evicted` so the caller can
    /// drop its own cache entries.
    pub fn allocate(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        if w == 0 || h == 0 || w > self.config.page_size || h > self.config.page_size {
            return None;
        }
        if let Some(existing) = self.touch(&key) {
            return Some(existing);
        }
        let pad = self.config.padding;
        let need_w = w + pad;
        let need_h = h + pad;

        // First try without evicting anything.
        if let Some(slot) = self.try_place(need_w, need_h, w, h) {
            self.slots.insert(key, slot);
            return Some(slot);
        }

        // No room anywhere: evict the coldest glyph and retry once. A second
        // eviction round is not attempted because it cannot help when the
        // request itself is too large.
        //
        // The eviction is only committed if the retry succeeds. A failure
        // would otherwise destroy a live, unrelated glyph for nothing.
        let (evicted_key, slot) = self.evict_coldest()?;
        if let Some(slot) = self.try_place(need_w, need_h, w, h) {
            self.slots.insert(key, slot);
            evicted.push(evicted_key);
            return Some(slot);
        }
        // The retry failed: put the evicted glyph back where it was, so the
        // atlas is left exactly as it was found.
        self.slots.insert(evicted_key, slot);
        self.free.push(FreeRect {
            layer: slot.layer,
            x: slot.x,
            y: slot.y,
            w: slot.w + pad,
            h: slot.h + pad,
        });
        self.evictions = self.evictions.saturating_sub(1);
        None
    }

    fn try_place(&mut self, need_w: u32, need_h: u32, w: u32, h: u32) -> Option<GlyphSlot> {
        let page_size = self.config.page_size;

        // 1. Best-fit from recycled rectangles.
        let mut best: Option<usize> = None;
        for (i, r) in self.free.iter().enumerate() {
            if r.w >= need_w && r.h >= need_h {
                let better = best
                    .map(|b| {
                        let cur = self.free[b].w * self.free[b].h;
                        r.w * r.h < cur
                    })
                    .unwrap_or(true);
                if better {
                    best = Some(i);
                }
            }
        }
        if let Some(i) = best {
            let r = self.free.swap_remove(i);
            if r.w > need_w {
                self.free.push(FreeRect {
                    layer: r.layer,
                    x: r.x + need_w,
                    y: r.y,
                    w: r.w - need_w,
                    h: need_h,
                });
            }
            if r.h > need_h {
                self.free.push(FreeRect {
                    layer: r.layer,
                    x: r.x,
                    y: r.y + need_h,
                    w: r.w,
                    h: r.h - need_h,
                });
            }
            return Some(self.make_slot(r.layer, r.x, r.y, w, h));
        }

        // 2. Existing shelves (first fit). Find the target by index first so the
        //    mutable borrow of `self.pages` happens exactly once.
        let mut hit: Option<(usize, usize, u32, u32)> = None;
        'outer: for (layer, page) in self.pages.iter().enumerate() {
            for (si, shelf) in page.shelves.iter().enumerate() {
                if shelf.height >= need_h && shelf.cursor + need_w <= page_size {
                    hit = Some((layer, si, shelf.cursor, shelf.y));
                    break 'outer;
                }
            }
        }
        if let Some((layer, si, x, y)) = hit {
            self.pages[layer].shelves[si].cursor = x + need_w;
            return Some(self.make_slot(layer as u32, x, y, w, h));
        }

        // 3. A new shelf at the bottom of an existing page.
        let mut new_shelf: Option<(usize, u32)> = None;
        for (layer, page) in self.pages.iter().enumerate() {
            if page.next_y + need_h <= page_size {
                new_shelf = Some((layer, page.next_y));
                break;
            }
        }
        if let Some((layer, y)) = new_shelf {
            let page = &mut self.pages[layer];
            page.shelves.push(Shelf {
                y,
                height: need_h,
                cursor: need_w,
            });
            page.next_y = y + need_h;
            return Some(self.make_slot(layer as u32, 0, y, w, h));
        }

        // 4. A brand new layer.
        if (self.pages.len() as u32) < self.config.max_layers {
            let mut page = Page::default();
            page.shelves.push(Shelf {
                y: 0,
                height: need_h,
                cursor: need_w,
            });
            page.next_y = need_h;
            self.pages.push(page);
            let layer = self.pages.len() as u32 - 1;
            return Some(self.make_slot(layer, 0, 0, w, h));
        }

        None
    }

    fn make_slot(&mut self, layer: u32, x: u32, y: u32, w: u32, h: u32) -> GlyphSlot {
        let sz = self.config.page_size as f32;
        GlyphSlot {
            layer,
            x,
            y,
            w,
            h,
            uv: [
                x as f32 / sz,
                y as f32 / sz,
                (x + w) as f32 / sz,
                (y + h) as f32 / sz,
            ],
            last_used: self.clock,
        }
    }

    /// Evict the least-recently-used slot, returning its rectangle to the pool.
    ///
    /// The evicted key and slot are returned so that the caller can report the
    /// eviction *and* put the glyph back if the allocation it was made for
    /// still fails: the rectangle goes back into the free pool and is handed
    /// to the next glyph, so a stale cache entry would point at somebody
    /// else's texels.
    fn evict_coldest(&mut self) -> Option<(GlyphKey, GlyphSlot)> {
        let (&key, &slot) = self
            .slots
            .iter()
            .min_by_key(|(_, s)| (s.last_used, s.layer, s.x, s.y))?;
        self.slots.remove(&key);
        let pad = self.config.padding;
        self.free.push(FreeRect {
            layer: slot.layer,
            x: slot.x,
            y: slot.y,
            w: slot.w + pad,
            h: slot.h + pad,
        });
        self.evictions += 1;
        Some((key, slot))
    }

    /// Drop a specific glyph (e.g. the face was unloaded).
    pub fn remove(&mut self, key: &GlyphKey) -> Option<GlyphSlot> {
        let slot = self.slots.remove(key)?;
        let pad = self.config.padding;
        self.free.push(FreeRect {
            layer: slot.layer,
            x: slot.x,
            y: slot.y,
            w: slot.w + pad,
            h: slot.h + pad,
        });
        Some(slot)
    }

    /// Occupancy in `0..=1` of the allocated layers.
    #[must_use]
    pub fn occupancy(&self) -> f32 {
        if self.pages.is_empty() {
            return 0.0;
        }
        let used: u64 = self.slots.values().map(|s| u64::from(s.w * s.h)).sum();
        let total = self.pages.len() as u64
            * u64::from(self.config.page_size)
            * u64::from(self.config.page_size);
        used as f32 / total as f32
    }
}

/// The pair of atlases the renderer binds (`Atlas_R8` + `Atlas_RGBA8`).
#[derive(Debug)]
pub struct GlyphAtlas {
    /// Four subpixel phases per glyph, one byte per texel.
    pub r8: Atlas,
    /// Colour emoji and distance fields, four bytes per texel.
    pub rgba: Atlas,
    /// Generation counter the renderer increments each frame.
    pub frame: u64,
}

/// Number of horizontal subpixel phases the R8 atlas stores.
pub const SUBPIXEL_PHASES: u8 = 4;

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(AtlasConfig::default())
    }
}

impl GlyphAtlas {
    #[must_use]
    pub fn new(config: AtlasConfig) -> Self {
        Self {
            r8: Atlas::new(config, 1),
            rgba: Atlas::new(config, 4),
            frame: 0,
        }
    }

    /// Start a new frame; also refreshes the eviction clock.
    pub fn begin_frame(&mut self, frame: u64) {
        self.frame = frame;
        self.r8.begin_frame(frame);
        self.rgba.begin_frame(frame);
    }

    /// Allocate coverage storage for a glyph at a subpixel phase.
    ///
    /// The keys of evicted glyphs are appended to `evicted` so the caller can
    /// drop its own cache entries.
    pub fn allocate_coverage(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        self.r8.allocate(key, w, h, evicted)
    }

    /// Allocate RGBA storage (emoji, or a distance field).
    ///
    /// The key carries the phase, the weight and the italic slant, because a
    /// raster produced for a different phase (or weight, or slant) is a
    /// different image. Dropping any of them would make two variants share
    /// one atlas rectangle, and the second upload would overwrite the first
    /// one's texels.
    ///
    /// The keys of evicted glyphs are appended to `evicted` so the caller can
    /// drop its own cache entries.
    pub fn allocate_color(
        &mut self,
        key: GlyphKey,
        w: u32,
        h: u32,
        evicted: &mut Vec<GlyphKey>,
    ) -> Option<GlyphSlot> {
        self.rgba.allocate(key, w, h, evicted)
    }

    /// Total texel memory of both atlases.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.r8.memory_bytes() + self.rgba.memory_bytes()
    }

    /// Number of layers the renderer must bind.
    #[must_use]
    pub fn layer_count(&self) -> u32 {
        self.r8.layer_count().max(self.rgba.layer_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> AtlasConfig {
        AtlasConfig {
            page_size: 256,
            max_layers: 2,
            padding: 1,
        }
    }

    #[test]
    fn allocation_returns_a_slot_inside_the_page() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(42, 16.0, 0, false, 1);
        let slot = a.allocate(key, 20, 24, &mut evicted).expect("slot");
        assert_eq!(slot.layer, 0);
        assert!(slot.x + slot.w <= 256 && slot.y + slot.h <= 256);
        assert_eq!(slot.w, 20);
        assert_eq!(slot.uv[0], 0.0);
        assert!((slot.uv[2] - 20.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn same_key_returns_the_same_slot_and_refreshes_it() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(7, 16.0, 2, false, 1);
        a.begin_frame(1);
        let first = a.allocate(key, 10, 10, &mut evicted).unwrap();
        a.begin_frame(50);
        let again = a.allocate(key, 10, 10, &mut evicted).unwrap();
        assert_eq!(
            (first.layer, first.x, first.y),
            (again.layer, again.x, again.y)
        );
        assert_eq!(again.last_used, 50);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn slots_never_overlap() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let mut rects = Vec::new();
        for i in 0..60u16 {
            let key = GlyphKey::new(i, 12.0, (i % 4) as u8, false, 1);
            if let Some(s) = a.allocate(key, 18, 22, &mut evicted) {
                for (l, x, y, w, h) in &rects {
                    let overlap = *l == s.layer
                        && s.x < x + w
                        && x < &(s.x + s.w)
                        && s.y < y + h
                        && y < &(s.y + s.h);
                    assert!(!overlap, "glyph {i} overlaps a previous slot");
                }
                rects.push((s.layer, s.x, s.y, s.w, s.h));
            }
        }
        assert!(rects.len() > 20, "only {} glyphs fit", rects.len());
    }

    #[test]
    fn eviction_recycles_the_coldest_glyph() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        // Fill the single 64x64 page with 16x16 glyphs.
        a.begin_frame(1);
        let mut keys = Vec::new();
        for i in 0..16u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(
                a.allocate(key, 16, 16, &mut evicted).is_some(),
                "glyph {i} should fit"
            );
            keys.push(key);
        }
        // Touch everything except glyph 0 so it becomes the coldest.
        a.begin_frame(2);
        for key in keys.iter().skip(1) {
            assert!(a.allocate(*key, 16, 16, &mut evicted).is_some());
        }
        a.begin_frame(3);
        let fresh = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(
            a.allocate(fresh, 16, 16, &mut evicted).is_some(),
            "must evict to fit"
        );
        assert_eq!(a.evictions(), 1);
        assert!(
            a.touch(&keys[0]).is_none(),
            "glyph 0 must have been evicted"
        );
        assert!(a.touch(&keys[1]).is_some(), "glyph 1 was hot, keep it");
    }

    #[test]
    fn oversized_glyph_is_rejected_not_corrupted() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(1, 16.0, 0, false, 1);
        assert!(a.allocate(key, 512, 10, &mut evicted).is_none());
        assert!(a.allocate(key, 0, 10, &mut evicted).is_none());
        assert_eq!(a.len(), 0);
    }

    #[test]
    fn multiple_layers_are_used_before_evicting() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        for i in 0..40u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 40, 40, &mut evicted).is_some(), "glyph {i}");
        }
        assert!(a.layer_count() >= 2, "should spill into a second layer");
        assert_eq!(a.evictions(), 0);
    }

    #[test]
    fn phase_and_face_are_part_of_the_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let base = GlyphKey::new(5, 16.0, 0, false, 1);
        let phase = GlyphKey::new(5, 16.0, 2, false, 1);
        let face = GlyphKey::new(5, 16.0, 0, false, 2);
        let s0 = a.allocate(base, 8, 8, &mut evicted).unwrap();
        let s1 = a.allocate(phase, 8, 8, &mut evicted).unwrap();
        let s2 = a.allocate(face, 8, 8, &mut evicted).unwrap();
        assert_ne!((s0.x, s0.y), (s1.x, s1.y));
        assert_ne!((s0.x, s0.y), (s2.x, s2.y));
    }

    /// The weight and the italic slant change the raster, so they have to be
    /// part of the key.
    ///
    /// `FAKE_ITALIC` makes swash produce a *skewed* raster from the same face
    /// id, so an italic and an upright run of the same glyph share every
    /// other part of the key. Without the slant in the key they would share
    /// one atlas rectangle and the second one would overwrite the first.
    #[test]
    fn weight_and_italic_are_part_of_the_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let upright = GlyphKey::styled(5, 16.0, 0, false, 1, 400, false);
        let italic = GlyphKey::styled(5, 16.0, 0, false, 1, 400, true);
        let bold = GlyphKey::styled(5, 16.0, 0, false, 1, 700, false);
        let s0 = a.allocate(upright, 8, 8, &mut evicted).unwrap();
        let s1 = a.allocate(italic, 8, 8, &mut evicted).unwrap();
        let s2 = a.allocate(bold, 8, 8, &mut evicted).unwrap();
        assert_ne!((s0.x, s0.y), (s1.x, s1.y), "italic must not share a slot");
        assert_ne!((s0.x, s0.y), (s2.x, s2.y), "bold must not share a slot");
    }

    /// The colour atlas must key on the phase too.
    ///
    /// A distance field (or a colour emoji) rasterised at a different
    /// subpixel phase is a different image. Dropping the phase made two
    /// phases share one rectangle, and the second upload overwrote the
    /// first one's texels.
    #[test]
    fn the_colour_atlas_keys_on_the_phase() {
        let mut evicted = Vec::new();
        let mut atlas = GlyphAtlas::new(small());
        atlas.begin_frame(1);
        let p0 = GlyphKey::styled(5, 16.0, 0, true, 1, 400, false);
        let p1 = GlyphKey::styled(5, 16.0, 2, true, 1, 400, false);
        let s0 = atlas.allocate_color(p0, 12, 12, &mut evicted).unwrap();
        let s1 = atlas.allocate_color(p1, 12, 12, &mut evicted).unwrap();
        assert_ne!(
            (s0.x, s0.y),
            (s1.x, s1.y),
            "two phases must not share an atlas rectangle"
        );
    }

    /// An evicted glyph must be reported so the caller can drop its cache
    /// entry: the rectangle goes back into the free pool and is handed to the
    /// next glyph.
    #[test]
    fn eviction_reports_the_evicted_key() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        a.begin_frame(1);
        let mut keys = Vec::new();
        for i in 0..16u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 16, 16, &mut evicted).is_some());
            keys.push(key);
        }
        a.begin_frame(3);
        let fresh = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(
            a.allocate(fresh, 16, 16, &mut evicted).is_some(),
            "must evict"
        );
        assert_eq!(a.evictions(), 1);
        assert_eq!(evicted.len(), 1, "the evicted key must be reported");
        assert_eq!(evicted[0], keys[0], "the coldest glyph is evicted");
    }

    /// A failed allocation must not destroy a live glyph.
    ///
    /// The eviction that made room was destructive: if the retry still
    /// failed, an unrelated, perfectly good glyph was gone for nothing.
    #[test]
    fn a_failed_allocation_leaves_the_atlas_untouched() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(
            AtlasConfig {
                page_size: 64,
                max_layers: 1,
                padding: 0,
            },
            1,
        );
        a.begin_frame(1);
        // Fill the page with 16x16 glyphs.
        for i in 0..16u16 {
            let key = GlyphKey::new(i, 16.0, 0, false, 1);
            assert!(a.allocate(key, 16, 16, &mut evicted).is_some());
        }
        let before = a.len();
        assert_eq!(before, 16);
        // A glyph that does not fit even in a whole page: the eviction is
        // attempted and the retry fails, so the atlas must be left exactly
        // as it was found.
        let too_big = GlyphKey::new(999, 16.0, 0, false, 1);
        assert!(a.allocate(too_big, 40, 40, &mut evicted).is_none());
        assert_eq!(
            a.len(),
            before,
            "the failed allocation must not destroy a live glyph"
        );
        assert!(evicted.is_empty(), "nothing was really evicted");
        assert_eq!(a.evictions(), 0);
    }

    #[test]
    fn sub_quarter_pixel_size_changes_share_a_slot() {
        let _a = Atlas::new(small(), 1);
        let k1 = GlyphKey::new(5, 16.01, 0, false, 1);
        let k2 = GlyphKey::new(5, 16.10, 0, false, 1);
        assert_eq!(k1, k2, "sizes must bucket to ¼ px");
        let k3 = GlyphKey::new(5, 16.4, 0, false, 1);
        assert_ne!(k1, k3);
    }

    #[test]
    fn dual_atlas_tracks_both_formats() {
        let mut evicted = Vec::new();
        let mut atlas = GlyphAtlas::new(small());
        atlas.begin_frame(1);
        assert!(atlas
            .allocate_coverage(GlyphKey::new(1, 16.0, 0, false, 1), 12, 12, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_coverage(GlyphKey::new(1, 16.0, 1, false, 1), 12, 12, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_color(GlyphKey::new(2, 16.0, 0, false, 1), 16, 16, &mut evicted)
            .is_some());
        assert!(atlas
            .allocate_color(GlyphKey::new(3, 16.0, 0, true, 1), 32, 32, &mut evicted)
            .is_some());
        assert_eq!(atlas.r8.len(), 2);
        assert_eq!(atlas.rgba.len(), 2);
        assert!(atlas.memory_bytes() > 0);
        assert!(atlas.layer_count() >= 1);
    }

    #[test]
    fn removing_a_glyph_frees_its_rectangle() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        let key = GlyphKey::new(3, 16.0, 0, false, 1);
        let slot = a.allocate(key, 32, 32, &mut evicted).unwrap();
        assert!(a.remove(&key).is_some());
        assert_eq!(a.len(), 0);
        let next = a
            .allocate(GlyphKey::new(4, 16.0, 0, false, 1), 32, 32, &mut evicted)
            .unwrap();
        assert_eq!((next.layer, next.x, next.y), (slot.layer, slot.x, slot.y));
    }

    #[test]
    fn occupancy_grows_with_allocations() {
        let mut evicted = Vec::new();
        let mut a = Atlas::new(small(), 1);
        for i in 0..8u16 {
            a.allocate(GlyphKey::new(i, 16.0, 0, false, 1), 40, 40, &mut evicted);
        }
        let occ = a.occupancy();
        assert!(occ > 0.05 && occ < 0.9, "occupancy {occ}");
    }
}

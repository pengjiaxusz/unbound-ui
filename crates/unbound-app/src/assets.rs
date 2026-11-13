//! The asset hub: `unbound-asset` wired to the atlas and the font system.
//!
//! The async pipeline decodes off the render thread; this module is the
//! main-thread half. Every frame [`AssetHub::poll`] drains whatever finished,
//! packs decoded images into a shared RGBA8 atlas page, injects fonts into the
//! text engine, and returns the dirty flags the rebuild needs. Images that are
//! still in flight are drawn as placeholders by
//! [`unbound_widgets::emit_image_with_assets`], so a decode never blocks the
//! frame and the layout never jumps when the texture lands.

use rustc_hash::FxHashMap;

use unbound_asset::{AssetId, AssetKind, AssetPipeline, Completed, PipelineConfig};
use unbound_gpu::atlas::{AtlasKind, AtlasUpload, ATLAS_LAYERS, PAGE_SIZE};
use unbound_math::{Rect, Vec2};
use unbound_text::TextEngine;
use unbound_widgets::{ImageAssetResolver, ResolvedImage};

use crate::DirtyFlags;

/// Where an asset is in its lifecycle.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AssetResidency {
    /// The decode worker still has it.
    Loading,
    /// Decoded, packed into `layer` at `uv`, with its natural size.
    Ready {
        /// The bindless atlas layer.
        layer: u32,
        /// The region of the layer, normalised to `0..1`.
        uv: Rect,
        /// The image's natural size, in pixels.
        size: Vec2,
    },
    /// The decode or the atlas allocation failed.
    Failed,
}

/// One placed rectangle in the shared image atlas.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Allocation {
    layer: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Allocation {
    fn uv(self) -> Rect {
        let page = PAGE_SIZE as f32;
        Rect::new(
            self.x as f32 / page,
            self.y as f32 / page,
            self.width as f32 / page,
            self.height as f32 / page,
        )
    }
}

/// A shelf packer over the RGBA8 atlas pages.
///
/// Images are packed left-to-right in shelves, top-to-bottom. When a page is
/// full the next array layer starts. The packer never frees: an image lives
/// for the session, which is what the bindless atlas expects.
#[derive(Debug, Default)]
struct ShelfAtlas {
    layer: u32,
    shelf_y: u32,
    shelf_height: u32,
    cursor_x: u32,
}

impl ShelfAtlas {
    /// One texel of padding keeps bilinear sampling from bleeding between
    /// neighbouring images.
    const PADDING: u32 = 1;

    fn allocate(&mut self, width: u32, height: u32) -> Option<Allocation> {
        if width == 0 || height == 0 || width > PAGE_SIZE || height > PAGE_SIZE {
            return None;
        }
        if self.cursor_x + width > PAGE_SIZE {
            self.shelf_y = self.shelf_y.saturating_add(self.shelf_height);
            self.shelf_height = 0;
            self.cursor_x = 0;
        }
        if self.shelf_y + height > PAGE_SIZE {
            self.layer += 1;
            self.shelf_y = 0;
            self.shelf_height = 0;
            self.cursor_x = 0;
        }
        if self.layer >= ATLAS_LAYERS {
            return None;
        }
        let allocation = Allocation {
            layer: self.layer,
            x: self.cursor_x,
            y: self.shelf_y,
            width,
            height,
        };
        self.cursor_x = self.cursor_x.saturating_add(width + Self::PADDING);
        self.shelf_height = self.shelf_height.max(height + Self::PADDING);
        Some(allocation)
    }
}

/// The main-thread asset front end.
///
/// Owns the decode pipeline and the residency table; [`AssetHub::poll`] is the
/// only method the frame loop calls.
pub struct AssetHub {
    pipeline: AssetPipeline,
    residency: FxHashMap<AssetId, AssetResidency>,
    atlas: ShelfAtlas,
    uploads: Vec<AtlasUpload>,
    next_id: u32,
}

impl std::fmt::Debug for AssetHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetHub")
            .field("residency_len", &self.residency.len())
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl Default for AssetHub {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetHub {
    /// Start the hub with the default decode pool.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(PipelineConfig::default())
    }

    /// Start the hub with an explicit pipeline configuration.
    #[must_use]
    pub fn with_config(config: PipelineConfig) -> Self {
        Self {
            pipeline: AssetPipeline::new(config),
            residency: FxHashMap::default(),
            atlas: ShelfAtlas::default(),
            uploads: Vec::new(),
            next_id: 1,
        }
    }

    /// Submit an encoded PNG for background decoding.
    pub fn request_png(&mut self, bytes: Vec<u8>) -> AssetId {
        self.request(AssetKind::Image, bytes)
    }

    /// Submit a font file for background parsing.
    pub fn request_font(&mut self, bytes: Vec<u8>) -> AssetId {
        self.request(AssetKind::Font, bytes)
    }

    fn request(&mut self, kind: AssetKind, bytes: Vec<u8>) -> AssetId {
        let id = AssetId(self.next_id);
        self.next_id += 1;
        let state = match self.pipeline.request(id, kind, bytes) {
            Ok(()) => AssetResidency::Loading,
            Err(_) => AssetResidency::Failed,
        };
        self.residency.insert(id, state);
        id
    }

    /// Drain finished decodes, upload the images and inject the fonts.
    ///
    /// Returns the flags a rebuild needs: `PAINT` for a new texture,
    /// `LAYOUT | TEXT` for a new font.
    pub fn poll(&mut self, text: &mut TextEngine) -> DirtyFlags {
        self.pipeline.begin_frame();
        let completed = match self.pipeline.poll() {
            Ok(completed) => completed,
            Err(_) => {
                // Every worker died; nothing in flight will ever finish.
                for state in self.residency.values_mut() {
                    if *state == AssetResidency::Loading {
                        *state = AssetResidency::Failed;
                    }
                }
                return DirtyFlags::PAINT;
            }
        };
        let mut dirty = DirtyFlags::NONE;
        for completed in completed {
            dirty = dirty.merge(self.accept(completed, text));
        }
        dirty
    }

    fn accept(&mut self, completed: Completed, text: &mut TextEngine) -> DirtyFlags {
        let Some(asset) = completed.asset else {
            self.residency.insert(completed.id, AssetResidency::Failed);
            return DirtyFlags::PAINT;
        };
        match asset.kind {
            AssetKind::Font => {
                if text.load_font_bytes(asset.bytes) == 0 {
                    self.residency.insert(completed.id, AssetResidency::Failed);
                    return DirtyFlags::NONE;
                }
                self.residency.insert(
                    completed.id,
                    AssetResidency::Ready {
                        layer: 0,
                        uv: Rect::ZERO,
                        size: Vec2::ZERO,
                    },
                );
                DirtyFlags::LAYOUT.merge(DirtyFlags::TEXT)
            }
            _ => {
                if asset.width == 0 || asset.height == 0 {
                    self.residency.insert(completed.id, AssetResidency::Failed);
                    return DirtyFlags::PAINT;
                }
                let Some(allocation) = self.atlas.allocate(asset.width, asset.height) else {
                    self.residency.insert(completed.id, AssetResidency::Failed);
                    return DirtyFlags::PAINT;
                };
                let expected = allocation.width as usize * allocation.height as usize * 4;
                if asset.bytes.len() < expected {
                    self.residency.insert(completed.id, AssetResidency::Failed);
                    return DirtyFlags::PAINT;
                }
                self.uploads.push(AtlasUpload {
                    kind: AtlasKind::Rgba8,
                    layer: allocation.layer,
                    x: allocation.x,
                    y: allocation.y,
                    width: allocation.width,
                    height: allocation.height,
                    data: asset.bytes[..expected].to_vec(),
                });
                self.residency.insert(
                    completed.id,
                    AssetResidency::Ready {
                        layer: allocation.layer,
                        uv: allocation.uv(),
                        size: Vec2::new(allocation.width as f32, allocation.height as f32),
                    },
                );
                DirtyFlags::PAINT
            }
        }
    }

    /// Take the atlas uploads accumulated since the previous call.
    pub fn take_uploads(&mut self) -> Vec<AtlasUpload> {
        core::mem::take(&mut self.uploads)
    }

    /// The state of one asset.
    #[must_use]
    pub fn residency(&self, id: AssetId) -> Option<AssetResidency> {
        self.residency.get(&id).copied()
    }

    /// Number of decodes still in flight.
    #[must_use]
    pub const fn inflight(&self) -> usize {
        self.pipeline.inflight()
    }

    /// The decode failures, with their reasons.
    #[must_use]
    pub fn failures(&self) -> &[(AssetId, String)] {
        self.pipeline.failures()
    }
}

impl ImageAssetResolver for AssetHub {
    fn resolve(&self, id: AssetId) -> Option<ResolvedImage> {
        match self.residency.get(&id)? {
            AssetResidency::Ready { layer, uv, size } => Some(ResolvedImage {
                layer: *layer,
                uv: *uv,
                size: *size,
            }),
            AssetResidency::Loading | AssetResidency::Failed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![128u8; (width * height * 4) as usize])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn the_shelf_packer_advances_shelves_and_pages_without_overlapping() {
        let mut atlas = ShelfAtlas::default();
        let first = atlas.allocate(2000, 10).expect("first shelf");
        assert_eq!((first.layer, first.x, first.y), (0, 0, 0));
        // 2000 + 1 + 2000 overflows the page: a new shelf starts.
        let second = atlas.allocate(2000, 10).expect("second shelf");
        assert_eq!((second.layer, second.x, second.y), (0, 0, 11));
        // A 2040-pixel-tall image cannot fit below the two shelves.
        let third = atlas.allocate(4, 2040).expect("next page");
        assert_eq!(third.layer, 1);
        assert_eq!((third.x, third.y), (0, 0));
    }

    #[test]
    fn an_oversized_image_is_refused() {
        let mut atlas = ShelfAtlas::default();
        assert!(atlas.allocate(PAGE_SIZE + 1, 1).is_none());
        assert!(atlas.allocate(1, 0).is_none());
    }

    #[test]
    fn a_png_becomes_ready_with_an_atlas_rect_and_an_upload() {
        let mut hub = AssetHub::new();
        let mut text = TextEngine::new();
        let id = hub.request_png(tiny_png(4, 3));
        assert_eq!(hub.residency(id), Some(AssetResidency::Loading));
        let mut dirty = DirtyFlags::NONE;
        for _ in 0..200 {
            dirty = dirty.merge(hub.poll(&mut text));
            if hub.residency(id) != Some(AssetResidency::Loading) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(dirty.contains(DirtyFlags::PAINT));
        let Some(AssetResidency::Ready { layer, uv, size }) = hub.residency(id) else {
            panic!("the asset never became ready: {:?}", hub.failures());
        };
        assert_eq!(layer, 0);
        assert_eq!(size, Vec2::new(4.0, 3.0));
        assert_eq!(uv, Rect::new(0.0, 0.0, 4.0 / 2048.0, 3.0 / 2048.0));
        let uploads = hub.take_uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].data.len(), 4 * 3 * 4);
        let resolved = hub.resolve(id).expect("a ready asset resolves");
        assert_eq!(resolved.layer, layer);
    }

    #[test]
    fn a_malformed_png_fails_without_retrying() {
        let mut hub = AssetHub::new();
        let mut text = TextEngine::new();
        let id = hub.request_png(b"not a png".to_vec());
        for _ in 0..200 {
            hub.poll(&mut text);
            if hub.residency(id) != Some(AssetResidency::Loading) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(hub.residency(id), Some(AssetResidency::Failed));
        assert_eq!(hub.inflight(), 0);
    }
}

//! 资产中心：将 `unbound-asset` 接入图集（atlas）与字体系统。
//!
//! 异步管线在渲染线程之外完成解码；本模块是主线程一侧。
//! 每帧 [`AssetHub::poll`] 会排空已完成的任务，将解码后的图像打包进共享
//! RGBA8 图集页，将字体注入文本引擎，并返回重建所需的脏标志。
//! 同时支持通过 [`AssetHub::upload_rgba8`] 直接上传已解码的 RGBA8 原始像素，
//! 并提供显式释放（[`AssetHub::release_asset`]）与按字节预算的 LRU 淘汰
//! （[`AssetHub::evict_lru_to_budget`]），满足高频缩略图与画布瓦片的显存驻留治理（ADR-0019）。

use std::sync::Arc;

use rustc_hash::FxHashMap;

use unbound_asset::{AssetId, AssetKind, AssetPipeline, Completed, PipelineConfig};
use unbound_gpu::atlas::{AtlasKind, AtlasUpload, PAGE_SIZE};
use unbound_gpu::pipeline::{ExternalTextureSlots, EXTERNAL_TEXTURE_BASE};
use unbound_math::{Rect, Vec2};
use unbound_text::TextEngine;
use unbound_widgets::{ImageAssetResolver, ResolvedImage};

use crate::DirtyFlags;

/// 共享图集接纳的单边尺寸上限：超过该阈值的动态 RGBA8 纹理走独立外部纹理槽，避免撑爆图集页。
pub const DYNAMIC_ATLAS_MAX_EDGE: u32 = 512;

/// 资产在其生命周期中所处的阶段。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AssetResidency {
    /// 解码工作线程仍在处理中。
    Loading,
    /// 已解码并打包到 `layer` 的 `uv` 区域，附带其原始尺寸。
    Ready {
        /// 无绑定（bindless）图集数组层索引或外部纹理槽位。
        layer: u32,
        /// 层内归一化至 `0..1` 的区域。
        uv: Rect,
        /// 图像的原始像素尺寸。
        size: Vec2,
    },
    /// 解码或图集分配失败。
    Failed,
}

/// 动态 RGBA8 像素上传错误（ADR-0019）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetUploadError {
    /// 图像宽或高为零。
    ZeroDimension,
    /// 行步长小于 `width * 4` 字节。
    InvalidStride {
        /// 允许的最小行步长（字节）。
        expected_min: u32,
        /// 调用方实际传入的行步长（字节）。
        actual: u32,
    },
    /// 像素缓冲字节长度不足以覆盖 `stride_bytes * height`。
    BufferTooSmall {
        /// 所需最小字节数。
        expected_min: usize,
        /// 实际缓冲长度。
        actual: usize,
    },
    /// 共享 RGBA8 图集层已全部占满且无法再分配。
    AtlasExhausted,
    /// 外部独立纹理槽已全部分配完毕。
    ExternalSlotExhausted,
}

/// 待提交至 GPU 外部纹理槽的 RGBA8 原始像素上传请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalTextureUpload {
    /// 关联的资产标识。
    pub asset_id: AssetId,
    /// 已分配的外部纹理 Bindless 槽位号。
    pub slot: u32,
    /// 纹理像素宽度。
    pub width: u32,
    /// 纹理像素高度。
    pub height: u32,
    /// 紧凑排列（`width * height * 4` 字节）的 RGBA8 像素数据。
    pub data: Vec<u8>,
}

/// 动态资产的存储归属与 LRU 追踪元数据。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DynamicBacking {
    Atlas { layer: u32 },
    External { slot: u32 },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct DynamicAssetRecord {
    backing: DynamicBacking,
    byte_size: usize,
    last_used_frame: u64,
}

/// 共享图像图集中的单个已放置矩形。
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

/// 基于 RGBA8 图集页的货架式打包器（shelf packer）。
#[derive(Debug, Default)]
struct ShelfAtlas {
    layer: u32,
    shelf_y: u32,
    shelf_height: u32,
    cursor_x: u32,
}

impl ShelfAtlas {
    /// 1 纹素的内边距可防止双线性采样在相邻图像间发生渗色。
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
        // `AtlasKind::Rgba8` 在 GPU 上分配了 `EXTERNAL_TEXTURE_BASE`（48）层，
        // 48..64 预留给外部注册的独立 Vulkan 纹理。
        if self.layer >= EXTERNAL_TEXTURE_BASE {
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

    fn reset(&mut self) {
        self.layer = 0;
        self.shelf_y = 0;
        self.shelf_height = 0;
        self.cursor_x = 0;
    }
}

/// 主线程侧的资产前端。
///
/// 持有解码管线、共享图集、外部纹理槽位与 LRU 驻留表；帧循环仅需调用 [`AssetHub::poll`]。
pub struct AssetHub {
    pipeline: AssetPipeline,
    residency: FxHashMap<AssetId, AssetResidency>,
    dynamic_records: FxHashMap<AssetId, DynamicAssetRecord>,
    atlas: ShelfAtlas,
    external_slots: ExternalTextureSlots,
    uploads: Vec<AtlasUpload>,
    external_uploads: Vec<ExternalTextureUpload>,
    external_releases: Vec<u32>,
    next_id: u32,
    touch_clock: u64,
}

impl std::fmt::Debug for AssetHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetHub")
            .field("residency_len", &self.residency.len())
            .field("dynamic_len", &self.dynamic_records.len())
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
    /// 使用默认解码线程池启动资产中心。
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(PipelineConfig::default())
    }

    /// 使用显式管线配置启动资产中心。
    #[must_use]
    pub fn with_config(config: PipelineConfig) -> Self {
        Self {
            pipeline: AssetPipeline::new(config),
            residency: FxHashMap::default(),
            dynamic_records: FxHashMap::default(),
            atlas: ShelfAtlas::default(),
            external_slots: ExternalTextureSlots::new(),
            uploads: Vec::new(),
            external_uploads: Vec::new(),
            external_releases: Vec::new(),
            next_id: 1,
            touch_clock: 0,
        }
    }

    /// 提交已编码的 PNG 进行后台解码。
    pub fn request_png(&mut self, bytes: Vec<u8>) -> AssetId {
        self.request(AssetKind::Image, bytes)
    }

    /// 提交字体文件进行后台解析。
    pub fn request_font(&mut self, bytes: Vec<u8>) -> AssetId {
        self.request(AssetKind::Font, bytes)
    }

    /// 直接上传已解码的 RGBA8 原始像素缓冲区（ADR-0019，免 PNG 重编码）。
    ///
    /// - 宽高均不超过 [`DYNAMIC_ATLAS_MAX_EDGE`]（512px）时优先打入共享 RGBA8 图集；
    /// - 超过阈值或图集已满时自动分配独立外部纹理槽（`ExternalTextureSlots`）。
    pub fn upload_rgba8(
        &mut self,
        width: u32,
        height: u32,
        stride_bytes: u32,
        pixels: Arc<[u8]>,
    ) -> Result<AssetId, AssetUploadError> {
        if width == 0 || height == 0 {
            return Err(AssetUploadError::ZeroDimension);
        }
        let row_bytes = width.saturating_mul(4);
        if stride_bytes < row_bytes {
            return Err(AssetUploadError::InvalidStride {
                expected_min: row_bytes,
                actual: stride_bytes,
            });
        }
        let required_len = (stride_bytes as usize).saturating_mul(height as usize);
        if pixels.len() < required_len {
            return Err(AssetUploadError::BufferTooSmall {
                expected_min: required_len,
                actual: pixels.len(),
            });
        }

        let compact = compact_rgba8_rows(&pixels, width, height, stride_bytes);
        let byte_size = compact.len();
        let id = self.alloc_id();
        self.touch_clock = self.touch_clock.saturating_add(1);
        let stamp = self.touch_clock;

        let prefer_atlas = width <= DYNAMIC_ATLAS_MAX_EDGE && height <= DYNAMIC_ATLAS_MAX_EDGE;
        if prefer_atlas {
            if let Some(allocation) = self.atlas.allocate(width, height) {
                self.uploads.push(AtlasUpload {
                    kind: AtlasKind::Rgba8,
                    layer: allocation.layer,
                    x: allocation.x,
                    y: allocation.y,
                    width: allocation.width,
                    height: allocation.height,
                    data: compact,
                });
                self.residency.insert(
                    id,
                    AssetResidency::Ready {
                        layer: allocation.layer,
                        uv: allocation.uv(),
                        size: Vec2::new(width as f32, height as f32),
                    },
                );
                self.dynamic_records.insert(
                    id,
                    DynamicAssetRecord {
                        backing: DynamicBacking::Atlas {
                            layer: allocation.layer,
                        },
                        byte_size,
                        last_used_frame: stamp,
                    },
                );
                return Ok(id);
            }
        }

        let Some(slot) = self.external_slots.allocate() else {
            return Err(if prefer_atlas {
                AssetUploadError::AtlasExhausted
            } else {
                AssetUploadError::ExternalSlotExhausted
            });
        };
        self.external_uploads.push(ExternalTextureUpload {
            asset_id: id,
            slot,
            width,
            height,
            data: compact,
        });
        self.residency.insert(
            id,
            AssetResidency::Ready {
                layer: slot,
                uv: Rect::new(0.0, 0.0, 1.0, 1.0),
                size: Vec2::new(width as f32, height as f32),
            },
        );
        self.dynamic_records.insert(
            id,
            DynamicAssetRecord {
                backing: DynamicBacking::External { slot },
                byte_size,
                last_used_frame: stamp,
            },
        );
        Ok(id)
    }

    /// 显式释放指定资产并回收其外部纹理槽；当全部动态图集资产清空时自动复用图集页。
    pub fn release_asset(&mut self, id: AssetId) -> bool {
        let removed_residency = self.residency.remove(&id).is_some();
        if let Some(record) = self.dynamic_records.remove(&id) {
            match record.backing {
                DynamicBacking::External { slot } => {
                    if self.external_slots.release(slot) {
                        self.external_releases.push(slot);
                    }
                }
                DynamicBacking::Atlas { .. } => {
                    let has_atlas_assets = self
                        .dynamic_records
                        .values()
                        .any(|r| matches!(r.backing, DynamicBacking::Atlas { .. }));
                    if !has_atlas_assets && self.residency.is_empty() {
                        self.atlas.reset();
                    }
                }
            }
            return true;
        }
        removed_residency
    }

    /// 刷新指定动态资产的最近访问帧号，供 [`Self::evict_lru_to_budget`] 按 LRU 淘汰。
    pub fn touch_asset(&mut self, id: AssetId, frame_index: u64) {
        if let Some(record) = self.dynamic_records.get_mut(&id) {
            record.last_used_frame = frame_index;
        }
    }

    /// 按字节预算淘汰最久未使用的动态 RGBA8 资产，返回被淘汰的 [`AssetId`] 列表。
    pub fn evict_lru_to_budget(&mut self, max_bytes: usize) -> Vec<AssetId> {
        let mut current_bytes = self.dynamic_bytes();
        if current_bytes <= max_bytes {
            return Vec::new();
        }
        let mut candidates: Vec<(AssetId, u64, usize)> = self
            .dynamic_records
            .iter()
            .map(|(&id, rec)| (id, rec.last_used_frame, rec.byte_size))
            .collect();
        candidates.sort_by_key(|&(id, frame, _)| (frame, id.0));

        let mut evicted = Vec::new();
        for (id, _, size) in candidates {
            if current_bytes <= max_bytes {
                break;
            }
            if self.release_asset(id) {
                current_bytes = current_bytes.saturating_sub(size);
                evicted.push(id);
            }
        }
        evicted
    }

    /// 当前由 [`Self::upload_rgba8`] 登记的动态像素驻留总字节数。
    #[must_use]
    pub fn dynamic_bytes(&self) -> usize {
        self.dynamic_records.values().map(|r| r.byte_size).sum()
    }

    /// 取出待上传至 GPU 外部纹理槽的 RGBA8 上传请求。
    pub fn take_external_uploads(&mut self) -> Vec<ExternalTextureUpload> {
        core::mem::take(&mut self.external_uploads)
    }

    /// 取出待从 GPU 解绑回收的外部纹理槽位号列表。
    pub fn take_external_releases(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.external_releases)
    }

    fn alloc_id(&mut self) -> AssetId {
        let id = AssetId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn request(&mut self, kind: AssetKind, bytes: Vec<u8>) -> AssetId {
        let id = self.alloc_id();
        let state = match self.pipeline.request(id, kind, bytes) {
            Ok(()) => AssetResidency::Loading,
            Err(_) => AssetResidency::Failed,
        };
        self.residency.insert(id, state);
        id
    }

    /// 排空已完成的解码任务，上传图像并注入字体。
    ///
    /// 返回重建所需的脏标志：新纹理对应 `PAINT`，新字体对应 `LAYOUT | TEXT`。
    pub fn poll(&mut self, text: &mut TextEngine) -> DirtyFlags {
        self.pipeline.begin_frame();
        let completed = match self.pipeline.poll() {
            Ok(completed) => completed,
            Err(_) => {
                // 所有工作线程均已退出；飞行中的任务永远不会完成。
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

    /// 取出自上次调用以来累积的图集上传任务。
    pub fn take_uploads(&mut self) -> Vec<AtlasUpload> {
        core::mem::take(&mut self.uploads)
    }

    /// 单个资产的状态。
    #[must_use]
    pub fn residency(&self, id: AssetId) -> Option<AssetResidency> {
        self.residency.get(&id).copied()
    }

    /// 仍在解码中的任务数量。
    #[must_use]
    pub const fn inflight(&self) -> usize {
        self.pipeline.inflight()
    }

    /// 解码失败记录及其原因。
    #[must_use]
    pub fn failures(&self) -> &[(AssetId, String)] {
        self.pipeline.failures()
    }
}

fn compact_rgba8_rows(pixels: &[u8], width: u32, height: u32, stride_bytes: u32) -> Vec<u8> {
    let row_bytes = (width as usize) * 4;
    let stride = stride_bytes as usize;
    let total = row_bytes * (height as usize);
    if stride == row_bytes {
        return pixels[..total].to_vec();
    }
    let mut out = Vec::with_capacity(total);
    for row in 0..(height as usize) {
        let start = row * stride;
        out.extend_from_slice(&pixels[start..start + row_bytes]);
    }
    out
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
        // 2000 + 1 + 2000 超出页宽：开启新货架。
        let second = atlas.allocate(2000, 10).expect("second shelf");
        assert_eq!((second.layer, second.x, second.y), (0, 0, 11));
        // 2040 像素高的图像无法放入两层货架下方。
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

    #[test]
    fn shelf_atlas_never_spills_into_external_texture_slots() {
        let mut atlas = ShelfAtlas::default();
        for expected_layer in 0..EXTERNAL_TEXTURE_BASE {
            let slot = atlas
                .allocate(PAGE_SIZE, PAGE_SIZE)
                .expect("layer fits in RGBA8 pool");
            assert_eq!(slot.layer, expected_layer);
        }
        assert!(
            atlas.allocate(16, 16).is_none(),
            "layer >= EXTERNAL_TEXTURE_BASE must be refused"
        );
    }

    #[test]
    fn upload_rgba8_validates_dimensions_stride_and_buffer_length() {
        let mut hub = AssetHub::new();
        let empty: Arc<[u8]> = Arc::from([]);
        assert_eq!(
            hub.upload_rgba8(0, 4, 16, empty.clone()),
            Err(AssetUploadError::ZeroDimension)
        );
        assert_eq!(
            hub.upload_rgba8(4, 4, 8, Arc::from(vec![0u8; 64])),
            Err(AssetUploadError::InvalidStride {
                expected_min: 16,
                actual: 8,
            })
        );
        assert_eq!(
            hub.upload_rgba8(4, 4, 16, Arc::from(vec![0u8; 32])),
            Err(AssetUploadError::BufferTooSmall {
                expected_min: 64,
                actual: 32,
            })
        );
    }

    #[test]
    fn upload_rgba8_packs_small_image_into_atlas_and_compacts_stride() {
        let mut hub = AssetHub::new();
        // 2x2 RGBA8，行步长 12 字节（含 4 字节对齐填充）
        let pixels: Arc<[u8]> = Arc::from(vec![
            1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 11, 12, 13, 14, 15, 16, 17, 18, 88, 88, 88, 88,
        ]);
        let id = hub
            .upload_rgba8(2, 2, 12, pixels)
            .expect("small rgba8 upload succeeds");
        assert!(matches!(
            hub.residency(id),
            Some(AssetResidency::Ready { layer: 0, .. })
        ));
        let uploads = hub.take_uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(
            uploads[0].data,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18]
        );
    }

    #[test]
    fn upload_rgba8_routes_large_image_to_external_slot_and_releases_cleanly() {
        let mut hub = AssetHub::new();
        let pixels: Arc<[u8]> = Arc::from(vec![255u8; 520 * 520 * 4]);
        let id = hub
            .upload_rgba8(520, 520, 520 * 4, pixels)
            .expect("large tile routes to external slot");
        let Some(AssetResidency::Ready { layer, uv, size }) = hub.residency(id) else {
            panic!("external asset must be ready");
        };
        assert!(layer >= EXTERNAL_TEXTURE_BASE);
        assert_eq!(uv, Rect::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(size, Vec2::new(520.0, 520.0));
        assert_eq!(hub.take_external_uploads().len(), 1);

        assert!(hub.release_asset(id));
        assert_eq!(hub.residency(id), None);
        assert_eq!(hub.take_external_releases(), vec![layer]);
    }

    #[test]
    fn evict_lru_to_budget_reclaims_oldest_dynamic_assets_first() {
        let mut hub = AssetHub::new();
        let buf: Arc<[u8]> = Arc::from(vec![7u8; 16 * 16 * 4]); // 1024 bytes each
        let a = hub.upload_rgba8(16, 16, 64, buf.clone()).unwrap();
        let b = hub.upload_rgba8(16, 16, 64, buf.clone()).unwrap();
        let c = hub.upload_rgba8(16, 16, 64, buf).unwrap();
        hub.touch_asset(a, 10);
        hub.touch_asset(b, 5);
        hub.touch_asset(c, 20);

        let evicted = hub.evict_lru_to_budget(2048);
        assert_eq!(evicted, vec![b]);
        assert!(hub.dynamic_bytes() <= 2048);
        assert_eq!(hub.residency(b), None);
        assert!(hub.residency(a).is_some());
        assert!(hub.residency(c).is_some());
    }
}

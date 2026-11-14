//! 资产中心：将 `unbound-asset` 接入图集（atlas）与字体系统。
//!
//! 异步管线在渲染线程之外完成解码；本模块是主线程一侧。
//! 每帧 [`AssetHub::poll`] 会排空已完成的任务，将解码后的图像打包进共享
//! RGBA8 图集页，将字体注入文本引擎，并返回重建所需的脏标志。
//! 仍在解码中的图像由 [`unbound_widgets::emit_image_with_assets`] 绘制为占位符，
//! 因此解码绝不会阻塞帧渲染，纹理到达时布局也不会跳动。
//!
//!

use rustc_hash::FxHashMap;

use unbound_asset::{AssetId, AssetKind, AssetPipeline, Completed, PipelineConfig};
use unbound_gpu::atlas::{AtlasKind, AtlasUpload, ATLAS_LAYERS, PAGE_SIZE};
use unbound_math::{Rect, Vec2};
use unbound_text::TextEngine;
use unbound_widgets::{ImageAssetResolver, ResolvedImage};

use crate::DirtyFlags;

/// 资产在其生命周期中所处的阶段。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AssetResidency {
    /// 解码工作线程仍在处理中。
    Loading,
    /// 已解码并打包到 `layer` 的 `uv` 区域，附带其原始尺寸。
    Ready {
        /// 无绑定（bindless）图集数组层索引。
        layer: u32,
        /// 层内归一化至 `0..1` 的区域。
        uv: Rect,
        /// 图像的原始像素尺寸。
        size: Vec2,
    },
    /// 解码或图集分配失败。
    Failed,
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
///
/// 图像按从左到右、从上到下的顺序排入货架；当前页满时开启下一数组层。
/// 打包器永不释放空间：图像在会话期间常驻，符合无绑定图集的预期。
///
#[derive(Debug, Default)]
struct ShelfAtlas {
    layer: u32,
    shelf_y: u32,
    shelf_height: u32,
    cursor_x: u32,
}

impl ShelfAtlas {
    /// 1 纹素的内边距可防止双线性采样在相邻图像间发生渗色。
    ///
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

/// 主线程侧的资产前端。
///
/// 持有解码管线与驻留表；帧循环仅需调用 [`AssetHub::poll`]。
///
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
            atlas: ShelfAtlas::default(),
            uploads: Vec::new(),
            next_id: 1,
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

    /// 排空已完成的解码任务，上传图像并注入字体。
    ///
    /// 返回重建所需的脏标志：新纹理对应 `PAINT`，新字体对应 `LAYOUT | TEXT`。
    ///
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
}

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

use unbound_asset::{
    AssetError, AssetId, AssetKind, AssetPipeline, AtlasPageId, Completed,
    CompressedTexturePayload, PipelineConfig, UploadPacingBudget, VideoFrameError,
    VideoFramePayload, VideoPixelFormat,
};
use unbound_gpu::atlas::{AtlasKind, AtlasUpload, PAGE_SIZE};
use unbound_gpu::device::CompressedTextureCapabilities;
use unbound_gpu::pipeline::EXTERNAL_TEXTURE_BASE;
use unbound_gpu::VideoUploadTier;
use unbound_math::{Rect, Vec2};
use unbound_text::TextEngine;
use unbound_widgets::{ImageAssetResolver, ResolvedImage};

use crate::bc7_assets::{Bc7AssetPool, Bc7GpuOp};
use crate::video::{
    VideoSurfaceHandle, VideoSurfaceManager, VideoSurfaceMetrics, VideoSurfaceOp, VideoSurfaceState,
};
use crate::DirtyFlags;

/// 外部托管 RGBA8 纹理的逻辑句柄。
///
/// 由 [`AssetHub::upload_external_rgba8`] 分配，在纹理上传完成前即可用于视图声明；
/// 窗口运行器在每帧渲染前自动向 GPU 渲染器执行上传/更新/释放并回写实际 Bindless 槽位。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalTextureHandle(pub u64);

/// 外部托管 RGBA8 纹理的生命周期状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExternalTextureState {
    /// 上传请求已入队，尚未完成 GPU 槽位分配。
    Pending,
    /// 已绑定到有效 GPU 外部 Bindless 纹理槽。
    Ready {
        /// 外部 Bindless 纹理槽位索引（`EXTERNAL_TEXTURE_BASE..`）。
        slot: u32,
        /// 纹理宽度（像素）。
        width: u32,
        /// 纹理高度（像素）。
        height: u32,
    },
    /// 上传、更新或设备恢复导致纹理不可用。
    Failed,
    /// 句柄已主动释放。
    Released,
}

/// 供窗口运行器在渲染前消费的外部纹理 GPU 操作队列项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalTextureOp {
    /// 分配新的外部纹理槽并上传 RGBA8 像素数据。
    Upload {
        /// 逻辑句柄。
        handle: ExternalTextureHandle,
        /// 纹理宽度（像素）。
        width: u32,
        /// 纹理高度（像素）。
        height: u32,
        /// 为 `true` 时按 sRGB 采样，否则按线性 UNORM 采样。
        srgb: bool,
        /// 紧凑 RGBA8 像素字节（长度等于 `width * height * 4`）。
        rgba: Vec<u8>,
    },
    /// 对已存在的外部纹理执行像素更新。
    Update {
        /// 逻辑句柄。
        handle: ExternalTextureHandle,
        /// 目标宽度（像素）。
        width: u32,
        /// 目标高度（像素）。
        height: u32,
        /// 紧凑 RGBA8 像素字节（长度等于 `width * height * 4`）。
        rgba: Vec<u8>,
    },
    /// 销毁外部纹理并归还其 GPU Bindless 槽位。
    Release {
        /// 逻辑句柄。
        handle: ExternalTextureHandle,
    },
}

#[derive(Copy, Clone, Debug)]
struct ExternalTextureRecord {
    state: ExternalTextureState,
    width: u32,
    height: u32,
}

fn validate_rgba8_len(width: u32, height: u32, len: usize) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    expected == Some(len)
}

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
    external_textures: FxHashMap<ExternalTextureHandle, ExternalTextureRecord>,
    external_ops: Vec<ExternalTextureOp>,
    next_external_id: u64,
    external_dirty: bool,
    video_surfaces: VideoSurfaceManager,
    bc7: Bc7AssetPool,
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
            external_textures: FxHashMap::default(),
            external_ops: Vec::new(),
            next_external_id: 1,
            external_dirty: false,
            video_surfaces: VideoSurfaceManager::new(),
            bc7: Bc7AssetPool::new(),
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
        if core::mem::take(&mut self.external_dirty) {
            dirty = dirty.merge(DirtyFlags::PAINT);
        }
        if self.video_surfaces.take_dirty() {
            dirty = dirty.merge(DirtyFlags::PAINT);
        }
        dirty = dirty.merge(self.bc7.poll_frame());
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
        self.residency
            .get(&id)
            .copied()
            .or_else(|| self.bc7.residency(id))
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

    /// 提交原始 RGBA8 像素上传至独立外部纹理槽，返回逻辑句柄。
    ///
    /// 若宽高为 0 或像素字节长度不等于 `width * height * 4`，则直接将该句柄标记为
    /// [`ExternalTextureState::Failed`] 且不入队 GPU 上传操作。
    pub fn upload_external_rgba8(
        &mut self,
        width: u32,
        height: u32,
        srgb: bool,
        rgba: Vec<u8>,
    ) -> ExternalTextureHandle {
        let handle = ExternalTextureHandle(self.next_external_id);
        self.next_external_id = self.next_external_id.wrapping_add(1).max(1);
        if !validate_rgba8_len(width, height, rgba.len()) {
            self.external_textures.insert(
                handle,
                ExternalTextureRecord {
                    state: ExternalTextureState::Failed,
                    width,
                    height,
                },
            );
            self.external_dirty = true;
            return handle;
        }
        self.external_textures.insert(
            handle,
            ExternalTextureRecord {
                state: ExternalTextureState::Pending,
                width,
                height,
            },
        );
        self.external_ops.push(ExternalTextureOp::Upload {
            handle,
            width,
            height,
            srgb,
            rgba,
        });
        self.external_dirty = true;
        handle
    }

    /// 请求原地更新已注册外部纹理的 RGBA8 像素。
    ///
    /// 对不存在、已释放、已失败的句柄或非法像素缓冲区返回 `false`。
    pub fn update_external_rgba8(
        &mut self,
        handle: ExternalTextureHandle,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> bool {
        if !validate_rgba8_len(width, height, rgba.len()) {
            return false;
        }
        let Some(record) = self.external_textures.get_mut(&handle) else {
            return false;
        };
        match record.state {
            ExternalTextureState::Pending | ExternalTextureState::Ready { .. } => {
                record.width = width;
                record.height = height;
                if let ExternalTextureState::Ready { slot, .. } = record.state {
                    record.state = ExternalTextureState::Ready {
                        slot,
                        width,
                        height,
                    };
                }
                self.external_ops.push(ExternalTextureOp::Update {
                    handle,
                    width,
                    height,
                    rgba,
                });
                self.external_dirty = true;
                true
            }
            ExternalTextureState::Failed | ExternalTextureState::Released => false,
        }
    }

    /// 释放外部纹理逻辑句柄并入队 GPU 槽位回收操作。
    ///
    /// 若同一帧内该句柄的 [`ExternalTextureOp::Upload`] 尚未出队，则自动合并抵消其全部
    /// 未消费操作，不向 GPU 提交冗余的上传与释放指令。
    pub fn release_external_texture(&mut self, handle: ExternalTextureHandle) -> bool {
        let Some(record) = self.external_textures.get_mut(&handle) else {
            return false;
        };
        if record.state == ExternalTextureState::Released {
            return false;
        }
        let had_pending_upload = self.external_ops.iter().any(|op| {
            matches!(
                op,
                ExternalTextureOp::Upload { handle: h, .. } if *h == handle
            )
        });
        // 丢弃该句柄尚未出队的上传或更新操作。
        self.external_ops.retain(|op| match op {
            ExternalTextureOp::Upload { handle: h, .. }
            | ExternalTextureOp::Update { handle: h, .. } => *h != handle,
            ExternalTextureOp::Release { .. } => true,
        });
        let was_failed = record.state == ExternalTextureState::Failed;
        record.state = ExternalTextureState::Released;
        if !had_pending_upload && !was_failed {
            self.external_ops
                .push(ExternalTextureOp::Release { handle });
        }
        self.external_dirty = true;
        true
    }

    /// 查询外部纹理当前绑定的 GPU 槽位与尺寸，返回 `Some((slot, width, height))`。
    #[must_use]
    pub fn external_texture_slot(&self, handle: ExternalTextureHandle) -> Option<(u32, u32, u32)> {
        match self.external_textures.get(&handle)?.state {
            ExternalTextureState::Ready {
                slot,
                width,
                height,
            } => Some((slot, width, height)),
            ExternalTextureState::Pending
            | ExternalTextureState::Failed
            | ExternalTextureState::Released => None,
        }
    }

    /// 查询外部纹理逻辑句柄当前的完整状态。
    #[must_use]
    pub fn external_texture_state(
        &self,
        handle: ExternalTextureHandle,
    ) -> Option<ExternalTextureState> {
        self.external_textures.get(&handle).map(|r| r.state)
    }

    /// 取出并清空本帧待由窗口运行器执行的外部纹理 GPU 操作队列。
    pub fn take_external_texture_ops(&mut self) -> Vec<ExternalTextureOp> {
        core::mem::take(&mut self.external_ops)
    }

    /// 由窗口运行器在执行 GPU 上传/更新后回写槽位分配结果。
    pub fn commit_external_texture_slot(
        &mut self,
        handle: ExternalTextureHandle,
        slot: Result<u32, String>,
    ) {
        let Some(record) = self.external_textures.get_mut(&handle) else {
            return;
        };
        if record.state == ExternalTextureState::Released {
            return;
        }
        match slot {
            Ok(slot) => {
                record.state = ExternalTextureState::Ready {
                    slot,
                    width: record.width,
                    height: record.height,
                };
            }
            Err(_) => {
                record.state = ExternalTextureState::Failed;
            }
        }
        self.external_dirty = true;
    }

    /// 在 GPU 设备丢失恢复或销毁 Vulkan 上下文时清空全部外部纹理槽位绑定。
    ///
    /// 所有原 `Pending` / `Ready` 句柄均转为 [`ExternalTextureState::Failed`]，
    /// 防止渲染阶段继续采样失效的外部槽位索引。
    pub fn invalidate_all_external_textures(&mut self) {
        self.external_ops.clear();
        for record in self.external_textures.values_mut() {
            match record.state {
                ExternalTextureState::Pending | ExternalTextureState::Ready { .. } => {
                    record.state = ExternalTextureState::Failed;
                }
                ExternalTextureState::Failed | ExternalTextureState::Released => {}
            }
        }
        self.external_dirty = true;
    }

    /// 创建一个新的视频表面逻辑句柄，初始状态为 [`VideoSurfaceState::PendingFirstFrame`]。
    pub fn create_video_surface(&mut self) -> VideoSurfaceHandle {
        self.video_surfaces.create_surface()
    }

    /// 向指定视频表面提交一帧载荷，采用 Latest-Wins 覆盖策略并累计丢帧数。
    ///
    /// 同尺寸新帧提交至已 `Ready` 的表面时仅标记轻量绘制脏标志，绝不触发 `DirtyFlags::LAYOUT`。
    pub fn submit_video_frame(
        &mut self,
        handle: VideoSurfaceHandle,
        frame: VideoFramePayload,
    ) -> Result<(), VideoFrameError> {
        self.video_surfaces.submit_frame(handle, frame)
    }

    /// 主动释放指定视频表面，并入队对应的 GPU 槽位回收操作。
    pub fn release_video_surface(&mut self, handle: VideoSurfaceHandle) {
        self.video_surfaces.release_surface(handle);
    }

    /// 查询指定视频表面当前的生命周期状态。
    #[must_use]
    pub fn video_surface_state(&self, handle: VideoSurfaceHandle) -> Option<&VideoSurfaceState> {
        self.video_surfaces.state(handle)
    }

    /// 查询指定视频表面当前可采样的 GPU 槽位与分辨率 `(slot, width, height)`。
    ///
    /// `Resizing` 阶段继续返回旧槽位实现零闪烁保帧；未就绪、已驱逐或已释放时返回 `None`。
    #[must_use]
    pub fn video_surface_slot(&self, handle: VideoSurfaceHandle) -> Option<(u32, u32, u32)> {
        self.video_surfaces.slot(handle)
    }

    /// 标记指定视频表面在当前帧索引 `frame_index` 下位于视口中可见。
    pub fn mark_video_surface_visible(&mut self, handle: VideoSurfaceHandle, frame_index: u64) {
        self.video_surfaces.mark_visible(handle, frame_index);
    }

    /// 设置多路视频表面的 VRAM 显存预算上限（字节）。
    pub fn set_video_vram_budget_bytes(&mut self, budget: u64) {
        self.video_surfaces.set_vram_budget_bytes(budget);
    }

    /// 查询当前多路视频表面的 VRAM 显存预算上限（字节）。
    #[must_use]
    pub const fn video_vram_budget_bytes(&self) -> u64 {
        self.video_surfaces.vram_budget_bytes()
    }

    /// 同步底层 GPU 渲染器协商激活的视频上传档位。
    pub fn set_video_upload_tier(&mut self, tier: VideoUploadTier) {
        self.video_surfaces.set_upload_tier(tier);
    }

    /// 排空本帧待执行的视频表面上传、离屏 LRU 驱逐与释放操作队列。
    pub fn drain_video_surface_ops(&mut self, frame_index: u64) -> Vec<VideoSurfaceOp> {
        self.video_surfaces.drain_ops(frame_index)
    }

    /// 在 GPU 上传完成后原子提交新纹理槽位与显存占用，若替换了旧槽位则返回旧槽位供延迟销毁。
    pub fn commit_video_surface_slot(
        &mut self,
        handle: VideoSurfaceHandle,
        slot: u32,
        width: u32,
        height: u32,
        format: VideoPixelFormat,
        vram_bytes: u64,
    ) -> Option<u32> {
        self.video_surfaces
            .commit_slot(handle, slot, width, height, format, vram_bytes)
    }

    /// 查询当前视频表面子系统的帧级观测指标快照。
    #[must_use]
    pub fn video_surface_metrics(&self) -> VideoSurfaceMetrics {
        self.video_surfaces.metrics()
    }

    /// 在 GPU 设备丢失或重建上下文时清空所有视频表面的活跃槽位绑定。
    pub fn invalidate_all_video_surfaces(&mut self) {
        self.video_surfaces.invalidate_all();
    }

    /// 同步当前 GPU 的 BC7 压缩纹理硬件能力快照。
    pub fn set_bc7_capabilities(&mut self, caps: CompressedTextureCapabilities) {
        self.bc7.set_capabilities(caps);
    }

    /// 标记 BC7 压缩资产池是否已挂载真实 GPU 渲染器。
    pub fn set_bc7_gpu_backed(&mut self, gpu_backed: bool) {
        self.bc7.set_gpu_backed(gpu_backed);
    }

    /// 查询当前生效的 BC7 压缩纹理能力快照。
    #[must_use]
    pub const fn bc7_capabilities(&self) -> CompressedTextureCapabilities {
        self.bc7.capabilities()
    }

    /// 设置 BC7 压缩资产池的显存预算上限（字节，按真实 1 B/px 记账）。
    pub fn set_vram_budget_bytes(&mut self, budget_bytes: u64) {
        self.bc7.set_vram_budget_bytes(budget_bytes);
    }

    /// 查询当前 BC7 压缩资产池的显存预算上限（字节）。
    #[must_use]
    pub const fn vram_budget_bytes(&self) -> u64 {
        self.bc7.vram_budget_bytes()
    }

    /// 设置单帧 BC7 上传节拍预算（条数与字节数双重封顶）。
    pub fn set_bc7_upload_pacing(&mut self, budget: UploadPacingBudget) {
        self.bc7.set_upload_pacing(budget);
    }

    /// 查询当前单帧 BC7 上传节拍预算。
    #[must_use]
    pub const fn bc7_upload_pacing(&self) -> UploadPacingBudget {
        self.bc7.upload_pacing()
    }

    /// 设置 BC7 多页图集与独立外部槽的尺寸分流阈值（默认 `512` 像素）。
    pub fn set_bc7_atlas_threshold(&mut self, threshold_px: u32) {
        self.bc7.set_atlas_threshold(threshold_px);
    }

    /// 提交 BC7 压缩纹理载荷并分配新的 [`AssetId`]，排入帧级节拍上传队列。
    ///
    /// # Errors
    /// 当待传队列达到容量上限时返回 [`AssetError::QueueFull`]。
    pub fn submit_compressed_bc7(
        &mut self,
        payload: CompressedTexturePayload,
    ) -> Result<AssetId, AssetError> {
        let id = AssetId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.bc7.submit_for(id, payload)
    }

    /// 为指定 [`AssetId`] 提交 BC7 压缩纹理载荷（用于同 ID 去重覆写或在驱逐 / `recover_gpu` 失效后无缝恢复）。
    ///
    /// # Errors
    /// 当该 ID 不在队列中且待传队列达到容量上限时返回 [`AssetError::QueueFull`]。
    pub fn submit_compressed_bc7_for(
        &mut self,
        id: AssetId,
        payload: CompressedTexturePayload,
    ) -> Result<AssetId, AssetError> {
        if id.raw() >= self.next_id {
            self.next_id = id.raw().saturating_add(1).max(1);
        }
        self.bc7.submit_for(id, payload)
    }

    /// 立即取消待传队列中尚未上传的 BC7 压缩纹理任务。
    pub fn cancel_compressed_upload(&mut self, id: AssetId) -> bool {
        self.bc7.cancel_upload(id)
    }

    /// 显式标记指定资产在本帧活跃，保护其不被帧末 LRU 驱逐。
    pub fn touch(&self, id: AssetId) {
        self.bc7.touch_explicit(id);
    }

    /// 取出并清空本帧待由窗口运行器执行的 BC7 GPU 操作队列。
    pub fn take_bc7_ops(&mut self) -> Vec<Bc7GpuOp> {
        self.bc7.take_ops()
    }

    /// 由窗口运行器回写动态 BC7 图集页的 Bindless 槽位分配结果。
    pub fn commit_bc7_page_slot(&mut self, page_id: AtlasPageId, slot: Result<u32, String>) {
        self.bc7.commit_page_slot(page_id, slot);
    }

    /// 由窗口运行器回写独立 BC7 外部纹理的 Bindless 槽位分配结果。
    pub fn commit_bc7_standalone_slot(&mut self, asset_id: AssetId, slot: Result<u32, String>) {
        self.bc7.commit_standalone_slot(asset_id, slot);
    }

    /// 在设备丢失恢复（`recover_gpu`）或销毁 Vulkan 上下文时失效清空全部 BC7 页与独立槽。
    pub fn invalidate_all_bc7_pages(&mut self) {
        self.bc7.invalidate_all_pages();
    }

    /// 当前活跃的 BC7 动态图集页数量。
    #[must_use]
    pub fn bc7_active_pages(&self) -> usize {
        self.bc7.active_pages()
    }

    /// 当前已驻留 BC7 资产占用的真实显存字节数（1 B/px）。
    #[must_use]
    pub const fn bc7_vram_bytes(&self) -> u64 {
        self.bc7.vram_bytes()
    }

    /// 本帧从节拍队列出队上传的 BC7 纹理数量。
    #[must_use]
    pub const fn bc7_uploads_this_frame(&self) -> usize {
        self.bc7.uploads_this_frame()
    }

    /// 当前待传队列中剩余的 BC7 纹理数量。
    #[must_use]
    pub fn bc7_pending_uploads(&self) -> usize {
        self.bc7.pending_uploads()
    }

    /// 累计因显存预算超限触发的 BC7 离屏 LRU 驱逐次数。
    #[must_use]
    pub const fn bc7_eviction_count(&self) -> u64 {
        self.bc7.eviction_count()
    }

    /// 当前是否处于非原生 BC7 透明降级模式。
    #[must_use]
    pub const fn bc7_fallback_active(&self) -> bool {
        self.bc7.fallback_active()
    }

    /// 判断指定 [`AssetId`] 当前是否驻留在独立 Bindless 外部槽（而非多页图集）。
    #[must_use]
    pub fn is_bc7_standalone(&self, id: AssetId) -> bool {
        self.bc7.is_standalone(id)
    }

    /// 在视图构建开始前清空上一帧由 `resolve` 采集的 BC7 可见保护集。
    pub(crate) fn begin_bc7_view_pass(&mut self) {
        self.bc7.begin_view_pass();
    }

    /// 在帧末执行 BC7 显存预算检查与离屏 LRU 驱逐，返回是否发生了驱逐。
    pub(crate) fn finish_bc7_frame_lru(&mut self) -> bool {
        self.bc7.finish_frame_lru()
    }
}

impl ImageAssetResolver for AssetHub {
    fn resolve(&self, id: AssetId) -> Option<ResolvedImage> {
        if let Some(residency) = self.residency.get(&id) {
            return match residency {
                AssetResidency::Ready { layer, uv, size } => Some(ResolvedImage {
                    layer: *layer,
                    uv: *uv,
                    size: *size,
                }),
                AssetResidency::Loading | AssetResidency::Failed => None,
            };
        }
        self.bc7.resolve(id)
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
    fn external_texture_upload_update_release_lifecycle() {
        let mut hub = AssetHub::new();
        let rgba = vec![255u8; 4 * 4 * 4];
        let handle = hub.upload_external_rgba8(4, 4, true, rgba.clone());
        assert_eq!(
            hub.external_texture_state(handle),
            Some(ExternalTextureState::Pending)
        );
        assert_eq!(hub.external_texture_slot(handle), None);

        let ops = hub.take_external_texture_ops();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            &ops[0],
            ExternalTextureOp::Upload {
                handle: h,
                width: 4,
                height: 4,
                srgb: true,
                rgba: data,
            } if *h == handle && data.len() == 64
        ));

        hub.commit_external_texture_slot(handle, Ok(48));
        assert_eq!(hub.external_texture_slot(handle), Some((48, 4, 4)));

        let updated = vec![128u8; 4 * 4 * 4];
        assert!(hub.update_external_rgba8(handle, 4, 4, updated));
        let ops = hub.take_external_texture_ops();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            &ops[0],
            ExternalTextureOp::Update {
                handle: h,
                width: 4,
                height: 4,
                ..
            } if *h == handle
        ));

        assert!(hub.release_external_texture(handle));
        assert_eq!(
            hub.external_texture_state(handle),
            Some(ExternalTextureState::Released)
        );
        assert_eq!(hub.external_texture_slot(handle), None);
        assert!(!hub.update_external_rgba8(handle, 4, 4, rgba));

        let ops = hub.take_external_texture_ops();
        assert_eq!(ops, vec![ExternalTextureOp::Release { handle }]);
    }

    #[test]
    fn same_frame_upload_and_release_coalesces_to_noop() {
        let mut hub = AssetHub::new();
        let rgba = vec![42u8; 2 * 2 * 4];
        let handle = hub.upload_external_rgba8(2, 2, false, rgba.clone());
        assert!(hub.update_external_rgba8(handle, 2, 2, rgba.clone()));
        assert!(hub.release_external_texture(handle));
        assert_eq!(
            hub.external_texture_state(handle),
            Some(ExternalTextureState::Released)
        );
        assert!(
            hub.take_external_texture_ops().is_empty(),
            "same-frame upload + release must coalesce without GPU ops"
        );
        assert!(!hub.update_external_rgba8(handle, 2, 2, rgba));
    }

    #[test]
    fn invalidate_all_external_textures_clears_stale_slots() {
        let mut hub = AssetHub::new();
        let h1 = hub.upload_external_rgba8(2, 2, true, vec![1u8; 16]);
        let _ = hub.take_external_texture_ops();
        hub.commit_external_texture_slot(h1, Ok(49));
        let h2 = hub.upload_external_rgba8(2, 2, true, vec![2u8; 16]);
        assert_eq!(hub.external_texture_slot(h1), Some((49, 2, 2)));

        hub.invalidate_all_external_textures();
        assert_eq!(hub.external_texture_slot(h1), None);
        assert_eq!(
            hub.external_texture_state(h1),
            Some(ExternalTextureState::Failed)
        );
        assert_eq!(
            hub.external_texture_state(h2),
            Some(ExternalTextureState::Failed)
        );
        assert!(hub.take_external_texture_ops().is_empty());
    }
}

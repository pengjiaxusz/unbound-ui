//! 双路径视频表面上传管线、设备能力画像与纯 CPU 参考色彩转换。
//!
//! 管线按平台与驱动能力自动分档：
//! * **Tier 1（[`VideoUploadTier::ExternalMemoryZeroCopy`]）**：Windows 下且驱动启用
//!   `VK_KHR_external_memory_win32` 时，通过 `VkExternalMemoryImageCreateInfo` +
//!   `VkImportMemoryWin32HandleInfoKHR` + `VkMemoryDedicatedAllocateInfo` 将 D3D11/D3D12
//!   DXGI 共享句柄零拷贝导入为只读多平面 `VkImage`，直接由 `video_convert.comp` 采样转换；
//! * **Tier 2（[`VideoUploadTier::AsyncStaging`]）**：跨平台通用路径，使用按
//!   `frames_in_flight` 切分的持久映射 `HostVisible` 异步 Staging 环形池与独立命令缓冲 +
//!   Vulkan Timeline Semaphore 推进上传与转换，严禁逐帧调用 `end_single_time` / `vkQueueWaitIdle`；
//! * **Headless（[`VideoUploadTier::HeadlessStub`]）**：无 GPU 环境下的能力占位。

#![allow(unsafe_op_in_unsafe_fn)]

use std::collections::{HashMap, VecDeque};

use ash::vk;
use bytemuck::{Pod, Zeroable};
use unbound_asset::video::{
    VideoColorMatrix, VideoColorRange, VideoColorSpace, VideoExternalHandleDesc,
    VideoExternalHandleKind, VideoFramePayload, VideoFramePlane, VideoFrameStorage,
    VideoPixelFormat, VideoTransferFunction,
};

use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use crate::inspector::{
    ExternalImageDesc, ExternalMemoryHandleKind, ExternalMemoryImporter, EXTENDED_EXTERNAL_SLOTS,
};
use crate::pipeline::{ExternalTextureSlots, Pipelines};

/// 默认 HDR 峰值亮度（单位：nits）。
pub const DEFAULT_HDR_PEAK_NITS: f32 = 1000.0;
/// SDR 参考白亮度（ITU-R BT.2408 基准 203 nits）。
pub const SDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// 视频帧上传路径层级。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum VideoUploadTier {
    /// Tier 1：Windows DXGI / D3D11/D3D12 外部显存句柄零拷贝导入。
    ExternalMemoryZeroCopy,
    /// Tier 2：多槽持久映射异步 Staging 环形池 + Timeline Semaphore 零阻塞上传。
    AsyncStaging,
    /// 无头 / 无 GPU 占位模式。
    HeadlessStub,
}

/// 视频像素格式支持位掩码（覆盖 NV12 / P010 / I420）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VideoFormatMask(u32);

impl VideoFormatMask {
    /// 空掩码。
    pub const NONE: Self = Self(0);
    /// 支持 8-bit 半平面 NV12。
    pub const NV12: Self = Self(1 << 0);
    /// 支持 10-bit 高位对齐半平面 P010。
    pub const P010: Self = Self(1 << 1);
    /// 支持 8-bit 三平面 I420。
    pub const I420: Self = Self(1 << 2);
    /// 支持全部内置视频像素格式（NV12 | P010 | I420）。
    pub const ALL: Self = Self(Self::NV12.0 | Self::P010.0 | Self::I420.0);

    /// 返回底层位掩码数值。
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// 从原始位掩码构造格式掩码（仅保留已定义位）。
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// 判断掩码是否为空。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 判断是否包含另一组格式掩码。
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// 合并两组格式掩码。
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self((self.0 | other.0) & Self::ALL.0)
    }

    /// 将单个 [`VideoPixelFormat`] 转换为对应格式掩码位。
    #[must_use]
    pub const fn from_format(format: VideoPixelFormat) -> Self {
        match format {
            VideoPixelFormat::Nv12 => Self::NV12,
            VideoPixelFormat::P010 => Self::P010,
            VideoPixelFormat::I420 => Self::I420,
        }
    }

    /// 查询当前掩码是否支持指定像素格式。
    #[must_use]
    pub const fn supports(self, format: VideoPixelFormat) -> bool {
        self.contains(Self::from_format(format))
    }
}

/// 当前 GPU 设备的视频帧上传能力画像。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VideoUploadCapabilities {
    /// 当前默认激活的视频上传层级。
    pub active_tier: VideoUploadTier,
    /// 逻辑设备是否已启用 `VK_KHR_external_memory_win32`。
    pub external_memory_win32: bool,
    /// 当前着色器管线支持的源像素格式掩码。
    pub supported_formats: VideoFormatMask,
    /// Tier 2 异步 Staging 环形缓冲槽位数量。
    pub async_staging_ring_slots: usize,
    /// 可供视频表面绑定的最大 Bindless 外部纹理槽位数。
    pub max_bindless_video_slots: usize,
}

impl VideoUploadCapabilities {
    /// 构造无头 / 无 GPU 环境下的占位能力画像。
    #[must_use]
    pub const fn headless_stub() -> Self {
        Self {
            active_tier: VideoUploadTier::HeadlessStub,
            external_memory_win32: false,
            supported_formats: VideoFormatMask::ALL,
            async_staging_ring_slots: 0,
            max_bindless_video_slots: EXTENDED_EXTERNAL_SLOTS as usize,
        }
    }

    /// 根据设备扩展协商结果与在飞帧槽位数构造能力画像。
    #[must_use]
    pub const fn from_device_extensions(has_win32_ext: bool, ring_slots: usize) -> Self {
        let active_tier = if has_win32_ext {
            VideoUploadTier::ExternalMemoryZeroCopy
        } else {
            VideoUploadTier::AsyncStaging
        };
        let normalized_slots = if ring_slots == 0 { 1 } else { ring_slots };
        Self {
            active_tier,
            external_memory_win32: has_win32_ext,
            supported_formats: VideoFormatMask::ALL,
            async_staging_ring_slots: normalized_slots,
            max_bindless_video_slots: EXTENDED_EXTERNAL_SLOTS as usize,
        }
    }
}

/// `video_convert.comp` 的 32 字节 Push Constant 布局。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct VideoConvertPush {
    /// 目标视频宽度（像素）。
    pub width: u32,
    /// 目标视频高度（像素）。
    pub height: u32,
    /// 像素格式编码：`0 = NV12`，`1 = P010`，`2 = I420`。
    pub format: u32,
    /// 色彩矩阵编码：`0 = BT.601`，`1 = BT.709`，`2 = BT.2020`。
    pub color_matrix: u32,
    /// 色彩量化范围编码：`0 = Limited`，`1 = Full`。
    pub color_range: u32,
    /// 光电传递函数编码：`0 = SDR`，`1 = PQ`，`2 = HLG`。
    pub transfer_fn: u32,
    /// HDR 峰值亮度（单位：nits，默认 `1000.0`）。
    pub hdr_peak_nits: f32,
    /// 保留对齐填充字。
    pub _pad: u32,
}

impl VideoConvertPush {
    /// Push Constant 字节大小（严格 32 字节）。
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;

    /// 从视频帧载荷与目标 HDR 峰值亮度构造 Push Constant。
    #[must_use]
    pub fn from_payload(frame: &VideoFramePayload, hdr_peak_nits: f32) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            format: frame.format.gpu_format_code(),
            color_matrix: frame.color_space.matrix.gpu_matrix_code(),
            color_range: frame.color_space.range.gpu_range_code(),
            transfer_fn: frame.color_space.transfer.gpu_transfer_code(),
            hdr_peak_nits: if hdr_peak_nits > 0.0 {
                hdr_peak_nits
            } else {
                DEFAULT_HDR_PEAK_NITS
            },
            _pad: 0,
        }
    }
}

/// 单帧视频上传并转换为外部 Bindless RGBA8 纹理后的结果摘要。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VideoFrameUploadResult {
    /// 目标 Bindless 外部纹理槽位（`48..304`）。
    pub slot: u32,
    /// 视频帧宽度（像素）。
    pub width: u32,
    /// 视频帧高度（像素）。
    pub height: u32,
    /// 源视频像素格式。
    pub format: VideoPixelFormat,
    /// 本次上传实际执行的路径层级。
    pub tier_used: VideoUploadTier,
    /// 本视频表面估算占用的显存字节数（源平面 + 目标 RGBA8）。
    pub vram_bytes: u64,
}

/// 将上游 `VideoExternalHandleKind` 映射为 `inspector::ExternalMemoryHandleKind`。
#[must_use]
pub const fn map_external_handle_kind(kind: VideoExternalHandleKind) -> ExternalMemoryHandleKind {
    match kind {
        VideoExternalHandleKind::OpaqueWin32 => ExternalMemoryHandleKind::OpaqueWin32,
        VideoExternalHandleKind::OpaqueWin32Kmt => ExternalMemoryHandleKind::OpaqueWin32Kmt,
        VideoExternalHandleKind::D3D11Texture => ExternalMemoryHandleKind::D3D11Texture,
        VideoExternalHandleKind::D3D11TextureKmt => ExternalMemoryHandleKind::D3D11TextureKmt,
        VideoExternalHandleKind::D3D12Resource => ExternalMemoryHandleKind::D3D12Resource,
        VideoExternalHandleKind::D3D12Heap => ExternalMemoryHandleKind::D3D12Heap,
    }
}

/// 返回多平面视频格式对应的 Vulkan 外部共享图像格式。
#[must_use]
pub const fn external_multiplanar_vk_format(format: VideoPixelFormat) -> vk::Format {
    match format {
        VideoPixelFormat::Nv12 => vk::Format::G8_B8R8_2PLANE_420_UNORM,
        VideoPixelFormat::P010 => vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
        VideoPixelFormat::I420 => vk::Format::G8_B8_R8_3PLANE_420_UNORM,
    }
}

/// 根据视频帧几何、格式与外部句柄构造 `inspector::ExternalImageDesc`。
#[must_use]
pub fn build_external_image_desc(
    width: u32,
    height: u32,
    format: VideoPixelFormat,
    handle: &VideoExternalHandleDesc,
) -> ExternalImageDesc {
    let allocation_size = if handle.allocation_size > 0 {
        handle.allocation_size
    } else {
        format.estimated_source_vram_bytes(width, height)
    };
    ExternalImageDesc {
        width,
        height,
        mip_levels: 1,
        array_layers: 1,
        vk_format_raw: external_multiplanar_vk_format(format).as_raw(),
        handle_kind: map_external_handle_kind(handle.kind),
        raw_handle: handle.raw_handle,
        allocation_size,
    }
}

/// 根据设备能力与帧存储类型推导上传层级（含 Tier 1 → Tier 2 平滑降级判定）。
///
/// # Errors
/// 当帧载荷校验失败，或外部句柄不可导入且缺少 `fallback_cpu` 时返回 [`GpuError::InvalidArgument`]。
pub fn resolve_upload_tier(
    capabilities: VideoUploadCapabilities,
    frame: &VideoFramePayload,
) -> Result<VideoUploadTier, GpuError> {
    if !capabilities.supported_formats.supports(frame.format) {
        return Err(GpuError::InvalidArgument(format!(
            "unsupported video pixel format: {:?}",
            frame.format
        )));
    }
    match &frame.storage {
        VideoFrameStorage::CpuPlanes { .. } => {
            frame
                .validate()
                .map_err(|e| GpuError::InvalidArgument(format!("invalid video frame: {e}")))?;
            if capabilities.active_tier == VideoUploadTier::HeadlessStub {
                Ok(VideoUploadTier::HeadlessStub)
            } else {
                Ok(VideoUploadTier::AsyncStaging)
            }
        }
        VideoFrameStorage::ExternalHandle {
            handle,
            fallback_cpu,
        } => {
            let desc = build_external_image_desc(frame.width, frame.height, frame.format, handle);
            let handle_valid = ExternalMemoryImporter::prepare_import(&desc).is_ok();
            let can_zero_copy = cfg!(windows) && capabilities.external_memory_win32 && handle_valid;
            if can_zero_copy {
                Ok(VideoUploadTier::ExternalMemoryZeroCopy)
            } else if fallback_cpu.is_some() {
                if capabilities.active_tier == VideoUploadTier::HeadlessStub {
                    Ok(VideoUploadTier::HeadlessStub)
                } else {
                    Ok(VideoUploadTier::AsyncStaging)
                }
            } else if !handle_valid {
                Err(GpuError::InvalidArgument(
                    "invalid external video handle with no CPU fallback".into(),
                ))
            } else {
                Err(GpuError::InvalidArgument(
                    "external Win32 memory import is unavailable on this device and no CPU fallback was provided".into(),
                ))
            }
        }
    }
}

/// 纯 CPU 异步 Staging 环形槽位调度器（按 Timeline Semaphore 值追踪槽位复用，便于无 GPU 单测）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagingRingScheduler {
    slot_timelines: Vec<u64>,
    cursor: usize,
}

impl StagingRingScheduler {
    /// 创建包含 `slot_count` 个槽位的异步环形调度器（至少 1 槽）。
    #[must_use]
    pub fn new(slot_count: usize) -> Self {
        let count = slot_count.max(1);
        Self {
            slot_timelines: vec![0; count],
            cursor: 0,
        }
    }

    /// 环形槽位总数。
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slot_timelines.len()
    }

    /// 选取下一个环形槽位，并返回复用前需等待的 Timeline 值（若该槽位已退役则为 `None`）。
    pub fn acquire(&mut self, completed_timeline: u64) -> (usize, Option<u64>) {
        let slot = self.cursor % self.slot_timelines.len();
        self.cursor = self.cursor.wrapping_add(1);
        let pending = self.slot_timelines[slot];
        let wait_value = (pending > completed_timeline).then_some(pending);
        (slot, wait_value)
    }

    /// 记录指定槽位刚刚提交的 Timeline 信号值。
    pub fn mark_submitted(&mut self, slot: usize, signal_timeline: u64) {
        let idx = slot % self.slot_timelines.len();
        self.slot_timelines[idx] = signal_timeline;
    }

    /// 查询指定槽位最近一次提交的 Timeline 信号值。
    #[must_use]
    pub fn slot_timeline(&self, slot: usize) -> u64 {
        self.slot_timelines[slot % self.slot_timelines.len()]
    }
}

/// 面向视频表面与临时外部导入图像的按帧代次延迟销毁队列。
#[derive(Debug)]
pub struct VideoDeferredDestroyQueue<T> {
    pending: VecDeque<(u64, T)>,
}

impl<T> Default for VideoDeferredDestroyQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> VideoDeferredDestroyQueue<T> {
    /// 创建空的视频延迟销毁队列。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }

    /// 将待销毁资源登记到队列中，至少在 `frames_in_flight` 帧退役后才允许回收。
    pub fn defer(&mut self, current_frame: u64, frames_in_flight: u32, item: T) {
        let retire_at = current_frame.saturating_add(u64::from(frames_in_flight.max(1)));
        self.pending.push_back((retire_at, item));
    }

    /// 回收所有在 `completed_frame` 代次时已安全退役的对象。
    pub fn drain_ready_with(&mut self, completed_frame: u64, mut on_reclaim: impl FnMut(T)) {
        while let Some((retire_at, _)) = self.pending.front() {
            if *retire_at > completed_frame {
                break;
            }
            if let Some((_, item)) = self.pending.pop_front() {
                on_reclaim(item);
            }
        }
    }

    /// 在设备空闲销毁阶段强制排空队列中的全部资源。
    pub fn drain_all_with(&mut self, mut on_reclaim: impl FnMut(T)) {
        while let Some((_, item)) = self.pending.pop_front() {
            on_reclaim(item);
        }
    }

    /// 当前等待退役的对象数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// 队列是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// 单个托管视频图像资源（用于源多平面纹理或目标 RGBA8 纹理）。
#[derive(Debug)]
struct VideoImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    layout: vk::ImageLayout,
    width: u32,
    height: u32,
    format: vk::Format,
}

impl VideoImage {
    fn new(
        gpu: &Gpu,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, GpuError> {
        let width = width.max(1);
        let height = height.max(1);
        // SAFETY: 顺序创建图像、显存与视图，任一步骤失败均完整回滚。
        unsafe {
            let device = &gpu.device;
            let image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D {
                            width,
                            height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(usage)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
                .map_err(GpuError::from)?;
            let requirements = device.get_image_memory_requirements(image);
            let Some(type_index) = gpu
                .memory_type_index_for_bits(requirements.memory_type_bits, MemoryKind::DeviceLocal)
            else {
                device.destroy_image(image, None);
                return Err(GpuError::Allocation(
                    "no device-local memory for video image".into(),
                ));
            };
            let memory = match device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(type_index),
                None,
            ) {
                Ok(m) => m,
                Err(err) => {
                    device.destroy_image(image, None);
                    return Err(GpuError::from(err));
                }
            };
            if let Err(err) = device.bind_image_memory(image, memory, 0) {
                device.destroy_image(image, None);
                device.free_memory(memory, None);
                return Err(GpuError::from(err));
            }
            let view = match device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            ) {
                Ok(v) => v,
                Err(err) => {
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                    return Err(GpuError::from(err));
                }
            };
            Ok(Self {
                image,
                memory,
                view,
                layout: vk::ImageLayout::UNDEFINED,
                width,
                height,
                format,
            })
        }
    }

    unsafe fn destroy(self, gpu: &Gpu) {
        gpu.device.destroy_image_view(self.view, None);
        gpu.device.destroy_image(self.image, None);
        gpu.device.free_memory(self.memory, None);
    }
}

/// Tier 1 Windows 外部共享显存零拷贝导入的临时源视频图像。
#[cfg(windows)]
#[derive(Debug)]
struct ImportedExternalVideoImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    plane_views: Vec<vk::ImageView>,
}

#[cfg(windows)]
impl ImportedExternalVideoImage {
    unsafe fn destroy(self, gpu: &Gpu) {
        for view in self.plane_views {
            gpu.device.destroy_image_view(view, None);
        }
        gpu.device.destroy_image(self.image, None);
        gpu.device.free_memory(self.memory, None);
    }
}

/// 单个视频表面持有的目标 RGBA8 外部纹理与可复用源平面纹理集合。
#[derive(Debug)]
struct VideoSurface {
    width: u32,
    height: u32,
    format: VideoPixelFormat,
    target_rgba: VideoImage,
    source_planes: Vec<VideoImage>,
}

impl VideoSurface {
    unsafe fn destroy(self, gpu: &Gpu) {
        self.target_rgba.destroy(gpu);
        for plane in self.source_planes {
            plane.destroy(gpu);
        }
    }
}

/// 推入延迟销毁队列的视频 GPU 资源枚举。
#[derive(Debug)]
enum VideoDeferredResource {
    Surface(VideoSurface),
    #[cfg(windows)]
    Imported(ImportedExternalVideoImage),
}

impl VideoDeferredResource {
    unsafe fn destroy(self, gpu: &Gpu) {
        match self {
            Self::Surface(surface) => surface.destroy(gpu),
            #[cfg(windows)]
            Self::Imported(imported) => imported.destroy(gpu),
        }
    }
}

/// 异步 Staging 环形池中的单个提交槽位。
#[derive(Debug)]
struct AsyncStagingSlot {
    command_buffer: vk::CommandBuffer,
    descriptor_set: vk::DescriptorSet,
    staging_buffer: Option<Buffer>,
}

/// Tier 2 多槽持久映射异步 Staging 环形池与 Compute 提交执行器。
///
/// 每个槽位拥有独立的命令缓冲区、描述符集与按需扩容的持久映射 `HostVisible` 暂存缓冲；
/// 提交时通过 [`Gpu::timeline`] 异步推进，严禁调用 `end_single_time` 或 `vkQueueWaitIdle`。
#[derive(Debug)]
pub struct VideoAsyncStagingRing {
    command_pool: vk::CommandPool,
    descriptor_pool: vk::DescriptorPool,
    slots: Vec<AsyncStagingSlot>,
    scheduler: StagingRingScheduler,
}

impl VideoAsyncStagingRing {
    /// 为当前设备创建按 `frames_in_flight` 切分的异步视频上传环形池。
    ///
    /// # Errors
    /// 透传 Vulkan 命令池、描述符池或描述符集创建错误。
    pub fn new(gpu: &Gpu, pipelines: &Pipelines) -> Result<Self, GpuError> {
        let slot_count = (gpu.frames_in_flight() as usize).max(2);
        // SAFETY: 在有效设备上顺序创建命令池、描述符池、命令缓冲与描述符集，失败即回滚。
        unsafe {
            let command_pool = gpu
                .device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(gpu.families.graphics)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(GpuError::from)?;
            let cmd_buffers = match gpu.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(slot_count as u32),
            ) {
                Ok(cmds) => cmds,
                Err(err) => {
                    gpu.device.destroy_command_pool(command_pool, None);
                    return Err(GpuError::from(err));
                }
            };

            let pool_sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count((slot_count * 3) as u32),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_IMAGE)
                    .descriptor_count(slot_count as u32),
            ];
            let descriptor_pool = match gpu.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(slot_count as u32)
                    .pool_sizes(&pool_sizes),
                None,
            ) {
                Ok(pool) => pool,
                Err(err) => {
                    gpu.device.destroy_command_pool(command_pool, None);
                    return Err(GpuError::from(err));
                }
            };
            let set_layouts = vec![pipelines.video_set_layout; slot_count];
            let descriptor_sets = match gpu.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&set_layouts),
            ) {
                Ok(sets) => sets,
                Err(err) => {
                    gpu.device.destroy_descriptor_pool(descriptor_pool, None);
                    gpu.device.destroy_command_pool(command_pool, None);
                    return Err(GpuError::from(err));
                }
            };

            let slots = cmd_buffers
                .into_iter()
                .zip(descriptor_sets)
                .map(|(command_buffer, descriptor_set)| AsyncStagingSlot {
                    command_buffer,
                    descriptor_set,
                    staging_buffer: None,
                })
                .collect();

            Ok(Self {
                command_pool,
                descriptor_pool,
                slots,
                scheduler: StagingRingScheduler::new(slot_count),
            })
        }
    }

    /// 环形池槽位总数。
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// 获取下一个可安全复用的环形槽位（仅在全部在飞槽位均未退役时按 Timeline 值等待该槽位）。
    fn acquire_ready_slot(&mut self, gpu: &Gpu) -> Result<usize, GpuError> {
        // SAFETY: 读取设备共享时间线信号量的当前硬件完成值。
        let completed = unsafe {
            gpu.device
                .get_semaphore_counter_value(gpu.timeline.semaphore)
                .map_err(GpuError::from)?
        };
        gpu.timeline.completed(completed);
        let (slot_idx, wait_val) = self.scheduler.acquire(completed);
        if let Some(wait_val) = wait_val {
            gpu.wait_timeline(wait_val)?;
        }
        Ok(slot_idx)
    }

    /// 确保目标槽位的持久映射 Staging 缓冲区容量不少于 `needed_bytes`。
    fn ensure_slot_staging(
        &mut self,
        gpu: &Gpu,
        slot_idx: usize,
        needed_bytes: u64,
    ) -> Result<(), GpuError> {
        let needed = needed_bytes.max(64 * 1024);
        let needs_realloc = self.slots[slot_idx]
            .staging_buffer
            .as_ref()
            .is_none_or(|buf| buf.size < needed);
        if !needs_realloc {
            return Ok(());
        }
        if let Some(old) = self.slots[slot_idx].staging_buffer.take() {
            // SAFETY: 当前槽位已通过 Timeline 退役，旧 Staging 缓冲不再被 GPU 引用。
            unsafe { gpu.destroy_buffer(&old) };
        }
        let capacity = needed.next_power_of_two();
        let buffer = gpu.create_buffer(
            capacity,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryKind::HostVisible,
            "video-async-staging",
        )?;
        self.slots[slot_idx].staging_buffer = Some(buffer);
        Ok(())
    }

    /// 更新指定槽位描述符集的 3 个源平面采样器与 1 个输出存储图像绑定。
    unsafe fn update_descriptor_set(
        gpu: &Gpu,
        pipelines: &Pipelines,
        set: vk::DescriptorSet,
        plane_views: [vk::ImageView; 3],
        target_view: vk::ImageView,
    ) {
        let p0 = [vk::DescriptorImageInfo::default()
            .sampler(pipelines.sampler)
            .image_view(plane_views[0])
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let p1 = [vk::DescriptorImageInfo::default()
            .sampler(pipelines.sampler)
            .image_view(plane_views[1])
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let p2 = [vk::DescriptorImageInfo::default()
            .sampler(pipelines.sampler)
            .image_view(plane_views[2])
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let dst = [vk::DescriptorImageInfo::default()
            .image_view(target_view)
            .image_layout(vk::ImageLayout::GENERAL)];

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&p0),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&p1),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&p2),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&dst),
        ];
        gpu.device.update_descriptor_sets(&writes, &[]);
    }

    /// 执行 Tier 2 异步 Staging 上传 + `video_convert.comp` 转换并提交至图形队列（不阻塞等待队列空闲）。
    #[allow(clippy::too_many_arguments)]
    fn submit_cpu_planes_convert(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        frame: &VideoFramePayload,
        data: &[u8],
        planes: &[VideoFramePlane],
        source_planes: &mut [VideoImage],
        target_rgba: &mut VideoImage,
    ) -> Result<(), GpuError> {
        let plane_count = frame.format.plane_count();
        if planes.len() != plane_count || source_planes.len() != plane_count {
            return Err(GpuError::InvalidArgument(format!(
                "plane count mismatch for {:?}: expected {plane_count}",
                frame.format
            )));
        }

        // 计算各平面紧凑打包后的 Staging 缓冲偏移（16 字节对齐）。
        let mut plane_offsets = [0u64; 3];
        let mut plane_row_bytes = [0usize; 3];
        let mut total_bytes = 0u64;
        for (i, plane) in planes.iter().enumerate() {
            let min_stride = frame
                .format
                .min_stride_bytes(frame.width, i)
                .ok_or_else(|| GpuError::InvalidArgument(format!("invalid plane index {i}")))?;
            if plane.stride_bytes < min_stride {
                return Err(GpuError::InvalidArgument(format!(
                    "plane {i} stride {} < min stride {min_stride}",
                    plane.stride_bytes
                )));
            }
            let aligned_offset = align_up_u64(total_bytes, 16);
            plane_offsets[i] = aligned_offset;
            plane_row_bytes[i] = min_stride;
            let packed_plane_bytes = (min_stride as u64).saturating_mul(u64::from(plane.height));
            total_bytes = aligned_offset.saturating_add(packed_plane_bytes);
        }

        let slot_idx = self.acquire_ready_slot(gpu)?;
        self.ensure_slot_staging(gpu, slot_idx, total_bytes)?;

        // 将源平面像素写入当前槽位持久映射 Staging 缓冲（支持任意行步长对齐）。
        {
            let staging = self.slots[slot_idx]
                .staging_buffer
                .as_ref()
                .ok_or_else(|| GpuError::Allocation("video staging buffer missing".into()))?;
            for (i, plane) in planes.iter().enumerate() {
                let row_bytes = plane_row_bytes[i];
                let dst_base = plane_offsets[i];
                if plane.stride_bytes == row_bytes {
                    let byte_len = row_bytes.saturating_mul(plane.height as usize);
                    let src_end = plane.offset_bytes.saturating_add(byte_len);
                    if src_end > data.len() {
                        return Err(GpuError::InvalidArgument(format!(
                            "plane {i} range {src_end} exceeds buffer length {}",
                            data.len()
                        )));
                    }
                    // SAFETY: 目标槽位已退役且偏移区间经边界校验位于 Staging 缓冲内。
                    let ok = unsafe {
                        gpu.write_host(staging, dst_base, &data[plane.offset_bytes..src_end])
                    };
                    if !ok {
                        return Err(GpuError::Allocation(
                            "failed to write video plane to staging buffer".into(),
                        ));
                    }
                } else {
                    for row in 0..(plane.height as usize) {
                        let src_start = plane
                            .offset_bytes
                            .saturating_add(row.saturating_mul(plane.stride_bytes));
                        let src_end = src_start.saturating_add(row_bytes);
                        if src_end > data.len() {
                            return Err(GpuError::InvalidArgument(format!(
                                "plane {i} row {row} exceeds buffer length {}",
                                data.len()
                            )));
                        }
                        let dst_off =
                            dst_base.saturating_add((row as u64).saturating_mul(row_bytes as u64));
                        // SAFETY: 目标槽位已退役且逐行写入均在 Staging 缓冲边界内。
                        let ok =
                            unsafe { gpu.write_host(staging, dst_off, &data[src_start..src_end]) };
                        if !ok {
                            return Err(GpuError::Allocation(
                                "failed to write strided video row to staging buffer".into(),
                            ));
                        }
                    }
                }
            }
        }

        let plane_views = [
            source_planes[0].view,
            source_planes[1].view,
            if plane_count >= 3 {
                source_planes[2].view
            } else {
                pipelines.dummy_r8.view
            },
        ];
        let push = VideoConvertPush::from_payload(frame, DEFAULT_HDR_PEAK_NITS);
        let cmd = self.slots[slot_idx].command_buffer;
        let set = self.slots[slot_idx].descriptor_set;
        let staging_raw = self.slots[slot_idx]
            .staging_buffer
            .as_ref()
            .map(|b| b.raw)
            .ok_or_else(|| GpuError::Allocation("video staging buffer missing".into()))?;

        // SAFETY: 槽位已退役，命令缓冲与描述符集均由本次提交独占。
        let signal_value = unsafe {
            Self::update_descriptor_set(gpu, pipelines, set, plane_views, target_rgba.view);

            let device = &gpu.device;
            device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                .map_err(GpuError::from)?;
            device
                .begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(GpuError::from)?;

            // 1. 源平面图像转换至 TRANSFER_DST_OPTIMAL。
            let mut to_transfer = Vec::with_capacity(plane_count);
            for plane_img in source_planes.iter() {
                let (src_stage, src_access) = if plane_img.layout == vk::ImageLayout::UNDEFINED {
                    (vk::PipelineStageFlags2::TOP_OF_PIPE, vk::AccessFlags2::NONE)
                } else {
                    (
                        vk::PipelineStageFlags2::COMPUTE_SHADER,
                        vk::AccessFlags2::SHADER_SAMPLED_READ,
                    )
                };
                to_transfer.push(video_image_barrier(
                    plane_img.image,
                    plane_img.layout,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    src_stage,
                    src_access,
                    vk::PipelineStageFlags2::COPY,
                    vk::AccessFlags2::TRANSFER_WRITE,
                ));
            }
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_transfer),
            );

            // 2. 从 Staging 缓冲异步拷贝至各源平面图像。
            for (i, plane_img) in source_planes.iter_mut().enumerate() {
                let region = vk::BufferImageCopy::default()
                    .buffer_offset(plane_offsets[i])
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: plane_img.width,
                        height: plane_img.height,
                        depth: 1,
                    });
                device.cmd_copy_buffer_to_image(
                    cmd,
                    staging_raw,
                    plane_img.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                plane_img.layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
            }

            // 3. 源平面转换至 SHADER_READ_ONLY_OPTIMAL，同时将目标 RGBA8 转换至 GENERAL。
            let mut pre_compute = Vec::with_capacity(plane_count + 1);
            for plane_img in source_planes.iter_mut() {
                pre_compute.push(video_image_barrier(
                    plane_img.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::COPY,
                    vk::AccessFlags2::TRANSFER_WRITE,
                    vk::PipelineStageFlags2::COMPUTE_SHADER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ,
                ));
                plane_img.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
            }
            let (dst_src_stage, dst_src_access) =
                if target_rgba.layout == vk::ImageLayout::UNDEFINED {
                    (vk::PipelineStageFlags2::TOP_OF_PIPE, vk::AccessFlags2::NONE)
                } else {
                    (
                        vk::PipelineStageFlags2::FRAGMENT_SHADER,
                        vk::AccessFlags2::SHADER_SAMPLED_READ,
                    )
                };
            pre_compute.push(video_image_barrier(
                target_rgba.image,
                target_rgba.layout,
                vk::ImageLayout::GENERAL,
                dst_src_stage,
                dst_src_access,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_STORAGE_WRITE,
            ));
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&pre_compute),
            );

            // 4. 调度 video_convert.comp 计算着色器执行 YUV/HDR → sRGB RGBA8 转换。
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipelines.video_convert);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                pipelines.video_layout,
                0,
                &[set],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                pipelines.video_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&push),
            );
            device.cmd_dispatch(
                cmd,
                frame.width.div_ceil(8).max(1),
                frame.height.div_ceil(8).max(1),
                1,
            );

            // 5. 将目标 RGBA8 转换至 SHADER_READ_ONLY_OPTIMAL 供 uber.frag 采样。
            let post_compute = [video_image_barrier(
                target_rgba.image,
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_STORAGE_WRITE,
                vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::SHADER_SAMPLED_READ,
            )];
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&post_compute),
            );
            target_rgba.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;

            device.end_command_buffer(cmd).map_err(GpuError::from)?;

            let signal_value = gpu.timeline.next();
            let command_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            let signal_infos = [vk::SemaphoreSubmitInfo::default()
                .semaphore(gpu.timeline.semaphore)
                .value(signal_value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let submit = vk::SubmitInfo2::default()
                .command_buffer_infos(&command_infos)
                .signal_semaphore_infos(&signal_infos);
            gpu.submit_graphics_queue2(&[submit], vk::Fence::null())?;
            signal_value
        };

        self.scheduler.mark_submitted(slot_idx, signal_value);
        Ok(())
    }

    /// 执行 Tier 1 零拷贝外部图像直接采样 + `video_convert.comp` 异步转换提交。
    #[cfg(windows)]
    fn submit_imported_planes_convert(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        frame: &VideoFramePayload,
        imported: &ImportedExternalVideoImage,
        target_rgba: &mut VideoImage,
    ) -> Result<(), GpuError> {
        let slot_idx = self.acquire_ready_slot(gpu)?;
        let plane_views = [
            imported.plane_views[0],
            imported.plane_views[1],
            imported
                .plane_views
                .get(2)
                .copied()
                .unwrap_or(pipelines.dummy_r8.view),
        ];
        let push = VideoConvertPush::from_payload(frame, DEFAULT_HDR_PEAK_NITS);
        let cmd = self.slots[slot_idx].command_buffer;
        let set = self.slots[slot_idx].descriptor_set;

        // SAFETY: 槽位已退役，导入图像在本次提交期间存活并由延迟销毁队列保护。
        let signal_value = unsafe {
            Self::update_descriptor_set(gpu, pipelines, set, plane_views, target_rgba.view);

            let device = &gpu.device;
            device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                .map_err(GpuError::from)?;
            device
                .begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(GpuError::from)?;

            let (dst_src_stage, dst_src_access) =
                if target_rgba.layout == vk::ImageLayout::UNDEFINED {
                    (vk::PipelineStageFlags2::TOP_OF_PIPE, vk::AccessFlags2::NONE)
                } else {
                    (
                        vk::PipelineStageFlags2::FRAGMENT_SHADER,
                        vk::AccessFlags2::SHADER_SAMPLED_READ,
                    )
                };
            let pre_compute = [
                video_image_barrier(
                    imported.image,
                    vk::ImageLayout::GENERAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    vk::AccessFlags2::NONE,
                    vk::PipelineStageFlags2::COMPUTE_SHADER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ,
                ),
                video_image_barrier(
                    target_rgba.image,
                    target_rgba.layout,
                    vk::ImageLayout::GENERAL,
                    dst_src_stage,
                    dst_src_access,
                    vk::PipelineStageFlags2::COMPUTE_SHADER,
                    vk::AccessFlags2::SHADER_STORAGE_WRITE,
                ),
            ];
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&pre_compute),
            );

            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipelines.video_convert);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                pipelines.video_layout,
                0,
                &[set],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                pipelines.video_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&push),
            );
            device.cmd_dispatch(
                cmd,
                frame.width.div_ceil(8).max(1),
                frame.height.div_ceil(8).max(1),
                1,
            );

            let post_compute = [video_image_barrier(
                target_rgba.image,
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_STORAGE_WRITE,
                vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::SHADER_SAMPLED_READ,
            )];
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&post_compute),
            );
            target_rgba.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;

            device.end_command_buffer(cmd).map_err(GpuError::from)?;

            let signal_value = gpu.timeline.next();
            let command_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            let signal_infos = [vk::SemaphoreSubmitInfo::default()
                .semaphore(gpu.timeline.semaphore)
                .value(signal_value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let submit = vk::SubmitInfo2::default()
                .command_buffer_infos(&command_infos)
                .signal_semaphore_infos(&signal_infos);
            gpu.submit_graphics_queue2(&[submit], vk::Fence::null())?;
            signal_value
        };

        self.scheduler.mark_submitted(slot_idx, signal_value);
        Ok(())
    }

    /// 销毁环形池持有的全部 Staging 缓冲、描述符池与命令池。
    ///
    /// # Safety
    /// 调用方须确保设备处于空闲状态。
    pub unsafe fn destroy(&mut self, gpu: &Gpu) {
        for slot in &mut self.slots {
            if let Some(buf) = slot.staging_buffer.take() {
                gpu.destroy_buffer(&buf);
            }
        }
        gpu.device
            .destroy_descriptor_pool(self.descriptor_pool, None);
        gpu.device.destroy_command_pool(self.command_pool, None);
    }
}

/// 渲染器内部视频表面管理器（统一管理 Tier 1 零拷贝导入、Tier 2 异步 Staging 环与延迟销毁）。
#[derive(Debug)]
pub(crate) struct VideoSurfaceManager {
    ring: VideoAsyncStagingRing,
    surfaces: HashMap<u32, VideoSurface>,
    deferred_destroys: VideoDeferredDestroyQueue<VideoDeferredResource>,
}

impl VideoSurfaceManager {
    pub(crate) fn new(gpu: &Gpu, pipelines: &Pipelines) -> Result<Self, GpuError> {
        let ring = VideoAsyncStagingRing::new(gpu, pipelines)?;
        Ok(Self {
            ring,
            surfaces: HashMap::new(),
            deferred_destroys: VideoDeferredDestroyQueue::new(),
        })
    }

    pub(crate) fn contains_slot(&self, slot: u32) -> bool {
        self.surfaces.contains_key(&slot)
    }

    #[must_use]
    pub(crate) fn pending_destroys(&self) -> usize {
        self.deferred_destroys.len()
    }

    /// 上传一帧视频并转换为目标 Bindless 外部 RGBA8 纹理槽。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn upload_video_frame(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        external_slots: &mut ExternalTextureSlots,
        current_frame: u64,
        frames_in_flight: u32,
        target_slot: Option<u32>,
        frame: &VideoFramePayload,
    ) -> Result<VideoFrameUploadResult, GpuError> {
        frame
            .validate()
            .map_err(|e| GpuError::InvalidArgument(format!("invalid video frame: {e}")))?;

        let (slot, allocated_new) = match target_slot {
            Some(existing) => {
                if !ExternalTextureSlots::owns(existing) {
                    return Err(GpuError::InvalidArgument(format!(
                        "bindless slot {existing} is not an external texture slot"
                    )));
                }
                if !self.surfaces.contains_key(&existing) {
                    return Err(GpuError::InvalidArgument(format!(
                        "bindless slot {existing} is not an active video surface"
                    )));
                }
                (existing, false)
            }
            None => {
                let s = external_slots.allocate().ok_or_else(|| {
                    GpuError::InvalidArgument(format!(
                        "all {} external bindless slots are in use",
                        crate::pipeline::EXTERNAL_TEXTURE_SLOTS
                    ))
                })?;
                (s, true)
            }
        };

        // 确保目标 RGBA8 表面存在且尺寸与格式匹配；尺寸变化时重建并将旧资源推入延迟销毁队列。
        let needs_new_surface = self.surfaces.get(&slot).is_none_or(|s| {
            s.width != frame.width || s.height != frame.height || s.format != frame.format
        });

        if needs_new_surface {
            let target_rgba = match VideoImage::new(
                gpu,
                frame.width,
                frame.height,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::STORAGE
                    | vk::ImageUsageFlags::TRANSFER_DST,
            ) {
                Ok(img) => img,
                Err(err) => {
                    if allocated_new {
                        external_slots.release(slot);
                    }
                    return Err(err);
                }
            };
            if let Err(err) =
                pipelines.bind_external_texture(gpu, slot, target_rgba.view, pipelines.sampler)
            {
                // SAFETY: 目标图像尚未被任何描述符集使用，可立即释放。
                unsafe { target_rgba.destroy(gpu) };
                if allocated_new {
                    external_slots.release(slot);
                }
                return Err(err);
            }
            if let Some(old_surface) = self.surfaces.insert(
                slot,
                VideoSurface {
                    width: frame.width,
                    height: frame.height,
                    format: frame.format,
                    target_rgba,
                    source_planes: Vec::new(),
                },
            ) {
                self.deferred_destroys.defer(
                    current_frame,
                    frames_in_flight,
                    VideoDeferredResource::Surface(old_surface),
                );
            }
        }

        let upload_res = match &frame.storage {
            VideoFrameStorage::CpuPlanes { data, planes } => self
                .upload_via_async_staging(gpu, pipelines, slot, frame, data, planes)
                .map(|()| VideoUploadTier::AsyncStaging),
            VideoFrameStorage::ExternalHandle {
                handle,
                fallback_cpu,
            } => {
                match self.try_upload_via_win32_zero_copy(
                    gpu,
                    pipelines,
                    slot,
                    current_frame,
                    frames_in_flight,
                    frame,
                    handle,
                ) {
                    Ok(()) => Ok(VideoUploadTier::ExternalMemoryZeroCopy),
                    Err(tier1_err) => {
                        if let Some((fb_data, fb_planes)) = fallback_cpu {
                            self.upload_via_async_staging(
                                gpu, pipelines, slot, frame, fb_data, fb_planes,
                            )
                            .map(|()| VideoUploadTier::AsyncStaging)
                        } else {
                            Err(tier1_err)
                        }
                    }
                }
            }
        };

        let tier_used = match upload_res {
            Ok(tier) => tier,
            Err(err) => {
                if allocated_new {
                    let _ = pipelines.unbind_external_texture(gpu, slot);
                    external_slots.release(slot);
                    if let Some(surface) = self.surfaces.remove(&slot) {
                        self.deferred_destroys.defer(
                            current_frame,
                            frames_in_flight,
                            VideoDeferredResource::Surface(surface),
                        );
                    }
                }
                return Err(err);
            }
        };

        let vram_bytes = frame.estimated_vram_bytes();

        Ok(VideoFrameUploadResult {
            slot,
            width: frame.width,
            height: frame.height,
            format: frame.format,
            tier_used,
            vram_bytes,
        })
    }

    /// Tier 2 路径：按需创建源多平面 `VkImage` 并通过异步 Staging 环提交转换。
    fn upload_via_async_staging(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        slot: u32,
        frame: &VideoFramePayload,
        data: &[u8],
        planes: &[VideoFramePlane],
    ) -> Result<(), GpuError> {
        let surface = self.surfaces.get_mut(&slot).ok_or_else(|| {
            GpuError::InvalidArgument(format!("video surface slot {slot} not found"))
        })?;
        ensure_source_plane_images(
            gpu,
            frame.width,
            frame.height,
            frame.format,
            &mut surface.source_planes,
        )?;
        self.ring.submit_cpu_planes_convert(
            gpu,
            pipelines,
            frame,
            data,
            planes,
            &mut surface.source_planes,
            &mut surface.target_rgba,
        )
    }

    /// Tier 1 路径：在 Windows 且启用 `VK_KHR_external_memory_win32` 时零拷贝导入 DXGI 共享句柄。
    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    fn try_upload_via_win32_zero_copy(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        slot: u32,
        current_frame: u64,
        frames_in_flight: u32,
        frame: &VideoFramePayload,
        handle: &VideoExternalHandleDesc,
    ) -> Result<(), GpuError> {
        if !gpu.features.external_memory_win32 {
            return Err(GpuError::InvalidArgument(
                "VK_KHR_external_memory_win32 is not enabled on this GPU".into(),
            ));
        }
        let imported = import_win32_external_video_image(
            gpu,
            frame.width,
            frame.height,
            frame.format,
            handle,
        )?;
        let Some(surface) = self.surfaces.get_mut(&slot) else {
            // SAFETY: 刚创建的导入图像未被提交，可立即释放。
            unsafe { imported.destroy(gpu) };
            return Err(GpuError::InvalidArgument(format!(
                "video surface slot {slot} not found"
            )));
        };
        let submit_result = self.ring.submit_imported_planes_convert(
            gpu,
            pipelines,
            frame,
            &imported,
            &mut surface.target_rgba,
        );
        if submit_result.is_ok() {
            // 零拷贝导入图像在当前帧渲染前提交至计算管线，须多保留一帧待本帧围栏退役。
            self.deferred_destroys.defer(
                current_frame,
                frames_in_flight.saturating_add(1),
                VideoDeferredResource::Imported(imported),
            );
            Ok(())
        } else {
            // SAFETY: 提交失败，导入图像未被 GPU 引用，可立即释放。
            unsafe { imported.destroy(gpu) };
            submit_result
        }
    }

    /// 非 Windows 平台不支持 Win32 NT 共享句柄导入，返回结构化错误以触发上层 Tier 2 回退。
    #[cfg(not(windows))]
    #[allow(clippy::too_many_arguments)]
    fn try_upload_via_win32_zero_copy(
        &mut self,
        _gpu: &Gpu,
        _pipelines: &Pipelines,
        _slot: u32,
        _current_frame: u64,
        _frames_in_flight: u32,
        _frame: &VideoFramePayload,
        _handle: &VideoExternalHandleDesc,
    ) -> Result<(), GpuError> {
        Err(GpuError::InvalidArgument(
            "Win32 external memory zero-copy import is only supported on Windows".into(),
        ))
    }

    /// 解绑并延迟销毁指定视频表面槽位。
    pub(crate) fn destroy_video_surface(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        external_slots: &mut ExternalTextureSlots,
        current_frame: u64,
        frames_in_flight: u32,
        slot: u32,
    ) -> Result<(), GpuError> {
        if !ExternalTextureSlots::owns(slot) {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not an external texture slot"
            )));
        }
        let Some(surface) = self.surfaces.remove(&slot) else {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not an active video surface"
            )));
        };
        if let Err(err) = pipelines.unbind_external_texture(gpu, slot) {
            self.surfaces.insert(slot, surface);
            return Err(err);
        }
        external_slots.release(slot);
        self.deferred_destroys.defer(
            current_frame,
            frames_in_flight,
            VideoDeferredResource::Surface(surface),
        );
        Ok(())
    }

    /// 在渲染帧围栏退役后回收所有已安全退役的视频资源。
    pub(crate) fn reclaim_deferred(&mut self, gpu: &Gpu, completed_generation: u64) {
        self.deferred_destroys
            .drain_ready_with(completed_generation, |res| {
                // SAFETY: 对应在飞帧均已退役，GPU 不再引用该视频资源。
                unsafe { res.destroy(gpu) };
            });
    }

    /// 销毁全部活跃与延迟销毁的视频表面以及异步 Staging 环。
    ///
    /// # Safety
    /// 调用方须确保设备处于空闲状态。
    pub(crate) unsafe fn destroy(&mut self, gpu: &Gpu) {
        for (_, surface) in self.surfaces.drain() {
            surface.destroy(gpu);
        }
        self.deferred_destroys.drain_all_with(|res| {
            res.destroy(gpu);
        });
        self.ring.destroy(gpu);
    }
}

/// 返回 Tier 2 各源平面对应的 Vulkan 格式。
fn source_plane_vk_formats(format: VideoPixelFormat) -> &'static [vk::Format] {
    match format {
        VideoPixelFormat::Nv12 => &[vk::Format::R8_UNORM, vk::Format::R8G8_UNORM],
        VideoPixelFormat::P010 => &[vk::Format::R16_UNORM, vk::Format::R16G16_UNORM],
        VideoPixelFormat::I420 => &[
            vk::Format::R8_UNORM,
            vk::Format::R8_UNORM,
            vk::Format::R8_UNORM,
        ],
    }
}

/// 按需创建 Tier 2 源多平面 `VkImage`。
fn ensure_source_plane_images(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: VideoPixelFormat,
    planes: &mut Vec<VideoImage>,
) -> Result<(), GpuError> {
    let formats = source_plane_vk_formats(format);
    if planes.len() == formats.len() {
        let dims_match = planes.iter().enumerate().all(|(i, img)| {
            format.expected_plane_dims(width, height, i) == Some((img.width, img.height))
                && img.format == formats[i]
        });
        if dims_match {
            return Ok(());
        }
    }
    let mut created = Vec::with_capacity(formats.len());
    for (i, &vk_fmt) in formats.iter().enumerate() {
        let (pw, ph) = format
            .expected_plane_dims(width, height, i)
            .ok_or_else(|| GpuError::InvalidArgument(format!("invalid plane index {i}")))?;
        match VideoImage::new(
            gpu,
            pw,
            ph,
            vk_fmt,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        ) {
            Ok(img) => created.push(img),
            Err(err) => {
                for img in created {
                    // SAFETY: 新建中途失败，图像尚未被使用，立即释放。
                    unsafe { img.destroy(gpu) };
                }
                return Err(err);
            }
        }
    }
    for old in planes.drain(..) {
        // SAFETY: 仅在首次填充或同槽尺寸重建路径调用，且旧表面已由延迟销毁队列保护。
        unsafe { old.destroy(gpu) };
    }
    *planes = created;
    Ok(())
}

/// Windows 平台 Tier 1 零拷贝导入 D3D11/D3D12 DXGI 外部显存句柄为只读多平面 `VkImage`。
#[cfg(windows)]
fn import_win32_external_video_image(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: VideoPixelFormat,
    handle: &VideoExternalHandleDesc,
) -> Result<ImportedExternalVideoImage, GpuError> {
    let desc = build_external_image_desc(width, height, format, handle);
    let (vk_format, handle_type) = ExternalMemoryImporter::prepare_import(&desc)
        .map_err(|msg| GpuError::InvalidArgument(msg.to_string()))?;

    // SAFETY: 已校验设备启用 `VK_KHR_external_memory_win32` 且描述符合法；失败时完整回滚已创建句柄。
    unsafe {
        let device = &gpu.device;
        let mut ext_image_info =
            vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
        let image = device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk_format)
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::SAMPLED)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .push_next(&mut ext_image_info),
                None,
            )
            .map_err(GpuError::from)?;

        let requirements = device.get_image_memory_requirements(image);
        let mut allowed_type_bits = requirements.memory_type_bits;
        let is_opaque = matches!(
            handle_type,
            vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
                | vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32_KMT
        );
        if !is_opaque {
            let ext_win32 = ash::khr::external_memory_win32::Device::new(&gpu.instance, device);
            let mut handle_props = vk::MemoryWin32HandlePropertiesKHR::default();
            if let Err(err) = ext_win32.get_memory_win32_handle_properties(
                handle_type,
                handle.raw_handle as vk::HANDLE,
                &mut handle_props,
            ) {
                device.destroy_image(image, None);
                return Err(GpuError::from(err));
            }
            allowed_type_bits &= handle_props.memory_type_bits;
        }
        let Some(type_index) =
            gpu.memory_type_index_for_bits(allowed_type_bits, MemoryKind::DeviceLocal)
        else {
            device.destroy_image(image, None);
            return Err(GpuError::Allocation(
                "no compatible device-local memory type for imported Win32 video image".into(),
            ));
        };

        let mut import_win32 = vk::ImportMemoryWin32HandleInfoKHR::default()
            .handle_type(handle_type)
            .handle(handle.raw_handle as vk::HANDLE);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let alloc_size = requirements.size.max(desc.allocation_size);
        let mut alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(alloc_size)
            .memory_type_index(type_index)
            .push_next(&mut import_win32);
        if desc.handle_kind.requires_dedicated_allocation() {
            alloc_info = alloc_info.push_next(&mut dedicated);
        }

        let memory = match device.allocate_memory(&alloc_info, None) {
            Ok(mem) => mem,
            Err(err) => {
                device.destroy_image(image, None);
                return Err(GpuError::from(err));
            }
        };
        if let Err(err) = device.bind_image_memory(image, memory, 0) {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
            return Err(GpuError::from(err));
        }

        let plane_formats = source_plane_vk_formats(format);
        let plane_aspects = [
            vk::ImageAspectFlags::PLANE_0,
            vk::ImageAspectFlags::PLANE_1,
            vk::ImageAspectFlags::PLANE_2,
        ];
        let mut plane_views = Vec::with_capacity(plane_formats.len());
        for (i, &plane_fmt) in plane_formats.iter().enumerate() {
            let view = match device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(plane_fmt)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(plane_aspects[i])
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            ) {
                Ok(v) => v,
                Err(err) => {
                    for created in plane_views {
                        device.destroy_image_view(created, None);
                    }
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                    return Err(GpuError::from(err));
                }
            };
            plane_views.push(view);
        }

        Ok(ImportedExternalVideoImage {
            image,
            memory,
            plane_views,
        })
    }
}

/// 构造视频图像同步屏障。
#[allow(clippy::too_many_arguments)]
fn video_image_barrier(
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags2,
    src_access: vk::AccessFlags2,
    dst_stage: vk::PipelineStageFlags2,
    dst_access: vk::AccessFlags2,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage)
        .src_access_mask(src_access)
        .dst_stage_mask(dst_stage)
        .dst_access_mask(dst_access)
        .old_layout(old_layout)
        .new_layout(new_layout)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
        )
}

#[inline]
const fn align_up_u64(value: u64, align: u64) -> u64 {
    let a = if align == 0 { 1 } else { align };
    let rem = value % a;
    if rem == 0 {
        value
    } else {
        value + (a - rem)
    }
}

// ---------------------------------------------------------------------------
// 纯 CPU 参考色彩转换实现（数学公式与 `shaders/video_convert.comp` 严格一致）
// ---------------------------------------------------------------------------

#[inline]
fn pq_eotf_ch(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 32.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 128.0;
    const C3: f32 = 2392.0 / 128.0;

    let p = e.max(0.0).powf(1.0 / M2);
    let num = (p - C1).max(0.0);
    let den = (C2 - C3 * p).max(1e-6);
    (num / den).powf(1.0 / M1)
}

#[inline]
fn hlg_oetf_inv_ch(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    if e <= 0.5 {
        (e * e) / 3.0
    } else {
        (((e - C) / A).exp() + B) / 12.0
    }
}

#[inline]
fn bt2020_to_bt709_cpu(rgb2020: [f32; 3]) -> [f32; 3] {
    [
        (1.660_491 * rgb2020[0] - 0.587_641_1 * rgb2020[1] - 0.072_849_9 * rgb2020[2]).max(0.0),
        (-0.124_550_5 * rgb2020[0] + 1.132_899_9 * rgb2020[1] - 0.008_349_4 * rgb2020[2]).max(0.0),
        (-0.018_150_8 * rgb2020[0] - 0.100_578_9 * rgb2020[1] + 1.118_729_7 * rgb2020[2]).max(0.0),
    ]
}

#[inline]
fn reinhard_extended_tonemap_cpu(linear_rgb: [f32; 3], white_point: f32) -> [f32; 3] {
    let luma = 0.2126 * linear_rgb[0] + 0.7152 * linear_rgb[1] + 0.0722 * linear_rgb[2];
    if luma <= 1e-6 {
        return [0.0, 0.0, 0.0];
    }
    let wp = white_point.max(1.0);
    let w2 = wp * wp;
    let mapped_luma = (luma * (1.0 + luma / w2)) / (1.0 + luma);
    let scale = mapped_luma / luma;
    [
        (linear_rgb[0] * scale).clamp(0.0, 1.0),
        (linear_rgb[1] * scale).clamp(0.0, 1.0),
        (linear_rgb[2] * scale).clamp(0.0, 1.0),
    ]
}

#[inline]
fn linear_to_srgb_ch_cpu(c: f32) -> f32 {
    let x = c.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// 纯 CPU 单像素 YUV/HDR 至 sRGB 编码 RGBA8 转换（与 `video_convert.comp` 数学完全一致）。
///
/// `y_norm`、`u_norm`、`v_norm` 为归一化采样值（范围 `[0.0, 1.0]`），
/// 其中 `P010` 按 `R16_UNORM` 高位对齐 10-bit 约定解包。
#[must_use]
pub fn convert_yuv_pixel_cpu(
    format: VideoPixelFormat,
    color_space: VideoColorSpace,
    y_norm: f32,
    u_norm: f32,
    v_norm: f32,
) -> [u8; 4] {
    convert_yuv_pixel_cpu_with_peak(
        format,
        color_space,
        y_norm,
        u_norm,
        v_norm,
        DEFAULT_HDR_PEAK_NITS,
    )
}

/// 支持自定义 HDR 峰值亮度的纯 CPU 单像素转换参考实现。
#[must_use]
pub fn convert_yuv_pixel_cpu_with_peak(
    format: VideoPixelFormat,
    color_space: VideoColorSpace,
    y_norm: f32,
    u_norm: f32,
    v_norm: f32,
    hdr_peak_nits: f32,
) -> [u8; 4] {
    let (y, u, v) = match format {
        VideoPixelFormat::P010 => {
            let y10 = (y_norm * 65535.0).round() / 64.0;
            let u10 = (u_norm * 65535.0).round() / 64.0;
            let v10 = (v_norm * 65535.0).round() / 64.0;
            match color_space.range {
                VideoColorRange::Limited => (
                    ((y10 - 64.0) / 876.0).clamp(0.0, 1.0),
                    ((u10 - 512.0) / 896.0).clamp(-0.5, 0.5),
                    ((v10 - 512.0) / 896.0).clamp(-0.5, 0.5),
                ),
                VideoColorRange::Full => (
                    (y10 / 1023.0).clamp(0.0, 1.0),
                    ((u10 - 512.0) / 1023.0).clamp(-0.5, 0.5),
                    ((v10 - 512.0) / 1023.0).clamp(-0.5, 0.5),
                ),
            }
        }
        VideoPixelFormat::Nv12 | VideoPixelFormat::I420 => match color_space.range {
            VideoColorRange::Limited => (
                ((y_norm - (16.0 / 255.0)) * (255.0 / 219.0)).clamp(0.0, 1.0),
                ((u_norm - (128.0 / 255.0)) * (255.0 / 224.0)).clamp(-0.5, 0.5),
                ((v_norm - (128.0 / 255.0)) * (255.0 / 224.0)).clamp(-0.5, 0.5),
            ),
            VideoColorRange::Full => (
                y_norm.clamp(0.0, 1.0),
                (u_norm - (128.0 / 255.0)).clamp(-0.5, 0.5),
                (v_norm - (128.0 / 255.0)).clamp(-0.5, 0.5),
            ),
        },
    };

    let mut rgb = match color_space.matrix {
        VideoColorMatrix::Bt601 => [
            y + 1.4020 * v,
            y - 0.344_136_3 * u - 0.714_136_3 * v,
            y + 1.7720 * u,
        ],
        VideoColorMatrix::Bt709 => [
            y + 1.5748 * v,
            y - 0.187_324_27 * u - 0.468_124_27 * v,
            y + 1.8556 * u,
        ],
        VideoColorMatrix::Bt2020 => [
            y + 1.4746 * v,
            y - 0.164_553_13 * u - 0.571_353_13 * v,
            y + 1.8814 * u,
        ],
    };
    for c in &mut rgb {
        *c = c.clamp(0.0, 1.0);
    }

    if matches!(
        color_space.transfer,
        VideoTransferFunction::Pq | VideoTransferFunction::Hlg
    ) {
        let peak_nits = if hdr_peak_nits > 0.0 {
            hdr_peak_nits.max(SDR_REFERENCE_WHITE_NITS)
        } else {
            DEFAULT_HDR_PEAK_NITS
        };
        let white_point = peak_nits / SDR_REFERENCE_WHITE_NITS;
        let linear_rgb = match color_space.transfer {
            VideoTransferFunction::Pq => {
                let scale = 10000.0 / SDR_REFERENCE_WHITE_NITS;
                [
                    pq_eotf_ch(rgb[0]) * scale,
                    pq_eotf_ch(rgb[1]) * scale,
                    pq_eotf_ch(rgb[2]) * scale,
                ]
            }
            VideoTransferFunction::Hlg => {
                let scene = [
                    hlg_oetf_inv_ch(rgb[0]),
                    hlg_oetf_inv_ch(rgb[1]),
                    hlg_oetf_inv_ch(rgb[2]),
                ];
                let ys = 0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2];
                let ootf_scale = ys.max(1e-6).powf(0.2) * (peak_nits / SDR_REFERENCE_WHITE_NITS);
                [
                    scene[0] * ootf_scale,
                    scene[1] * ootf_scale,
                    scene[2] * ootf_scale,
                ]
            }
            VideoTransferFunction::Sdr => rgb,
        };
        let rgb709 = bt2020_to_bt709_cpu(linear_rgb);
        let sdr_linear = reinhard_extended_tonemap_cpu(rgb709, white_point);
        rgb = [
            linear_to_srgb_ch_cpu(sdr_linear[0]),
            linear_to_srgb_ch_cpu(sdr_linear[1]),
            linear_to_srgb_ch_cpu(sdr_linear[2]),
        ];
    }

    [
        (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

/// 将完整 [`VideoFramePayload`] 在 CPU 上转换为紧凑 sRGB 编码 RGBA8 字节数组。
///
/// # Errors
/// 当帧载荷校验失败，或外部句柄载荷不含 `fallback_cpu` 平面数据时返回 [`GpuError::InvalidArgument`]。
pub fn convert_video_frame_cpu(frame: &VideoFramePayload) -> Result<Vec<u8>, GpuError> {
    frame
        .validate()
        .map_err(|e| GpuError::InvalidArgument(format!("invalid video frame: {e}")))?;

    let (data, planes): (&[u8], &[VideoFramePlane]) = match &frame.storage {
        VideoFrameStorage::CpuPlanes { data, planes } => (data.as_ref(), planes.as_slice()),
        VideoFrameStorage::ExternalHandle { fallback_cpu, .. } => match fallback_cpu {
            Some((data, planes)) => (data.as_ref(), planes.as_slice()),
            None => {
                return Err(GpuError::InvalidArgument(
                    "external handle frame has no CPU fallback planes for CPU reference conversion"
                        .into(),
                ));
            }
        },
    };

    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut out = vec![0u8; width.saturating_mul(height).saturating_mul(4)];

    for y in 0..height {
        let cy = y / 2;
        for x in 0..width {
            let cx = x / 2;
            let (y_norm, u_norm, v_norm) = match frame.format {
                VideoPixelFormat::Nv12 => {
                    let y_off = planes[0].offset_bytes + y * planes[0].stride_bytes + x;
                    let uv_off = planes[1].offset_bytes + cy * planes[1].stride_bytes + cx * 2;
                    (
                        f32::from(data[y_off]) / 255.0,
                        f32::from(data[uv_off]) / 255.0,
                        f32::from(data[uv_off + 1]) / 255.0,
                    )
                }
                VideoPixelFormat::P010 => {
                    let y_off = planes[0].offset_bytes + y * planes[0].stride_bytes + x * 2;
                    let uv_off = planes[1].offset_bytes + cy * planes[1].stride_bytes + cx * 4;
                    let y_val = u16::from_le_bytes([data[y_off], data[y_off + 1]]);
                    let u_val = u16::from_le_bytes([data[uv_off], data[uv_off + 1]]);
                    let v_val = u16::from_le_bytes([data[uv_off + 2], data[uv_off + 3]]);
                    (
                        f32::from(y_val) / 65535.0,
                        f32::from(u_val) / 65535.0,
                        f32::from(v_val) / 65535.0,
                    )
                }
                VideoPixelFormat::I420 => {
                    let y_off = planes[0].offset_bytes + y * planes[0].stride_bytes + x;
                    let u_off = planes[1].offset_bytes + cy * planes[1].stride_bytes + cx;
                    let v_off = planes[2].offset_bytes + cy * planes[2].stride_bytes + cx;
                    (
                        f32::from(data[y_off]) / 255.0,
                        f32::from(data[u_off]) / 255.0,
                        f32::from(data[v_off]) / 255.0,
                    )
                }
            };

            let rgba =
                convert_yuv_pixel_cpu(frame.format, frame.color_space, y_norm, u_norm, v_norm);
            let dst = (y * width + x) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 将非线性 RGB 编码为指定矩阵与量化范围的归一化 `(y_norm, u_norm, v_norm)`（用于测试真值合成）。
    fn encode_rgb_to_yuv_norm(
        format: VideoPixelFormat,
        color_space: VideoColorSpace,
        r: f32,
        g: f32,
        b: f32,
    ) -> (f32, f32, f32) {
        let (kr, kb) = match color_space.matrix {
            VideoColorMatrix::Bt601 => (0.299f32, 0.114f32),
            VideoColorMatrix::Bt709 => (0.2126f32, 0.0722f32),
            VideoColorMatrix::Bt2020 => (0.2627f32, 0.0593f32),
        };
        let kg = 1.0 - kr - kb;
        let y = kr * r + kg * g + kb * b;
        let cb = (b - y) / (2.0 * (1.0 - kb));
        let cr = (r - y) / (2.0 * (1.0 - kr));

        match format {
            VideoPixelFormat::P010 => {
                let (y10, u10, v10) = match color_space.range {
                    VideoColorRange::Limited => (
                        (64.0 + y * 876.0).round().clamp(0.0, 1023.0) as u16,
                        (512.0 + cb * 896.0).round().clamp(0.0, 1023.0) as u16,
                        (512.0 + cr * 896.0).round().clamp(0.0, 1023.0) as u16,
                    ),
                    VideoColorRange::Full => (
                        (y * 1023.0).round().clamp(0.0, 1023.0) as u16,
                        (512.0 + cb * 1023.0).round().clamp(0.0, 1023.0) as u16,
                        (512.0 + cr * 1023.0).round().clamp(0.0, 1023.0) as u16,
                    ),
                };
                // P010 将 10-bit 样本高位对齐存放在 16-bit UNORM 中。
                (
                    f32::from(y10 << 6) / 65535.0,
                    f32::from(u10 << 6) / 65535.0,
                    f32::from(v10 << 6) / 65535.0,
                )
            }
            VideoPixelFormat::Nv12 | VideoPixelFormat::I420 => {
                let (y8, u8v, v8v) = match color_space.range {
                    VideoColorRange::Limited => (
                        (16.0 + y * 219.0).round().clamp(0.0, 255.0) as u8,
                        (128.0 + cb * 224.0).round().clamp(0.0, 255.0) as u8,
                        (128.0 + cr * 224.0).round().clamp(0.0, 255.0) as u8,
                    ),
                    VideoColorRange::Full => (
                        (y * 255.0).round().clamp(0.0, 255.0) as u8,
                        (128.0 + cb * 255.0).round().clamp(0.0, 255.0) as u8,
                        (128.0 + cr * 255.0).round().clamp(0.0, 255.0) as u8,
                    ),
                };
                (
                    f32::from(y8) / 255.0,
                    f32::from(u8v) / 255.0,
                    f32::from(v8v) / 255.0,
                )
            }
        }
    }

    /// SMPTE ST 2084（PQ）逆 EOTF，用于构造指定绝对亮度的 HDR 测试样本。
    fn pq_oetf(linear_nits: f32) -> f32 {
        const M1: f32 = 2610.0 / 16384.0;
        const M2: f32 = 2523.0 / 32.0;
        const C1: f32 = 3424.0 / 4096.0;
        const C2: f32 = 2413.0 / 128.0;
        const C3: f32 = 2392.0 / 128.0;
        let y = (linear_nits / 10000.0).clamp(0.0, 1.0);
        let ym1 = y.powf(M1);
        ((C1 + C2 * ym1) / (1.0 + C3 * ym1)).powf(M2)
    }

    #[test]
    fn video_convert_push_layout_is_32_bytes() {
        assert_eq!(core::mem::size_of::<VideoConvertPush>(), 32);
        assert_eq!(VideoConvertPush::SIZE, 32);

        let data = vec![128u8; 4 * 4 + 2 * 4];
        let payload = VideoFramePayload::from_nv12(4, 4, VideoColorSpace::bt2020_pq(), 1234, data)
            .expect("valid nv12");
        let push = VideoConvertPush::from_payload(&payload, 1000.0);
        assert_eq!(push.width, 4);
        assert_eq!(push.height, 4);
        assert_eq!(push.format, 0);
        assert_eq!(push.color_matrix, 2);
        assert_eq!(push.color_range, 0);
        assert_eq!(push.transfer_fn, 1);
        assert!((push.hdr_peak_nits - 1000.0).abs() < 1e-4);
    }

    #[test]
    fn sdr_matrix_and_range_combinations_match_gray_and_primary_bars() {
        let formats = [
            VideoPixelFormat::Nv12,
            VideoPixelFormat::P010,
            VideoPixelFormat::I420,
        ];
        let matrices = [
            VideoColorMatrix::Bt601,
            VideoColorMatrix::Bt709,
            VideoColorMatrix::Bt2020,
        ];
        let ranges = [VideoColorRange::Limited, VideoColorRange::Full];

        // 覆盖纯黑、中灰、纯白与纯红/绿/蓝彩条。
        let samples = [
            ([0.0f32, 0.0, 0.0], [0u8, 0, 0]),
            ([0.5, 0.5, 0.5], [128, 128, 128]),
            ([1.0, 1.0, 1.0], [255, 255, 255]),
            ([1.0, 0.0, 0.0], [255, 0, 0]),
            ([0.0, 1.0, 0.0], [0, 255, 0]),
            ([0.0, 0.0, 1.0], [0, 0, 255]),
        ];

        for format in formats {
            for matrix in matrices {
                for range in ranges {
                    let cs = VideoColorSpace::new(matrix, range, VideoTransferFunction::Sdr);
                    for (rgb_in, expected) in samples {
                        let (yn, un, vn) =
                            encode_rgb_to_yuv_norm(format, cs, rgb_in[0], rgb_in[1], rgb_in[2]);
                        let out = convert_yuv_pixel_cpu(format, cs, yn, un, vn);
                        assert_eq!(out[3], 255);
                        // 8-bit 量化误差不超过 3 LSB；中灰无二次 sRGB 伽马偏移（若错误施加二次编码会偏至 ~188）。
                        for ch in 0..3 {
                            let diff = out[ch].abs_diff(expected[ch]);
                            assert!(
                                diff <= 3,
                                "mismatch for {format:?}/{matrix:?}/{range:?} in={rgb_in:?}: got {out:?}, expected {expected:?}"
                            );
                        }
                        if expected[0] == expected[1] && expected[1] == expected[2] {
                            assert_eq!(out[0], out[1], "gray must stay neutral for {format:?}");
                            assert_eq!(out[1], out[2], "gray must stay neutral for {format:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hdr_pq_and_hlg_tonemap_and_srgb_output_stay_bounded_and_neutral() {
        for transfer in [VideoTransferFunction::Pq, VideoTransferFunction::Hlg] {
            for range in [VideoColorRange::Limited, VideoColorRange::Full] {
                let cs = VideoColorSpace::new(VideoColorMatrix::Bt2020, range, transfer);

                // 纯黑映射为 0。
                let (yb, ub, vb) =
                    encode_rgb_to_yuv_norm(VideoPixelFormat::P010, cs, 0.0, 0.0, 0.0);
                let black = convert_yuv_pixel_cpu(VideoPixelFormat::P010, cs, yb, ub, vb);
                assert_eq!(black, [0, 0, 0, 255]);

                // 1000 nits 峰值白经 Reinhard 扩展平滑压缩后应逼近 255 且保持灰阶中性。
                let peak_sig = match transfer {
                    VideoTransferFunction::Pq => pq_oetf(1000.0),
                    VideoTransferFunction::Hlg => 1.0,
                    VideoTransferFunction::Sdr => 1.0,
                };
                let (yp, up, vp) = encode_rgb_to_yuv_norm(
                    VideoPixelFormat::P010,
                    cs,
                    peak_sig,
                    peak_sig,
                    peak_sig,
                );
                let peak_out = convert_yuv_pixel_cpu(VideoPixelFormat::P010, cs, yp, up, vp);
                assert!(
                    peak_out[0] >= 245,
                    "HDR peak should map near white for {transfer:?}/{range:?}, got {peak_out:?}"
                );
                assert!(peak_out[0].abs_diff(peak_out[1]) <= 1);
                assert!(peak_out[1].abs_diff(peak_out[2]) <= 1);

                // 中等亮度（PQ 100 nits / HLG 0.5 信号）应单调介于纯黑与峰值白之间，且非二次伽马压暗。
                let mid_sig = match transfer {
                    VideoTransferFunction::Pq => pq_oetf(100.0),
                    VideoTransferFunction::Hlg => 0.5,
                    VideoTransferFunction::Sdr => 0.5,
                };
                let (ym, um, vm) =
                    encode_rgb_to_yuv_norm(VideoPixelFormat::P010, cs, mid_sig, mid_sig, mid_sig);
                let mid_out = convert_yuv_pixel_cpu(VideoPixelFormat::P010, cs, ym, um, vm);
                assert!(
                    mid_out[0] > 80 && mid_out[0] < peak_out[0],
                    "HDR mid-tone out of expected sRGB range for {transfer:?}/{range:?}: {mid_out:?}"
                );
                assert!(mid_out[0].abs_diff(mid_out[1]) <= 1);
                assert!(mid_out[1].abs_diff(mid_out[2]) <= 1);

                // BT.2020 纯红彩条经色域映射与压缩后仍以红通道为主导。
                let (yr, ur, vr) =
                    encode_rgb_to_yuv_norm(VideoPixelFormat::P010, cs, peak_sig, 0.0, 0.0);
                let red_out = convert_yuv_pixel_cpu(VideoPixelFormat::P010, cs, yr, ur, vr);
                assert!(red_out[0] > 200 && red_out[1] < 40 && red_out[2] < 40);
            }
        }
    }

    #[test]
    fn convert_video_frame_cpu_supports_nv12_p010_and_i420_payloads() {
        let cs = VideoColorSpace::bt709_limited();
        // NV12 2×2 纯白（Limited Y=235, Cb=128, Cr=128）。
        let nv12 = VideoFramePayload::from_nv12(2, 2, cs, 0, vec![235, 235, 235, 235, 128, 128])
            .expect("valid nv12");
        let rgba_nv12 = convert_video_frame_cpu(&nv12).expect("convert nv12");
        assert_eq!(rgba_nv12.len(), 16);
        for px in rgba_nv12.chunks_exact(4) {
            assert_eq!(px, [255, 255, 255, 255]);
        }

        // P010 2×2 纯白（Limited Y=940<<6, Cb=512<<6, Cr=512<<6）。
        let y16 = (940u16 << 6).to_le_bytes();
        let c16 = (512u16 << 6).to_le_bytes();
        let mut p010_bytes = Vec::new();
        for _ in 0..4 {
            p010_bytes.extend_from_slice(&y16);
        }
        p010_bytes.extend_from_slice(&c16);
        p010_bytes.extend_from_slice(&c16);
        let p010 = VideoFramePayload::from_p010(2, 2, cs, 16_667, p010_bytes).expect("valid p010");
        let rgba_p010 = convert_video_frame_cpu(&p010).expect("convert p010");
        for px in rgba_p010.chunks_exact(4) {
            assert_eq!(px, [255, 255, 255, 255]);
        }

        // I420 2×2 纯黑（Limited Y=16, U=128, V=128）。
        let i420 = VideoFramePayload::from_i420(2, 2, cs, 33_333, vec![16, 16, 16, 16, 128, 128])
            .expect("valid i420");
        let rgba_i420 = convert_video_frame_cpu(&i420).expect("convert i420");
        for px in rgba_i420.chunks_exact(4) {
            assert_eq!(px, [0, 0, 0, 255]);
        }
    }

    #[test]
    fn video_upload_capabilities_and_format_mask_work_across_tiers() {
        let tier1 = VideoUploadCapabilities::from_device_extensions(true, 3);
        assert_eq!(tier1.active_tier, VideoUploadTier::ExternalMemoryZeroCopy);
        assert!(tier1.external_memory_win32);
        assert_eq!(tier1.async_staging_ring_slots, 3);
        assert_eq!(
            tier1.max_bindless_video_slots,
            EXTENDED_EXTERNAL_SLOTS as usize
        );
        assert!(tier1.supported_formats.supports(VideoPixelFormat::Nv12));
        assert!(tier1.supported_formats.supports(VideoPixelFormat::P010));
        assert!(tier1.supported_formats.supports(VideoPixelFormat::I420));

        let tier2 = VideoUploadCapabilities::from_device_extensions(false, 2);
        assert_eq!(tier2.active_tier, VideoUploadTier::AsyncStaging);
        assert!(!tier2.external_memory_win32);
        assert_eq!(tier2.async_staging_ring_slots, 2);

        let stub = VideoUploadCapabilities::headless_stub();
        assert_eq!(stub.active_tier, VideoUploadTier::HeadlessStub);
        assert!(!stub.external_memory_win32);
        assert_eq!(stub.async_staging_ring_slots, 0);

        let mask = VideoFormatMask::NV12.union(VideoFormatMask::P010);
        assert!(mask.supports(VideoPixelFormat::Nv12));
        assert!(mask.supports(VideoPixelFormat::P010));
        assert!(!mask.supports(VideoPixelFormat::I420));
        assert_eq!(VideoFormatMask::from_bits(u32::MAX), VideoFormatMask::ALL);
    }

    #[test]
    fn external_handle_validation_and_tier1_to_tier2_fallback() {
        let fallback_data: Arc<[u8]> = Arc::from(vec![235, 235, 235, 235, 128, 128]);
        let fallback_planes = vec![
            VideoFramePlane {
                offset_bytes: 0,
                stride_bytes: 2,
                width: 2,
                height: 2,
            },
            VideoFramePlane {
                offset_bytes: 4,
                stride_bytes: 2,
                width: 1,
                height: 1,
            },
        ];

        // 1. 驱动不支持 Win32 外部显存扩展，但带有 fallback_cpu：无缝降级至 Tier 2 AsyncStaging。
        let frame_with_fallback = VideoFramePayload {
            width: 2,
            height: 2,
            format: VideoPixelFormat::Nv12,
            color_space: VideoColorSpace::bt709_limited(),
            pts_us: 0,
            storage: VideoFrameStorage::ExternalHandle {
                handle: VideoExternalHandleDesc {
                    kind: VideoExternalHandleKind::D3D11Texture,
                    raw_handle: 0x1234,
                    allocation_size: 4096,
                    subresource_index: 0,
                    fence_value: 1,
                },
                fallback_cpu: Some((fallback_data.clone(), fallback_planes.clone())),
            },
        };
        let tier2_caps = VideoUploadCapabilities::from_device_extensions(false, 2);
        assert_eq!(
            resolve_upload_tier(tier2_caps, &frame_with_fallback).expect("fallback to tier 2"),
            VideoUploadTier::AsyncStaging
        );

        // 2. 句柄无效且带有 fallback_cpu：即使声明支持扩展，也应回退至 Tier 2 而非报错。
        let invalid_handle_with_fallback = VideoFramePayload {
            width: 2,
            height: 2,
            format: VideoPixelFormat::Nv12,
            color_space: VideoColorSpace::bt709_limited(),
            pts_us: 0,
            storage: VideoFrameStorage::ExternalHandle {
                handle: VideoExternalHandleDesc {
                    kind: VideoExternalHandleKind::D3D11Texture,
                    raw_handle: 0,
                    allocation_size: 4096,
                    subresource_index: 0,
                    fence_value: 0,
                },
                fallback_cpu: Some((fallback_data, fallback_planes)),
            },
        };
        let tier1_caps = VideoUploadCapabilities::from_device_extensions(true, 2);
        assert_eq!(
            resolve_upload_tier(tier1_caps, &invalid_handle_with_fallback)
                .expect("invalid handle falls back to tier 2"),
            VideoUploadTier::AsyncStaging
        );

        // 3. 句柄无效且无 fallback_cpu：返回结构化 GpuError::InvalidArgument。
        let invalid_handle_no_fallback = VideoFramePayload {
            width: 2,
            height: 2,
            format: VideoPixelFormat::Nv12,
            color_space: VideoColorSpace::bt709_limited(),
            pts_us: 0,
            storage: VideoFrameStorage::ExternalHandle {
                handle: VideoExternalHandleDesc {
                    kind: VideoExternalHandleKind::D3D11Texture,
                    raw_handle: -1,
                    allocation_size: 4096,
                    subresource_index: 0,
                    fence_value: 0,
                },
                fallback_cpu: None,
            },
        };
        assert!(matches!(
            resolve_upload_tier(tier1_caps, &invalid_handle_no_fallback),
            Err(GpuError::InvalidArgument(_))
        ));
    }

    #[test]
    fn async_staging_ring_scheduler_advances_and_gates_on_timeline() {
        let mut ring = StagingRingScheduler::new(2);
        assert_eq!(ring.slot_count(), 2);

        // 前两次获取槽位 0 与槽位 1，均无需等待。
        assert_eq!(ring.acquire(0), (0, None));
        ring.mark_submitted(0, 10);
        assert_eq!(ring.acquire(0), (1, None));
        ring.mark_submitted(1, 20);

        // 回绕至槽位 0，此时 GPU 仅完成到 5，必须等待 10。
        assert_eq!(ring.acquire(5), (0, Some(10)));
        ring.mark_submitted(0, 30);

        // 下一次回绕至槽位 1，若 GPU 已完成到 25，则无需等待即可零阻塞复用。
        assert_eq!(ring.acquire(25), (1, None));
        assert_eq!(ring.slot_timeline(0), 30);
    }

    #[test]
    fn video_deferred_destroy_queue_retires_after_frames_in_flight() {
        let mut queue = VideoDeferredDestroyQueue::new();
        assert!(queue.is_empty());

        queue.defer(10, 2, "frame10-surface");
        queue.defer(11, 2, "frame11-imported");
        assert_eq!(queue.len(), 2);

        let mut reclaimed = Vec::new();
        queue.drain_ready_with(11, |item| reclaimed.push(item));
        assert!(reclaimed.is_empty());
        assert_eq!(queue.len(), 2);

        queue.drain_ready_with(12, |item| reclaimed.push(item));
        assert_eq!(reclaimed, vec!["frame10-surface"]);
        assert_eq!(queue.len(), 1);

        queue.drain_all_with(|item| reclaimed.push(item));
        assert_eq!(reclaimed, vec!["frame10-surface", "frame11-imported"]);
        assert!(queue.is_empty());
    }
}

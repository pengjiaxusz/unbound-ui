//! 动态多页 BC7 压缩纹理图集与持久化异步 Staging 上传环。
//!
//! 本模块实现 Issue #246（#268 / #270）的 GPU 侧压缩纹理基础设施：
//!
//! * 按需创建 `2048×2048` 的 `VK_FORMAT_BC7_UNORM_BLOCK` 2D 图集页与 `> 512px` 独立 BC7 图像；
//! * 新页创建时自动通过异步 Staging 环以 BC7 Mode 6 透明黑块（`0x40` + 15 字节 `0x00`）整页清零，
//!   严禁调用阻塞式 `end_single_time` 或 `vkQueueWaitIdle`；
//! * 按 `frames_in_flight` 槽位隔离持久化 `TRANSFER_SRC` Staging 缓冲区，在帧录制开头
//!   聚合为每张目标 `VkImage` 单次 `vkCmdCopyBufferToImage`；
//! * 冷页与独立 BC7 纹理销毁时立即解绑 Bindless 槽位恢复占位纹理，底层 Vulkan 对象与
//!   Bindless 槽位经 [`DeferredDestroyQueue`] 在 `frames_in_flight` 帧退役后安全回收。

use std::collections::HashMap;

use ash::vk;

use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use crate::frame::{align_up, DeferredDestroyQueue};
use crate::pipeline::{ExternalTextureSlots, Pipelines};

/// 单张 BC7 动态图集页的固定宽高（像素）。
pub const BC7_PAGE_SIZE: u32 = 2048;

/// BC7 块压缩的像素块边长（4×4 像素）。
pub const BC7_BLOCK_DIM: u32 = 4;

/// 单个 4×4 BC7 压缩块的字节数（128 bit = 16 B）。
pub const BC7_BLOCK_BYTES: usize = 16;

/// BC7 Mode 6 全透明黑块常量（16 字节）。
///
/// 第 0 字节 `0x40`（`0b0100_0000`）为 Mode 6 合法 7 位模式前缀，后续端点、P-bit 与
/// 纹素索引全为 0，硬件采样固定解码为 `(0, 0, 0, 0)`，确保未写入区域无花屏与零串色。
pub const BC7_MODE6_TRANSPARENT_BLACK: [u8; 16] =
    [0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// 整张 `2048×2048` BC7 图集页的字节数（512 × 512 × 16 B = 4 MiB）。
const BC7_PAGE_BYTES: usize = ((BC7_PAGE_SIZE / BC7_BLOCK_DIM) as usize)
    * ((BC7_PAGE_SIZE / BC7_BLOCK_DIM) as usize)
    * BC7_BLOCK_BYTES;

/// 每个在飞帧槽位持久化 Staging 缓冲区的默认初始容量（8 MiB）。
const DEFAULT_STAGING_SLOT_BYTES: u64 = 8 << 20;

/// 校验 BC7 子区域坐标、尺寸的 4×4 块对齐约束以及压缩字节长度。
///
/// 校验规则：
/// * `width > 0 && height > 0`
/// * `x % 4 == 0 && y % 4 == 0`
/// * `width % 4 == 0 && height % 4 == 0`
/// * `x + width <= img_w && y + height <= img_h`
/// * `bytes.len() == (width / 4) * (height / 4) * 16`
///
/// # Errors
/// 任一约束不满足或计算溢出时返回 [`GpuError::InvalidArgument`]。
pub fn validate_bc7_subregion(
    img_w: u32,
    img_h: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    bytes: &[u8],
) -> Result<(), GpuError> {
    if width == 0 || height == 0 {
        return Err(GpuError::InvalidArgument(format!(
            "BC7 subregion dimensions must be non-zero, got {width}x{height}"
        )));
    }
    if x % BC7_BLOCK_DIM != 0 || y % BC7_BLOCK_DIM != 0 {
        return Err(GpuError::InvalidArgument(format!(
            "BC7 subregion origin ({x}, {y}) must be aligned to {BC7_BLOCK_DIM}x{BC7_BLOCK_DIM} blocks"
        )));
    }
    if width % BC7_BLOCK_DIM != 0 || height % BC7_BLOCK_DIM != 0 {
        return Err(GpuError::InvalidArgument(format!(
            "BC7 subregion size {width}x{height} must be aligned to {BC7_BLOCK_DIM}x{BC7_BLOCK_DIM} blocks"
        )));
    }
    let end_x = x.checked_add(width).ok_or_else(|| {
        GpuError::InvalidArgument(format!(
            "BC7 subregion x + width overflows u32: {x} + {width}"
        ))
    })?;
    let end_y = y.checked_add(height).ok_or_else(|| {
        GpuError::InvalidArgument(format!(
            "BC7 subregion y + height overflows u32: {y} + {height}"
        ))
    })?;
    if end_x > img_w || end_y > img_h {
        return Err(GpuError::InvalidArgument(format!(
            "BC7 subregion ({x}, {y}, {width}x{height}) exceeds image bounds {img_w}x{img_h}"
        )));
    }
    let blocks_x = u64::from(width / BC7_BLOCK_DIM);
    let blocks_y = u64::from(height / BC7_BLOCK_DIM);
    let expected = blocks_x
        .checked_mul(blocks_y)
        .and_then(|blocks| blocks.checked_mul(BC7_BLOCK_BYTES as u64))
        .and_then(|total| usize::try_from(total).ok())
        .ok_or_else(|| {
            GpuError::InvalidArgument(format!(
                "BC7 byte length overflows usize for {width}x{height}"
            ))
        })?;
    if bytes.len() != expected {
        return Err(GpuError::InvalidArgument(format!(
            "BC7 subregion {width}x{height} expects {expected} bytes, got {}",
            bytes.len()
        )));
    }
    Ok(())
}

/// 单个托管 BC7 2D 纹理（动态图集页或独立大图）及其当前布局状态。
#[derive(Debug)]
struct Bc7Image {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    width: u32,
    height: u32,
    bindless_slot: u32,
    layout: vk::ImageLayout,
}

impl Bc7Image {
    /// 创建 `VK_FORMAT_BC7_UNORM_BLOCK` 2D 图像、设备本地显存与图像视图。
    fn create(gpu: &Gpu, width: u32, height: u32, bindless_slot: u32) -> Result<Self, GpuError> {
        // SAFETY: 顺序创建图像、显存与视图；任意步骤失败均会完整回滚已创建对象。
        unsafe {
            let device = &gpu.device;
            let image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(vk::Format::BC7_UNORM_BLOCK)
                        .extent(vk::Extent3D {
                            width,
                            height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
                .map_err(GpuError::from)?;
            let requirements = device.get_image_memory_requirements(image);
            let type_index = match gpu
                .memory_type_index_for_bits(requirements.memory_type_bits, MemoryKind::DeviceLocal)
            {
                Some(index) => index,
                None => {
                    device.destroy_image(image, None);
                    return Err(GpuError::Allocation(
                        "no device-local memory for BC7 texture".into(),
                    ));
                }
            };
            let memory = match device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(type_index),
                None,
            ) {
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
            let view = match device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::BC7_UNORM_BLOCK)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            ) {
                Ok(view) => view,
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
                width,
                height,
                bindless_slot,
                layout: vk::ImageLayout::UNDEFINED,
            })
        }
    }

    /// 销毁图像视图、图像与显存。
    ///
    /// # Safety
    /// 调用方须确保已无任何在飞命令缓冲区引用该图像。
    unsafe fn destroy(self, gpu: &Gpu) {
        // SAFETY: 由调用方保证无在飞引用。
        unsafe {
            gpu.device.destroy_image_view(self.view, None);
            gpu.device.destroy_image(self.image, None);
            gpu.device.free_memory(self.memory, None);
        }
    }
}

/// 异步上传任务的目标图像标识。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum Bc7UploadTarget {
    /// 动态 BC7 图集页（按 `page_id` 索引）。
    Page(u32),
    /// 独立 BC7 纹理（按 `bindless_slot` 索引）。
    External(u32),
}

/// 待上传的 BC7 压缩数据来源。
#[derive(Debug)]
enum PendingUploadData {
    /// 新建 `2048×2048` 图集页的 Mode 6 透明黑整页清零（4 MiB，直接写入 Staging 环）。
    ClearPageMode6,
    /// 已脱离调用方生命周期的独立 BC7 字节副本。
    Bytes(Vec<u8>),
}

impl PendingUploadData {
    /// 返回本次上传所需的字节数。
    fn byte_len(&self) -> usize {
        match self {
            Self::ClearPageMode6 => BC7_PAGE_BYTES,
            Self::Bytes(data) => data.len(),
        }
    }

    /// 是否为新页整页清零任务。
    fn is_clear(&self) -> bool {
        matches!(self, Self::ClearPageMode6)
    }
}

/// 排队等待在下一帧命令缓冲区中录制的异步 BC7 拷贝任务。
#[derive(Debug)]
struct PendingBc7Upload {
    target: Bc7UploadTarget,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    data: PendingUploadData,
}

/// 动态 BC7 多页图集与独立 BC7 纹理管理器。
///
/// 嵌入 [`crate::frame::Renderer`]，统一管理：
/// * 动态 `2048×2048` BC7 图集页（`pages`）；
/// * `> 512px` 独立 BC7 纹理（`external_images`）；
/// * 按 `frames_in_flight` 分槽的持久化异步 Staging 环（`staging_slots`）；
/// * 按在飞帧代次对齐的延迟销毁队列（`deferred_images` / `deferred_staging`）。
#[derive(Debug)]
pub(crate) struct Bc7AtlasManager {
    pages: HashMap<u32, Bc7Image>,
    external_images: HashMap<u32, Bc7Image>,
    staging_slots: Vec<Option<Buffer>>,
    pending_uploads: Vec<PendingBc7Upload>,
    deferred_images: DeferredDestroyQueue<Bc7Image>,
    deferred_staging: DeferredDestroyQueue<Buffer>,
}

impl Bc7AtlasManager {
    /// 创建管理器并为每个在飞帧槽位预留持久化 Staging 槽位（按需懒分配显存）。
    pub(crate) fn new(frames_in_flight: usize) -> Self {
        let slot_count = frames_in_flight.max(1);
        let mut staging_slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            staging_slots.push(None);
        }
        Self {
            pages: HashMap::new(),
            external_images: HashMap::new(),
            staging_slots,
            pending_uploads: Vec::new(),
            deferred_images: DeferredDestroyQueue::new(),
            deferred_staging: DeferredDestroyQueue::new(),
        }
    }

    /// 校验当前设备是否支持原生 BC7 硬件采样。
    fn ensure_bc7_supported(gpu: &Gpu) -> Result<(), GpuError> {
        if !gpu.compressed_texture_capabilities().supports_bc7 {
            return Err(GpuError::InvalidArgument(
                "native BC7 compressed textures are not supported by this GPU".into(),
            ));
        }
        Ok(())
    }

    /// 按需创建一张 `2048×2048` 的 BC7 动态图集页，提交整页 Mode 6 透明黑清零任务并绑定 Bindless 槽位。
    pub(crate) fn create_page(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        external_slots: &mut ExternalTextureSlots,
        page_id: u32,
    ) -> Result<u32, GpuError> {
        Self::ensure_bc7_supported(gpu)?;
        if self.pages.contains_key(&page_id) {
            return Err(GpuError::InvalidArgument(format!(
                "BC7 atlas page {page_id} already exists"
            )));
        }
        let slot = external_slots.allocate().ok_or_else(|| {
            GpuError::InvalidArgument(format!(
                "all {} external bindless slots are in use",
                crate::pipeline::EXTERNAL_TEXTURE_SLOTS
            ))
        })?;
        let image = match Bc7Image::create(gpu, BC7_PAGE_SIZE, BC7_PAGE_SIZE, slot) {
            Ok(img) => img,
            Err(err) => {
                external_slots.release(slot);
                return Err(err);
            }
        };
        if let Err(err) = pipelines.bind_external_texture(gpu, slot, image.view, pipelines.sampler)
        {
            // SAFETY: 描述符绑定失败，尚无任何在飞帧引用该图像。
            unsafe { image.destroy(gpu) };
            external_slots.release(slot);
            return Err(err);
        }
        self.pages.insert(page_id, image);
        // 新页自动入队整页 4 MiB Mode 6 透明黑块异步清零，严禁同步阻塞等待。
        self.pending_uploads.push(PendingBc7Upload {
            target: Bc7UploadTarget::Page(page_id),
            x: 0,
            y: 0,
            width: BC7_PAGE_SIZE,
            height: BC7_PAGE_SIZE,
            data: PendingUploadData::ClearPageMode6,
        });
        Ok(slot)
    }

    /// 校验并将 BC7 子区域压缩数据加入下一帧异步 Staging 批量上传队列。
    pub(crate) fn stage_page_subregion(
        &mut self,
        page_id: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        bc7_bytes: &[u8],
    ) -> Result<(), GpuError> {
        let Some(page) = self.pages.get(&page_id) else {
            return Err(GpuError::InvalidArgument(format!(
                "BC7 atlas page {page_id} does not exist"
            )));
        };
        validate_bc7_subregion(page.width, page.height, x, y, width, height, bc7_bytes)?;
        // 立即拷贝输入切片以解除对调用方缓冲区的借用。
        self.pending_uploads.push(PendingBc7Upload {
            target: Bc7UploadTarget::Page(page_id),
            x,
            y,
            width,
            height,
            data: PendingUploadData::Bytes(bc7_bytes.to_vec()),
        });
        Ok(())
    }

    /// 解绑并延迟销毁指定 BC7 图集页；底层 Vulkan 对象与 Bindless 槽位在 `frames_in_flight` 帧后回收。
    pub(crate) fn destroy_page(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        page_id: u32,
        current_frame: u64,
        frames_in_flight: u32,
    ) -> Result<(), GpuError> {
        let Some(page) = self.pages.remove(&page_id) else {
            return Err(GpuError::InvalidArgument(format!(
                "BC7 atlas page {page_id} does not exist"
            )));
        };
        let slot = page.bindless_slot;
        if let Err(err) = pipelines.unbind_external_texture(gpu, slot) {
            self.pages.insert(page_id, page);
            return Err(err);
        }
        // 清理尚未录制的同页上传任务，避免向已退役图像发起拷贝。
        self.pending_uploads
            .retain(|u| u.target != Bc7UploadTarget::Page(page_id));
        self.deferred_images
            .defer(current_frame, frames_in_flight, page);
        Ok(())
    }

    /// 创建并异步上传一张独立 BC7 纹理（适用于 `> 512px` 大图），返回分配的 Bindless 槽位。
    pub(crate) fn upload_external(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        external_slots: &mut ExternalTextureSlots,
        width: u32,
        height: u32,
        bc7_bytes: &[u8],
    ) -> Result<u32, GpuError> {
        Self::ensure_bc7_supported(gpu)?;
        validate_bc7_subregion(width, height, 0, 0, width, height, bc7_bytes)?;
        let slot = external_slots.allocate().ok_or_else(|| {
            GpuError::InvalidArgument(format!(
                "all {} external bindless slots are in use",
                crate::pipeline::EXTERNAL_TEXTURE_SLOTS
            ))
        })?;
        let image = match Bc7Image::create(gpu, width, height, slot) {
            Ok(img) => img,
            Err(err) => {
                external_slots.release(slot);
                return Err(err);
            }
        };
        if let Err(err) = pipelines.bind_external_texture(gpu, slot, image.view, pipelines.sampler)
        {
            // SAFETY: 描述符绑定失败，尚无任何在飞帧引用该图像。
            unsafe { image.destroy(gpu) };
            external_slots.release(slot);
            return Err(err);
        }
        self.external_images.insert(slot, image);
        self.pending_uploads.push(PendingBc7Upload {
            target: Bc7UploadTarget::External(slot),
            x: 0,
            y: 0,
            width,
            height,
            data: PendingUploadData::Bytes(bc7_bytes.to_vec()),
        });
        Ok(slot)
    }

    /// 解绑并延迟销毁独立 BC7 纹理；底层 Vulkan 对象与 Bindless 槽位在 `frames_in_flight` 帧后回收。
    pub(crate) fn destroy_external(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        slot: u32,
        current_frame: u64,
        frames_in_flight: u32,
    ) -> Result<(), GpuError> {
        if !ExternalTextureSlots::owns(slot) {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not an external texture slot"
            )));
        }
        let Some(image) = self.external_images.remove(&slot) else {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not a managed external BC7 texture"
            )));
        };
        if let Err(err) = pipelines.unbind_external_texture(gpu, slot) {
            self.external_images.insert(slot, image);
            return Err(err);
        }
        self.pending_uploads
            .retain(|u| u.target != Bc7UploadTarget::External(slot));
        self.deferred_images
            .defer(current_frame, frames_in_flight, image);
        Ok(())
    }

    /// 判断指定 Bindless 槽位是否由本管理器托管为独立 BC7 纹理。
    pub(crate) fn contains_external(&self, slot: u32) -> bool {
        self.external_images.contains_key(&slot)
    }

    /// 将当前排队的所有 BC7 异步上传任务写入 `frame_slot` 对应的持久化 Staging 环，
    /// 并录制非阻塞屏障转换与按图像聚合的 `vkCmdCopyBufferToImage` 命令。
    pub(crate) fn record_pending_uploads(
        &mut self,
        gpu: &Gpu,
        cmd: vk::CommandBuffer,
        frame_slot: usize,
        current_frame: u64,
        frames_in_flight: u32,
    ) -> Result<(), GpuError> {
        if self.pending_uploads.is_empty() {
            return Ok(());
        }

        // 先按顺序过滤出仍存活的目标上传任务，并计算本批次所需 Staging 字节总量。
        let pending = core::mem::take(&mut self.pending_uploads);
        let copy_align = gpu
            .properties
            .limits
            .optimal_buffer_copy_offset_alignment
            .max(BC7_BLOCK_BYTES as u64);

        let mut resolved: Vec<(Bc7UploadTarget, vk::Image, PendingBc7Upload, u64)> =
            Vec::with_capacity(pending.len());
        let mut cursor: u64 = 0;

        for upload in pending {
            let vk_image = match upload.target {
                Bc7UploadTarget::Page(id) => self.pages.get(&id).map(|p| p.image),
                Bc7UploadTarget::External(slot) => self.external_images.get(&slot).map(|e| e.image),
            };
            let Some(image) = vk_image else {
                continue;
            };
            let aligned_offset = align_up(cursor, copy_align);
            let byte_len = upload.data.byte_len() as u64;
            let next_cursor = aligned_offset.checked_add(byte_len).ok_or_else(|| {
                GpuError::Allocation("BC7 staging upload batch size overflow".into())
            })?;
            resolved.push((upload.target, image, upload, aligned_offset));
            cursor = next_cursor;
        }

        if resolved.is_empty() {
            return Ok(());
        }

        // 确保当前帧槽位拥有足够容量的持久化 Staging 缓冲区；超限时安全扩容并延迟销毁旧缓冲。
        let slot_idx = frame_slot % self.staging_slots.len().max(1);
        let needs_alloc = self.staging_slots[slot_idx]
            .as_ref()
            .map_or(true, |buf| buf.size < cursor);
        if needs_alloc {
            let old_cap = self.staging_slots[slot_idx]
                .as_ref()
                .map_or(0, |buf| buf.size);
            let new_cap = cursor
                .max(DEFAULT_STAGING_SLOT_BYTES)
                .max(old_cap.saturating_mul(2));
            let new_buf = gpu.create_buffer(
                new_cap,
                vk::BufferUsageFlags::TRANSFER_SRC,
                MemoryKind::HostVisible,
                "bc7-async-staging",
            )?;
            if let Some(old_buf) = self.staging_slots[slot_idx].replace(new_buf) {
                self.deferred_staging
                    .defer(current_frame, frames_in_flight, old_buf);
            }
        }

        let staging = self.staging_slots[slot_idx]
            .as_ref()
            .ok_or_else(|| GpuError::Allocation("BC7 staging slot missing".into()))?;

        // 1. 将本批次全部 BC7 块数据顺序写入持久化 Staging 缓冲，并执行单次非一致内存刷写。
        // SAFETY: `frame_slot` 对应的前序帧围栏已退役，且 `staging.size >= cursor`。
        unsafe {
            for (_, _, upload, offset) in &resolved {
                let ptr = staging
                    .host_ptr(*offset)
                    .ok_or_else(|| GpuError::Allocation("BC7 staging buffer not mapped".into()))?;
                match &upload.data {
                    PendingUploadData::ClearPageMode6 => {
                        let block_count = BC7_PAGE_BYTES / BC7_BLOCK_BYTES;
                        let dst_blocks =
                            core::slice::from_raw_parts_mut(ptr.cast::<[u8; 16]>(), block_count);
                        dst_blocks.fill(BC7_MODE6_TRANSPARENT_BLACK);
                    }
                    PendingUploadData::Bytes(bytes) => {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
                    }
                }
            }
            gpu.flush(staging, 0, cursor);
        }

        // 2. 收集本批次涉及的去重目标图像，批量发出 `old_layout -> TRANSFER_DST_OPTIMAL` 屏障。
        let mut touched_targets: Vec<(Bc7UploadTarget, vk::Image, vk::ImageLayout)> = Vec::new();
        for (target, image, _, _) in &resolved {
            if !touched_targets.iter().any(|(t, _, _)| t == target) {
                let old_layout = match *target {
                    Bc7UploadTarget::Page(id) => self.pages[&id].layout,
                    Bc7UploadTarget::External(slot) => self.external_images[&slot].layout,
                };
                touched_targets.push((*target, *image, old_layout));
            }
        }

        let pre_barriers: Vec<vk::ImageMemoryBarrier2<'_>> = touched_targets
            .iter()
            .map(|&(_, image, old_layout)| {
                let (src_stage, src_access) = if old_layout == vk::ImageLayout::UNDEFINED {
                    (vk::PipelineStageFlags2::TOP_OF_PIPE, vk::AccessFlags2::NONE)
                } else {
                    (
                        vk::PipelineStageFlags2::FRAGMENT_SHADER,
                        vk::AccessFlags2::SHADER_SAMPLED_READ,
                    )
                };
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(src_stage)
                    .src_access_mask(src_access)
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(old_layout)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .image(image)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    )
            })
            .collect();

        // SAFETY: 向当前帧独占命令缓冲区录制管线屏障与批量缓冲到图像拷贝。
        unsafe {
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&pre_barriers),
            );
        }

        // 3. 按目标 `VkImage` 聚合拷贝区域：若同一图像在同帧既有整页清零又有子区域上传，
        //    先执行整页清零并插入 `COPY -> COPY` 屏障，避免单次调用内重叠目标区域未定义行为。
        let mut clear_groups: Vec<(vk::Image, Vec<vk::BufferImageCopy>)> = Vec::new();
        let mut update_groups: Vec<(vk::Image, Vec<vk::BufferImageCopy>)> = Vec::new();

        for (_, image, upload, offset) in &resolved {
            let region = vk::BufferImageCopy::default()
                .buffer_offset(*offset)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D {
                    x: upload.x as i32,
                    y: upload.y as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: upload.width,
                    height: upload.height,
                    depth: 1,
                });

            let target_groups = if upload.data.is_clear() {
                &mut clear_groups
            } else {
                &mut update_groups
            };
            if let Some((_, regions)) = target_groups.iter_mut().find(|(img, _)| *img == *image) {
                regions.push(region);
            } else {
                target_groups.push((*image, vec![region]));
            }
        }

        let has_overlap = clear_groups
            .iter()
            .any(|(img, _)| update_groups.iter().any(|(u_img, _)| u_img == img));

        // SAFETY: 所有图像均已转换至 `TRANSFER_DST_OPTIMAL`，缓冲偏移与尺寸均已校验。
        unsafe {
            for (image, regions) in &clear_groups {
                gpu.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.raw,
                    *image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    regions,
                );
            }
            if has_overlap {
                let copy_barrier = [vk::MemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)];
                gpu.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&copy_barrier),
                );
            }
            for (image, regions) in &update_groups {
                gpu.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.raw,
                    *image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    regions,
                );
            }
        }

        // 4. 批量发出 `TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL` 屏障并更新记录布局。
        let post_barriers: Vec<vk::ImageMemoryBarrier2<'_>> = touched_targets
            .iter()
            .map(|&(_, image, _)| {
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(image)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    )
            })
            .collect();

        // SAFETY: 所有目标图像均已完成拷贝录制，转换为片元着色器只读采样布局。
        unsafe {
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&post_barriers),
            );
        }

        for (target, _, _) in touched_targets {
            match target {
                Bc7UploadTarget::Page(id) => {
                    if let Some(page) = self.pages.get_mut(&id) {
                        page.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
                    }
                }
                Bc7UploadTarget::External(slot) => {
                    if let Some(ext) = self.external_images.get_mut(&slot) {
                        ext.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
                    }
                }
            }
        }

        Ok(())
    }

    /// 回收已完成全部在飞帧引用的延迟销毁 BC7 图像、旧 Staging 缓冲并归还 Bindless 槽位。
    pub(crate) fn reclaim_deferred(
        &mut self,
        gpu: &Gpu,
        external_slots: &mut ExternalTextureSlots,
        completed_generation: u64,
    ) {
        self.deferred_images
            .drain_ready_with(completed_generation, |image| {
                let slot = image.bindless_slot;
                // SAFETY: 对应在飞帧围栏已退役，GPU 不再引用该图像。
                unsafe { image.destroy(gpu) };
                external_slots.release(slot);
            });
        self.deferred_staging
            .drain_ready_with(completed_generation, |buffer| {
                // SAFETY: 对应在飞帧围栏已退役，GPU 不再引用该旧 Staging 缓冲。
                unsafe { gpu.destroy_buffer(&buffer) };
            });
    }

    /// 在设备空闲后排空并销毁全部活跃页、独立 BC7 纹理、延迟队列与 Staging 环缓冲。
    ///
    /// # Safety
    /// 调用方须确保已执行 `gpu.wait_idle()` 且设备处于完全空闲状态。
    pub(crate) unsafe fn destroy_all(&mut self, gpu: &Gpu) {
        self.pending_uploads.clear();
        for (_, page) in self.pages.drain() {
            // SAFETY: 设备已空闲。
            unsafe { page.destroy(gpu) };
        }
        for (_, ext) in self.external_images.drain() {
            // SAFETY: 设备已空闲。
            unsafe { ext.destroy(gpu) };
        }
        self.deferred_images.drain_all_with(|image| {
            // SAFETY: 设备已空闲。
            unsafe { image.destroy(gpu) };
        });
        self.deferred_staging.drain_all_with(|buffer| {
            // SAFETY: 设备已空闲。
            unsafe { gpu.destroy_buffer(&buffer) };
        });
        for slot in &mut self.staging_slots {
            if let Some(buffer) = slot.take() {
                // SAFETY: 设备已空闲。
                unsafe { gpu.destroy_buffer(&buffer) };
            }
        }
    }

    /// 当前活跃的动态 BC7 图集页数量。
    #[must_use]
    pub(crate) fn active_page_count(&self) -> usize {
        self.pages.len()
    }

    /// 当前处于延迟销毁队列中等待在飞帧退役的 BC7 图像数量。
    #[must_use]
    pub(crate) fn deferred_destroy_count(&self) -> usize {
        self.deferred_images.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{
        BlendMode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PassLayer, PrimType, CLIP_NONE,
    };
    use crate::device::{CompressedTextureBackend, GpuError};
    use crate::frame::{Camera, Renderer, SceneUpload};

    /// 编码单个纯色 BC7 Mode 6 压缩块（4×4 像素全为同一颜色）。
    ///
    /// Mode 6 位布局（128 bit 小端序）：
    /// * `[0..=6]`：7 位模式前缀 `0b100_0000`；
    /// * `[7..=62]`：8 个 7 位通道端点（R0, R1, G0, G1, B0, B1, A0, A1）；
    /// * `[63..=64]`：2 个端点各自独立的 P-bit（P0, P1）；
    /// * `[65..=127]`：16 个纹素索引（首纹素 3 位 + 其余 15 个各 4 位），纯色块全置 0。
    fn encode_bc7_mode6_solid(r: u8, g: u8, b: u8, a: u8) -> [u8; 16] {
        let p = u128::from(a & 1);
        let r7 = u128::from(r >> 1);
        let g7 = u128::from(g >> 1);
        let b7 = u128::from(b >> 1);
        let a7 = u128::from(a >> 1);
        let mut bits: u128 = 1u128 << 6;
        bits |= r7 << 7;
        bits |= r7 << 14;
        bits |= g7 << 21;
        bits |= g7 << 28;
        bits |= b7 << 35;
        bits |= b7 << 42;
        bits |= a7 << 49;
        bits |= a7 << 56;
        bits |= p << 63;
        bits |= p << 64;
        bits.to_le_bytes()
    }

    /// 生成填充指定纯色 Mode 6 块的 BC7 子区域字节数组。
    fn make_solid_bc7_region(width: u32, height: u32, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let block = encode_bc7_mode6_solid(r, g, b, a);
        let block_count = ((width / BC7_BLOCK_DIM) * (height / BC7_BLOCK_DIM)) as usize;
        let mut out = Vec::with_capacity(block_count * BC7_BLOCK_BYTES);
        for _ in 0..block_count {
            out.extend_from_slice(&block);
        }
        out
    }

    #[test]
    fn mode6_transparent_black_constant_matches_encoder() {
        assert_eq!(
            BC7_MODE6_TRANSPARENT_BLACK,
            encode_bc7_mode6_solid(0, 0, 0, 0)
        );
        assert_eq!(BC7_MODE6_TRANSPARENT_BLACK[0], 0x40);
        assert!(BC7_MODE6_TRANSPARENT_BLACK[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn validate_bc7_subregion_enforces_alignment_bounds_and_byte_len() {
        // 合法 8x4 区域需要 2 * 1 * 16 = 32 字节。
        let ok_bytes = vec![0u8; 32];
        assert!(validate_bc7_subregion(2048, 2048, 0, 0, 8, 4, &ok_bytes).is_ok());
        assert!(validate_bc7_subregion(2048, 2048, 2040, 2044, 8, 4, &ok_bytes).is_ok());

        // 零尺寸拒绝。
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 0, 4, &[]),
            Err(GpuError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 4, 0, &[]),
            Err(GpuError::InvalidArgument(_))
        ));

        // 坐标未按 4x4 对齐拒绝。
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 2, 0, 8, 4, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 3, 8, 4, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));

        // 宽高未按 4x4 对齐拒绝。
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 6, 4, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 8, 5, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));

        // 越界拒绝。
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 2044, 0, 8, 4, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 2048, 8, 4, &ok_bytes),
            Err(GpuError::InvalidArgument(_))
        ));

        // 字节长度不匹配拒绝。
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 8, 4, &[0u8; 16]),
            Err(GpuError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_bc7_subregion(2048, 2048, 0, 0, 8, 4, &[0u8; 48]),
            Err(GpuError::InvalidArgument(_))
        ));
    }

    #[test]
    fn offscreen_bc7_multipage_atlas_srgb_roundtrip_and_deferred_reclaim() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };

        let caps = gpu.compressed_texture_capabilities();
        if !caps.supports_bc7 {
            assert_eq!(caps.active_backend, CompressedTextureBackend::Rgba8Fallback);
            eprintln!("skipping BC7 offscreen test: native BC7 unsupported on this GPU");
            gpu.destroy();
            return;
        }
        assert_eq!(caps.active_backend, CompressedTextureBackend::NativeBc7);

        let frames_in_flight = gpu.frames_in_flight();
        let viewport = (128u32, 64u32);
        let mut renderer = Renderer::new(gpu, viewport.0, viewport.1).expect("renderer");
        // 将背景清屏为完全透明黑，便于断言未写入区域零串色。
        renderer.set_clear_alpha(0.0);

        let slot = renderer
            .create_bc7_atlas_page(0)
            .expect("create BC7 atlas page");
        assert!(ExternalTextureSlots::owns(slot));
        assert_eq!(renderer.bc7_active_page_count(), 1);
        assert_eq!(renderer.bc7_deferred_destroy_count(), 0);

        // 两个相邻纯色子块（均含中间灰阶通道以验证 sRGB 零二次压暗），中间相隔 4px 未写入区：
        // - 块 A：(0, 0, 16x16)，sRGB (230, 128, 32, 255)
        // - 未写入间隙：(16, 0, 4x16)，由新页 Mode 6 透明黑清零
        // - 块 B：(20, 0, 16x16)，sRGB (32, 180, 128, 255)
        let block_a = make_solid_bc7_region(16, 16, 230, 128, 32, 255);
        let block_b = make_solid_bc7_region(16, 16, 32, 180, 128, 255);
        renderer
            .stage_bc7_atlas_subregion(0, 0, 0, 16, 16, &block_a)
            .expect("stage block A");
        renderer
            .stage_bc7_atlas_subregion(0, 20, 0, 16, 16, &block_b)
            .expect("stage block B");

        let page_f = BC7_PAGE_SIZE as f32;
        let image_quad = |x: f32, y: f32, w: f32, h: f32, payload_idx: u32| GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: [0; 4],
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: CLIP_NONE,
            // 令牌 0（Token::NONE）表示原色无 tint。
            fill_token_or_uv: 0,
            border_token_or_tex: slot,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::Image,
                BlendMode::Normal,
                0,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(1, 1.0),
            extra_payload_idx: payload_idx,
        };

        // 分别采样块 A 中心区、未写入 4px 间隙中心区与块 B 中心区（内缩 1 texel 避开线性过滤边缘）。
        let payloads = [
            GpuPayload {
                p0: [2.0 / page_f, 2.0 / page_f],
                p1: [14.0 / page_f, 14.0 / page_f],
                ..Default::default()
            },
            GpuPayload {
                p0: [17.0 / page_f, 2.0 / page_f],
                p1: [19.0 / page_f, 14.0 / page_f],
                ..Default::default()
            },
            GpuPayload {
                p0: [22.0 / page_f, 2.0 / page_f],
                p1: [34.0 / page_f, 14.0 / page_f],
                ..Default::default()
            },
        ];
        let prims = [
            image_quad(0.0, 0.0, 32.0, 64.0, 0),
            image_quad(40.0, 0.0, 32.0, 64.0, 1),
            image_quad(80.0, 0.0, 32.0, 64.0, 2),
        ];
        let transforms = [GpuTransform::IDENTITY];
        let scene = SceneUpload {
            primitives: &prims,
            transforms: &transforms,
            clips: &[],
            payloads: &payloads,
            generation: 1,
            camera: Camera::ui((viewport.0 as f32, viewport.1 as f32), 1.0),
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges: &[],
            aux_changed: false,
            backdrops: &[],
        };

        renderer.render(&scene).expect("render BC7 atlas frame");
        let pixels = renderer.read_pixels().expect("readback BC7 pixels");
        assert_eq!(pixels.len(), (viewport.0 * viewport.1 * 4) as usize);

        let sample_bgra = |px: u32, py: u32| -> [u8; 4] {
            let idx = ((py * viewport.0 + px) * 4) as usize;
            [
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            ]
        };

        // 块 A 中心（BGRA 顺序：B=32, G=128, R=230, A=255）。
        let pa = sample_bgra(16, 32);
        assert!(
            pa[2].abs_diff(230) <= 4
                && pa[1].abs_diff(128) <= 4
                && pa[0].abs_diff(32) <= 4
                && pa[3] >= 251,
            "block A sRGB roundtrip mismatch (expected ~RGBA(230,128,32,255), got BGRA={pa:?})"
        );

        // 未写入间隙中心：必须保持透明黑（零串色）。
        let pgap = sample_bgra(56, 32);
        assert!(
            pgap[0] == 0 && pgap[1] == 0 && pgap[2] == 0 && pgap[3] == 0,
            "unwritten Mode 6 gap must be transparent black without bleed, got BGRA={pgap:?}"
        );

        // 块 B 中心（BGRA 顺序：B=128, G=180, R=32, A=255）。
        let pb = sample_bgra(96, 32);
        assert!(
            pb[2].abs_diff(32) <= 4
                && pb[1].abs_diff(180) <= 4
                && pb[0].abs_diff(128) <= 4
                && pb[3] >= 251,
            "block B sRGB roundtrip mismatch (expected ~RGBA(32,180,128,255), got BGRA={pb:?})"
        );

        // 销毁冷页并连续推进 `frames_in_flight + 1` 帧，验证延迟队列归零且活跃页数为 0。
        renderer
            .destroy_bc7_atlas_page(0)
            .expect("destroy BC7 atlas page");
        assert_eq!(renderer.bc7_active_page_count(), 0);
        assert_eq!(renderer.bc7_deferred_destroy_count(), 1);

        for step in 0..=frames_in_flight {
            let empty_scene = SceneUpload {
                primitives: &[],
                transforms: &[],
                clips: &[],
                payloads: &[],
                generation: 2 + u64::from(step),
                camera: Camera::ui((viewport.0 as f32, viewport.1 as f32), 1.0),
                key_prefix: (PassLayer::Ui as u64) << 56,
                dirty_ranges: &[],
                aux_changed: false,
                backdrops: &[],
            };
            renderer
                .render(&empty_scene)
                .expect("advance frame for BC7 reclaim");
        }
        assert_eq!(renderer.bc7_active_page_count(), 0);
        assert_eq!(renderer.bc7_deferred_destroy_count(), 0);

        renderer.destroy();
    }
}

//! GPU 侧字形与图像图集（参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 渲染器绑定两个 Bindless `Texture2DArray` 采样器：用于覆盖率字形的 `R8` 图集，
//! 以及用于彩色字形和图像的 `RGBA8` 图集。CPU 侧（`unbound-text`）负责货架装箱并决定
//! 每个字形落在何处；本模块持有 GPU 纹理并负责将像素搬运到对应位置。
//!
//! CPU 图集的每一页对应一个数组图层，因此一次更新只需通过单次 `vkCmdCopyBufferToImage`
//! 写入某一图层的子区域。

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use ash::vk;

/// 每个图集暴露的 Bindless 图层数量。
///
/// 必须与管线模块中的 `MAX_ATLAS_LAYERS` 一致：描述符数组大小在管线创建时固定。
pub const ATLAS_LAYERS: u32 = crate::pipeline::MAX_ATLAS_LAYERS;

/// 单张图集页的边长（纹素）。
pub const PAGE_SIZE: u32 = 2048;

/// 指定更新操作针对哪一个图集。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AtlasKind {
    /// 单通道覆盖率图集（字形）。
    R8,
    /// 四通道彩色图集（Emoji、图像）。
    Rgba8,
}

impl AtlasKind {
    /// 该图集使用的 Vulkan 像素格式。
    #[must_use]
    pub const fn format(self) -> vk::Format {
        match self {
            Self::R8 => vk::Format::R8_UNORM,
            Self::Rgba8 => vk::Format::R8G8B8A8_UNORM,
        }
    }

    /// 每纹素占用的字节数。
    #[must_use]
    pub const fn bpp(self) -> u32 {
        match self {
            Self::R8 => 1,
            Self::Rgba8 => 4,
        }
    }
}

/// 两个图集纹理之一。
///
/// Vulkan 对象一旦创建便不可变；仅各图层的图像布局会变化，因此将其单独追踪，
/// 使图集可在 `Arc` 共享的同时继续接收上传。
pub struct AtlasTexture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    /// 每层一个图像视图；Bindless 描述符数组按顺序绑定它们。
    views: Vec<vk::ImageView>,
    kind: AtlasKind,
    /// 设备的共享句柄，使图集即便生命周期长于创建它的 [`Gpu`] 也能自行释放资源。
    lifetime: crate::device::DeviceLifetime,
    /// 图像释放后置为 true。
    ///
    /// 图集集合与应用层共享，因此渲染器会在设备销毁前显式释放图像；随后的 `Drop`
    /// 会发现图像已释放。该标志确保两条释放路径均幂等。
    freed: std::sync::atomic::AtomicBool,
    /// 每个图层的当前图像布局。
    ///
    /// 从未写入的图层为 `UNDEFINED`；首次上传后转为 `SHADER_READ_ONLY_OPTIMAL`。
    /// 必须追踪布局，因为对同一图层的第二次上传绝不能将 `old_layout` 声明为
    /// `UNDEFINED`，否则会丢弃该图层中已有的字形。
    layouts: std::sync::Mutex<Vec<vk::ImageLayout>>,
}

impl core::fmt::Debug for AtlasTexture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AtlasTexture")
            .field("image", &self.image)
            .field("memory", &self.memory)
            .field("views", &self.views.len())
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl AtlasTexture {
    /// 释放图集图像（幂等：重复调用无副作用）。
    ///
    /// 渲染器在销毁设备前会显式释放图像；当图集生命周期长于设备时，则由 `Drop`
    /// 负责释放——除非设备已提前销毁，此时旧句柄已随设备失效，严禁再次触碰。
    pub fn free(&self) {
        if self.freed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        if !self.lifetime.is_alive() {
            return;
        }
        // SAFETY: 设备仍存活且图像属于该设备；渲染器在调用前已等待设备空闲。
        unsafe {
            let device = self.lifetime.device();
            for view in &self.views {
                device.destroy_image_view(*view, None);
            }
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

impl Drop for AtlasTexture {
    fn drop(&mut self) {
        // 图集可能比创建它的 `Gpu` 存活更久：应用层会在整个会话期间持有图集集合的 `Arc`。
        self.free();
    }
}

impl AtlasTexture {
    /// 创建空图集纹理数组。
    ///
    /// # Errors
    /// 透传 Vulkan 内存分配错误。
    pub fn new(gpu: &Gpu, kind: AtlasKind, layers: u32) -> Result<Self, GpuError> {
        // SAFETY: 仅创建资源；所有权移交至返回的结构体。
        unsafe {
            let device = &gpu.device;
            let image = device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(kind.format())
                    .extent(vk::Extent3D {
                        width: PAGE_SIZE,
                        height: PAGE_SIZE,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(layers)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )?;
            let requirements = device.get_image_memory_requirements(image);
            let type_index = gpu
                .memory_type_index_for_bits(requirements.memory_type_bits, MemoryKind::DeviceLocal)
                .ok_or_else(|| GpuError::Allocation("no device-local memory".into()))?;
            let memory = device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(type_index),
                None,
            )?;
            device.bind_image_memory(image, memory, 0)?;

            // 每层一个视图：Bindless 描述符数组为每层分配一个槽位，每个槽位指向单个数组图层。
            let mut views = Vec::with_capacity(layers as usize);
            for layer in 0..layers {
                let view = device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(kind.format())
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(layer)
                                .layer_count(1),
                        ),
                    None,
                )?;
                views.push(view);
            }

            let layouts = std::sync::Mutex::new(vec![vk::ImageLayout::UNDEFINED; views.len()]);
            Ok(Self {
                image,
                memory,
                views,
                kind,
                lifetime: gpu.lifetime.clone(),
                freed: std::sync::atomic::AtomicBool::new(false),
                layouts,
            })
        }
    }

    /// 返回指定图层的图像视图。
    #[must_use]
    pub fn layer_view(&self, layer: u32) -> Option<vk::ImageView> {
        self.views.get(layer as usize).copied()
    }

    /// 返回该图集暴露的图层数量。
    #[must_use]
    pub fn layers(&self) -> u32 {
        self.views.len() as u32
    }

    /// 获取指定图层的当前布局。
    fn current_layout(&self, layer: u32) -> vk::ImageLayout {
        self.layouts
            .lock()
            .map(|layouts| layouts[layer as usize])
            .unwrap_or(vk::ImageLayout::UNDEFINED)
    }

    /// 记录指定图层的新布局。
    fn set_layout(&self, layer: u32, layout: vk::ImageLayout) {
        if let Ok(mut layouts) = self.layouts.lock() {
            layouts[layer as usize] = layout;
        }
    }

    /// 返回图集类型。
    #[must_use]
    pub const fn kind(&self) -> AtlasKind {
        self.kind
    }

    /// 返回底层 Vulkan 图像句柄。
    #[must_use]
    pub const fn image(&self) -> vk::Image {
        self.image
    }
}

/// 需要从 CPU 上传到图集某图层的矩形纹素区域。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlasUpload {
    /// 接收像素的目标图集。
    pub kind: AtlasKind,
    /// 目标数组图层索引。
    pub layer: u32,
    /// 图层内的目标起始坐标。
    pub x: u32,
    pub y: u32,
    /// 上传区域的尺寸。
    pub width: u32,
    pub height: u32,
    /// 按行主序排列的纹素数据；长度为 `width * height * bpp` 字节。
    pub data: Vec<u8>,
}

impl AtlasUpload {
    /// 本次上传每纹素的字节数。
    #[must_use]
    pub const fn bpp(&self) -> u32 {
        self.kind.bpp()
    }

    /// 预期的载荷字节长度。
    #[must_use]
    pub fn expected_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * (self.bpp() as usize)
    }
}

/// 单批图集上传在暂存缓冲区中的布局方案。
///
/// 所有矩形均放入同一个暂存缓冲区，且每个涉及的图层仅记录一次，
/// 确保其布局转换只执行一次。
#[derive(Clone, Debug, PartialEq, Eq)]
struct BatchPlan {
    /// 每次上传在暂存缓冲区中的字节偏移；空矩形对应 `None`。
    offsets: Vec<Option<u64>>,
    /// 暂存缓冲区所需总字节大小。
    total: u64,
    /// 本批次涉及的所有 `(kind, layer)` 二元组（已去重）。
    layers: Vec<(AtlasKind, u32)>,
}

/// 将一批上传请求排布到单个暂存缓冲区中。
///
/// 刻意设计为纯函数：排布逻辑最容易出错（如偏移未按纹素块对齐、同一图层被转换两次），
/// 因此适合在 CPU 端进行单元测试。
///
/// # Errors
/// 若上传参数不合法则返回 [`GpuError::Allocation`]。
fn plan_batch(
    layer_count: impl Fn(AtlasKind) -> u32,
    updates: &[AtlasUpload],
) -> Result<BatchPlan, GpuError> {
    let mut offsets: Vec<Option<u64>> = Vec::with_capacity(updates.len());
    let mut layers: Vec<(AtlasKind, u32)> = Vec::new();
    let mut total = 0u64;
    for upload in updates {
        let expected = upload.expected_len();
        if upload.data.len() < expected {
            return Err(GpuError::Allocation(format!(
                "atlas upload is {} bytes, expected {expected}",
                upload.data.len()
            )));
        }
        if upload.width == 0 || upload.height == 0 {
            offsets.push(None);
            continue;
        }
        let available = layer_count(upload.kind);
        if upload.layer >= available {
            return Err(GpuError::Allocation(format!(
                "atlas layer {} is out of range ({} layers)",
                upload.layer, available
            )));
        }
        // 拷贝偏移必须是纹素块大小的整数倍；4 字节对齐可同时覆盖 1 字节和 4 字节图集。
        total = (total + 3) & !3;
        offsets.push(Some(total));
        total += expected as u64;
        // 接收多个矩形的同一图层仅转换一次布局。
        if !layers
            .iter()
            .any(|(kind, layer)| *kind == upload.kind && *layer == upload.layer)
        {
            layers.push((upload.kind, upload.layer));
        }
    }
    Ok(BatchPlan {
        offsets,
        total,
        layers,
    })
}

/// 渲染器绑定到描述符槽位中的一对图集。
pub struct AtlasSet {
    /// 覆盖率图集（R8）。
    pub r8: AtlasTexture,
    /// 彩色图集（RGBA8）。
    pub rgba: AtlasTexture,
    /// 所有上传批次复用的持久化暂存缓冲区。
    ///
    /// 字形缓存经常在多帧中零星上传少量小字形；若每批都分配和释放 `VkBuffer` + `VkDeviceMemory`，
    /// 仅几千字节数据就要付出两次驱动往返开销。因此该缓冲区按需扩容并伴随图集集合整个生命周期。
    staging: std::sync::Mutex<Option<StagingSlot>>,
}

/// 持久化暂存缓冲区持有的底层句柄。
///
/// 仅存储普通整数以在不编写 `unsafe impl` 的前提下保持 `Send + Sync`：
/// 映射地址以 `usize` 保存，在使用点再转回指针。
struct StagingSlot {
    raw: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
    allocation_size: u64,
    mapped: usize,
    host_coherent: bool,
    lifetime: crate::device::DeviceLifetime,
}

impl core::fmt::Debug for AtlasSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AtlasSet")
            .field("r8", &self.r8)
            .field("rgba", &self.rgba)
            .finish_non_exhaustive()
    }
}

/// 按 bindless 描述符数组实际可暴露的槽位钳制图集层数。
///
/// R8 占用全部 [`crate::pipeline::MAX_ATLAS_LAYERS`] 个槽位，而 RGBA 绑定
/// 末尾的 [`crate::pipeline::EXTERNAL_TEXTURE_SLOTS`] 个槽位留给调用方纹理。
/// RGBA 图集层数一旦超过 [`crate::pipeline::EXTERNAL_TEXTURE_BASE`] 就无法
/// 完整绑定：超出范围的页会采样到外部槽位，`Renderer::install_atlases` 也会
/// 拒绝这种不匹配。在此处钳制可保证所有调用方（它们都请求完整 64 层）
/// 始终落在绑定范围内。
#[must_use]
pub(crate) fn atlas_layer_counts(layers: u32) -> (u32, u32) {
    (
        layers.clamp(1, crate::pipeline::MAX_ATLAS_LAYERS),
        layers.clamp(1, crate::pipeline::EXTERNAL_TEXTURE_BASE),
    )
}

impl AtlasSet {
    /// 创建覆盖率与彩色两个图集。
    ///
    /// RGBA 图集上限为 [`crate::pipeline::EXTERNAL_TEXTURE_BASE`] 层，
    /// 因为该绑定剩余槽位属于外部纹理。
    ///
    /// # Errors
    /// 透传 Vulkan 内存分配错误。
    pub fn new(gpu: &Gpu, layers: u32) -> Result<Self, GpuError> {
        let (r8_layers, rgba_layers) = atlas_layer_counts(layers);
        Ok(Self {
            r8: AtlasTexture::new(gpu, AtlasKind::R8, r8_layers)?,
            rgba: AtlasTexture::new(gpu, AtlasKind::Rgba8, rgba_layers)?,
            staging: std::sync::Mutex::new(None),
        })
    }

    /// 返回与 `kind` 匹配的图集引用。
    #[must_use]
    pub const fn get(&self, kind: AtlasKind) -> &AtlasTexture {
        match kind {
            AtlasKind::R8 => &self.r8,
            AtlasKind::Rgba8 => &self.rgba,
        }
    }

    /// 释放两个图集的图像资源（幂等）。
    ///
    /// 渲染器在销毁设备前调用此方法：Vulkan 要求所有子对象必须在设备前销毁，
    /// 而应用层此时可能仍持有图集集合的 `Arc`。
    pub fn free(&self) {
        self.r8.free();
        self.rgba.free();
        // 暂存缓冲区归属于图集集合而非单个图集，因此也必须在此一并释放。
        if let Ok(mut slot) = self.staging.lock() {
            if let Some(staging) = slot.take() {
                if staging.lifetime.is_alive() {
                    // SAFETY: 设备仍存活且缓冲区属于该设备；调用方已等待设备空闲。
                    unsafe {
                        let device = staging.lifetime.device();
                        device.destroy_buffer(staging.raw, None);
                        device.free_memory(staging.memory, None);
                    }
                }
            }
        }
    }

    /// 将单个矩形纹素区域上传到对应图集。
    ///
    /// 像素经宿主可见暂存缓冲区通过单次 `vkCmdCopyBufferToImage` 拷贝到目标图层。
    /// 拷贝在图形队列上录制并等待完成，确保 GPU 绝不会采样到半写入的字形。
    ///
    /// # Errors
    /// 当拷贝命令录制失败时返回 [`GpuError::Vulkan`]。
    pub fn upload(&self, gpu: &Gpu, upload: &AtlasUpload) -> Result<(), GpuError> {
        self.upload_batch(gpu, std::slice::from_ref(upload))
    }

    /// 通过**单次**队列往返批量上传多个矩形区域到图集。
    ///
    /// 逐字形上传曾是整个渲染器最严重的卡顿源：每个字形都分配一次暂存缓冲区、
    /// 开启一次性命令缓冲区并调用 `vkQueueWaitIdle`，导致渲染整页文本时管线连续停顿数十次。
    ///
    /// 批量上传改为：
    ///
    /// * 仅使用**一个**暂存缓冲区容纳所有像素；
    /// * 仅录制**一个**命令缓冲区；
    /// * 将每个涉及的 `(kind, layer)` 图层**一次性**转换到 `TRANSFER_DST_OPTIMAL`；
    /// * 上传后**一次性**转回 `SHADER_READ_ONLY_OPTIMAL`。
    ///
    /// 最终仅在末尾等待队列空闲一次。
    ///
    /// # Errors
    /// 上传参数不合法时返回 [`GpuError::Allocation`]，录制拷贝失败时返回 [`GpuError::Vulkan`]。
    pub fn upload_batch(&self, gpu: &Gpu, updates: &[AtlasUpload]) -> Result<(), GpuError> {
        // 1. 校验并将所有像素排布到单个暂存缓冲区中。
        let plan = plan_batch(|kind| self.get(kind).layers(), updates)?;
        let BatchPlan {
            offsets,
            total,
            layers,
        } = plan;
        if total == 0 {
            return Ok(());
        }

        // 按需扩容持久化暂存缓冲区，而非每批重新分配。整个上传过程持有互斥锁：
        // 暂存区内容只需保持到 `end_single_time` 完成即可。
        let mut slot = self
            .staging
            .lock()
            .map_err(|_| GpuError::Allocation("atlas staging poisoned".into()))?;
        let needs_grow = slot.as_ref().is_none_or(|buffer| buffer.size < total);
        if needs_grow {
            if let Some(old) = slot.take() {
                if old.lifetime.is_alive() {
                    // SAFETY: 每批均以 `end_single_time` 结束并等待队列空闲，因此没有拷贝操作仍引用旧缓冲区。
                    unsafe {
                        let device = old.lifetime.device();
                        device.destroy_buffer(old.raw, None);
                        device.free_memory(old.memory, None);
                    }
                }
            }
            let buffer = gpu.create_buffer(
                total,
                vk::BufferUsageFlags::TRANSFER_SRC,
                MemoryKind::HostVisible,
                "atlas-staging",
            )?;
            *slot = Some(StagingSlot {
                raw: buffer.raw,
                memory: buffer.memory,
                size: buffer.size,
                allocation_size: buffer.allocation_size,
                mapped: buffer.mapped as usize,
                host_coherent: buffer.host_coherent,
                lifetime: gpu.lifetime.clone(),
            });
        }
        let staging = slot
            .as_ref()
            .ok_or_else(|| GpuError::Allocation("atlas staging missing".into()))?;
        // 构造临时 `Buffer` 视图，以复用标准的刷写路径（对齐与一致性规则）而不重复代码。
        let staging_view = Buffer {
            raw: staging.raw,
            memory: staging.memory,
            size: staging.size,
            allocation_size: staging.allocation_size,
            address: 0,
            mapped: staging.mapped as *mut u8,
            host_visible: true,
            host_coherent: staging.host_coherent,
        };
        // SAFETY: 在拷贝运行前暂存缓冲区由本批次独占。
        unsafe {
            let base = staging_view
                .host_ptr(0)
                .ok_or_else(|| GpuError::Allocation("atlas staging not mapped".into()))?;
            for (index, upload) in updates.iter().enumerate() {
                let Some(offset) = offsets[index] else {
                    continue;
                };
                let expected = upload.expected_len();
                // SAFETY: `offset + expected` 位于暂存缓冲区范围内，且当前由本批次独占。
                core::ptr::copy_nonoverlapping(
                    upload.data.as_ptr(),
                    base.add(offset as usize),
                    expected,
                );
            }
            // 对整段范围执行一次刷写：内存可能非宿主一致。
            gpu.flush(&staging_view, 0, total);
        }

        let cmd = gpu.begin_single_time()?;
        // SAFETY: `cmd` 是属于本调用的新命令缓冲区。
        unsafe {
            let to_copy: Vec<vk::ImageMemoryBarrier2> = layers
                .iter()
                .map(|(kind, layer)| {
                    let atlas = self.get(*kind);
                    vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::NONE)
                        .src_access_mask(vk::AccessFlags2::NONE)
                        .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        // 图层当前布局已追踪，因此对同一图层的后续上传会保留其中已有的字形。
                        .old_layout(atlas.current_layout(*layer))
                        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(atlas.image())
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(*layer)
                                .layer_count(1),
                        )
                })
                .collect();
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );

            for (index, upload) in updates.iter().enumerate() {
                if upload.width == 0 || upload.height == 0 {
                    continue;
                }
                let Some(offset) = offsets[index] else {
                    continue;
                };
                let atlas = self.get(upload.kind);
                gpu.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.raw,
                    atlas.image(),
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[vk::BufferImageCopy::default()
                        .buffer_offset(offset)
                        .buffer_row_length(0)
                        .buffer_image_height(0)
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .mip_level(0)
                                .base_array_layer(upload.layer)
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
                        })],
                );
            }

            // 转回着色器可读布局，以便图形队列采样。
            let to_read: Vec<vk::ImageMemoryBarrier2> = layers
                .iter()
                .map(|(kind, layer)| {
                    let atlas = self.get(*kind);
                    vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::COPY)
                        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                        .dst_access_mask(vk::AccessFlags2::SHADER_READ)
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(atlas.image())
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(*layer)
                                .layer_count(1),
                        )
                })
                .collect();
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        }
        gpu.end_single_time(cmd)?;

        // 至此所有图层均已处于着色器可读状态。
        for (kind, layer) in layers {
            self.get(kind)
                .set_layout(layer, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        }

        // 暂存缓冲区保持存活，供下一批上传复用。
        Ok(())
    }

    /// 从图集的指定图层回读一个矩形区域的纹素。
    ///
    /// 用于调试验证：证明像素已真正写入 GPU 图像而非仅停留在暂存缓冲区。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    #[allow(clippy::too_many_arguments)]
    pub fn read_layer(
        &self,
        gpu: &Gpu,
        kind: AtlasKind,
        layer: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, GpuError> {
        let atlas = self.get(kind);
        if layer >= atlas.layers() {
            return Err(GpuError::Allocation("atlas layer out of range".into()));
        }
        let bpp = kind.bpp();
        let bytes_per_row = width as u64 * u64::from(bpp);
        let size = bytes_per_row * u64::from(height);
        let staging = gpu.create_buffer(
            size,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryKind::HostVisible,
            "atlas-readback",
        )?;

        let cmd = gpu.begin_single_time()?;
        // SAFETY: 该命令缓冲区归本调用所有。
        unsafe {
            let barrier = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .old_layout(atlas.current_layout(layer))
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(atlas.image())
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(layer)
                        .layer_count(1),
                )];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&barrier),
            );
            gpu.device.cmd_copy_image_to_buffer(
                cmd,
                atlas.image(),
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                staging.raw,
                &[vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(layer)
                            .layer_count(1),
                    )
                    .image_offset(vk::Offset3D {
                        x: x as i32,
                        y: y as i32,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })],
            );
            // 转回着色器可读布局，使渲染器可继续采样。
            let to_read = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(atlas.image())
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(layer)
                        .layer_count(1),
                )];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        }
        gpu.end_single_time(cmd)?;
        self.get(kind)
            .set_layout(layer, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

        let mut out = vec![0u8; size as usize];
        // SAFETY: 拷贝已完成；暂存缓冲区为宿主可见内存。
        unsafe {
            std::ptr::copy_nonoverlapping(staging.mapped, out.as_mut_ptr(), out.len());
            gpu.destroy_buffer(&staging);
        }
        Ok(out)
    }
}

/// 为重复图集上传保留的持久化暂存缓冲区。
///
/// 复用单个缓冲区可避免每批字形都向驱动申请内存。
#[derive(Debug)]
pub struct UploadStaging {
    buffer: Buffer,
}

impl UploadStaging {
    /// 分配 `capacity` 字节的暂存缓冲区。
    ///
    /// # Errors
    /// 透传分配错误。
    pub fn new(gpu: &Gpu, capacity: u64) -> Result<Self, GpuError> {
        let buffer = gpu.create_buffer(
            capacity.max(1),
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryKind::HostVisible,
            "atlas-staging",
        )?;
        Ok(Self { buffer })
    }

    /// 缓冲区容量（字节）。
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.buffer.size
    }

    /// 底层 Vulkan 缓冲区句柄。
    #[must_use]
    pub const fn raw(&self) -> vk::Buffer {
        self.buffer.raw
    }

    /// # Safety
    /// 设备必须处于空闲状态。
    pub unsafe fn destroy(&self, gpu: &Gpu) {
        // SAFETY: 调用方保证设备处于空闲状态，因此无任何命令缓冲区仍引用该缓冲区。
        unsafe {
            gpu.destroy_buffer(&self.buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_sizes_match_the_shader_contract() {
        assert_eq!(AtlasKind::R8.format(), vk::Format::R8_UNORM);
        assert_eq!(AtlasKind::Rgba8.format(), vk::Format::R8G8B8A8_UNORM);
        assert_eq!(AtlasKind::R8.bpp(), 1);
        assert_eq!(AtlasKind::Rgba8.bpp(), 4);
        // 着色器采样的图集页尺寸为 2048²。
        assert_eq!(PAGE_SIZE, 2048);
    }

    /// 窗口调用方请求完整 64 层，而 RGBA 绑定只有 48 个内部槽位，
    /// 直接传入 64 层会让每个窗口的 `install_atlases` 触发调试断言。
    #[test]
    fn the_rgba_atlas_stays_inside_its_bindless_range() {
        use crate::pipeline::{EXTERNAL_TEXTURE_BASE, MAX_ATLAS_LAYERS};

        let (r8, rgba) = atlas_layer_counts(ATLAS_LAYERS);
        assert_eq!(r8, MAX_ATLAS_LAYERS);
        assert_eq!(rgba, EXTERNAL_TEXTURE_BASE);
        assert!(
            rgba < MAX_ATLAS_LAYERS,
            "external textures need their slots"
        );

        // 较小的请求保持原值，过大的请求被钳制，
        // 零请求仍产出有效的（非零）图像数组。
        assert_eq!(atlas_layer_counts(4), (4, 4));
        assert_eq!(atlas_layer_counts(0), (1, 1));
        assert_eq!(
            atlas_layer_counts(1000),
            (MAX_ATLAS_LAYERS, EXTERNAL_TEXTURE_BASE)
        );
    }

    #[test]
    fn an_upload_knows_its_payload_size() {
        let upload = AtlasUpload {
            kind: AtlasKind::Rgba8,
            layer: 0,
            x: 4,
            y: 8,
            width: 10,
            height: 6,
            data: vec![0; 10 * 6 * 4],
        };
        assert_eq!(upload.expected_len(), 240);
        assert_eq!(upload.data.len(), upload.expected_len());
    }

    #[test]
    fn the_layer_count_matches_the_bindless_descriptor_array() {
        // 管线绑定了 `MAX_ATLAS_LAYERS` 个槽位；图集层数不得少于此值，否则描述符写入会越界。
        assert_eq!(ATLAS_LAYERS, crate::pipeline::MAX_ATLAS_LAYERS);
    }

    /// 构造一个字形尺寸的上传请求。
    fn glyph(layer: u32) -> AtlasUpload {
        AtlasUpload {
            kind: AtlasKind::R8,
            layer,
            x: 0,
            y: 0,
            width: 8,
            height: 8,
            data: vec![0; 64],
        }
    }

    #[test]
    fn a_batch_packs_every_glyph_into_one_staging_buffer() {
        let updates = [glyph(0), glyph(0), glyph(1)];
        let plan = plan_batch(|_| 4, &updates).expect("a valid batch");
        // 前两个字形紧邻排布；第三个字形位于新图层但共享同一暂存缓冲区。
        assert_eq!(plan.offsets[0], Some(0));
        assert_eq!(plan.offsets[1], Some(64));
        assert_eq!(plan.offsets[2], Some(128));
        assert_eq!(plan.total, 192);
        // 两个不同图层各只记录一次。
        assert_eq!(plan.layers, vec![(AtlasKind::R8, 0), (AtlasKind::R8, 1)]);
    }

    #[test]
    fn a_batch_offset_is_always_a_multiple_of_the_texel_block() {
        // 彩色字形每纹素占 4 字节，若其前方的 1 字节覆盖率上传未对齐，会导致拷贝偏移错位。
        let mut colour = glyph(0);
        colour.kind = AtlasKind::Rgba8;
        colour.width = 1;
        colour.height = 1;
        colour.data = vec![0; 4];
        let updates = [glyph(0), colour];
        let plan = plan_batch(|_| 4, &updates).expect("a valid batch");
        for offset in plan.offsets.iter().flatten() {
            assert_eq!(offset % 4, 0, "the copy offset must be texel aligned");
        }
        // 彩色字形紧随 64 字节覆盖率字形之后，且满足 4 字节对齐。
        assert_eq!(plan.offsets[1], Some(64));
        assert_eq!(plan.total, 68);
    }

    #[test]
    fn an_empty_rectangle_takes_no_space_and_touches_no_layer() {
        let mut empty = glyph(0);
        empty.width = 0;
        empty.height = 0;
        empty.data = Vec::new();
        let updates = [empty, glyph(0)];
        let plan = plan_batch(|_| 4, &updates).expect("a valid batch");
        assert_eq!(plan.offsets[0], None);
        assert_eq!(plan.offsets[1], Some(0));
        assert_eq!(plan.total, 64);
        assert_eq!(plan.layers, vec![(AtlasKind::R8, 0)]);
    }

    #[test]
    fn a_malformed_batch_is_rejected() {
        let mut short = glyph(0);
        short.data = vec![0; 8];
        assert!(plan_batch(|_| 4, &[short]).is_err());
        assert!(plan_batch(|_| 4, &[glyph(9)]).is_err());
    }
}

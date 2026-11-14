//! 帧编排：端到端的 GPU 驱动渲染管线。
//!
//! 单次 [`Renderer::render`] 调用按顺序执行：
//!
//! ```text
//! CPU：在瞬态环中线性分配、对场景哈希/差分并构建补丁
//!  1. upload      ：宿主写入瞬态环，补丁描述目标地址
//!  2. scatter     ：计算着色器将补丁拷贝到持久化池
//!  3. cull        ：计算着色器执行视口剔除并生成 64 位排序键
//!  4. sort        ：对 (key, index) 执行 8 趟稳定 LSD 基数排序
//!  5. indirect    ：计算着色器将连续同前缀键压缩为间接绘制命令
//!  6. opaque core ：由前向后的纯深度预 Pass，构建 Hi-Z 深度遮挡
//!  7. uber pass   ：单次 MDI 绘制（由后向前），Early-Z 剔除被遮挡片元
//! ```
//!
//! CPU 每帧仅处理 O(变更节点数)；O(可见图元数) 的工作全部在 GPU 完成。
//! 仅移动相机的帧上传 **0 字节**——测试 `camera_only_frame_uploads_nothing` 对此进行了断言。

#![allow(unsafe_op_in_unsafe_fn)]

use ash::vk;

use crate::data::{
    BackdropCompositePush, BlurPush, FramePushConstants, GpuClipNode, GpuPayload,
    GpuPrimitiveInstance, GpuScatterPatch, GpuTransform, PassLayer, PushConstants,
};
use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use crate::pipeline::{ExternalTextureSlots, Pipelines, COLOR_FORMAT, DEPTH_FORMAT};
use crate::pool::{PersistentPool, PoolHandle};
use crate::ring::TransientRing;

/// 完整 64 位键所需的基数位趟数（每趟 8 位）。
const RADIX_PASSES: u32 = 8;
/// 每个基数排序块的元素数；必须与排序着色器中的 `BLOCK` 一致。
const RADIX_BLOCK: u32 = 256;
/// 每个图层 Pass 允许的间接绘制批次上限。
pub const MAX_DRAW_RUNS: u32 = 16;
/// 写入命令缓冲区的阶段时间戳标记数量。
const STAGE_MARKS: u32 = 6;

/// 离屏渲染目标（颜色 + 深度）。
#[derive(Debug)]
pub struct RenderTarget {
    pub width: u32,
    pub height: u32,
    pub color: vk::Image,
    color_memory: vk::DeviceMemory,
    pub color_view: vk::ImageView,
    pub depth: vk::Image,
    depth_memory: vk::DeviceMemory,
    pub depth_view: vk::ImageView,
    /// 深度图像实际分配的尺寸。
    ///
    /// 可能大于 `width × height`：窗口化路径按 256 px 步长向上增长且永不收缩，
    /// 避免拖拽缩放窗口时频繁重建分配。
    depth_alloc: (u32, u32),
    layout: vk::ImageLayout,
}

/// 单个图层 Pass 的相机状态。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Camera {
    pub chunk: [i32; 2],
    pub local: [f32; 2],
    pub zoom: f32,
    pub dpi_scale: f32,
}

impl Camera {
    /// 静态 UI 图层的相机。
    ///
    /// 约定：`local`/`chunk` 表示映射到视口**中心**的世界坐标点，因此 UI 相机
    /// 位于逻辑坐标 `viewport_size / (2 · dpi_scale)`，画布相机位于 `canvas_centre`。
    /// 这使同一套着色器公式无需分支即可同时服务可平移画布与静态面板。
    #[must_use]
    pub fn ui(viewport: (f32, f32), dpi_scale: f32) -> Self {
        let scale = if dpi_scale > 0.0 { dpi_scale } else { 1.0 };
        Self {
            chunk: [0, 0],
            local: [viewport.0 * 0.5 / scale, viewport.1 * 0.5 / scale],
            zoom: 1.0,
            dpi_scale: scale,
        }
    }

    /// 对准指定画布世界坐标中心的相机。
    #[must_use]
    pub fn canvas(chunk: [i32; 2], local: [f32; 2], zoom: f32, dpi_scale: f32) -> Self {
        Self {
            chunk,
            local,
            zoom,
            dpi_scale,
        }
    }
}

/// UI 请求的毛玻璃背景虚化区域。
///
/// 矩形使用渲染目标的**物理像素**坐标。GPU 渲染主场景后，使用 Dual-Kawase 滤镜
/// 模糊该矩形后方区域并按圆角遮罩合成回目标，随后模态层自身半透明表面由常规图元 Pass 绘制。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BackdropRequest {
    /// 物理像素坐标 `(x, y, width, height)`。
    pub rect: [f32; 4],
    /// 统一物理像素圆角半径。
    pub radius: f32,
    /// 四角独立圆角半径 `(左上, 右上, 右下, 左下)`。
    pub corner_radii: [f32; 4],
    /// 所属图元所在的图层，保留供未来分层渲染器在对应图层下方精确合成。
    pub layer_key_prefix: u64,
}

impl Default for BackdropRequest {
    fn default() -> Self {
        Self {
            rect: [0.0; 4],
            radius: 0.0,
            corner_radii: [0.0; 4],
            layer_key_prefix: (PassLayer::Overlay as u64) << 56,
        }
    }
}

/// 渲染器执行单个图层 Pass 所需的全部输入数据。
#[derive(Debug)]
pub struct SceneUpload<'a> {
    pub primitives: &'a [GpuPrimitiveInstance],
    pub transforms: &'a [GpuTransform],
    pub clips: &'a [GpuClipNode],
    pub payloads: &'a [GpuPayload],
    /// 当四个数组任意一项变更时由调用方递增。版本号相同时完全跳过上传（使平移零开销）。
    pub generation: u64,
    pub camera: Camera,
    /// 64 位排序键的前缀：`(layer << 56) | (barrier << 40) | (variant << 36)`。
    pub key_prefix: u64,
    /// 自上次上传以来发生变更的图元索引区间。
    ///
    /// 空切片表示“全量上传”；非空切片表示“仅这些图元变更”：渲染器跳过其余场景，
    /// 仅将这些切片以补丁形式写入持久化池，使悬停或滑块拖拽仅消耗数字节上传。
    ///
    /// 区间为半开图元索引。变换、裁剪与载荷不会在此路径重传。
    pub dirty_ranges: &'a [(u32, u32)],
    /// 标记图元池未变但轻量级辅助表已变更，可在局部更新路径中单独暂存而无需全量上传。
    pub aux_changed: bool,
    /// 在主 Pass 之后、呈现之前合成的毛玻璃背景区域。
    pub backdrops: &'a [BackdropRequest],
}

impl Default for SceneUpload<'_> {
    fn default() -> Self {
        Self {
            primitives: &[],
            transforms: &[],
            clips: &[],
            payloads: &[],
            generation: 0,
            camera: Camera::ui((128.0, 128.0), 1.0),
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges: &[],
            aux_changed: false,
            backdrops: &[],
        }
    }
}

/// 逐帧统计计数器，供调试覆盖层展示及 KPI 测试断言。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub submitted: u32,
    pub visible: u32,
    pub draw_calls: u32,
    pub patches: u32,
    pub bytes_uploaded: u32,
    pub sort_dispatches: u32,
    pub compute_dispatches: u32,
}

impl FrameStats {
    /// 主 Pass 发出的间接绘制批次数，KPI 红线为 `<= 4`。
    #[must_use]
    pub fn mdi_draw_calls(&self) -> u32 {
        self.draw_calls
    }
}

/// 离屏池单个槽位持有的驱动图像资源。
struct BlurTexture {
    desc: unbound_graph::TextureDesc,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

/// 背景模糊暂存目标及其校验渲染图。
///
/// [pool](unbound_graph::OffscreenPool) 负责槽位簿记；本结构体拥有 Vulkan 图像。
/// 槽位跨帧保留图像，因此稳态下零资源创建、零描述符写入。
struct BlurTargets {
    pool: unbound_graph::OffscreenPool,
    textures: Vec<Option<BlurTexture>>,
    /// 当前写入每个预分配描述符集的 `(source view, source layout, destination view)` 记录。
    bindings: Vec<Option<(vk::ImageView, vk::ImageLayout, vk::ImageView)>>,
    /// 描述 拷贝 → 模糊 → 合成 流程的已编译渲染图。
    graph: unbound_graph::CompiledGraph,
}

impl BlurTargets {
    fn new() -> Self {
        Self {
            pool: unbound_graph::OffscreenPool::new(),
            textures: Vec::new(),
            bindings: Vec::new(),
            graph: Self::compile_graph(),
        }
    }

    /// 声明背景虚化处理链并由渲染图校验合法性。
    ///
    /// 渲染图仅在初始化时编译一次。
    fn compile_graph() -> unbound_graph::CompiledGraph {
        use unbound_graph::{PassKind, RenderGraph, TextureRole};
        use unbound_graph::{TextureDesc, TextureFormat};

        let mut graph = RenderGraph::new();
        let desc = TextureDesc::new(1, 1, TextureFormat::Rgba8);
        let scene = graph.add_texture(
            desc,
            TextureRole::External, // 场景颜色附件
        );
        let half = graph.add_texture(desc, TextureRole::Transient);
        let full = graph.add_texture(desc, TextureRole::Transient);
        graph.add_pass(
            "blur-down",
            PassKind::Blur { radius: 20 },
            &[scene],
            &[half],
        );
        graph.add_pass("blur-up", PassKind::Blur { radius: 20 }, &[half], &[full]);
        graph.add_pass(
            "backdrop-composite",
            PassKind::Composite,
            &[full, scene],
            &[scene],
        );
        graph.compile().unwrap_or(unbound_graph::CompiledGraph {
            passes: Vec::new(),
            lifetimes: Vec::new(),
        })
    }

    /// 确保 `slot` 持有匹配 `desc` 的图像并返回其视图。
    ///
    /// 第二个返回值若为 `true`，表示图像刚被（重）创建，尚需执行 `UNDEFINED → GENERAL` 布局转换。
    fn ensure(
        &mut self,
        gpu: &Gpu,
        slot: u32,
        desc: unbound_graph::TextureDesc,
    ) -> Result<(vk::ImageView, vk::Image, bool), GpuError> {
        let index = slot as usize;
        if self.textures.len() <= index {
            self.textures.resize_with(index + 1, || None);
        }
        if let Some(texture) = &self.textures[index] {
            if texture.desc == desc {
                return Ok((texture.view, texture.image, false));
            }
        }
        if let Some(old) = self.textures[index].take() {
            // SAFETY: 正在重绑槽位且帧间设备空闲；旧图像归本结构体所有。
            unsafe {
                gpu.device.destroy_image_view(old.view, None);
                gpu.device.destroy_image(old.image, None);
                gpu.device.free_memory(old.memory, None);
            }
        }
        // SAFETY: 仅创建资源；所有权存入对应槽位。
        let (image, memory, view) = unsafe {
            RenderTarget::create_image(
                gpu,
                desc.width,
                desc.height,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
                false,
            )?
        };
        self.textures[index] = Some(BlurTexture {
            desc,
            image,
            memory,
            view,
        });
        Ok((view, image, true))
    }

    /// # Safety
    /// 设备必须处于空闲状态。
    unsafe fn destroy(&mut self, gpu: &Gpu) {
        for texture in self.textures.drain(..).flatten() {
            gpu.device.destroy_image_view(texture.view, None);
            gpu.device.destroy_image(texture.image, None);
            gpu.device.free_memory(texture.memory, None);
        }
    }
}

/// 渲染器主体：管理设备资源与帧执行调度。
pub struct Renderer {
    pub gpu: Gpu,
    pub pipelines: Pipelines,
    pub target: RenderTarget,
    pool: PersistentPool,
    pool_buffer: Buffer,
    ring: TransientRing,
    ring_buffer: Buffer,
    lut_buffer: Buffer,
    /// 启用计时时的 GPU 时间戳作用域。
    timing: Option<crate::timing::TimestampScope>,

    prim_handle: PoolHandle,
    transform_handle: PoolHandle,
    clip_handle: PoolHandle,
    payload_handle: PoolHandle,
    prim_capacity: u64,
    transform_capacity: u64,
    clip_capacity: u64,
    payload_capacity: u64,

    keys_a: Buffer,
    keys_b: Buffer,
    idx_a: Buffer,
    idx_b: Buffer,
    sort_scratch: Buffer,
    commands: Buffer,
    counters: Buffer,
    patches: Buffer,
    staging: Buffer,

    capacity_primitives: u32,
    /// 持久化池当前所处的上传版本号。
    generation: UploadGeneration,
    frame_index: u64,
    pub stats: FrameStats,
    /// 背景模糊暂存目标。
    blur: BlurTargets,
    /// 用于暂存场景字节的复用缓冲区（跨帧保留容量，仅清空内容以实现每帧零分配）。
    staging_bytes: Vec<u8>,
    /// 跨帧复用的暂存字节区间列表。
    staging_spans: Vec<(u32, u32, u32)>,
    /// 跨帧复用的散布补丁列表。
    staging_patches: Vec<GpuScatterPatch>,
    /// 安装后绑定到 Bindless 描述符槽位的图集集合。
    atlases: Option<std::sync::Arc<crate::atlas::AtlasSet>>,
    /// 供调用方注册外部纹理的 Bindless 槽位（`48..64`）簿记器。
    external_slots: ExternalTextureSlots,
    /// 若已设置，则渲染到外部拥有的颜色附件。
    /// 借用的颜色附件：图像句柄、视图与尺寸。
    ///
    /// 必须传入图像句柄，因为仅图像本体可执行布局转换（视图不携带布局信息）。
    external_color: Option<(vk::Image, vk::ImageView, u32, u32)>,
    /// 借用图像是否为交换链图像（渲染后须转换至 `PRESENT_SRC_KHR` 交给呈现引擎）。
    ///
    /// 直接以 `COLOR_ATTACHMENT_OPTIMAL` 呈现会触发 `VUID-VkPresentInfoKHR-pImageIndices-01430`；
    /// 离屏路径与借用附件测试则保留附件布局。
    external_present: bool,
    /// 借用图像就绪时触发的信号量；渲染提交在访问该图像前须等待它。
    external_ready: vk::Semaphore,
    /// 待呈现帧在渲染提交完成时触发的信号量。
    ///
    /// 即当前已获取交换链图像专属的渲染完成信号量（每张图像一个）；
    /// 若为空则回退到 [`Self::present_semaphore`]。
    external_signal: vk::Semaphore,
    /// 本帧渲染完成时触发的信号量，供呈现引擎在显示前等待。
    present_semaphore: vk::Semaphore,
    /// `begin_rendering` 使用的线性清屏色。
    ///
    /// 跟随主题的 `SURFACE` 令牌，确保缩放窗口时浅色 UI 后方不会闪现黑边。
    clear_color: [f32; 4],
}

impl core::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Renderer")
            .field(
                "target",
                &format_args!("{}x{}", self.target.width, self.target.height),
            )
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

#[allow(clippy::too_many_arguments)]
fn image_barrier(
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags2,
    src_access: vk::AccessFlags2,
    dst_stage: vk::PipelineStageFlags2,
    dst_access: vk::AccessFlags2,
    is_depth: bool,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage)
        .src_access_mask(src_access)
        .dst_stage_mask(dst_stage)
        .dst_access_mask(dst_access)
        .old_layout(old)
        .new_layout(new)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(if is_depth {
                    vk::ImageAspectFlags::DEPTH
                } else {
                    vk::ImageAspectFlags::COLOR
                })
                .level_count(1)
                .layer_count(1),
        )
}

impl RenderTarget {
    fn new(gpu: &Gpu, width: u32, height: u32) -> Result<Self, GpuError> {
        // SAFETY: 仅创建资源；所有权移交调用方。
        unsafe {
            let (color, color_memory, color_view) = Self::create_image(
                gpu,
                width,
                height,
                COLOR_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST,
                false,
            )?;
            let (depth, depth_memory, depth_view) = Self::create_image(
                gpu,
                width,
                height,
                DEPTH_FORMAT,
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST,
                true,
            )?;
            Ok(Self {
                width,
                height,
                color,
                color_memory,
                color_view,
                depth,
                depth_memory,
                depth_view,
                depth_alloc: (width.max(1), height.max(1)),
                layout: vk::ImageLayout::UNDEFINED,
            })
        }
    }

    unsafe fn create_image(
        gpu: &Gpu,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        is_depth: bool,
    ) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), GpuError> {
        let device = &gpu.device;
        let image = device.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: width.max(1),
                    height: height.max(1),
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
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
        let view = device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(if is_depth {
                            vk::ImageAspectFlags::DEPTH
                        } else {
                            vk::ImageAspectFlags::COLOR
                        })
                        .level_count(1)
                        .layer_count(1),
                ),
            None,
        )?;
        Ok((image, memory, view))
    }

    /// # Safety
    /// 设备必须处于空闲状态。
    unsafe fn destroy(&self, gpu: &Gpu) {
        let device = &gpu.device;
        device.destroy_image_view(self.color_view, None);
        device.destroy_image(self.color, None);
        device.free_memory(self.color_memory, None);
        device.destroy_image_view(self.depth_view, None);
        device.destroy_image(self.depth, None);
        device.free_memory(self.depth_memory, None);
    }
}

impl Renderer {
    /// 为离屏渲染目标构建全部持久化 GPU 资源。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn new(gpu: Gpu, width: u32, height: u32) -> Result<Self, GpuError> {
        let pipelines = Pipelines::new(&gpu)?;
        let target = RenderTarget::new(&gpu, width, height)?;
        // 标记“帧已完全绘制完成”的二值信号量，供呈现引擎等待。
        let present_semaphore = unsafe {
            gpu.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(GpuError::from)?
        };

        let pool_bytes = gpu.config.pool_capacity;
        let mut pool = PersistentPool::new(pool_bytes as usize);
        let prim_handle = pool
            .allocate(64 * 1024)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        let transform_handle = pool
            .allocate(16 * 1024)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        let clip_handle = pool
            .allocate(16 * 1024)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        let payload_handle = pool
            .allocate(16 * 1024)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        // 容量计数器以字节为单位：须匹配分配器实际切分的容量而非请求大小。
        let prim_capacity = u64::from(pool.capacity_of(prim_handle).unwrap_or(0));
        let transform_capacity = u64::from(pool.capacity_of(transform_handle).unwrap_or(0));
        let clip_capacity = u64::from(pool.capacity_of(clip_handle).unwrap_or(0));
        let payload_capacity = u64::from(pool.capacity_of(payload_handle).unwrap_or(0));

        let bda_usage = vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::TRANSFER_DST
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let pool_buffer =
            gpu.create_buffer(pool_bytes, bda_usage, MemoryKind::DeviceLocal, "pool")?;
        let ring_bytes = gpu.config.ring_capacity;
        let ring_buffer = gpu.create_buffer(
            ring_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            MemoryKind::HostVisible,
            "transient-ring",
        )?;
        let lut_buffer = gpu.create_buffer(
            4096,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            MemoryKind::HostVisible,
            "theme-lut",
        )?;

        // 最坏情况下的图元数量受持久化池与瞬态环容量共同约束。
        let capacity_primitives = (pool_bytes / 64).min(1 << 20) as u32;
        let max = capacity_primitives as u64;
        let keys_a = gpu.create_buffer(max * 8, bda_usage, MemoryKind::DeviceLocal, "keys-a")?;
        let keys_b = gpu.create_buffer(max * 8, bda_usage, MemoryKind::DeviceLocal, "keys-b")?;
        let idx_a = gpu.create_buffer(max * 4, bda_usage, MemoryKind::DeviceLocal, "idx-a")?;
        let idx_b = gpu.create_buffer(max * 4, bda_usage, MemoryKind::DeviceLocal, "idx-b")?;
        let blocks = max.div_ceil(RADIX_BLOCK as u64);
        let sort_scratch = gpu.create_buffer(
            blocks * RADIX_BLOCK as u64 * 4 * 2,
            bda_usage,
            MemoryKind::DeviceLocal,
            "sort-scratch",
        )?;
        let commands = gpu.create_buffer(
            u64::from(MAX_DRAW_RUNS) * 16,
            // `vkCmdDrawIndirectCount` 要求命令缓冲区与计数缓冲区除计算阶段所需的
            // BDA/STORAGE 用途外，还必须具备 `INDIRECT_BUFFER` 用途。
            bda_usage | vk::BufferUsageFlags::INDIRECT_BUFFER,
            MemoryKind::DeviceLocal,
            "indirect",
        )?;
        let counters = gpu.create_buffer(
            64,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::INDIRECT_BUFFER
                // 剔除与间接压缩阶段通过 BDA 寻址该缓冲区；若缺少该用途标志，地址为 0，
                // 着色器写入 NULL 会立即导致设备丢失。
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            MemoryKind::HostVisible,
            "counters",
        )?;
        let patches = gpu.create_buffer(
            64 * 1024 * 16,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::TRANSFER_DST,
            MemoryKind::HostVisible,
            "scatter-patches",
        )?;
        let staging = gpu.create_buffer(
            16 << 20,
            vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST,
            MemoryKind::HostVisible,
            "staging",
        )?;

        // SAFETY: 缓冲区为持久化资源且当前尚无在飞任务。
        unsafe { pipelines.bind_sorted_indices(&gpu, &idx_a) };

        let ring = TransientRing::new(ring_bytes as u32, gpu.frames_in_flight());

        let mut renderer = Self {
            gpu,
            pipelines,
            target,
            pool,
            pool_buffer,
            ring,
            ring_buffer,
            lut_buffer,
            timing: None,
            prim_handle,
            transform_handle,
            clip_handle,
            payload_handle,
            prim_capacity,
            transform_capacity,
            clip_capacity,
            payload_capacity,
            keys_a,
            keys_b,
            idx_a,
            idx_b,
            sort_scratch,
            commands,
            counters,
            patches,
            staging,
            capacity_primitives,
            generation: UploadGeneration::empty(),
            frame_index: 0,
            stats: FrameStats::default(),
            blur: BlurTargets::new(),
            staging_bytes: Vec::new(),
            staging_spans: Vec::new(),
            staging_patches: Vec::new(),
            atlases: None,
            external_slots: ExternalTextureSlots::default(),
            external_color: None,
            external_present: false,
            external_ready: vk::Semaphore::null(),
            external_signal: vk::Semaphore::null(),
            present_semaphore,
            clear_color: [0.07, 0.07, 0.08, 1.0],
        };
        renderer.write_default_lut()?;
        Ok(renderer)
    }

    /// 写入 4 KiB 主题 LUT（参见 `docs/architecture/gpu-renderer.md`）。
    /// 切换主题时仅更新此缓冲区，无需重传图元或重新布局。
    ///
    /// # Errors
    /// 透传写入错误。
    pub fn write_theme(&mut self, colors: &[[u16; 4]]) -> Result<(), GpuError> {
        // 使用栈数组：LUT 固定为 4 KiB，每次切换主题无需堆分配。
        let mut entries = [[0u16; 4]; 512];
        for (i, c) in colors.iter().take(512).enumerate() {
            entries[i] = *c;
        }
        // 硬件清屏在绘制任何图元前执行，无法读取 LUT；因此在线性空间保存表面色用于清屏。
        let surface = entries[unbound_theme::Token::SURFACE.raw() as usize];
        self.clear_color = [
            unbound_math::f16_to_f32(surface[0]),
            unbound_math::f16_to_f32(surface[1]),
            unbound_math::f16_to_f32(surface[2]),
            unbound_math::f16_to_f32(surface[3]).clamp(0.0, 1.0),
        ];
        // 布局：512 × RGBA16F = 4 KiB，每项存储四个 u16。
        // SAFETY: LUT 缓冲区宿主可见且长度为 4096 字节。
        unsafe {
            let ptr = self
                .lut_buffer
                .host_ptr(0)
                .ok_or_else(|| GpuError::Allocation("lut not mapped".into()))?;
            core::ptr::copy_nonoverlapping(entries.as_ptr().cast::<u8>(), ptr, entries.len() * 8);
            // 内存可能非宿主一致，必须在 GPU 读取前刷写，否则着色器可能采样到旧主题。
            self.gpu
                .flush(&self.lut_buffer, 0, (entries.len() * 8) as u64);
        }
        Ok(())
    }

    /// 上传语义化主题（参见 `docs/architecture/gpu-renderer.md`）。
    ///
    /// 主题切换仅需一次 4 KiB 写入：图元引用颜色令牌，由 LUT 在着色器中实时解析。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn write_theme_lut(&mut self, lut: &unbound_theme::ThemeLut) -> Result<(), GpuError> {
        self.write_theme(lut.entries())
    }

    fn write_default_lut(&mut self) -> Result<(), GpuError> {
        use unbound_math::color::Color;
        let mut colors = vec![[0u16; 4]; 512];
        colors[1] = Color::from_srgb8(0, 0, 0, 40).to_rgba16f(); // 全局阴影颜色
        colors[2] = Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
        colors[3] = Color::from_srgb8(24, 24, 27, 255).to_rgba16f();
        colors[4] = Color::from_srgb8(63, 63, 70, 255).to_rgba16f();
        colors[5] = Color::from_srgb8(228, 228, 231, 255).to_rgba16f();
        self.write_theme(&colors)
    }

    /// 持久化池可容纳的图元容量（用于诊断）。
    #[must_use]
    pub fn capacity_primitives(&self) -> u32 {
        self.capacity_primitives
    }

    /// 当前渲染目标尺寸。
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.target.width, self.target.height)
    }

    /// 调整离屏渲染目标及所有依赖图像视图的尺寸。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if width == self.target.width && height == self.target.height {
            return Ok(());
        }
        self.gpu.wait_idle();
        // SAFETY: 设备处于空闲状态。
        unsafe { self.target.destroy(&self.gpu) };
        self.target = RenderTarget::new(&self.gpu, width, height)?;
        Ok(())
    }

    /// 确保深度附件足以覆盖 `width × height`。
    ///
    /// 拖拽缩放窗口时每次 `WM_SIZE` 若都重建深度图会触发 `vkDeviceWaitIdle`；
    /// 改为按 256 px 步长向上增长且不收缩。动态渲染允许附件大于渲染区域。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn ensure_depth_size(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if width <= self.target.depth_alloc.0 && height <= self.target.depth_alloc.1 {
            return Ok(());
        }
        let alloc_w = width.max(self.target.depth_alloc.0).div_ceil(256) * 256;
        let alloc_h = height.max(self.target.depth_alloc.1).div_ceil(256) * 256;
        let gpu = &self.gpu;
        gpu.wait_idle();
        // SAFETY: 设备空闲且旧图像归本结构体所有。
        unsafe {
            let device = &gpu.device;
            device.destroy_image_view(self.target.depth_view, None);
            device.destroy_image(self.target.depth, None);
            device.free_memory(self.target.depth_memory, None);
            let (depth, depth_memory, depth_view) = RenderTarget::create_image(
                gpu,
                alloc_w,
                alloc_h,
                DEPTH_FORMAT,
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST,
                true,
            )?;
            self.target.depth = depth;
            self.target.depth_memory = depth_memory;
            self.target.depth_view = depth_view;
            self.target.depth_alloc = (alloc_w, alloc_h);
        }
        Ok(())
    }

    /// 确保支撑 `which` 数组的池块可容纳 `len` 个元素。
    ///
    /// 比较容量前先按元素步长将元素数换算为字节，防止单位混淆导致越界覆写相邻数组。
    fn ensure_pool_capacity(&mut self, len: usize, which: Which) -> Result<(), GpuError> {
        let handle = match which {
            Which::Primitives => self.prim_handle,
            Which::Transforms => self.transform_handle,
            Which::Clips => self.clip_handle,
            Which::Payloads => self.payload_handle,
        };
        // 退役块需隔离等待的 epoch（即当前帧将触发的时间线值）。
        let epoch = self.pool.current_epoch();
        let (new_handle, capacity) = self
            .pool
            .ensure_capacity(handle, len as u64, which.stride() as u64, epoch)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        let moved = new_handle != handle;
        match which {
            Which::Primitives => {
                self.prim_handle = new_handle;
                self.prim_capacity = u64::from(capacity);
            }
            Which::Transforms => {
                self.transform_handle = new_handle;
                self.transform_capacity = u64::from(capacity);
            }
            Which::Clips => {
                self.clip_handle = new_handle;
                self.clip_capacity = u64::from(capacity);
            }
            Which::Payloads => {
                self.payload_handle = new_handle;
                self.payload_capacity = u64::from(capacity);
            }
        }
        if moved {
            // 数组发生搬迁：强制下一帧执行全量重传。
            self.generation.invalidate();
        }
        Ok(())
    }

    /// 离屏渲染单个图层 Pass 并返回帧统计信息。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含设备丢失）。
    pub fn render(&mut self, scene: &SceneUpload<'_>) -> Result<FrameStats, GpuError> {
        let frame_count = self.gpu.frames_in_flight();
        let frame_slot = (self.frame_index % u64::from(frame_count)) as usize;
        let cmd = self.gpu.frames[frame_slot].command_buffer;
        // 复用命令缓冲区前，上次使用该槽位的帧必须已退役。
        self.wait_for_previous_frame()?;
        {
            let gpu = &self.gpu;
            let sync = &gpu.frames[frame_slot];
            // SAFETY: 围栏、命令缓冲区与队列均属于本渲染器；等待自身围栏不会死锁。
            unsafe {
                gpu.device.reset_fences(&[sync.in_flight])?;
                gpu.device
                    .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            }
        }

        self.stats = FrameStats {
            submitted: scene.primitives.len() as u32,
            ..Default::default()
        };

        // 池的帧生命周期：设置本帧时间线 epoch，并回收 GPU 已越过对应 epoch 的隔离块。
        // 若缺少此步骤，隔离队列永远不会排空并最终导致内存泄漏。
        let timeline_value = self.frame_index + 1;
        self.pool.begin_frame(timeline_value, frame_slot as u32);
        // 上方围栏等待已完成，意味着 `frames_in_flight` 帧前使用该槽位的帧已退役，
        // 可依据该槽位记录的时间线值安全回收已隔离内存块。
        let retired = self.frame_index.saturating_sub(u64::from(frame_count)) + 1;
        self.pool.reclaim(retired);

        let upload = self.prepare_upload(scene)?;
        self.stats.patches = upload.patch_count;
        self.stats.bytes_uploaded = upload.bytes;

        // SAFETY: 在提交前命令缓冲区由本帧独占，且围栏已保证前次提交退役。
        unsafe {
            self.gpu.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        // 首个时间戳标记整帧在 GPU 上的起始时刻。
        if let Some(scope) = &self.timing {
            if self.gpu.features.host_query_reset {
                // SAFETY: 查询池未被使用：此前使用它的帧已退役。
                scope.reset(&self.gpu)?;
            } else {
                // 不支持宿主重置特性时，须在命令缓冲区内录制重置以保证早于后续时间戳写入。
                // SAFETY: 刚开启的自有命令缓冲区。
                unsafe {
                    scope.reset_in_command_buffer(&self.gpu, cmd);
                }
            }
            // SAFETY: 属于本渲染器的命令缓冲区。
            unsafe {
                scope.write_at(&self.gpu, cmd, 0, vk::PipelineStageFlags::TOP_OF_PIPE);
            }
        }
        // `record_frame` 内部的阶段标记负责闭合时间戳区间。
        self.record_frame(cmd, scene, &upload, frame_slot)?;

        {
            let gpu = &self.gpu;
            let sync = &gpu.frames[frame_slot];
            // SAFETY: 命令缓冲区刚结束录制且仅提交一次。
            unsafe {
                gpu.device.end_command_buffer(cmd)?;
                // 离屏路径无 `image_available` 等待；仅在需要呈现时才触发二值呈现信号量。
                // 使用 `vkQueueSubmit2` 以在同一提交中同时触发时间线信号量与二值呈现信号量。
                let mut signal_infos = [
                    // 时间线信号量携带本帧时间线值；二值信号量忽略 value 字段。
                    vk::SemaphoreSubmitInfo::default()
                        .semaphore(gpu.timeline.semaphore)
                        .value(timeline_value),
                    vk::SemaphoreSubmitInfo::default(),
                ];
                let signal_count = if self.external_present {
                    // 触发当前已获取交换链图像专属的信号量，避免与未完成呈现产生信号量复用冲突。
                    let signal_sem = if self.external_signal != vk::Semaphore::null() {
                        self.external_signal
                    } else {
                        self.present_semaphore
                    };
                    signal_infos[1] = vk::SemaphoreSubmitInfo::default()
                        .semaphore(signal_sem)
                        .value(0);
                    2
                } else {
                    1
                };
                // 若为借用的交换链图像，须等待呈现引擎交出所有权。等待数组置于栈上，零堆分配。
                let wait_info = vk::SemaphoreSubmitInfo::default()
                    .semaphore(self.external_ready)
                    .value(0)
                    .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
                let wait_infos: [vk::SemaphoreSubmitInfo; 1] =
                    if self.external_ready == vk::Semaphore::null() {
                        [vk::SemaphoreSubmitInfo::default()]
                    } else {
                        [wait_info]
                    };
                let wait_count = u32::from(self.external_ready != vk::Semaphore::null());
                let command_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
                let submit = vk::SubmitInfo2::default()
                    .command_buffer_infos(&command_infos)
                    .wait_semaphore_infos(&wait_infos[..wait_count as usize])
                    .signal_semaphore_infos(&signal_infos[..signal_count]);
                gpu.device
                    .queue_submit2(gpu.graphics_queue, &[submit], sync.in_flight)?;
            }
        }
        // 帧已成功提交至 GPU：持久化池现已持有该版本场景，可确认提交版本号。
        self.generation.commit();
        self.frame_index += 1;
        Ok(self.stats)
    }

    /// 重置可见性计数器并打包解析后的 BDA 帧推送常量。
    fn record_frame(
        &mut self,
        cmd: vk::CommandBuffer,
        scene: &SceneUpload<'_>,
        upload: &UploadPlan,
        _frame_slot: usize,
    ) -> Result<(), GpuError> {
        let gpu = &self.gpu;
        let device = &gpu.device;

        // ---- 清零计数器 -----------------------------------------------------
        // SAFETY: 自有命令缓冲区，向 64 字节缓冲区写入 64 字节。
        unsafe { device.cmd_fill_buffer(cmd, self.counters.raw, 0, 64, 0) };

        // ---- 1/2. 增量散布更新 -----------------------------------------------
        if upload.patch_count > 0 {
            let mut pc = PushConstants {
                prim_count: upload.patch_count,
                scratch_bda: self.patches.address,
                scratch_bda2: self.pool_buffer.address,
                scratch_bda3: self.ring_buffer.address,
                ..Default::default()
            };
            // 每个补丁一个工作组：着色器用全部 64 个通道并行拷贝。
            self.dispatch(
                cmd,
                crate::pipeline::ComputeStage::Scatter,
                &mut pc,
                upload.patch_count,
            );
            self.stats.compute_dispatches += 1;
            let barrier = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(
                    vk::PipelineStageFlags2::COMPUTE_SHADER
                        | vk::PipelineStageFlags2::VERTEX_SHADER,
                )
                .dst_access_mask(vk::AccessFlags2::SHADER_READ);
            let barriers = [barrier];
            // SAFETY: 向自有命令缓冲区录制命令。
            unsafe {
                device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&barriers),
                )
            };
        }

        self.mark(cmd, 1, vk::PipelineStageFlags::COMPUTE_SHADER);

        // ---- 3. 视口剔除 -----------------------------------------------------
        let n = scene.primitives.len() as u32;
        // 未使用槽位预填充最大键值使其排到末尾（供 `indirect.comp` 哨兵终止）；
        // 必须在剔除 Pass 之前执行，剔除 Pass 会覆写 `[0, visible)` 槽位。
        self.clear_keys_tail(cmd, n)?;
        let barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE);
        let barriers = [barrier];
        // SAFETY: 向自有命令缓冲区录制命令。
        unsafe {
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&barriers),
            )
        };
        let mut pc = PushConstants {
            frame: self.frame_constants(scene),
            prim_count: n,
            scratch_bda: self.keys_a.address,
            scratch_bda2: self.counters.address,
            scratch_bda3: self.idx_a.address,
            scratch_bda4: scene.key_prefix,
            ..Default::default()
        };
        self.dispatch(
            cmd,
            crate::pipeline::ComputeStage::Cull,
            &mut pc,
            n.div_ceil(256),
        );
        self.stats.compute_dispatches += 1;

        self.mark(cmd, 2, vk::PipelineStageFlags::COMPUTE_SHADER);

        // ---- 4. 基数排序 -----------------------------------------------------
        if n > 0 {
            let barrier = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE);
            let barriers = [barrier];
            // SAFETY: 向自有命令缓冲区录制命令。
            unsafe {
                device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&barriers),
                )
            };

            let blocks = n.div_ceil(RADIX_BLOCK);
            for pass in 0..RADIX_PASSES {
                let (src_keys, dst_keys, src_idx, dst_idx) = if pass % 2 == 0 {
                    (&self.keys_a, &self.keys_b, &self.idx_a, &self.idx_b)
                } else {
                    (&self.keys_b, &self.keys_a, &self.idx_b, &self.idx_a)
                };
                let shift = pass * 8;

                let mut hist = PushConstants {
                    prim_count: n,
                    digit: shift,
                    block_count: blocks,
                    scratch_bda: src_keys.address,
                    scratch_bda5: self.sort_scratch.address,
                    ..Default::default()
                };
                self.dispatch(
                    cmd,
                    crate::pipeline::ComputeStage::SortHist,
                    &mut hist,
                    blocks,
                );
                self.pipeline_barrier(cmd);

                let mut scan = PushConstants {
                    prim_count: n,
                    digit: shift,
                    block_count: blocks,
                    scratch_bda5: self.sort_scratch.address,
                    ..Default::default()
                };
                self.dispatch(cmd, crate::pipeline::ComputeStage::SortScan, &mut scan, 1);
                self.pipeline_barrier(cmd);

                let mut scatter = PushConstants {
                    prim_count: n,
                    digit: shift,
                    block_count: blocks,
                    scratch_bda: src_keys.address,
                    scratch_bda2: dst_keys.address,
                    scratch_bda3: src_idx.address,
                    scratch_bda4: dst_idx.address,
                    scratch_bda5: self.sort_scratch.address,
                    ..Default::default()
                };
                self.dispatch(
                    cmd,
                    crate::pipeline::ComputeStage::SortScatter,
                    &mut scatter,
                    blocks,
                );
                self.stats.sort_dispatches += 3;
                self.stats.compute_dispatches += 3;
                self.pipeline_barrier(cmd);
            }
        }

        self.mark(cmd, 3, vk::PipelineStageFlags::COMPUTE_SHADER);

        // ---- 5. 间接命令压缩 -------------------------------------------------
        let mut pc = PushConstants {
            prim_count: n,
            aux: MAX_DRAW_RUNS,
            scratch_bda: self.keys_a.address,
            scratch_bda2: self.commands.address,
            scratch_bda3: self.counters.address,
            ..Default::default()
        };
        self.dispatch(cmd, crate::pipeline::ComputeStage::Indirect, &mut pc, 1);
        self.stats.compute_dispatches += 1;

        // 计算阶段写入了绘制命令与批次数；必须通过屏障显式对 `DRAW_INDIRECT` 阶段可见，
        // 仅用 COMPUTE→COMPUTE 屏障会导致绘制零个实例。
        {
            let barrier = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(
                    vk::PipelineStageFlags2::DRAW_INDIRECT | vk::PipelineStageFlags2::VERTEX_SHADER,
                )
                .dst_access_mask(
                    vk::AccessFlags2::INDIRECT_COMMAND_READ | vk::AccessFlags2::SHADER_READ,
                );
            let barriers = [barrier];
            // SAFETY: 向自有命令缓冲区录制命令。
            unsafe {
                device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&barriers),
                )
            };
        }

        self.mark(cmd, 4, vk::PipelineStageFlags::DRAW_INDIRECT);

        // ---- 6/7. 图形渲染 ---------------------------------------------------
        self.begin_rendering(cmd);
        self.record_graphics(cmd, scene);
        self.end_rendering(cmd);

        // ---- 8. 毛玻璃背景虚化 -----------------------------------------------
        // 模糊 Pass 采样颜色附件，因此在主 Pass 完成后、图像交付呈现前执行。
        self.record_backdrop(cmd, scene)?;

        // 交换链图像必须以 `PRESENT_SRC_KHR` 布局交给呈现引擎，否则 `vkQueuePresentKHR` 会报错。
        if let (true, Some((image, _, _, _))) = (self.external_present, self.external_color) {
            let to_present = [image_barrier(
                image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::PRESENT_SRC_KHR,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                vk::AccessFlags2::NONE,
                false,
            )];
            // SAFETY: 向自有命令缓冲区录制；图像刚绘制完成且在呈现前由本帧独占。
            unsafe {
                self.gpu.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                );
            }
        }

        self.mark(cmd, 5, vk::PipelineStageFlags::BOTTOM_OF_PIPE);

        Ok(())
    }

    /// 背景虚化池累计创建的图像数量（稳态帧复用全部槽位，该计数不再增长）。
    #[must_use]
    pub fn blur_pool_creation_count(&self) -> u64 {
        self.blur.pool.creation_count()
    }

    /// 为每个请求执行背景虚化链：采样、模糊、合成。
    fn record_backdrop(
        &mut self,
        cmd: vk::CommandBuffer,
        scene: &SceneUpload<'_>,
    ) -> Result<(), GpuError> {
        if scene.backdrops.is_empty() {
            return Ok(());
        }
        debug_assert!(
            !self.blur.graph.passes.is_empty(),
            "the backdrop render graph must compile"
        );
        let (width, height, color_image, color_view) = match self.external_color {
            Some((image, view, w, h)) => (w, h, image, view),
            None => (
                self.target.width,
                self.target.height,
                self.target.color,
                self.target.color_view,
            ),
        };
        if width == 0 || height == 0 {
            return Ok(());
        }
        for request in scene.backdrops {
            self.record_one_backdrop(cmd, request, width, height, color_image, color_view)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_one_backdrop(
        &mut self,
        cmd: vk::CommandBuffer,
        request: &BackdropRequest,
        width: u32,
        height: u32,
        color_image: vk::Image,
        color_view: vk::ImageView,
    ) -> Result<(), GpuError> {
        use unbound_graph::TextureFormat;

        let radius = request.radius.max(1.0);
        let two_levels = radius > 8.0;

        // 场景颜色附件转换为着色器只读采样源。
        let to_sample = [image_barrier(
            color_image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            vk::PipelineStageFlags2::COMPUTE_SHADER | vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::SHADER_READ,
            false,
        )];
        // SAFETY: 向自有命令缓冲区录制命令。
        unsafe {
            self.gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_sample),
            );
        }
        if self.external_color.is_none() {
            self.target.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        }

        let half_desc = unbound_graph::TextureDesc::new(
            (width / 2).max(1),
            (height / 2).max(1),
            TextureFormat::Rgba8,
        );
        let full_desc =
            unbound_graph::TextureDesc::new(width.max(1), height.max(1), TextureFormat::Rgba8);
        let half = self.blur.pool.acquire(half_desc);
        let (half_view, half_image, half_new) =
            self.blur.ensure(&self.gpu, half.slot, half_desc)?;

        let mut quarter_handle = None;
        let (quarter_view, quarter_image, quarter_new) = if two_levels {
            let quarter_desc = unbound_graph::TextureDesc::new(
                (width / 4).max(1),
                (height / 4).max(1),
                TextureFormat::Rgba8,
            );
            let quarter = self.blur.pool.acquire(quarter_desc);
            quarter_handle = Some(quarter);
            let (view, image, new) = self.blur.ensure(&self.gpu, quarter.slot, quarter_desc)?;
            (view, image, new)
        } else {
            (vk::ImageView::null(), vk::Image::null(), false)
        };

        let full = self.blur.pool.acquire(full_desc);
        let (full_view, full_image, full_new) =
            self.blur.ensure(&self.gpu, full.slot, full_desc)?;

        if half_new {
            self.transition_blur_image(cmd, half_image);
        }
        if quarter_new {
            self.transition_blur_image(cmd, quarter_image);
        }
        if full_new {
            self.transition_blur_image(cmd, full_image);
        }

        let scene_size = (width, height);
        let half_size = (half_desc.width, half_desc.height);
        let quarter_size = ((width / 4).max(1), (height / 4).max(1));

        // 执行三轮 Dual-Kawase 链路：多轮叠加将高频棋盘格响应衰减至接近零。
        let mut source_view = color_view;
        let mut source_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        for round in 0..3usize {
            // 每轮独占 4 个描述符集，防止后续轮次更新覆盖前轮已录制的描述符。
            let base = round * 4;
            self.blur_pass(
                cmd,
                base,
                self.pipelines.blur_down,
                source_view,
                source_layout,
                half_view,
                half_size,
                scene_size,
                radius,
                0,
            );
            if two_levels {
                self.blur_pass(
                    cmd,
                    base + 1,
                    self.pipelines.blur_down,
                    half_view,
                    vk::ImageLayout::GENERAL,
                    quarter_view,
                    quarter_size,
                    half_size,
                    radius,
                    1,
                );
                self.blur_pass(
                    cmd,
                    base + 2,
                    self.pipelines.blur_up,
                    quarter_view,
                    vk::ImageLayout::GENERAL,
                    half_view,
                    half_size,
                    quarter_size,
                    radius,
                    1,
                );
                self.blur_pass(
                    cmd,
                    base + 3,
                    self.pipelines.blur_up,
                    half_view,
                    vk::ImageLayout::GENERAL,
                    full_view,
                    scene_size,
                    half_size,
                    radius,
                    0,
                );
            } else {
                self.blur_pass(
                    cmd,
                    base + 1,
                    self.pipelines.blur_up,
                    half_view,
                    vk::ImageLayout::GENERAL,
                    full_view,
                    scene_size,
                    half_size,
                    radius,
                    0,
                );
            }
            source_view = full_view;
            source_layout = vk::ImageLayout::GENERAL;
        }

        // 合成 Pass 采样最后一轮模糊的输出，因此其专属描述符集（槽位 8）指向全分辨率结果。
        self.bind_blur_binding(8, full_view, vk::ImageLayout::GENERAL, full_view);
        self.composite_backdrop(cmd, request, width, height, color_image, color_view, 8);

        self.blur.pool.release(half);
        if let Some(quarter) = quarter_handle {
            self.blur.pool.release(quarter);
        }
        self.blur.pool.release(full);
        Ok(())
    }

    /// 将新建模糊图像转换到计算着色器可读写的 `GENERAL` 布局。
    fn transition_blur_image(&self, cmd: vk::CommandBuffer, image: vk::Image) {
        let ready = [image_barrier(
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::GENERAL,
            vk::PipelineStageFlags2::TOP_OF_PIPE,
            vk::AccessFlags2::NONE,
            vk::PipelineStageFlags2::COMPUTE_SHADER,
            vk::AccessFlags2::SHADER_WRITE,
            false,
        )];
        // SAFETY: 向自有命令缓冲区录制命令。
        unsafe {
            self.gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&ready),
            );
        }
    }

    /// 将预分配描述符集指向源/目标图像对，仅当绑定发生变化时才写入描述符。
    fn bind_blur_binding(
        &mut self,
        set_index: usize,
        source_view: vk::ImageView,
        source_layout: vk::ImageLayout,
        dest_view: vk::ImageView,
    ) {
        if self.blur.bindings.len() <= set_index {
            self.blur.bindings.resize(set_index + 1, None);
        }
        let binding = (source_view, source_layout, dest_view);
        if self.blur.bindings[set_index] == Some(binding) {
            return;
        }
        // SAFETY: 录制前渲染器已等待该帧槽位退役，故该描述符集不在使用中。
        unsafe {
            self.pipelines.update_blur_set(
                &self.gpu,
                set_index,
                source_view,
                source_layout,
                dest_view,
            );
        }
        self.blur.bindings[set_index] = Some(binding);
    }

    /// 调度一步 Dual-Kawase 计算并插入屏障使写入对下一读取者可见。
    #[allow(clippy::too_many_arguments)]
    fn blur_pass(
        &mut self,
        cmd: vk::CommandBuffer,
        set_index: usize,
        pipeline: vk::Pipeline,
        source_view: vk::ImageView,
        source_layout: vk::ImageLayout,
        dest_view: vk::ImageView,
        dest_size: (u32, u32),
        _source_size: (u32, u32),
        radius: f32,
        iteration: u32,
    ) {
        self.bind_blur_binding(set_index, source_view, source_layout, dest_view);

        let push = BlurPush {
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            dst_size: [dest_size.0 as f32, dest_size.1 as f32],
            radius,
            iteration,
        };
        // SAFETY: 命令缓冲区处于录制状态；管线、布局与描述符集均归本渲染器所有。
        unsafe {
            let device = &self.gpu.device;
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipelines.blur_layout,
                0,
                &[self.pipelines.blur_sets[set_index]],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                self.pipelines.blur_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&push),
            );
            device.cmd_dispatch(
                cmd,
                dest_size.0.div_ceil(8).max(1),
                dest_size.1.div_ceil(8).max(1),
                1,
            );
        }
        self.stats.compute_dispatches += 1;

        let visible = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
            .dst_stage_mask(
                vk::PipelineStageFlags2::COMPUTE_SHADER | vk::PipelineStageFlags2::FRAGMENT_SHADER,
            )
            .dst_access_mask(vk::AccessFlags2::SHADER_READ)];
        // SAFETY: 向自有命令缓冲区录制命令。
        unsafe {
            self.gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&visible),
            );
        }
    }

    /// 将模糊后的全分辨率纹理合成回目标矩形区域。
    #[allow(clippy::too_many_arguments)]
    fn composite_backdrop(
        &mut self,
        cmd: vk::CommandBuffer,
        request: &BackdropRequest,
        width: u32,
        height: u32,
        color_image: vk::Image,
        color_view: vk::ImageView,
        descriptor_set: usize,
    ) {
        let radii = if request.corner_radii == [0.0; 4] {
            [request.radius; 4]
        } else {
            request.corner_radii
        };
        let push = BackdropCompositePush {
            rect: request.rect,
            radii,
        };
        let to_attachment = [image_barrier(
            color_image,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::SHADER_READ,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            false,
        )];
        // SAFETY: 处于录制状态且目标图像由本帧独占。
        unsafe {
            let device = &self.gpu.device;
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
            );
            let color_attachment = [vk::RenderingAttachmentInfo::default()
                .image_view(color_view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)];
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            };
            device.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(area)
                    .layer_count(1)
                    .color_attachments(&color_attachment),
            );
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipelines.backdrop,
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipelines.blur_layout,
                0,
                &[self.pipelines.blur_sets[descriptor_set]],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                self.pipelines.blur_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            device.cmd_set_viewport(cmd, 0, &[viewport]);
            device.cmd_set_scissor(cmd, 0, &[area]);
            device.cmd_draw(cmd, 3, 1, 0, 0);
            device.cmd_end_rendering(cmd);
        }
        self.stats.draw_calls += 1;
        if self.external_color.is_none() {
            self.target.layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
        }
    }

    /// 为后续渲染帧启用 GPU 时间戳计时。
    ///
    /// # Errors
    /// 创建查询池失败时返回 [`GpuError::Vulkan`]。
    /// 安装真实的 Bindless 图集。
    ///
    /// 调用前所有槽位指向 1x1 占位纹理；调用后槽位 `n` 指向图集第 `n` 层。
    ///
    /// # Errors
    /// 透传分配或描述符更新错误。
    pub fn install_atlases(
        &mut self,
        atlases: std::sync::Arc<crate::atlas::AtlasSet>,
    ) -> Result<(), GpuError> {
        self.pipelines.install_atlases(&self.gpu, atlases.clone())?;
        self.atlases = Some(atlases);
        Ok(())
    }

    /// 帧绘制完成时触发的信号量，供呈现引擎在显示前等待。
    #[must_use]
    pub const fn present_semaphore(&self) -> vk::Semaphore {
        self.present_semaphore
    }

    /// 返回已安装到渲染器的图集引用。
    #[must_use]
    pub fn atlases(&self) -> Option<&std::sync::Arc<crate::atlas::AtlasSet>> {
        self.atlases.as_ref()
    }

    /// 将调用方拥有的外部纹理注册到 Bindless 槽位。
    ///
    /// 返回图层 ID（存入 [`GpuPrimitiveInstance::border_token_or_tex`]）。
    /// 调用方须在调用 [`Self::release_external_texture`] 前保持 `view` 与 `sampler` 存活，
    /// 且纹理保持 `SHADER_READ_ONLY_OPTIMAL` 布局。
    ///
    /// # Errors
    /// 16 个外部槽位已满时返回 [`GpuError::InvalidArgument`]。
    pub fn register_external_texture(
        &mut self,
        view: vk::ImageView,
        sampler: vk::Sampler,
    ) -> Result<u32, GpuError> {
        let slot = self.external_slots.allocate().ok_or_else(|| {
            GpuError::InvalidArgument(format!(
                "all {} external bindless slots are in use",
                crate::pipeline::EXTERNAL_TEXTURE_SLOTS
            ))
        })?;
        if let Err(error) = self
            .pipelines
            .bind_external_texture(&self.gpu, slot, view, sampler)
        {
            self.external_slots.release(slot);
            return Err(error);
        }
        Ok(slot)
    }

    /// 释放外部槽位并重绑回 1×1 占位纹理。
    ///
    /// # Errors
    /// 若 `slot` 非有效外部槽位则返回 [`GpuError::InvalidArgument`]。
    pub fn release_external_texture(&mut self, slot: u32) -> Result<(), GpuError> {
        if !self.external_slots.release(slot) {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not a live external texture slot"
            )));
        }
        self.pipelines.unbind_external_texture(&self.gpu, slot)
    }

    /// 当前已注册的外部纹理槽位数量。
    #[must_use]
    pub const fn external_texture_count(&self) -> u32 {
        self.external_slots.used()
    }

    /// 等待上一次使用当前帧槽位的帧退役。
    ///
    /// 窗口化路径在获取下一张交换链图像前**必须**调用此方法，以确保对应的
    /// image-available 二值信号量已被前帧消费完毕。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn wait_for_previous_frame(&mut self) -> Result<(), GpuError> {
        let frame_count = self.gpu.frames_in_flight();
        let frame_slot = (self.frame_index % u64::from(frame_count)) as usize;
        let fence = self.gpu.frames[frame_slot].in_flight;
        // SAFETY: 围栏归本渲染器所有；等待自身围栏不会死锁。
        unsafe {
            self.gpu
                .device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(GpuError::from)?;
        }
        Ok(())
    }

    /// 将单帧渲染到外部拥有的颜色附件（格式须匹配 `COLOR_FORMAT`）。
    ///
    /// 渲染器保留自有深度缓冲区并借用颜色附件；提交会等待 `image_available` 信号量。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn render_to_view(
        &mut self,
        scene: &SceneUpload<'_>,
        color_image: vk::Image,
        color_view: vk::ImageView,
        width: u32,
        height: u32,
        image_available: vk::Semaphore,
    ) -> Result<FrameStats, GpuError> {
        self.render_external(
            scene,
            color_image,
            color_view,
            width,
            height,
            image_available,
            vk::Semaphore::null(),
            false,
        )
    }

    /// 渲染单帧到交换链图像并转换至 `VK_IMAGE_LAYOUT_PRESENT_SRC_KHR` 以便呈现。
    ///
    /// **建议优先使用 [`Self::render_into_swapchain`]**：本入口回退使用全局信号量，
    /// 在多缓冲呈现下可能触发 `VUID-vkQueueSubmit2-semaphore-03868`。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn render_to_swapchain(
        &mut self,
        scene: &SceneUpload<'_>,
        color_image: vk::Image,
        color_view: vk::ImageView,
        width: u32,
        height: u32,
        image_available: vk::Semaphore,
    ) -> Result<FrameStats, GpuError> {
        self.render_external(
            scene,
            color_image,
            color_view,
            width,
            height,
            image_available,
            vk::Semaphore::null(),
            true,
        )
    }

    /// 将单帧渲染到已获取的交换链图像中（安全的窗口化入口）。
    ///
    /// 自动使用该图像专属的 `image_available` 与 `render_finished` 信号量，
    /// 彻底避免 `VUID-vkQueueSubmit2-semaphore-03868`。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn render_into_swapchain(
        &mut self,
        scene: &SceneUpload<'_>,
        acquired: &crate::surface::AcquiredImage,
        width: u32,
        height: u32,
    ) -> Result<FrameStats, GpuError> {
        self.render_external(
            scene,
            acquired.image,
            acquired.view,
            width,
            height,
            acquired.available,
            acquired.render_finished,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_external(
        &mut self,
        scene: &SceneUpload<'_>,
        color_image: vk::Image,
        color_view: vk::ImageView,
        width: u32,
        height: u32,
        image_available: vk::Semaphore,
        render_finished: vk::Semaphore,
        present: bool,
    ) -> Result<FrameStats, GpuError> {
        // 借用的颜色图像决定 `renderArea`，因此自建深度附件必须覆盖该范围。
        // 过去窗口在 `Renderer::new` 之后变大时，深度图像仍停留在*初始*尺寸，
        // 属于非法用法（`VUID-VkRenderingInfo-pNext-06079/06080`），在会校验
        // 边界的驱动上还可能丢失设备。此处统一扩容，不依赖每个调用方记得处理。
        self.ensure_depth_size(width, height)?;
        self.external_color = Some((color_image, color_view, width, height));
        self.external_ready = image_available;
        self.external_signal = render_finished;
        self.external_present = present;
        let result = self.render(scene);
        self.external_color = None;
        self.external_ready = vk::Semaphore::null();
        self.external_signal = vk::Semaphore::null();
        self.external_present = false;
        result
    }

    pub fn enable_timing(&mut self) -> Result<(), GpuError> {
        let scope = crate::timing::TimestampScope::new(&self.gpu, STAGE_MARKS)?;
        self.timing = Some(scope);
        Ok(())
    }

    /// 向命令缓冲区写入阶段时间戳标记。
    fn mark(&self, cmd: vk::CommandBuffer, index: u32, stage: vk::PipelineStageFlags) {
        if let Some(scope) = &self.timing {
            // SAFETY: 属于本渲染器的命令缓冲区。
            unsafe { scope.write_at(&self.gpu, cmd, index, stage) };
        }
    }

    /// 获取上一帧的 GPU 总耗时（毫秒）。会等待帧退役，勿在渲染循环热路径调用。
    ///
    /// # Errors
    /// 无法读取时间戳时返回 [`GpuError::Vulkan`]。
    pub fn read_gpu_ms(&mut self) -> Result<Option<f32>, GpuError> {
        if self.timing.is_none() {
            return Ok(None);
        }
        {
            let frame_count = self.gpu.frames_in_flight();
            let frame_slot = ((self.frame_index - 1) % u64::from(frame_count)) as usize;
            let fence = self.gpu.frames[frame_slot].in_flight;
            // SAFETY: 等待自有围栏不会死锁。
            unsafe {
                self.gpu.device.wait_for_fences(&[fence], true, u64::MAX)?;
            }
        }
        let scope = self.timing.as_mut().expect("the scope exists");
        scope.read(&self.gpu)?;
        Ok(scope.elapsed_ms(0, 1))
    }

    /// 获取上一帧按阶段拆分的 GPU 耗时。会等待帧退役，勿在渲染循环热路径调用。
    ///
    /// # Errors
    /// 无法读取时间戳时返回 [`GpuError::Vulkan`]。
    pub fn read_frame_timing(&mut self) -> Result<Option<crate::timing::FrameTiming>, GpuError> {
        if self.timing.is_none() {
            return Ok(None);
        }
        {
            let frame_count = self.gpu.frames_in_flight();
            let frame_slot = ((self.frame_index - 1) % u64::from(frame_count)) as usize;
            let fence = self.gpu.frames[frame_slot].in_flight;
            // SAFETY: 等待自有围栏不会死锁。
            unsafe {
                self.gpu.device.wait_for_fences(&[fence], true, u64::MAX)?;
            }
        }
        // 帧已退役，计数器亦已最终落盘。
        self.refresh_counters();
        let scope = self.timing.as_mut().expect("the scope exists");
        scope.read(&self.gpu)?;
        Ok(Some(scope.frame_timing()))
    }

    /// 上一帧的统计信息（须在帧退役并调用 [`Self::read_frame_timing`] 或 [`Self::read_pixels`] 后读取）。
    #[must_use]
    pub fn last_stats(&self) -> FrameStats {
        self.stats
    }

    /// 将 GPU 写入的帧计数器拷贝到统计结构体（仅在帧退役后有效）。
    fn refresh_counters(&mut self) {
        // SAFETY: 缓冲区在渲染器全生命周期内保持映射，且当前帧已退役。
        let counters =
            unsafe { core::slice::from_raw_parts(self.counters.mapped.cast::<u32>(), 4) };
        self.stats.visible = counters[2];
        self.stats.draw_calls = counters[1];
    }

    fn pipeline_barrier(&self, cmd: vk::CommandBuffer) {
        let barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE);
        let barriers = [barrier];
        // SAFETY: 向自有命令缓冲区录制命令。
        unsafe {
            self.gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&barriers),
            );
        }
    }

    fn dispatch(
        &self,
        cmd: vk::CommandBuffer,
        stage: crate::pipeline::ComputeStage,
        pc: &mut PushConstants,
        groups: u32,
    ) {
        let groups = groups.max(1);
        // SAFETY: 向自有命令缓冲区录制；`pc` 为 128B。
        unsafe {
            let device = &self.gpu.device;
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipelines.compute[stage as usize],
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipelines.compute_layout,
                0,
                &[self.pipelines.set],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                self.pipelines.compute_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(pc),
            );
            device.cmd_dispatch(cmd, groups, 1, 1);
        }
    }

    fn frame_constants(&self, scene: &SceneUpload<'_>) -> FramePushConstants {
        FramePushConstants {
            primitives_bda: self.pool_buffer.address
                + u64::from(self.pool_offset(self.prim_handle)),
            transforms_bda: self.pool_buffer.address
                + u64::from(self.pool_offset(self.transform_handle)),
            clips_bda: self.pool_buffer.address + u64::from(self.pool_offset(self.clip_handle)),
            theme_lut_bda: self.lut_buffer.address,
            payloads_bda: self.pool_buffer.address
                + u64::from(self.pool_offset(self.payload_handle)),
            camera_chunk: scene.camera.chunk,
            camera_local: scene.camera.local,
            // 视口尺寸跟随实际绘制的目标附件。
            viewport_size: match self.external_color {
                Some((_, _, w, h)) => [w as f32, h as f32],
                None => [self.target.width as f32, self.target.height as f32],
            },
            camera_zoom: scene.camera.zoom,
            dpi_scale: scene.camera.dpi_scale,
        }
    }

    fn pool_offset(&self, handle: PoolHandle) -> u32 {
        self.pool.offset_of(handle).unwrap_or(0)
    }

    fn clear_keys_tail(&self, cmd: vk::CommandBuffer, n: u32) -> Result<(), GpuError> {
        // 键按升序排列，未用槽位须填充 `0xFFFF…` 使其排至末尾；`vkCmdFillBuffer`
        // 按 32 位模式填充，写入 `0xFFFFFFFF` 即对应最大 `u64` 键值。
        let bytes = u64::from(n) * 8;
        // SAFETY: 填充范围位于缓冲区内（capacity_primitives >= n）。
        unsafe {
            self.gpu
                .device
                .cmd_fill_buffer(cmd, self.keys_a.raw, 0, bytes, u32::MAX);
            self.gpu
                .device
                .cmd_fill_buffer(cmd, self.idx_a.raw, 0, u64::from(n) * 4, 0);
        }
        Ok(())
    }

    fn begin_rendering(&mut self, cmd: vk::CommandBuffer) {
        let device = &self.gpu.device;
        // SAFETY: 处于录制状态且目标图像由本帧独占。
        unsafe {
            // 颜色附件始末均需布局转换；借用的交换链图像初始为 `UNDEFINED`。
            // 屏障结构体置于栈上，零堆分配。
            let to_attachment = [match self.external_color {
                Some((image, _, _, _)) => image_barrier(
                    image,
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    vk::AccessFlags2::NONE,
                    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                    false,
                ),
                None => image_barrier(
                    self.target.color,
                    self.target.layout,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    vk::AccessFlags2::NONE,
                    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                    false,
                ),
            }];
            let depth_ready = [image_barrier(
                self.target.depth,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE,
                vk::AccessFlags2::NONE,
                vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
                vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
                    | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ,
                true,
            )];
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
            );
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&depth_ready),
            );
            if self.external_color.is_none() {
                self.target.layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            }

            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: self.clear_color,
                },
            }];
            let depth_clear = vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    // 反向 Z：远平面为 0。
                    depth: 0.0,
                    stencil: 0,
                },
            };
            // 颜色附件可能借用自交换链图像；深度附件始终归本渲染器所有。
            let color_view = match self.external_color {
                Some((_, view, _, _)) => view,
                None => self.target.color_view,
            };
            // 渲染区域跟随借用附件尺寸，避免交换链缩放竞态期间裁剪或越界。
            let (area_width, area_height) = match self.external_color {
                Some((_, _, w, h)) => (w, h),
                None => (self.target.width, self.target.height),
            };
            let color_attachment = [vk::RenderingAttachmentInfo::default()
                .image_view(color_view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(clear[0])];
            let depth_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(self.target.depth_view)
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(depth_clear);
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: area_width,
                    height: area_height,
                },
            };
            device.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(area)
                    .layer_count(1)
                    .color_attachments(&color_attachment)
                    .depth_attachment(&depth_attachment),
            );
        }
    }

    fn end_rendering(&mut self, cmd: vk::CommandBuffer) {
        // SAFETY: 与 `begin_rendering` 严格配对。
        unsafe {
            self.gpu.device.cmd_end_rendering(cmd);
        }
    }

    fn record_graphics(&mut self, cmd: vk::CommandBuffer, scene: &SceneUpload<'_>) {
        let device = &self.gpu.device;
        // 视口尺寸跟随实际绘制的目标附件。
        let (width, height) = match self.external_color {
            Some((_, _, w, h)) => (w, h),
            None => (self.target.width, self.target.height),
        };
        let extent = vk::Extent2D { width, height };
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        };
        let pc = PushConstants::from_frame(self.frame_constants(scene));

        // SAFETY: 处于录制状态；所有句柄均存活且归 `self` 所有。
        unsafe {
            device.cmd_set_viewport(cmd, 0, &[viewport]);
            device.cmd_set_scissor(cmd, 0, &[scissor]);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipelines.graphics_layout,
                0,
                &[self.pipelines.set],
                &[],
            );
            device.cmd_push_constants(
                cmd,
                self.pipelines.graphics_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&pc),
            );

            let draw = |pipeline: vk::Pipeline| {
                device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_draw_indirect_count(
                    cmd,
                    self.commands.raw,
                    0,
                    self.counters.raw,
                    4,
                    MAX_DRAW_RUNS,
                    16,
                );
            };

            // Pass A：不透明核心预 Pass，纯深度写入；配合 `GREATER_OR_EQUAL` 让最近核心占领深度缓冲以触发 Early-Z 剔除。
            draw(self.pipelines.opaque_core);

            // Pass B：单次带混合的 uber 绘制。
            draw(self.pipelines.uber_normal);

            self.stats.draw_calls = 2;
        }
    }

    /// 将颜色目标拷贝到宿主缓冲区并返回紧凑排列的 BGRA8 行数据（用于基准图测试与截图）。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn read_pixels(&mut self) -> Result<Vec<u8>, GpuError> {
        let width = self.target.width;
        let height = self.target.height;
        let row_bytes = u64::from(width) * 4;
        let size = row_bytes * u64::from(height);
        // 暂存缓冲区大小固定，录制拷贝前先校验尺寸防止越界读取。
        if size > self.staging.size {
            return Err(GpuError::Allocation(format!(
                "readback of {width}x{height} needs {size} bytes, the staging buffer holds {}",
                self.staging.size
            )));
        }
        let device = &self.gpu.device;
        // SAFETY: `wait_idle` 后 GPU 已空闲，拷贝无数据竞争。
        unsafe {
            self.gpu.wait_idle();
            let barrier = [image_barrier(
                self.target.color,
                self.target.layout,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                vk::PipelineStageFlags2::COPY,
                vk::AccessFlags2::TRANSFER_READ,
                false,
            )];
            let cmd = self.gpu.begin_single_time()?;
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&barrier),
            );
            device.cmd_copy_image_to_buffer(
                cmd,
                self.target.color,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.staging.raw,
                &[vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(width)
                    .buffer_image_height(height)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })],
            );
            self.gpu.end_single_time(cmd)?;
            self.target.layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;

            let mut out = vec![0u8; size as usize];
            core::ptr::copy_nonoverlapping(self.staging.mapped, out.as_mut_ptr(), size as usize);

            // GPU 已空闲，计数器已最终落盘，同步到统计结构体。
            let counters = core::slice::from_raw_parts(self.counters.mapped.cast::<u32>(), 4);
            self.stats.visible = counters[2];
            self.stats.draw_calls = counters[1];
            Ok(out)
        }
    }

    /// 仅当场景发生变化时将场景上传至持久化池（支持全量与局部补丁两条路径）。
    fn prepare_upload(&mut self, scene: &SceneUpload<'_>) -> Result<UploadPlan, GpuError> {
        // 剔除与顶点着色器对*每个*图元都会读取 `transforms[0]`。静态 UI 场景
        // 完全不发射变换表；`ensure_pool_capacity` 仍会预留一个 stride，但其内容
        // 是未初始化的显存，垃圾仿射矩阵会把所有四边形移出屏幕，剔除阶段统计到
        // 0 个可见图元。为此类场景补一个单位变换。
        static IDENTITY: [GpuTransform; 1] = [GpuTransform::IDENTITY];
        let transforms: &[GpuTransform] = if scene.transforms.is_empty() {
            &IDENTITY
        } else {
            scene.transforms
        };

        let full = self.generation.needs_full(scene.generation);
        let partial = !full && (!scene.dirty_ranges.is_empty() || scene.aux_changed);
        if !full && !partial {
            return Ok(UploadPlan::default());
        }

        let mut plan = UploadPlan::default();
        if full {
            self.ensure_pool_capacity(scene.primitives.len(), Which::Primitives)?;
            self.ensure_pool_capacity(transforms.len(), Which::Transforms)?;
            self.ensure_pool_capacity(scene.clips.len(), Which::Clips)?;
            self.ensure_pool_capacity(scene.payloads.len(), Which::Payloads)?;
            // 此处暂不提交版本号：后续录制或提交仍可能失败，待提交真正成功后才确认。
            self.generation.stage(scene.generation);
        } else if scene.aux_changed {
            // 辅助表体积较小，刷新它们时无需连带重传整个图元池。
            self.ensure_pool_capacity(transforms.len(), Which::Transforms)?;
            self.ensure_pool_capacity(scene.clips.len(), Which::Clips)?;
            self.ensure_pool_capacity(scene.payloads.len(), Which::Payloads)?;
        }

        // 分配前先轮转瞬态环：`begin_frame` 选定本帧槽位并重置游标。
        self.ring.begin_frame(self.frame_index);

        // 暂存缓冲区跨帧复用，渲染路径每帧零堆分配。
        self.staging_bytes.clear();
        self.staging_spans.clear();
        // 在借用暂存缓冲区前先读取各池块偏移。
        let prim_dst = self.pool_offset(self.prim_handle);
        let transform_dst = self.pool_offset(self.transform_handle);
        let clip_dst = self.pool_offset(self.clip_handle);
        let payload_dst = self.pool_offset(self.payload_handle);
        let bytes = &mut self.staging_bytes;
        let spans = &mut self.staging_spans;
        let stage =
            |dst: u32, data: &[u8], bytes: &mut Vec<u8>, spans: &mut Vec<(u32, u32, u32)>| {
                if data.is_empty() {
                    return;
                }
                // `scatter.comp` 按 16 字节字拷贝；未对齐区间会静默丢失尾部字节。
                debug_assert_eq!(dst % 16, 0, "scatter destination must be 16B aligned");
                debug_assert_eq!(
                    data.len() % 16,
                    0,
                    "scatter length must be a multiple of 16B"
                );
                let src_rel = bytes.len() as u32;
                bytes.extend_from_slice(data);
                spans.push((dst, src_rel, data.len() as u32));
            };

        if full {
            stage(
                prim_dst,
                bytemuck::cast_slice(scene.primitives),
                bytes,
                spans,
            );
            stage(
                transform_dst,
                bytemuck::cast_slice(transforms),
                bytes,
                spans,
            );
            stage(clip_dst, bytemuck::cast_slice(scene.clips), bytes, spans);
            stage(
                payload_dst,
                bytemuck::cast_slice(scene.payloads),
                bytes,
                spans,
            );
        } else {
            // 仅暂存脏切片；场景其余部分已在持久化池中且未变更，保持不动。
            let stride =
                u32::try_from(core::mem::size_of::<GpuPrimitiveInstance>()).unwrap_or(u32::MAX);
            for (start, end) in scene.dirty_ranges {
                let start = *start as usize;
                let end = (*end as usize).min(scene.primitives.len());
                if start >= end {
                    continue;
                }
                let data = bytemuck::cast_slice(&scene.primitives[start..end]);
                let dst = prim_dst + u32::try_from(start).unwrap_or(u32::MAX) * stride;
                stage(dst, data, bytes, spans);
            }
            if scene.aux_changed {
                stage(
                    transform_dst,
                    bytemuck::cast_slice(transforms),
                    bytes,
                    spans,
                );
                stage(clip_dst, bytemuck::cast_slice(scene.clips), bytes, spans);
                stage(
                    payload_dst,
                    bytemuck::cast_slice(scene.payloads),
                    bytes,
                    spans,
                );
            }
        }

        if spans.is_empty() {
            return Ok(plan);
        }

        // 在宿主可见瞬态环中预留载荷空间，直接拷贝无需额外暂存缓冲区。
        let ring_slot = self.ring.allocate(bytes.len() as u32, 256).ok_or_else(|| {
            GpuError::Allocation(format!(
                "transient ring exhausted: {} bytes needed, {} available (raise GpuConfig::ring_capacity)",
                bytes.len(),
                self.ring.slot_capacity()
            ))
        })?;

        // 补丁列表同样跨帧复用。将大区间拆分为 64 KiB 切片，
        // 使 `scatter.comp` 能调度多个工作组并行拷贝而非集中在单个 SM 上。
        const SCATTER_SLICE: u32 = 64 * 1024;
        self.staging_patches.clear();
        for (dst, src_rel, len) in spans.iter().copied() {
            let mut offset = 0u32;
            while offset < len {
                let slice = (len - offset).min(SCATTER_SLICE);
                self.staging_patches.push(GpuScatterPatch {
                    dst_offset: dst + offset,
                    src_offset: ring_slot + src_rel + offset,
                    len: slice,
                    _pad: 0,
                });
                offset += slice;
            }
        }

        // SAFETY: 两个缓冲区均宿主可见且全生命周期映射，范围刚刚预留完毕。
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ring_buffer
                    .host_ptr(u64::from(ring_slot))
                    .ok_or_else(|| GpuError::Allocation("ring not mapped".into()))?,
                bytes.len(),
            );
            let patches = &self.staging_patches;
            core::ptr::copy_nonoverlapping(
                patches.as_ptr().cast::<u8>(),
                self.patches
                    .host_ptr(0)
                    .ok_or_else(|| GpuError::Allocation("patches not mapped".into()))?,
                patches.len() * 16,
            );
            plan.patch_count = patches.len() as u32;
            // 缓冲区可能位于非一致性内存中，必须刷写以确保 GPU 不会读到上一帧的旧字节。
            self.gpu
                .flush(&self.ring_buffer, u64::from(ring_slot), bytes.len() as u64);
            self.gpu
                .flush(&self.patches, 0, (patches.len() * 16) as u64);
        }

        plan.bytes = bytes.len() as u32;
        plan.ring_offset = ring_slot;
        Ok(plan)
    }

    /// 释放全部 GPU 资源并消费渲染器实例。
    pub fn destroy(self) {
        // SAFETY: 本渲染器拥有该信号量且所有帧均已退役。
        unsafe {
            self.gpu
                .device
                .destroy_semaphore(self.present_semaphore, None);
        }
        self.gpu.wait_idle();
        if let Some(scope) = &self.timing {
            scope.destroy(&self.gpu);
        }
        // 管线描述符集引用了图集视图，因此必须先销毁管线，再释放图集与设备。
        let mut this = self;
        // SAFETY: 设备处于空闲状态。
        unsafe {
            this.pipelines.destroy(&this.gpu);
        }
        // 在设备销毁前显式释放图集图像（幂等），即使应用层仍持有 `Arc` 也不会在设备销毁后触碰悬空句柄。
        if let Some(atlases) = &this.atlases {
            atlases.free();
        }
        this.atlases = None;
        // SAFETY: 设备已空闲且 `self` 正在被消费。
        unsafe {
            this.target.destroy(&this.gpu);
            this.blur.destroy(&this.gpu);
            for b in [
                &this.pool_buffer,
                &this.ring_buffer,
                &this.lut_buffer,
                &this.keys_a,
                &this.keys_b,
                &this.idx_a,
                &this.idx_b,
                &this.sort_scratch,
                &this.commands,
                &this.counters,
                &this.patches,
                &this.staging,
            ] {
                this.gpu.destroy_buffer(b);
            }
        }
        this.gpu.destroy();
    }
}

#[derive(Copy, Clone, Debug)]
enum Which {
    Primitives,
    Transforms,
    Clips,
    Payloads,
}

impl Which {
    /// 返回该池块所支撑数组单个元素的字节步长（统一元素数到字节数的换算）。
    const fn stride(self) -> usize {
        match self {
            Self::Primitives => crate::data::PRIMITIVE_STRIDE,
            Self::Transforms => core::mem::size_of::<GpuTransform>(),
            Self::Clips => core::mem::size_of::<GpuClipNode>(),
            Self::Payloads => core::mem::size_of::<GpuPayload>(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct UploadPlan {
    patch_count: u32,
    bytes: u32,
    ring_offset: u32,
}

/// 持久化池当前所处的上传版本号追踪器（仅在帧成功提交到 GPU 后才确认）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct UploadGeneration {
    /// 池中当前已确认持有的版本号。
    committed: u64,
    /// 当前帧已暂存但尚未确认的版本号。
    staged: Option<u64>,
}

impl UploadGeneration {
    /// 尚无任何已上传内容的初始状态。
    const fn empty() -> Self {
        Self {
            committed: u64::MAX,
            staged: None,
        }
    }

    /// 判断当前场景是否需要全量上传。
    fn needs_full(&self, generation: u64) -> bool {
        self.committed != generation
    }

    /// 为即将绘制的帧暂存版本号。
    fn stage(&mut self, generation: u64) {
        self.staged = Some(generation);
    }

    /// 确认已暂存的版本号：帧已成功送达 GPU。
    fn commit(&mut self) {
        if let Some(generation) = self.staged.take() {
            self.committed = generation;
        }
    }

    /// 重置池版本号：强制下一帧执行全量上传。
    fn invalidate(&mut self) {
        self.committed = u64::MAX;
        self.staged = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{PrimFlags, SubElement};
    use unbound_math::Vec2;

    /// 暂存版本号在帧真正送达 GPU 前不得被确认，防止失败帧导致后续画面永久停留在旧状态。
    #[test]
    fn a_staged_generation_is_claimed_only_after_the_frame_reached_the_gpu() {
        let mut generation = UploadGeneration::empty();
        assert!(generation.needs_full(1), "an empty pool has to be filled");

        generation.stage(1);
        assert!(
            generation.needs_full(1),
            "a staged generation is not a committed one"
        );

        generation.commit();
        assert!(
            !generation.needs_full(1),
            "the committed generation is already in the pool"
        );
        assert!(
            generation.needs_full(2),
            "a new scene has to be uploaded in full"
        );
    }

    /// 未送达 GPU 的失败帧不得确认版本号。
    #[test]
    fn a_failed_frame_keeps_the_generation_unclaimed() {
        let mut generation = UploadGeneration::empty();
        generation.stage(7);
        // 帧失败：不提交版本号。
        assert!(generation.needs_full(7));
        // 下一次尝试必须重新全量上传。
        generation.stage(7);
        generation.commit();
        assert!(!generation.needs_full(7));
    }

    /// 发生搬迁的池块必须重新填充。
    #[test]
    fn an_invalidated_pool_has_to_be_filled_again() {
        let mut generation = UploadGeneration::empty();
        generation.stage(3);
        generation.commit();
        assert!(!generation.needs_full(3));
        generation.invalidate();
        assert!(generation.needs_full(3));
    }

    fn quad(x: f32, y: f32, w: f32, h: f32, token: u32) -> GpuPrimitiveInstance {
        GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: unbound_math::f16::pack_radii([4.0; 4]),
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: crate::data::CLIP_NONE,
            fill_token_or_uv: token,
            border_token_or_tex: token,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                crate::data::PrimType::RoundedRect,
                crate::data::BlendMode::Normal,
                PrimFlags::with_sub(SubElement::Background, PrimFlags::EMPTY).bits(),
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(1000, 1.0),
            extra_payload_idx: 0,
        }
    }

    fn scene<'a>(
        prims: &'a [GpuPrimitiveInstance],
        transforms: &'a [GpuTransform],
        gen: u64,
    ) -> SceneUpload<'a> {
        SceneUpload {
            primitives: prims,
            transforms,
            clips: &[],
            payloads: &[],
            generation: gen,
            camera: Camera::ui((128.0, 128.0), 1.0),
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges: &[],
            aux_changed: false,
            backdrops: &[],
        }
    }

    fn scene_at<'a>(
        prims: &'a [GpuPrimitiveInstance],
        transforms: &'a [GpuTransform],
        gen: u64,
        viewport: (f32, f32),
    ) -> SceneUpload<'a> {
        SceneUpload {
            camera: Camera::ui(viewport, 1.0),
            ..scene(prims, transforms, gen)
        }
    }

    #[test]
    fn renders_a_single_quad_to_the_expected_pixels() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        // 令牌 5 为填充色；令牌 2 为清屏表面色，两者错开以便区分矩形内外。
        let prims = vec![quad(16.0, 16.0, 96.0, 96.0, 5)];
        let transforms = vec![GpuTransform::IDENTITY];
        renderer
            .write_theme(&{
                let mut c = vec![[0u16; 4]; 512];
                c[1] = unbound_math::Color::from_srgb8(0, 0, 0, 40).to_rgba16f();
                c[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
                c[5] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
                c
            })
            .unwrap();

        renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        let pixels = renderer.read_pixels().expect("readback");
        assert_eq!(pixels.len(), 128 * 128 * 4);

        // 四边形中心必须为填充色（白色）。
        let idx = (64 * 128 + 64) * 4;
        assert!(
            pixels[idx] > 200 && pixels[idx + 1] > 200 && pixels[idx + 2] > 200,
            "centre pixel was {:?}",
            &pixels[idx..idx + 4]
        );
        // 四边形外部像素必须为清屏色。
        let outside = (2 * 128 + 2) * 4;
        let clear = pixels[outside];
        assert!(
            (10..=30).contains(&clear),
            "corner pixel was {:?}",
            &pixels[outside..outside + 4]
        );

        assert_eq!(renderer.stats.visible, 1, "the quad must survive culling");
        assert_eq!(renderer.stats.draw_calls, 1);
        renderer.destroy();
    }

    /// 静态 UI 场景完全没有动画变换表，但剔除与顶点着色器仍会读取
    /// `transforms[0]`。空表必须按单位变换处理，否则所有图元都会被剔除，
    /// 窗口只剩清屏色。
    #[test]
    fn a_scene_without_transforms_still_draws() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        let prims = vec![quad(16.0, 16.0, 96.0, 96.0, 5)];
        renderer
            .write_theme(&{
                let mut c = vec![[0u16; 4]; 512];
                c[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
                c[5] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
                c
            })
            .unwrap();

        renderer.render(&scene(&prims, &[], 1)).expect("render");
        let pixels = renderer.read_pixels().expect("readback");
        let idx = (64 * 128 + 64) * 4;
        assert!(
            pixels[idx] > 200 && pixels[idx + 1] > 200 && pixels[idx + 2] > 200,
            "the quad must survive culling with no transform table: {:?}",
            &pixels[idx..idx + 4]
        );
        assert_eq!(renderer.stats.visible, 1);
        renderer.destroy();
    }

    /// 计算矩形区域内红色通道的标准差（用于验证模糊是否有效消除棋盘格高频对比度）。
    fn region_stddev(pixels: &[u8], width: u32, rect: (u32, u32, u32, u32)) -> f32 {
        let (x0, y0, x1, y1) = rect;
        let mut sum = 0.0;
        let mut sum_sq = 0.0;
        let mut count = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let value = f32::from(pixels[((y * width + x) * 4) as usize]);
                sum += value;
                sum_sq += value * value;
                count += 1.0;
            }
        }
        if count == 0.0 {
            return 0.0;
        }
        let mean = sum / count;
        (sum_sq / count - mean * mean).max(0.0).sqrt()
    }

    /// P1.4：模态毛玻璃背景必须真正模糊其下方的棋盘格，且第二帧不得重复分配暂存图像。
    #[test]
    fn modal_backdrop_blur_smooths_underlying_checkerboard() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        let mut theme = vec![[0u16; 4]; 512];
        theme[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
        theme[5] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
        theme[6] = unbound_math::Color::from_srgb8(0, 0, 0, 255).to_rgba16f();
        renderer.write_theme(&theme).unwrap();

        let mut prims = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                let token = if (x + y) % 2 == 0 { 5 } else { 6 };
                let mut tile = quad(x as f32 * 8.0, y as f32 * 8.0, 8.0, 8.0, token);
                tile.corner_radii = unbound_math::f16::pack_radii([0.0; 4]);
                prims.push(tile);
            }
        }
        let transforms = vec![GpuTransform::IDENTITY];
        let base = scene(&prims, &transforms, 1);

        renderer.render(&base).expect("baseline render");
        let baseline = renderer.read_pixels().expect("baseline readback");
        let before = region_stddev(&baseline, 128, (16, 16, 112, 112));
        assert!(before > 100.0, "the checkerboard must be sharp: {before}");

        let backdrops = [BackdropRequest {
            rect: [0.0, 0.0, 128.0, 128.0],
            radius: 0.0,
            corner_radii: [0.0; 4],
            layer_key_prefix: (PassLayer::Overlay as u64) << 56,
        }];
        let blurred_scene = SceneUpload {
            backdrops: &backdrops,
            ..scene(&prims, &transforms, 1)
        };
        renderer.render(&blurred_scene).expect("blurred render");
        let blurred = renderer.read_pixels().expect("blurred readback");
        let after = region_stddev(&blurred, 128, (16, 16, 112, 112));
        assert!(
            after <= before * 0.2,
            "the backdrop must smooth the checkerboard: before={before}, after={after}"
        );

        let created = renderer.blur_pool_creation_count();
        renderer
            .render(&blurred_scene)
            .expect("steady-state render");
        let _ = renderer.read_pixels().expect("steady-state readback");
        assert_eq!(
            renderer.blur_pool_creation_count(),
            created,
            "a steady-state frame must recycle the offscreen pool"
        );
        renderer.destroy();
    }

    /// 参见 `docs/architecture/gpu-renderer.md`：切换主题时仅上传 4 KiB 颜色查找表即可重绘全帧。
    #[test]
    fn a_theme_switch_recolours_without_re_uploading_instances() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        // 令牌 5 在语义调色板中对应 `PRIMARY`。
        let prims = vec![quad(16.0, 16.0, 96.0, 96.0, 5)];
        let transforms = vec![GpuTransform::IDENTITY];

        let mut theme = unbound_theme::ThemeState::default();
        renderer.write_theme_lut(theme.lut()).unwrap();
        renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        let light = renderer.read_pixels().expect("readback");
        let centre = (64 * 128 + 64) * 4;

        let outcome = theme.switch(unbound_theme::ThemeKind::Dark);
        assert_eq!(outcome.bytes_uploaded, 4096);
        assert!(
            !outcome.relayout_required,
            "a theme switch must not re-layout"
        );
        assert!(
            !outcome.instance_reupload_required,
            "a theme switch must not re-upload instances"
        );
        renderer.write_theme_lut(theme.lut()).unwrap();

        // 版本号不变：仅颜色查找表发生变化。
        renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        let dark = renderer.read_pixels().expect("readback");

        let light_px = &light[centre..centre + 4];
        let dark_px = &dark[centre..centre + 4];
        assert_ne!(light_px, dark_px, "the theme switch changed nothing");
        // 回读格式为 BGRA，因此红色通道位于索引 2。
        assert!(
            dark_px[2] > light_px[2] + 20,
            "the dark primary must be lighter in red: {light_px:?} vs {dark_px:?}"
        );
        renderer.destroy();
    }

    /// 参见 `docs/architecture/gpu-renderer.md`：曲线通过二次 Bézier 解析距离光栅化而非折线近似。
    #[test]
    fn a_curve_is_rasterised_analytically() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        // 从 (10, 10) 到 (100, 10)、向上拱起至 y = 45 的二次弧线。
        let segment = unbound_vector::QuadSegment::new(
            Vec2::new(10.0, 10.0),
            Vec2::new(50.0, 80.0),
            Vec2::new(100.0, 10.0),
        );
        let instance = unbound_vector::CurveInstance::from_quad(&segment, 4.0);
        let draw = crate::curve::CurveDraw::new([0, 0], &instance, 5, 4.0, 100);
        let prims = vec![draw.primitive];
        let payloads = vec![draw.payload];
        let transforms = vec![draw.transform];
        renderer
            .write_theme(&{
                let mut colors = vec![[0u16; 4]; 512];
                colors[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
                colors[5] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
                colors
            })
            .unwrap();

        let mut upload = scene(&prims, &transforms, 1);
        upload.payloads = &payloads;
        renderer.render(&upload).expect("render");
        let pixels = renderer.read_pixels().expect("readback");
        assert_eq!(pixels.len(), 128 * 128 * 4);

        // t = 0.5 时曲线经过 (55, 45)。
        let on_curve = (45 * 128 + 55) * 4;
        // 曲线上方 15 像素处应为空白背景。
        let off_curve = (30 * 128 + 55) * 4;
        assert!(
            pixels[on_curve] > 200,
            "the curve must be lit at its midpoint: {:?}",
            &pixels[on_curve..on_curve + 4]
        );
        assert!(
            pixels[off_curve] < 120,
            "nothing may be drawn above the curve: {:?}",
            &pixels[off_curve..off_curve + 4]
        );
        assert_eq!(renderer.stats.visible, 1);
        renderer.destroy();
    }

    /// 参见 `docs/architecture/gpu-renderer.md`：屏幕上绘制一万条长连接线的 GPU 性能预算测试（0.8 ms）。
    #[test]
    fn ten_thousand_cubic_connectors_stay_inside_the_budget() {
        let _guard = crate::device::gpu_test_guard();
        // 一万条连接线需要比默认测试设备更大的瞬态环。
        let gpu = match crate::device::Gpu::new(crate::device::GpuConfig {
            pool_capacity: 64 << 20,
            ring_capacity: 64 << 20,
            ..Default::default()
        }) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("skipping GPU test: {error}");
                return;
            }
        };
        let mut renderer = Renderer::new(gpu, 512, 512).expect("renderer");
        renderer.enable_timing().expect("timing");

        const CURVES: u32 = 10_000;
        let mut prims = Vec::with_capacity(CURVES as usize);
        let mut payloads = Vec::with_capacity(CURVES as usize);
        let mut transforms = Vec::with_capacity(CURVES as usize);
        for index in 0..CURVES {
            // 分布于超大画布上的长斜向连接线，视口仅覆盖其中一小部分，主要开销在剔除阶段。
            let x = (index % 100) as f32 * 500.0;
            let y = (index / 100) as f32 * 500.0;
            let sag = 40.0 + (index % 17) as f32;
            // 先将三次曲线拆分为二次曲线段，使每段拥有紧凑 OBB。
            let draws = crate::curve::cubic_to_draws(
                Vec2::new(x, y),
                Vec2::new(x + 50.0, y - sag),
                Vec2::new(x + 100.0, y),
                Vec2::new(x + 150.0, y + sag * 0.5),
                0.5,
                2,
                1.5,
                100,
            );
            for draw in draws {
                let mut primitive = draw.primitive;
                primitive.transform_idx = transforms.len() as u32;
                primitive.extra_payload_idx = payloads.len() as u32;
                payloads.push(draw.payload);
                transforms.push(draw.transform);
                prims.push(primitive);
            }
        }
        renderer
            .write_theme(&{
                let mut colors = vec![[0u16; 4]; 512];
                colors[2] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
                colors
            })
            .unwrap();

        let mut upload = scene_at(&prims, &transforms, 1, (512.0, 512.0));
        upload.payloads = &payloads;
        // 相机对准画布角落：一万条连接线中仅少数处于屏幕内。
        upload.camera = Camera::canvas([0, 0], [256.0, 256.0], 1.0, 1.0);
        renderer.render(&upload).expect("render");
        renderer.render(&upload).expect("render");
        // GPU 时间戳存在波动：先预热，再取三帧稳态样本中的最佳值与预算比较。
        let mut best_ms = f32::MAX;
        let mut timing = None;
        for _ in 0..3 {
            renderer.render(&upload).expect("render");
            if let Some(sample) = renderer.read_frame_timing().expect("read timing") {
                if sample.total_ms <= best_ms {
                    best_ms = sample.total_ms;
                    timing = Some(sample);
                }
            }
        }
        let timing = timing.expect("timing is enabled");
        let average: f32 =
            prims.iter().map(|p| p.size[0] * p.size[1]).sum::<f32>() / prims.len().max(1) as f32;
        eprintln!(
            "{CURVES} curves -> {} primitives, avg box {average:.1} px², {:.3} ms on the GPU              (cull {:.3}, sort {:.3}, indirect {:.3}, graphics {:.3})",
            prims.len(),
            timing.total_ms,
            timing.cull_ms,
            timing.sort_ms,
            timing.indirect_ms,
            timing.graphics_ms
        );
        // 参考桌面 GPU 设计预算为 0.8 ms（参见 `docs/architecture/gpu-renderer.md`）；
        // 允许 1.5 倍余量以兼容较慢的测试硬件同时捕获真实性能退化。
        const DESIGN_BUDGET_MS: f32 = 0.8;
        const HARDWARE_ALLOWANCE: f32 = 1.5;
        assert!(
            timing.total_ms <= DESIGN_BUDGET_MS * HARDWARE_ALLOWANCE,
            "the curve frame took {:.3} ms, the guard is {:.2} ms (design {:.1} ms)",
            timing.total_ms,
            DESIGN_BUDGET_MS * HARDWARE_ALLOWANCE,
            DESIGN_BUDGET_MS
        );
        renderer.destroy();
    }

    /// 参见 `docs/architecture/gpu-renderer.md`：验证字形从真实 Bindless 图集而非 1x1 占位纹理采样。
    #[test]
    fn a_glyph_is_drawn_from_the_real_atlas() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");

        // 在单层图集原点上传一块白色图块。
        let atlases =
            std::sync::Arc::new(crate::atlas::AtlasSet::new(&renderer.gpu, 1).expect("atlas set"));
        let upload = crate::atlas::AtlasUpload {
            kind: crate::atlas::AtlasKind::R8,
            layer: 0,
            x: 0,
            y: 0,
            width: 32,
            height: 32,
            data: vec![255u8; 32 * 32],
        };
        atlases.upload(&renderer.gpu, &upload).expect("upload");
        // 验证像素已实际落地到 GPU 图集。
        let readback = atlases
            .read_layer(&renderer.gpu, crate::atlas::AtlasKind::R8, 0, 0, 0, 4, 4)
            .expect("readback");
        assert!(
            readback.iter().all(|b| *b == 255),
            "the atlas layer must contain the uploaded texels: {readback:?}"
        );
        renderer.install_atlases(atlases).expect("install");

        // 覆盖目标中心的字形图元；载荷携带图集 UV 范围，`border_or_tex` 指定图集图层。
        let prim = crate::data::GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [32.0, 32.0],
            size: [64.0, 64.0],
            prim_type_and_flags: crate::data::GpuPrimitiveInstance::pack_type(
                crate::data::PrimType::Glyph,
                crate::data::BlendMode::Normal,
                crate::data::SubElement::Content as u16,
            ),
            fill_token_or_uv: 5,
            // 着色器从 `border_or_tex` 读取图集图层。
            border_token_or_tex: 0,
            extra_payload_idx: 0, // `CLIP_NONE` 为 `u32::MAX` 而非 0，默认值 0 会指向不存在的裁剪节点并误剔除字形。
            clip_idx: crate::data::CLIP_NONE,
            // 必须显式设置不透明度：默认值为 0，零不透明度图元会在进入片元着色器前被剔除。
            z_and_opacity: crate::data::GpuPrimitiveInstance::pack_z_opacity(1000, 1.0),
            ..Default::default()
        };
        let mut payload = crate::data::GpuPayload::default();
        // UV 范围指向 2048x2048 页内原点处的 32x32 纹素图块，即 `[0, 32/2048]`。
        let extent = 32.0 / crate::atlas::PAGE_SIZE as f32;
        payload.p0 = [0.0, 0.0];
        payload.p1 = [extent, extent];

        // 参考矩形位于左半侧，字形位于右半侧。
        let reference = quad(16.0, 16.0, 48.0, 96.0, 5);
        let prims = vec![reference, prim];
        let payloads = vec![payload];
        let transforms = vec![GpuTransform::IDENTITY];
        let mut upload = scene(&prims, &transforms, 1);
        upload.payloads = &payloads;

        renderer
            .write_theme(&{
                let mut colors = vec![[0u16; 4]; 512];
                colors[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
                colors[5] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
                colors
            })
            .unwrap();

        renderer.render(&upload).expect("render");
        let pixels = renderer.read_pixels().expect("readback");

        // 参考矩形（左半侧）必须被点亮。
        let reference_px = (64 * 128 + 40) * 4;
        // 字形中心必须被点亮。
        let centre = (64 * 128 + 64) * 4;
        assert!(
            pixels[reference_px] > 200,
            "the reference quad must be lit: {:?}",
            &pixels[reference_px..reference_px + 4]
        );
        assert!(
            pixels[centre] > 200,
            "the glyph must be drawn from the atlas: {:?}",
            &pixels[centre..centre + 4]
        );
        // 两个图元上方的像素必须保持暗色。
        let outside = (4 * 128 + 100) * 4;
        // 清屏色在线性空间为 0.07，经 sRGB 编码后约 80；被点亮的像素远高于此值。
        assert!(
            pixels[outside] < 120,
            "nothing may be drawn above the glyph: {:?}",
            &pixels[outside..outside + 4]
        );
        renderer.destroy();
    }

    /// 验证带边框圆角矩形内部保持填充色、外环保持边框色。
    #[test]
    fn a_bordered_rectangle_keeps_its_fill_colour() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");

        // 令牌 2 为白色，令牌 1 为半透明黑色。
        let mut theme = vec![[0u16; 4]; 512];
        // 使用不透明边框以清晰区分外环与内部填充。
        theme[1] = unbound_math::Color::from_srgb8(0, 0, 0, 255).to_rgba16f();
        theme[2] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
        renderer.write_theme(&theme).unwrap();

        // 左侧：无边框；右侧：4 px 边框。
        let plain = crate::data::GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [8.0, 8.0],
            size: [48.0, 48.0],
            corner_radii: [0; 4],
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: crate::data::CLIP_NONE,
            fill_token_or_uv: 2,
            border_token_or_tex: 1,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                crate::data::PrimType::RoundedRect,
                crate::data::BlendMode::Normal,
                crate::data::SubElement::Background as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(10, 1.0),
            extra_payload_idx: 0,
        };
        let bordered = crate::data::GpuPrimitiveInstance {
            local_pos: [72.0, 8.0],
            size: [48.0, 48.0],
            border_width: unbound_math::f16::f32_to_f16(4.0),
            ..plain
        };
        let prims = vec![plain, bordered];
        let transforms = vec![GpuTransform::IDENTITY];

        renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        let pixels = renderer.read_pixels().expect("readback");

        // 普通矩形的内部区域。
        let plain_px = (32 * 128 + 32) * 4;
        assert!(
            pixels[plain_px] > 200,
            "the plain rectangle must be white: {:?}",
            &pixels[plain_px..plain_px + 4]
        );
        // 带边框矩形的内部：仍显示填充色。
        let bordered_px = (32 * 128 + 96) * 4;
        assert!(
            pixels[bordered_px] > 200,
            "the bordered rectangle must keep its fill: {:?}",
            &pixels[bordered_px..bordered_px + 4]
        );
        // 带边框矩形的外环区域：应为边框色（采样外环中点 x = 74）。
        for x in 72..76usize {
            let ring_px = (32 * 128 + x) * 4;
            assert!(
                pixels[ring_px] < 120,
                "the border ring at x = {x} must use the border token: {:?}",
                &pixels[ring_px..ring_px + 4]
            );
        }
        // 紧贴边框内侧恢复为填充色。
        let inner_edge_px = (32 * 128 + 76) * 4;
        assert!(
            pixels[inner_edge_px] > 200,
            "the fill must resume right after the ring: {:?}",
            &pixels[inner_edge_px..inner_edge_px + 4]
        );
        renderer.destroy();
    }

    /// 验证窗口化借用附件路径与离屏渲染路径逐字节输出一致。
    #[test]
    fn the_borrowed_attachment_path_matches_the_offscreen_path() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 256, 256).expect("renderer");
        let mut theme = vec![[0u16; 4]; 512];
        theme[1] = unbound_math::Color::from_srgb8(20, 20, 30, 255).to_rgba16f();
        theme[2] = unbound_math::Color::from_srgb8(52, 100, 220, 255).to_rgba16f();
        renderer.write_theme(&theme).unwrap();

        let background = quad(0.0, 0.0, 256.0, 256.0, 1);
        let mut rounded = quad(60.0, 60.0, 120.0, 64.0, 2);
        rounded.corner_radii = unbound_math::f16::pack_radii([12.0; 4]);
        rounded.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(2000, 1.0);
        let transforms = vec![GpuTransform::IDENTITY];
        let prims = [background, rounded];

        // 参考基准：渲染器自有的离屏目标。
        renderer
            .render(&scene_at(&prims, &transforms, 1, (256.0, 256.0)))
            .expect("render");
        let reference = renderer.read_pixels().expect("readback");

        // 具备交换链图像相同用途标志的借用附件。
        let (image, memory, view) = borrowed_attachment(&renderer, 256, 256);
        renderer
            .render_to_view(
                &scene_at(&prims, &transforms, 2, (256.0, 256.0)),
                image,
                view,
                256,
                256,
                // 测试中无呈现引擎：图像已归本测试所有，无需等待信号量。
                vk::Semaphore::null(),
            )
            .expect("render");
        let borrowed = read_back(&mut renderer, image, 256, 256).expect("readback");

        assert_eq!(
            borrowed.len(),
            reference.len(),
            "the borrowed path must produce a full frame"
        );
        assert_eq!(
            borrowed, reference,
            "the windowed path must draw the same picture as the offscreen path"
        );

        // 防止“两者皆空”通过上方比对；回读为 BGRA，RGB(52, 100, 220) 对应蓝色通道在前。
        let lit = borrowed
            .chunks(4)
            .filter(|pixel| pixel[0] > 180 && pixel[2] < 120)
            .count();
        assert!(
            lit > 1000,
            "the borrowed frame must actually contain the rectangle: {lit} pixels"
        );

        // SAFETY: 图像、视图与内存均归本测试所有。
        unsafe {
            renderer.gpu.device.destroy_image_view(view, None);
            renderer.gpu.device.destroy_image(image, None);
            renderer.gpu.device.free_memory(memory, None);
        }
        renderer.destroy();
    }

    /// 创建行为与交换链图像一致的颜色附件。
    fn borrowed_attachment(
        renderer: &Renderer,
        width: u32,
        height: u32,
    ) -> (vk::Image, vk::DeviceMemory, vk::ImageView) {
        // SAFETY: 图像所有权移交调用方并由其销毁。
        unsafe {
            RenderTarget::create_image(
                &renderer.gpu,
                width,
                height,
                COLOR_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                false,
            )
            .expect("borrowed attachment")
        }
    }

    /// 将任意颜色图像拷贝到渲染器暂存缓冲区。
    fn read_back(
        renderer: &mut Renderer,
        image: vk::Image,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, GpuError> {
        let size = u64::from(width) * u64::from(height) * 4;
        // 暂存缓冲区大小固定，禁止超出映射范围拷贝。
        if size > renderer.staging.size {
            return Err(GpuError::Allocation(format!(
                "readback of {width}x{height} needs {size} bytes, the staging buffer holds {}",
                renderer.staging.size
            )));
        }
        let device = &renderer.gpu.device;
        // SAFETY: `wait_idle` 后 GPU 已空闲，拷贝无数据竞争。
        unsafe {
            renderer.gpu.wait_idle();
            let barrier = [image_barrier(
                image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                vk::PipelineStageFlags2::COPY,
                vk::AccessFlags2::TRANSFER_READ,
                false,
            )];
            let cmd = renderer.gpu.begin_single_time()?;
            device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&barrier),
            );
            device.cmd_copy_image_to_buffer(
                cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                renderer.staging.raw,
                &[vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(width)
                    .buffer_image_height(height)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })],
            );
            renderer.gpu.end_single_time(cmd)?;
            let mut out = vec![0u8; size as usize];
            core::ptr::copy_nonoverlapping(
                renderer.staging.mapped,
                out.as_mut_ptr(),
                size as usize,
            );
            Ok(out)
        }
    }

    /// 验证圆角矩形从边到边完整填充，内部采样网格均呈现填充色。
    #[test]
    fn a_rounded_rectangle_is_filled_edge_to_edge() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 256, 256).expect("renderer");
        let mut theme = vec![[0u16; 4]; 512];
        theme[1] = unbound_math::Color::from_srgb8(20, 20, 30, 255).to_rgba16f();
        theme[2] = unbound_math::Color::from_srgb8(52, 100, 220, 255).to_rgba16f();
        renderer.write_theme(&theme).unwrap();

        // 背景铺满整个目标；圆角矩形叠放在上方。
        let background = quad(0.0, 0.0, 256.0, 256.0, 1);
        let mut rounded = quad(60.0, 60.0, 120.0, 64.0, 2);
        rounded.corner_radii = unbound_math::f16::pack_radii([12.0; 4]);
        // 反向 Z：更大的深度值更近且后绘制，故矩形深度大于背景。
        rounded.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(2000, 1.0);
        let transforms = vec![GpuTransform::IDENTITY];

        renderer
            .render(&scene_at(
                &[background, rounded],
                &transforms,
                1,
                (256.0, 256.0),
            ))
            .expect("render");
        let pixels = renderer.read_pixels().expect("readback");

        // 矩形内部所有采样点（避开圆角）均必须为填充色。
        let mut wrong = 0;
        let mut first_wrong = None;
        for y in 70..115u32 {
            for x in 70..170u32 {
                let idx = ((y * 256 + x) * 4) as usize;
                let pixel = &pixels[idx..idx + 4];
                // 回读格式为 BGRA：蓝色通道在前。
                if pixel[0] < 180 || pixel[2] > 120 {
                    wrong += 1;
                    first_wrong.get_or_insert((x, y, pixel.to_vec()));
                }
            }
        }
        assert_eq!(
            wrong, 0,
            "{wrong} pixels inside the rectangle are not the fill colour; \
             first offender {first_wrong:?}"
        );
        renderer.destroy();
    }

    /// 参见 `docs/architecture/gpu-renderer.md`：屏幕上 10 万个带阴影圆角画布节点必须合并为不超过 4 次 `vkCmdDrawIndirectCount` 调用。
    #[test]
    fn a_hundred_thousand_primitives_need_at_most_four_draw_calls() {
        let _guard = crate::device::gpu_test_guard();
        // 首次上传需在单个环槽位内暂存 10 万 × 64B 图元，故调大瞬态环容量。
        let gpu = match crate::device::Gpu::new(crate::device::GpuConfig {
            pool_capacity: 32 << 20,
            ring_capacity: 16 << 20,
            ..Default::default()
        }) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("skipping GPU test: {error}");
                return;
            }
        };
        const COUNT: u32 = 99_840; // 320 x 312 网格
        let mut renderer = Renderer::new(gpu, 512, 512).expect("renderer");
        // 320 x 312 个 1.6 px 单元恰好铺满 512 x 512 视口。
        let prims: Vec<_> = (0..COUNT)
            .map(|i| {
                let x = (i % 320) as f32 * 1.6;
                let y = (i / 320) as f32 * 1.64;
                quad(x, y, 2.0, 2.0, 2)
            })
            .collect();
        let transforms = vec![GpuTransform::IDENTITY];

        // 相机须覆盖整个网格，否则剔除 Pass 会合理丢弃视口外图元。
        renderer.enable_timing().expect("timing");
        let stats = renderer
            .render(&scene_at(&prims, &transforms, 1, (512.0, 512.0)))
            .expect("render");
        assert_eq!(stats.submitted, COUNT);

        // 后续帧场景完全不变，进入零上传稳态；取三帧稳态样本的最佳耗时与预算对比。
        let mut best_ms = f32::MAX;
        let mut timing = None;
        for _ in 0..3 {
            renderer
                .render(&scene_at(&prims, &transforms, 1, (512.0, 512.0)))
                .expect("render");
            if let Some(sample) = renderer.read_frame_timing().expect("read timing") {
                if sample.total_ms <= best_ms {
                    best_ms = sample.total_ms;
                    timing = Some(sample);
                }
            }
        }
        let timing = timing.expect("timing is enabled");
        eprintln!(
            "100k primitives: {:.3} ms total (cull {:.3}, sort {:.3}, indirect {:.3}, graphics {:.3})",
            timing.total_ms,
            timing.cull_ms,
            timing.sort_ms,
            timing.indirect_ms,
            timing.graphics_ms
        );

        let pixels = renderer.read_pixels().expect("readback");
        assert_eq!(pixels.len(), 512 * 512 * 4);
        assert!(
            renderer.stats.visible > COUNT - 1000,
            "culling dropped too many: {} of {COUNT}",
            renderer.stats.visible
        );
        // 10 万节点参考 GPU 预算为 1.5 ms 且 MDI 批次数 <= 4（参见 `docs/architecture/gpu-renderer.md`）；
        // 允许 1.5 倍硬件余量。
        const DESIGN_BUDGET_MS: f32 = 1.5;
        const HARDWARE_ALLOWANCE: f32 = 1.5;
        assert!(
            timing.total_ms <= DESIGN_BUDGET_MS * HARDWARE_ALLOWANCE,
            "the frame took {:.3} ms, the guard is {:.2} ms (design {:.1} ms)",
            timing.total_ms,
            DESIGN_BUDGET_MS * HARDWARE_ALLOWANCE,
            DESIGN_BUDGET_MS
        );
        assert!(
            renderer.stats.draw_calls <= 4,
            "MDI collapsed {} primitives into {} draw calls",
            renderer.stats.visible,
            renderer.stats.draw_calls
        );

        // 第二帧版本号相同：持久化池上传必须为 0 字节（参见 `docs/architecture/gpu-renderer.md`）。
        let again = renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        assert_eq!(again.bytes_uploaded, 0);
        renderer.destroy();
    }

    #[test]
    fn camera_only_frame_uploads_nothing() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 64, 64).expect("renderer");
        let prims: Vec<_> = (0..64)
            .map(|i| quad((i % 8) as f32 * 8.0, (i / 8) as f32 * 8.0, 6.0, 6.0, 2))
            .collect();
        let transforms = vec![GpuTransform::IDENTITY];

        let first = renderer
            .render(&scene_at(&prims, &transforms, 1, (64.0, 64.0)))
            .unwrap();
        assert!(first.bytes_uploaded > 0, "first frame uploads the pool");

        // 同一版本号仅移动相机：上传字节数必须为零。
        let mut moved = scene_at(&prims, &transforms, 1, (64.0, 64.0));
        moved.camera = Camera::canvas([0, 0], [32.0, 32.0], 1.5, 1.0);
        let second = renderer.render(&moved).unwrap();
        assert_eq!(second.bytes_uploaded, 0, "camera pan must be free");
        assert_eq!(second.patches, 0);
        renderer.destroy();
    }

    #[test]
    fn offscreen_viewport_culls_everything_outside() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 64, 64).expect("renderer");
        // 深色背景使“已剔除”状态一目了然。
        renderer
            .write_theme(&{
                let mut colors = vec![[0u16; 4]; 512];
                colors[2] = unbound_math::Color::from_srgb8(18, 18, 20, 255).to_rgba16f();
                colors
            })
            .unwrap();
        let prims = vec![quad(10_000.0, 10_000.0, 10.0, 10.0, 2)];
        let transforms = vec![GpuTransform::IDENTITY];
        renderer
            .render(&scene_at(&prims, &transforms, 1, (64.0, 64.0)))
            .unwrap();
        // 剔除着色器拒绝该图元；绘制命令数降至零。
        assert_eq!(renderer.stats.submitted, 1);
        let pixels = renderer.read_pixels().unwrap();
        let idx = (32 * 64 + 32) * 4;
        // 仅剩清屏色：图元在光栅化前已被剔除。
        assert!(
            (10..=30).contains(&pixels[idx]),
            "culled primitive painted: {:?}",
            &pixels[idx..idx + 4]
        );
        assert_eq!(renderer.stats.visible, 0, "culling must report 0 visible");
        renderer.destroy();
    }
}

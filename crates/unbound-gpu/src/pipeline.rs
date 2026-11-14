//! 图形与计算管线。
//!
//! 两条图形管线覆盖引擎中的所有图元：
//!
//! | 管线 | 深度 | 混合模式 | 用途 |
//! | :--- | :--- | :--- | :--- |
//! | `uber_normal` | 测试不写入 | src-alpha / one-minus-src-alpha | 99% 的常规 UI |
//! | `uber_additive` | 测试不写入 | one / one | 高亮、光标拖尾 |
//!
//! 此外还包括 `opaque_core`（仅写深度的预 Pass，使硬件 Early-Z 剔除大型面板后方
//! 被遮挡的所有图元，参见 `docs/architecture/gpu-renderer.md`），以及各 GPU 驱动阶段的
//! 计算管线（cull、scatter、3 趟 radix 排序、indirect 压缩）。
//!
//! 与 [`crate::device`] 类似，本模块是对 Vulkan C API 的薄封装，
//! 因此关闭了 crate 级的 `unsafe_op_in_unsafe_fn` 拒绝检查。

#![allow(unsafe_op_in_unsafe_fn)]

use ash::vk;

use crate::device::{Gpu, GpuError};
use crate::shader;

/// 管线编译所针对的颜色附件格式。
pub const COLOR_FORMAT: vk::Format = vk::Format::B8G8R8A8_SRGB;
/// 预 Pass 与 uber Pass 使用的深度格式（反向 Z）。
pub const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;

/// 每个绑定暴露的 Bindless 图集图层数。64 层 × 2048² R8 对应 256 MiB 地址空间，
/// 但图层按需分配，且 `PARTIALLY_BOUND` 使未使用槽位零开销。
pub const MAX_ATLAS_LAYERS: u32 = 64;

/// 供外部纹理使用的首个 Bindless RGBA 槽位索引。
///
/// `0..EXTERNAL_TEXTURE_BASE` 为内部图集页；其余 16 个槽位在运行时通过
/// [`crate::frame::Renderer::register_external_texture`] 注册。
pub const EXTERNAL_TEXTURE_BASE: u32 = 48;
/// 为外部纹理预留的 Bindless 槽位数量。
pub const EXTERNAL_TEXTURE_SLOTS: u32 = MAX_ATLAS_LAYERS - EXTERNAL_TEXTURE_BASE;

/// 外部 Bindless 槽位分配器。
///
/// 与设备对象解耦，使槽位簿记逻辑无需 GPU 即可进行单元测试。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalTextureSlots {
    used: u32,
}

impl ExternalTextureSlots {
    /// 分配索引最小的空闲槽位；若 16 个槽位已满则返回 `None`。
    pub fn allocate(&mut self) -> Option<u32> {
        for bit in 0..EXTERNAL_TEXTURE_SLOTS {
            if self.used & (1 << bit) == 0 {
                self.used |= 1 << bit;
                return Some(EXTERNAL_TEXTURE_BASE + bit);
            }
        }
        None
    }

    /// 释放由 [`Self::allocate`] 返回的槽位。若槽位越界或本就空闲则返回 `false`。
    pub fn release(&mut self, slot: u32) -> bool {
        if !Self::owns(slot) {
            return false;
        }
        let bit = 1 << (slot - EXTERNAL_TEXTURE_BASE);
        if self.used & bit == 0 {
            return false;
        }
        self.used &= !bit;
        true
    }

    /// 判断 `slot` 是否属于外部纹理槽位范围。
    #[must_use]
    pub const fn owns(slot: u32) -> bool {
        slot >= EXTERNAL_TEXTURE_BASE && slot < MAX_ATLAS_LAYERS
    }

    /// 当前已预留的外部槽位数量。
    #[must_use]
    pub const fn used(self) -> u32 {
        self.used.count_ones()
    }
}

/// GPU 驱动管线的各个计算阶段。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ComputeStage {
    Cull = 0,
    Scatter = 1,
    SortHist = 2,
    SortScan = 3,
    SortScatter = 4,
    Indirect = 5,
    /// MTSDF 打包 Pass；独立使用单独着色器，避免承担 1D 变换所需的共享内存开销。
    MtsdfPack = 6,
}

impl ComputeStage {
    const COUNT: usize = 7;

    #[must_use]
    pub const fn all() -> [Self; Self::COUNT] {
        [
            Self::Cull,
            Self::Scatter,
            Self::SortHist,
            Self::SortScan,
            Self::SortScatter,
            Self::Indirect,
            Self::MtsdfPack,
        ]
    }

    #[must_use]
    pub fn module(self) -> shader::ShaderModule {
        match self {
            Self::Cull => shader::cull(),
            Self::Scatter => shader::scatter(),
            Self::SortHist => shader::sort_hist(),
            Self::SortScan => shader::sort_scan(),
            Self::SortScatter => shader::sort_scatter(),
            Self::Indirect => shader::indirect(),
            Self::MtsdfPack => shader::mtsdf_pack(),
        }
    }

    /// 着色器中声明的工作组大小（`local_size_x`）。
    #[must_use]
    pub const fn workgroup_size(self) -> u32 {
        match self {
            Self::Scatter => 64,
            Self::Indirect => 1,
            Self::MtsdfPack => 64,
            _ => 256,
        }
    }
}

/// 用于保持 Bindless 槽位状态合法的 1×1 占位纹理。
#[derive(Debug)]
pub struct DummyTexture {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
}

/// 所有图形/计算管线以及全局唯一的 Bindless 描述符集。
pub struct Pipelines {
    pub graphics_layout: vk::PipelineLayout,
    pub compute_layout: vk::PipelineLayout,
    pub set_layout: vk::DescriptorSetLayout,
    pub descriptor_pool: vk::DescriptorPool,
    pub set: vk::DescriptorSet,
    pub uber_normal: vk::Pipeline,
    pub uber_additive: vk::Pipeline,
    pub opaque_core: vk::Pipeline,
    pub compute: [vk::Pipeline; ComputeStage::COUNT],
    /// 背景模糊描述符集布局：源采样器 + 存储图像。
    pub blur_set_layout: vk::DescriptorSetLayout,
    /// 两个模糊计算管线与背景合成管线共用的管线布局。
    pub blur_layout: vk::PipelineLayout,
    /// 分配模糊描述符集的描述符池。
    pub blur_pool: vk::DescriptorPool,
    /// 预分配的描述符集列表，每个模糊 Pass 对应一个。
    pub blur_sets: Vec<vk::DescriptorSet>,
    pub blur_down: vk::Pipeline,
    pub blur_up: vk::Pipeline,
    /// 全屏背景虚化合成图形管线。
    pub backdrop: vk::Pipeline,
    pub sampler: vk::Sampler,
    pub dummy_r8: DummyTexture,
    pub dummy_rgba: DummyTexture,
    /// 真实的 Bindless 图集；在渲染器安装前为 `None`。
    pub atlases: Option<std::sync::Arc<crate::atlas::AtlasSet>>,
}

impl core::fmt::Debug for Pipelines {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipelines")
            .field("normal", &self.uber_normal)
            .field("additive", &self.uber_additive)
            .field("opaque_core", &self.opaque_core)
            .finish_non_exhaustive()
    }
}

fn shader_module(gpu: &Gpu, module: shader::ShaderModule) -> Result<vk::ShaderModule, GpuError> {
    let words = module
        .words()
        .map_err(|e| GpuError::Allocation(format!("{}: {e}", module.name)))?;
    // SAFETY: `words` 为格式正确的 SPIR-V 模块（已在构建期校验）。
    unsafe {
        gpu.device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
            .map_err(GpuError::from)
    }
}

impl Pipelines {
    /// 针对给定的颜色附件格式构建所有管线。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn new(gpu: &Gpu) -> Result<Self, GpuError> {
        // SAFETY: 仅创建资源；所有对象均归返回的结构体所有，并由 `destroy` 统一销毁。
        unsafe { Self::new_inner(gpu) }
    }

    unsafe fn new_inner(gpu: &Gpu) -> Result<Self, GpuError> {
        let device = &gpu.device;
        let empty = vk::DescriptorSetLayoutBinding::default();

        // 描述符集布局
        // binding 0/1：Bindless 图集图层（R8 与 RGBA8）；binding 2：供顶点着色器读取的已排序图元索引。
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(MAX_ATLAS_LAYERS)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(MAX_ATLAS_LAYERS)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX),
        ];
        let _ = empty;

        let binding_flags = [
            vk::DescriptorBindingFlags::PARTIALLY_BOUND
                | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND,
            vk::DescriptorBindingFlags::PARTIALLY_BOUND
                | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND,
            vk::DescriptorBindingFlags::empty(),
        ];
        let mut flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);
        let set_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(&bindings)
                    .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
                    .push_next(&mut flags_info),
                None,
            )
            .map_err(GpuError::from)?;

        // 管线布局
        let set_layouts = [set_layout];
        let pc_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(crate::data::PushConstants::SIZE);
        let graphics_layout = device
            .create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&[pc_range]),
                None,
            )
            .map_err(GpuError::from)?;

        let compute_pc = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(crate::data::PushConstants::SIZE);
        let compute_layout = device
            .create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&[compute_pc]),
                None,
            )
            .map_err(GpuError::from)?;

        // 着色器模块
        let vert = shader_module(gpu, shader::uber_vert())?;
        let frag = shader_module(gpu, shader::uber_frag())?;
        let opaque_frag = shader_module(gpu, shader::opaque_core_frag())?;
        let opaque_vert = shader_module(gpu, shader::opaque_core_vert())?;

        let vert_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert)
            .name(c"main");
        let frag_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag)
            .name(c"main");
        let opaque_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(opaque_frag)
            .name(c"main");
        let opaque_vert_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(opaque_vert)
            .name(c"main");

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .primitive_restart_enable(false);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        // 反向 Z：最近为 1.0，因此“大于等于”通过测试的片元更近。
        // 不透明预 Pass 写入深度，uber Pass 仅执行深度测试。
        let depth_test_no_write = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);
        let depth_write = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let color_formats = [COLOR_FORMAT];
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats)
            .depth_attachment_format(DEPTH_FORMAT);

        // 混合状态
        let alpha_blend = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let additive_blend = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blends = [alpha_blend];
        let additive_blends = [additive_blend];
        let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&blends);
        let color_blend_additive = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&additive_blends);

        let create_graphics = |stages: &[vk::PipelineShaderStageCreateInfo],
                               depth: &vk::PipelineDepthStencilStateCreateInfo,
                               blend: &vk::PipelineColorBlendStateCreateInfo,
                               rendering: &mut vk::PipelineRenderingCreateInfo|
         -> Result<vk::Pipeline, GpuError> {
            device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::GraphicsPipelineCreateInfo::default()
                        .stages(stages)
                        .vertex_input_state(&vertex_input)
                        .input_assembly_state(&input_assembly)
                        .viewport_state(&viewport_state)
                        .rasterization_state(&raster)
                        .multisample_state(&multisample)
                        .depth_stencil_state(depth)
                        .color_blend_state(blend)
                        .dynamic_state(&dynamic)
                        .layout(graphics_layout)
                        .push_next(rendering)],
                    None,
                )
                .map_err(|(_, e)| GpuError::from(e))
                .map(|p| p[0])
        };

        let uber_normal = create_graphics(
            &[vert_stage, frag_stage],
            &depth_test_no_write,
            &color_blend,
            &mut rendering,
        )?;
        let uber_additive = create_graphics(
            &[vert_stage, frag_stage],
            &depth_test_no_write,
            &color_blend_additive,
            &mut rendering,
        )?;

        // 不透明预 Pass 仍须声明与所属动态渲染 Pass 相同的附件格式——在其中声明零颜色
        // 附件是非法的。此处改为屏蔽颜色写入掩码，零额外开销且保持纯深度写入。
        let mut depth_only = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats)
            .depth_attachment_format(DEPTH_FORMAT);
        let masked_attachment = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::empty())];
        let no_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&masked_attachment);
        let opaque_core = create_graphics(
            &[opaque_vert_stage, opaque_stage],
            &depth_write,
            &no_blend,
            &mut depth_only,
        )?;

        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
        device.destroy_shader_module(opaque_frag, None);
        device.destroy_shader_module(opaque_vert, None);

        // 计算管线
        let mut compute = [vk::Pipeline::null(); ComputeStage::COUNT];
        for stage in ComputeStage::all() {
            let module = shader_module(gpu, stage.module())?;
            let stage_info = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(c"main");
            let pipeline = device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage_info)
                        .layout(compute_layout)],
                    None,
                )
                .map_err(|(_, e)| GpuError::from(e))?[0];
            device.destroy_shader_module(module, None);
            compute[stage as usize] = pipeline;
        }

        // 背景模糊
        // 一个源纹理采样器，一个目标存储图像。合成 Pass 复用同一布局（仅使用其中采样器）。
        let blur_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE | vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let blur_set_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&blur_bindings),
                None,
            )
            .map_err(GpuError::from)?;

        let blur_set_layouts = [blur_set_layout];
        let blur_pc_range = vk::PushConstantRange::default()
            .stage_flags(
                vk::ShaderStageFlags::COMPUTE
                    | vk::ShaderStageFlags::VERTEX
                    | vk::ShaderStageFlags::FRAGMENT,
            )
            .offset(0)
            .size(crate::data::BlurPush::SIZE.max(crate::data::BackdropCompositePush::SIZE));
        let blur_layout = device
            .create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&blur_set_layouts)
                    .push_constant_ranges(&[blur_pc_range]),
                None,
            )
            .map_err(GpuError::from)?;

        let make_compute = |module: shader::ShaderModule| -> Result<vk::Pipeline, GpuError> {
            let module = shader_module(gpu, module)?;
            let stage_info = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(c"main");
            let pipeline = device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage_info)
                        .layout(blur_layout)],
                    None,
                )
                .map_err(|(_, e)| GpuError::from(e))?[0];
            device.destroy_shader_module(module, None);
            Ok(pipeline)
        };
        let blur_down = make_compute(shader::blur_down())?;
        let blur_up = make_compute(shader::blur_up())?;

        // 合成管线为无深度附件的普通 Alpha 混合全屏三角形：在主 Pass 之后运行，
        // 仅在请求矩形区域内绘制模糊背景。
        let backdrop = {
            let vert = shader_module(gpu, shader::backdrop_vert())?;
            let frag = shader_module(gpu, shader::backdrop_frag())?;
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vert)
                    .name(c"main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(frag)
                    .name(c"main"),
            ];
            let tri = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let no_depth = vk::PipelineDepthStencilStateCreateInfo::default();
            let mut rendering =
                vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
            let pipeline = device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::GraphicsPipelineCreateInfo::default()
                        .stages(&stages)
                        .vertex_input_state(&vertex_input)
                        .input_assembly_state(&tri)
                        .viewport_state(&viewport_state)
                        .rasterization_state(&raster)
                        .multisample_state(&multisample)
                        .depth_stencil_state(&no_depth)
                        .color_blend_state(&color_blend)
                        .dynamic_state(&dynamic)
                        .layout(blur_layout)
                        .push_next(&mut rendering)],
                    None,
                )
                .map_err(|(_, e)| GpuError::from(e))?[0];
            device.destroy_shader_module(vert, None);
            device.destroy_shader_module(frag, None);
            pipeline
        };

        let blur_pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(16),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(16),
        ];
        let blur_pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(16)
                    .pool_sizes(&blur_pool_sizes),
                None,
            )
            .map_err(GpuError::from)?;
        // 每轮模糊 4 个描述符集外加合成 Pass 1 个。当使用某描述符集的已录制调度仍在执行时
        // 不可重绑该集，因此每轮分配独立描述符集。
        let blur_layouts = [blur_set_layout; 13];
        let blur_sets = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(blur_pool)
                    .set_layouts(&blur_layouts),
            )
            .map_err(GpuError::from)?;

        // 描述符池与描述符集
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(MAX_ATLAS_LAYERS * 2),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(4),
        ];
        let descriptor_pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND)
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )
            .map_err(GpuError::from)?;
        let layouts = [set_layout];
        let set = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&layouts),
            )
            .map_err(GpuError::from)?[0];

        // 采样器与占位纹理
        let sampler = device
            .create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(16.0),
                None,
            )
            .map_err(GpuError::from)?;

        let dummy_r8 = create_dummy_texture(gpu, vk::Format::R8_UNORM)?;
        let dummy_rgba = create_dummy_texture(gpu, vk::Format::R8G8B8A8_UNORM)?;

        let pipelines = Self {
            graphics_layout,
            compute_layout,
            set_layout,
            descriptor_pool,
            set,
            uber_normal,
            uber_additive,
            opaque_core,
            compute,
            blur_set_layout,
            blur_layout,
            blur_pool,
            blur_sets,
            blur_down,
            blur_up,
            backdrop,
            sampler,
            dummy_r8,
            dummy_rgba,
            atlases: None,
        };
        pipelines.bind_dummy_atlases(gpu)?;

        Ok(pipelines)
    }

    fn bind_dummy_atlases(&self, gpu: &Gpu) -> Result<(), GpuError> {
        let image_infos: Vec<vk::DescriptorImageInfo> = (0..MAX_ATLAS_LAYERS)
            .map(|_| {
                vk::DescriptorImageInfo::default()
                    .sampler(self.sampler)
                    .image_view(self.dummy_r8.view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let rgba_infos: Vec<vk::DescriptorImageInfo> = (0..MAX_ATLAS_LAYERS)
            .map(|_| {
                vk::DescriptorImageInfo::default()
                    .sampler(self.sampler)
                    .image_view(self.dummy_rgba.view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_infos),
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&rgba_infos),
        ];
        // SAFETY: 描述符属于 `self.set` 且图像均处于存活状态。
        unsafe { gpu.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    /// 安装真实图集并重绑 Bindless 描述符槽位。
    ///
    /// 调用此方法前，所有槽位均指向 1x1 占位纹理；调用后，binding 0/1 的第 `n` 个
    /// 槽位分别指向 `R8`/`RGBA8` 图集的第 `n` 层。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn install_atlases(
        &mut self,
        gpu: &Gpu,
        atlases: std::sync::Arc<crate::atlas::AtlasSet>,
    ) -> Result<(), GpuError> {
        let r8: Vec<vk::DescriptorImageInfo> = (0..MAX_ATLAS_LAYERS)
            .map(|layer| {
                vk::DescriptorImageInfo::default()
                    .sampler(self.sampler)
                    .image_view(atlases.r8.layer_view(layer).unwrap_or(self.dummy_r8.view))
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        // 仅重写内部图集页对应的槽位：`EXTERNAL_TEXTURE_BASE` 及以上槽位归调用方所有，
        // 可能已指向已注册的外部纹理。
        debug_assert!(
            atlases.rgba.layers() <= EXTERNAL_TEXTURE_BASE,
            "the RGBA atlas grew past its {} bindless pages",
            EXTERNAL_TEXTURE_BASE
        );
        let rgba: Vec<vk::DescriptorImageInfo> = (0..EXTERNAL_TEXTURE_BASE)
            .map(|layer| {
                vk::DescriptorImageInfo::default()
                    .sampler(self.sampler)
                    .image_view(
                        atlases
                            .rgba
                            .layer_view(layer)
                            .unwrap_or(self.dummy_rgba.view),
                    )
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&r8),
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&rgba),
        ];
        // SAFETY: 描述符属于 `self.set`，且只要持有 `Arc`，对应图像即保持存活。
        unsafe { gpu.device.update_descriptor_sets(&writes, &[]) };
        self.atlases = Some(atlases);
        Ok(())
    }

    /// 将指定外部 Bindless 槽位指向调用方持有的纹理。
    ///
    /// `slot` 必须来自 [`ExternalTextureSlots::allocate`]。调用方保留句柄所有权，
    /// 并须在槽位释放前保证纹理视图与采样器存活。
    ///
    /// # Errors
    /// 当 `slot` 不属于外部槽位范围时返回 [`GpuError::InvalidArgument`]。
    pub fn bind_external_texture(
        &self,
        gpu: &Gpu,
        slot: u32,
        view: vk::ImageView,
        sampler: vk::Sampler,
    ) -> Result<(), GpuError> {
        if !ExternalTextureSlots::owns(slot) {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not an external texture slot \
                 ({EXTERNAL_TEXTURE_BASE}..{MAX_ATLAS_LAYERS})"
            )));
        }
        let info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(1)
            .dst_array_element(slot)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&info);
        // SAFETY: 描述符属于 `self.set`；调用方保证视图与采样器的生命周期长于槽位。
        unsafe { gpu.device.update_descriptor_sets(&[write], &[]) };
        Ok(())
    }

    /// 将已释放的外部槽位恢复为 1×1 占位纹理。
    ///
    /// # Errors
    /// 当 `slot` 不属于外部槽位范围时返回 [`GpuError::InvalidArgument`]。
    pub fn unbind_external_texture(&self, gpu: &Gpu, slot: u32) -> Result<(), GpuError> {
        if !ExternalTextureSlots::owns(slot) {
            return Err(GpuError::InvalidArgument(format!(
                "bindless slot {slot} is not an external texture slot"
            )));
        }
        self.bind_external_texture(gpu, slot, self.dummy_rgba.view, self.sampler)
    }

    /// 将顶点着色器的索引绑定指向当前帧的已排序索引列表。
    ///
    /// # Safety
    /// 在任何引用该缓冲区的帧处于在飞状态期间，`buffer` 必须保持存活。
    pub unsafe fn bind_sorted_indices(&self, gpu: &Gpu, buffer: &crate::device::Buffer) {
        let info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer.raw)
            .offset(0)
            .range(vk::WHOLE_SIZE)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&info);
        gpu.device.update_descriptor_sets(&[write], &[]);
    }

    /// 将一个预分配的模糊描述符集绑定到指定源/目标图像对。
    ///
    /// 渲染器仅在任一图像视图变化时调用此方法，因此稳态帧完全不执行描述符写入。
    ///
    /// # Safety
    /// 该描述符集当前不得被任何在飞帧使用。
    pub unsafe fn update_blur_set(
        &self,
        gpu: &Gpu,
        index: usize,
        source_view: vk::ImageView,
        source_layout: vk::ImageLayout,
        dest_view: vk::ImageView,
    ) {
        let source = vk::DescriptorImageInfo::default()
            .sampler(self.sampler)
            .image_view(source_view)
            .image_layout(source_layout);
        let dest = vk::DescriptorImageInfo::default()
            .image_view(dest_view)
            .image_layout(vk::ImageLayout::GENERAL);
        let set = self.blur_sets[index];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(core::slice::from_ref(&source)),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(core::slice::from_ref(&dest)),
        ];
        gpu.device.update_descriptor_sets(&writes, &[]);
    }

    /// # Safety
    /// 设备必须处于空闲状态。
    pub unsafe fn destroy(&self, gpu: &Gpu) {
        let device = &gpu.device;
        device.destroy_pipeline(self.uber_normal, None);
        device.destroy_pipeline(self.uber_additive, None);
        device.destroy_pipeline(self.opaque_core, None);
        for p in self.compute {
            if p != vk::Pipeline::null() {
                device.destroy_pipeline(p, None);
            }
        }
        device.destroy_pipeline(self.blur_down, None);
        device.destroy_pipeline(self.blur_up, None);
        device.destroy_pipeline(self.backdrop, None);
        device.destroy_descriptor_pool(self.blur_pool, None);
        device.destroy_pipeline_layout(self.blur_layout, None);
        device.destroy_descriptor_set_layout(self.blur_set_layout, None);
        device.destroy_sampler(self.sampler, None);
        for tex in [&self.dummy_r8, &self.dummy_rgba] {
            device.destroy_image_view(tex.view, None);
            device.destroy_image(tex.image, None);
            device.free_memory(tex.memory, None);
        }
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        device.destroy_pipeline_layout(self.graphics_layout, None);
        device.destroy_pipeline_layout(self.compute_layout, None);
        device.destroy_descriptor_set_layout(self.set_layout, None);
    }
}

unsafe fn create_dummy_texture(gpu: &Gpu, format: vk::Format) -> Result<DummyTexture, GpuError> {
    let device = &gpu.device;
    let image = device.create_image(
        &vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .initial_layout(vk::ImageLayout::UNDEFINED),
        None,
    )?;
    let requirements = device.get_image_memory_requirements(image);
    let type_index = gpu
        .memory_type_index_for_bits(
            requirements.memory_type_bits,
            crate::device::MemoryKind::DeviceLocal,
        )
        .ok_or_else(|| GpuError::Allocation("no device-local memory type".into()))?;
    let memory = device.allocate_memory(
        &vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(type_index),
        None,
    )?;
    device.bind_image_memory(image, memory, 0)?;

    // 转换到着色器可读布局并清除为纯白，使未绑定槽位采样结果为不透明白色而非未定义垃圾数据。
    let cmd = gpu.begin_single_time()?;
    let barrier = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::NONE)
        .src_access_mask(vk::AccessFlags2::NONE)
        .dst_stage_mask(vk::PipelineStageFlags2::CLEAR)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
        );
    let barriers = [barrier];
    device.cmd_pipeline_barrier2(
        cmd,
        &vk::DependencyInfo::default().image_memory_barriers(&barriers),
    );
    device.cmd_clear_color_image(
        cmd,
        image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        &vk::ClearColorValue {
            float32: [1.0, 1.0, 1.0, 1.0],
        },
        &[vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1)],
    );
    let to_read = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::CLEAR)
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
        );
    let read_barriers = [to_read];
    device.cmd_pipeline_barrier2(
        cmd,
        &vk::DependencyInfo::default().image_memory_barriers(&read_barriers),
    );
    gpu.end_single_time(cmd)?;

    let view = device.create_image_view(
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
    )?;
    Ok(DummyTexture {
        image,
        memory,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_stages_cover_every_shader() {
        let stages = ComputeStage::all();
        assert_eq!(stages.len(), ComputeStage::COUNT);
        for s in stages {
            assert!(s.module().magic_ok(), "{:?}", s);
            assert!(s.workgroup_size() > 0);
        }
    }

    #[test]
    fn external_slots_never_touch_the_atlas_pages() {
        assert_eq!(
            EXTERNAL_TEXTURE_BASE + EXTERNAL_TEXTURE_SLOTS,
            MAX_ATLAS_LAYERS
        );
        let mut slots = ExternalTextureSlots::default();
        let mut allocated = Vec::new();
        while let Some(slot) = slots.allocate() {
            assert!(slot >= EXTERNAL_TEXTURE_BASE);
            assert!(slot < MAX_ATLAS_LAYERS);
            allocated.push(slot);
        }
        assert_eq!(allocated.len(), EXTERNAL_TEXTURE_SLOTS as usize);
        assert_eq!(slots.allocate(), None);
        assert_eq!(slots.used(), EXTERNAL_TEXTURE_SLOTS);

        assert!(slots.release(allocated[3]));
        assert_eq!(slots.used(), EXTERNAL_TEXTURE_SLOTS - 1);
        assert_eq!(slots.allocate(), Some(allocated[3]));
        assert!(!slots.release(EXTERNAL_TEXTURE_BASE - 1));
        assert!(!slots.release(MAX_ATLAS_LAYERS));
    }

    #[test]
    fn pipelines_build_on_a_real_device() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let pipelines = Pipelines::new(&gpu).expect("pipelines");
        assert_ne!(pipelines.uber_normal, vk::Pipeline::null());
        assert_ne!(pipelines.opaque_core, vk::Pipeline::null());
        // SAFETY: 无在飞任务。
        unsafe { pipelines.destroy(&gpu) };
        gpu.destroy();
    }
}

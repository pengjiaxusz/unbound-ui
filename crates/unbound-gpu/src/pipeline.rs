//! Graphics and compute pipelines.
//!
//! Two graphics pipelines cover every primitive in the engine:
//!
//! | pipeline | depth | blend | used for |
//! | :--- | :--- | :--- | :--- |
//! | `uber_normal` | test, no write | src-alpha / one-minus-src-alpha | 99% of the UI |
//! | `uber_additive` | test, no write | one / one | highlights, cursor trails |
//!
//! plus `opaque_core`, the depth-only prepass that lets the hardware Early-Z
//! reject everything hidden behind a large panel (`docs` §3.1), and one compute
//! pipeline per GPU-driven stage (cull, scatter, 3× radix, indirect).
//!
//! Like [`crate::device`], this module is a thin wrapper over the Vulkan C API
//! and therefore opts out of the crate-wide `unsafe_op_in_unsafe_fn` deny.

#![allow(unsafe_op_in_unsafe_fn)]

use ash::vk;

use crate::device::{Gpu, GpuError};
use crate::shader;

/// Colour format the pipelines are compiled against.
pub const COLOR_FORMAT: vk::Format = vk::Format::B8G8R8A8_SRGB;
/// Depth format used by the prepass and the uber pass (reverse-Z).
pub const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;

/// Number of bindless atlas layers per binding. 64 layers × 2048² R8 is 256 MiB
/// of address space, but layers are allocated lazily and `PARTIALLY_BOUND`
/// makes unused slots free.
pub const MAX_ATLAS_LAYERS: u32 = 64;

/// Compute stages of the GPU-driven pipeline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ComputeStage {
    Cull = 0,
    Scatter = 1,
    SortHist = 2,
    SortScan = 3,
    SortScatter = 4,
    Indirect = 5,
}

impl ComputeStage {
    const COUNT: usize = 6;

    #[must_use]
    pub const fn all() -> [Self; Self::COUNT] {
        [
            Self::Cull,
            Self::Scatter,
            Self::SortHist,
            Self::SortScan,
            Self::SortScatter,
            Self::Indirect,
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
        }
    }

    /// Workgroup size declared in the shader (`local_size_x`).
    #[must_use]
    pub const fn workgroup_size(self) -> u32 {
        match self {
            Self::Scatter => 64,
            Self::Indirect => 1,
            _ => 256,
        }
    }
}

/// A 1×1 texture used to keep bindless slots well-defined.
#[derive(Debug)]
pub struct DummyTexture {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
}

/// All pipelines and the single bindless descriptor set.
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
    pub sampler: vk::Sampler,
    pub dummy_r8: DummyTexture,
    pub dummy_rgba: DummyTexture,
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
    // SAFETY: `words` is a well-formed SPIR-V blob (validated at build time).
    unsafe {
        gpu.device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
            .map_err(GpuError::from)
    }
}

impl Pipelines {
    /// Build every pipeline for the given colour attachment format.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn new(gpu: &Gpu) -> Result<Self, GpuError> {
        // SAFETY: creation only; every object is owned by the returned struct
        // and destroyed by `destroy`.
        unsafe { Self::new_inner(gpu) }
    }

    unsafe fn new_inner(gpu: &Gpu) -> Result<Self, GpuError> {
        let device = &gpu.device;
        let empty = vk::DescriptorSetLayoutBinding::default();

        // ---- descriptor set layout ----------------------------------------
        // binding 0/1: bindless atlas layers (R8 and RGBA8), binding 2: sorted
        // primitive indices consumed by the vertex shader.
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

        let mut binding_flags = [
            vk::DescriptorBindingFlags::PARTIALLY_BOUND
                | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND,
            vk::DescriptorBindingFlags::PARTIALLY_BOUND
                | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND,
            vk::DescriptorBindingFlags::empty(),
        ];
        let mut flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&mut binding_flags);
        let set_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(&bindings)
                    .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
                    .push_next(&mut flags_info),
                None,
            )
            .map_err(GpuError::from)?;

        // ---- pipeline layouts ---------------------------------------------
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

        // ---- modules -------------------------------------------------------
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

        // Reverse-Z: nearest == 1.0, so "greater or equal" passes for closer
        // fragments. The opaque prepass writes depth, the uber pass only tests.
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

        // ---- blend states ---------------------------------------------------
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

        // The opaque prepass must still declare the *same* attachment formats as
        // the dynamic-rendering pass it runs inside — a pipeline with zero
        // colour attachments is invalid there. Colour writes are masked off
        // instead, which costs nothing and keeps the pass purely depth.
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

        // ---- compute pipelines ---------------------------------------------
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

        // ---- descriptor pool + set ------------------------------------------
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

        // ---- sampler + dummy textures ---------------------------------------
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
            sampler,
            dummy_r8,
            dummy_rgba,
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
        // SAFETY: descriptors belong to `self.set` and the images are alive.
        unsafe { gpu.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    /// Point the vertex shader's index binding at the frame's sorted list.
    ///
    /// # Safety
    /// `buffer` must stay alive while any frame referencing it is in flight.
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

    /// # Safety
    /// The device must be idle.
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
        .memory_type_index_for_bits(requirements.memory_type_bits, crate::device::MemoryKind::DeviceLocal)
        .ok_or_else(|| GpuError::Allocation("no device-local memory type".into()))?;
    let memory = device.allocate_memory(
        &vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(type_index),
        None,
    )?;
    device.bind_image_memory(image, memory, 0)?;

    // Transition to shader-read and clear to white so an unbound slot reads as
    // an opaque white texel rather than undefined garbage.
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
        assert_eq!(stages.len(), 6);
        for s in stages {
            assert!(s.module().magic_ok(), "{:?}", s);
            assert!(s.workgroup_size() > 0);
        }
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
        // SAFETY: nothing is in flight.
        unsafe { pipelines.destroy(&gpu) };
        gpu.destroy();
    }
}

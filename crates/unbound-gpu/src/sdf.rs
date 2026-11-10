//! GPU distance-field generation (`docs` §3.5).
//!
//! A thin, self-contained wrapper around `mtsdf.comp`: it owns three device
//! buffers and a compute pipeline, so the caller can hand it a coverage bitmap
//! and get an RGBA8 distance field back without knowing anything about Vulkan.
//!
//! Why a separate pipeline instead of the frame's compute stages: glyph
//! generation happens on demand, once per (glyph, size) pair, and must never
//! block the frame's command buffer. Keeping its own buffers means it can run on
//! the transfer queue or an async compute queue later without touching the frame
//! graph.

#![allow(unsafe_op_in_unsafe_fn)]

use ash::vk;

use crate::data::PushConstants;
use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use crate::pipeline::Pipelines;
use crate::shader;

/// Largest tile the pipeline can process (both dimensions).
pub const MAX_TILE: u32 = 512;

/// GPU distance-field generator.
pub struct SdfPipeline {
    pipeline: vk::Pipeline,
    /// Borrowed from [`Pipelines`]; the owner outlives this struct.
    layout: vk::PipelineLayout,
    coverage: Buffer,
    temp: Buffer,
    output: Buffer,
    staging: Buffer,
}

impl core::fmt::Debug for SdfPipeline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SdfPipeline")
            .field("max_tile", &MAX_TILE)
            .finish_non_exhaustive()
    }
}

impl SdfPipeline {
    /// Create the pipeline and its buffers.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn new(gpu: &Gpu, pipelines: &Pipelines) -> Result<Self, GpuError> {
        let words = shader::mtsdf()
            .words()
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        // SAFETY: the SPIR-V blob is validated at build time and every object is
        // owned by the returned struct.
        unsafe {
            let module = gpu
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?;
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(c"main");
            let pipeline = gpu
                .device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage)
                        .layout(pipelines.compute_layout)],
                    None,
                )
                .map_err(|(_, e)| GpuError::from(e))?[0];
            gpu.device.destroy_shader_module(module, None);

            let texels = u64::from(MAX_TILE) * u64::from(MAX_TILE);
            let bda = vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC;
            let coverage = gpu.create_buffer(texels, bda, MemoryKind::HostVisible, "sdf-coverage")?;
            let temp = gpu.create_buffer(texels * 8, bda, MemoryKind::DeviceLocal, "sdf-temp")?;
            let output = gpu.create_buffer(texels * 4, bda, MemoryKind::DeviceLocal, "sdf-out")?;
            let staging = gpu.create_buffer(
                texels * 4,
                vk::BufferUsageFlags::TRANSFER_DST,
                MemoryKind::HostVisible,
                "sdf-readback",
            )?;
            Ok(Self {
                pipeline,
                layout: pipelines.compute_layout,
                coverage,
                temp,
                output,
                staging,
            })
        }
    }

    /// Generate an RGBA8 distance field for a coverage bitmap.
    ///
    /// # Errors
    /// Returns an error when the tile is larger than [`MAX_TILE`] or a Vulkan
    /// call fails.
    pub fn generate(
        &self,
        gpu: &Gpu,
        coverage: &[u8],
        width: u32,
        height: u32,
        spread: f32,
    ) -> Result<Vec<u8>, GpuError> {
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        if width > MAX_TILE || height > MAX_TILE {
            return Err(GpuError::Allocation(format!(
                "glyph tile {width}x{height} exceeds MAX_TILE {MAX_TILE}"
            )));
        }
        let texels = width as usize * height as usize;
        if coverage.len() < texels {
            return Err(GpuError::Allocation("coverage bitmap too small".into()));
        }

        // SAFETY: the buffers are exclusively ours and the device is idle after
        // the blocking submission below.
        unsafe {
            let ptr = self
                .coverage
                .host_ptr(0)
                .ok_or_else(|| GpuError::Allocation("sdf coverage not mapped".into()))?;
            core::ptr::copy_nonoverlapping(coverage.as_ptr(), ptr, texels);

            let cmd = gpu.begin_single_time()?;
            let packed_spread = (spread.clamp(0.25, 64.0) * 256.0).round() as u32;
            let base = PushConstants {
                prim_count: width,
                block_count: height,
                digit: packed_spread,
                scratch_bda: self.coverage.address,
                scratch_bda2: self.temp.address,
                scratch_bda3: self.output.address,
                ..Default::default()
            };

            gpu.device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline,
            );

            // pass 0: horizontal, one invocation per row.
            let mut pc = PushConstants { aux: 0, ..base };
            gpu.device.cmd_push_constants(
                cmd,
                // The layout is identical for every compute pipeline; reuse the
                // one the pipeline was created with.
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&pc),
            );
            gpu.device.cmd_dispatch(cmd, height, 1, 1);
            self.barrier(gpu, cmd);

            // pass 1: vertical, one invocation per column.
            pc.aux = 1;
            gpu.device.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&pc),
            );
            gpu.device.cmd_dispatch(cmd, width, 1, 1);
            self.barrier(gpu, cmd);

            // pass 2: pack, one invocation per texel.
            pc.aux = 2;
            // `local_size_x = 1` and one invocation handles exactly one texel, so
            // the group count is the texel count (not divided by 64).
            let groups = width * height;
            gpu.device.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&pc),
            );
            gpu.device.cmd_dispatch(cmd, groups, 1, 1);

            // Copy the result out.
            let copy_barrier = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&copy_barrier),
            );
            gpu.device.cmd_copy_buffer(
                cmd,
                self.output.raw,
                self.staging.raw,
                &[vk::BufferCopy::default().size(texels as u64 * 4)],
            );
            gpu.end_single_time(cmd)?;

            let mut out = vec![0u8; texels * 4];
            core::ptr::copy_nonoverlapping(self.staging.mapped, out.as_mut_ptr(), texels * 4);
            Ok(out)
        }
    }

    fn barrier(&self, gpu: &Gpu, cmd: vk::CommandBuffer) {
        let barrier = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE)];
        // SAFETY: recording into a command buffer we own.
        unsafe {
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&barrier),
            );
        }
    }

    /// # Safety
    /// The device must be idle.
    pub unsafe fn destroy(&self, gpu: &Gpu) {
        gpu.device.destroy_pipeline(self.pipeline, None);
        for b in [&self.coverage, &self.temp, &self.output, &self.staging] {
            gpu.destroy_buffer(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::gpu_test_guard;

    #[test]
    fn gpu_field_matches_the_expected_sign_and_edge() {
        let _guard = gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let pipelines = Pipelines::new(&gpu).expect("pipelines");
        let sdf = SdfPipeline::new(&gpu, &pipelines).expect("sdf pipeline");

        // 16x16 with a filled 8x8 block in the middle.
        let mut coverage = vec![0u8; 16 * 16];
        for y in 4..12 {
            for x in 4..12 {
                coverage[y * 16 + x] = 255;
            }
        }
        let field = sdf.generate(&gpu, &coverage, 16, 16, 4.0).expect("generate");
        assert_eq!(field.len(), 16 * 16 * 4);

        // Encoding matches `uber.frag`: > 0.5 inside, < 0.5 outside, 0.5 on the
        // edge. A deep interior texel saturates at 1.0.
        let sample = |x: usize, y: usize| field[(y * 16 + x) * 4] as f32 / 255.0;
        assert!(sample(8, 8) > 0.9, "centre = {}", sample(8, 8));
        assert!(sample(0, 0) < 0.1, "corner = {}", sample(0, 0));
        // One texel outside the block edge: 0.5 - 1/(2*4) = 0.375.
        let edge = sample(3, 8);
        assert!((0.25..0.5).contains(&edge), "edge = {edge}");
        // Alpha preserves the coverage.
        assert_eq!(field[(8 * 16 + 8) * 4 + 3], 255);
        assert_eq!(field[0 * 16 * 4 + 3], 0);

        // SAFETY: nothing in flight.
        unsafe {
            sdf.destroy(&gpu);
            pipelines.destroy(&gpu);
        }
        gpu.destroy();
    }

    #[test]
    fn oversized_tile_is_rejected() {
        let _guard = gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let pipelines = Pipelines::new(&gpu).expect("pipelines");
        let sdf = SdfPipeline::new(&gpu, &pipelines).expect("sdf pipeline");
        let cov = vec![0u8; 1024];
        assert!(sdf.generate(&gpu, &cov, MAX_TILE + 1, 1, 4.0).is_err());
        // SAFETY: nothing in flight.
        unsafe {
            sdf.destroy(&gpu);
            pipelines.destroy(&gpu);
        }
        gpu.destroy();
    }
}

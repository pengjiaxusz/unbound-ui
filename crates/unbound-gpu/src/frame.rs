//! Frame orchestration: the GPU-driven pipeline end to end.
//!
//! One call to [`Renderer::render`] performs, in order:
//!
//! ```text
//! CPU: bump-allocate the transient ring, hash/diff the scene, build patches
//!  1. upload      : host writes into the ring, patches describe destinations
//!  2. scatter     : compute copies the patches into the persistent pool
//!  3. cull        : compute frustum-culls and emits 64-bit keys
//!  4. sort        : 8 stable LSD radix passes over (key, index)
//!  5. indirect    : compute compacts equal-run keys into draw commands
//!  6. opaque core : depth-only prepass, front-to-back, builds the Hi-Z mask
//!  7. uber pass   : one MDI draw, back-to-front, Early-Z rejects the hidden
//! ```
//!
//! Everything the CPU touches per frame is O(changed nodes); the O(visible)
//! work happens on the GPU. A camera-only frame uploads **0 bytes** — the test
//! `camera_only_frame_uploads_nothing` asserts exactly that.

#![allow(unsafe_op_in_unsafe_fn)]

use ash::vk;

use crate::data::{
    DrawIndirectCommand, FramePushConstants, GpuClipNode, GpuPayload, GpuPrimitiveInstance,
    GpuScatterPatch, GpuTransform, PassLayer, PushConstants,
};
use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use crate::pipeline::{Pipelines, COLOR_FORMAT, DEPTH_FORMAT, MAX_ATLAS_LAYERS};
use crate::pool::{PersistentPool, PoolHandle};
use crate::ring::TransientRing;

/// Number of radix digits (8 bits each) needed for a full 64-bit key.
const RADIX_PASSES: u32 = 8;
/// Elements per radix block; must match `BLOCK` in the sort shaders.
const RADIX_BLOCK: u32 = 256;
/// Upper bound on indirect draws per layer pass.
pub const MAX_DRAW_RUNS: u32 = 16;

/// Offscreen render target (colour + depth).
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
    layout: vk::ImageLayout,
}

/// Camera state for one layer pass.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Camera {
    pub chunk: [i32; 2],
    pub local: [f32; 2],
    pub zoom: f32,
    pub dpi_scale: f32,
}

impl Camera {
    /// Camera for the static UI layer.
    ///
    /// Convention: `local`/`chunk` describe the world point that maps to the
    /// **centre** of the viewport, so a UI camera sits at
    /// `viewport_size / (2 · dpi_scale)` in logical units and a canvas camera
    /// sits at `canvas_centre`. This is what lets one shader formula serve a
    /// pannable canvas and a static panel without a branch.
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

    /// Camera centred on a canvas world position.
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

/// Everything the renderer needs for one layer pass.
#[derive(Debug)]
pub struct SceneUpload<'a> {
    pub primitives: &'a [GpuPrimitiveInstance],
    pub transforms: &'a [GpuTransform],
    pub clips: &'a [GpuClipNode],
    pub payloads: &'a [GpuPayload],
    /// Bumped by the caller whenever any of the four arrays changed. Equal
    /// generations skip the upload entirely (this is what makes a pan free).
    pub generation: u64,
    pub camera: Camera,
    /// `(layer << 56) | (barrier << 40) | (variant << 36)` prefix for the
    /// 64-bit sort key.
    pub key_prefix: u64,
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
        }
    }
}

/// Per-frame counters, surfaced by the debug overlay and asserted by the KPI
/// tests.
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
    /// Draw calls the main pass issued. The KPI red line is `<= 4`.
    #[must_use]
    pub fn mdi_draw_calls(&self) -> u32 {
        self.draw_calls
    }
}

/// The renderer: device resources + frame execution.
pub struct Renderer {
    pub gpu: Gpu,
    pub pipelines: Pipelines,
    pub target: RenderTarget,
    pool: PersistentPool,
    pool_buffer: Buffer,
    ring: TransientRing,
    ring_buffer: Buffer,
    lut_buffer: Buffer,

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
    last_generation: u64,
    frame_index: u64,
    pub stats: FrameStats,
}

impl core::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Renderer")
            .field("target", &format_args!("{}x{}", self.target.width, self.target.height))
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

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
        // SAFETY: creation only; ownership is returned to the caller.
        unsafe {
            let (color, color_memory, color_view) =
                Self::create_image(gpu, width, height, COLOR_FORMAT, vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST, false)?;
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
    /// The device must be idle.
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
    /// Build all persistent GPU resources for an offscreen target.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn new(gpu: Gpu, width: u32, height: u32) -> Result<Self, GpuError> {
        let pipelines = Pipelines::new(&gpu)?;
        let target = RenderTarget::new(&gpu, width, height)?;

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

        let bda_usage = vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::TRANSFER_DST
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let pool_buffer = gpu.create_buffer(pool_bytes, bda_usage, MemoryKind::DeviceLocal, "pool")?;
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

        // Worst-case primitive count is bounded by the pool and by the ring.
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
            // `vkCmdDrawIndirectCount` requires INDIRECT_BUFFER on both the
            // command buffer and the count buffer, on top of the BDA/STOGARE
            // usage the compute stage needs.
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
                // The culling/indirect stages address this buffer through BDA;
                // without the usage flag the address is 0 and the shader would
                // write to NULL (instant device loss).
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

        // SAFETY: the buffer is persistent and nothing is in flight yet.
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
            prim_handle,
            transform_handle,
            clip_handle,
            payload_handle,
            prim_capacity: 64 * 1024,
            transform_capacity: 16 * 1024,
            clip_capacity: 16 * 1024,
            payload_capacity: 16 * 1024,
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
            last_generation: u64::MAX,
            frame_index: 0,
            stats: FrameStats::default(),
        };
        renderer.write_default_lut()?;
        Ok(renderer)
    }

    /// Write the 4 KiB theme LUT (`docs` §4.5). Only this buffer changes when
    /// the theme flips; no primitive is re-uploaded and nothing re-lays out.
    ///
    /// # Errors
    /// Propagates write failures.
    pub fn write_theme(&mut self, colors: &[[u16; 4]]) -> Result<(), GpuError> {
        let mut entries = vec![[0u16; 4]; 512];
        for (i, c) in colors.iter().take(512).enumerate() {
            entries[i] = *c;
        }
        // Layout: 512 × (RGBA16F) = 4 KiB, stored as four u16 per entry.
        // SAFETY: the LUT buffer is host visible and 4096 bytes long.
        unsafe {
            let ptr = self
                .lut_buffer
                .host_ptr(0)
                .ok_or_else(|| GpuError::Allocation("lut not mapped".into()))?;
            core::ptr::copy_nonoverlapping(
                entries.as_ptr().cast::<u8>(),
                ptr,
                entries.len() * 8,
            );
        }
        Ok(())
    }

    fn write_default_lut(&mut self) -> Result<(), GpuError> {
        use unbound_math::color::Color;
        let mut colors = vec![[0u16; 4]; 512];
        colors[1] = Color::from_srgb8(0, 0, 0, 40).to_rgba16f(); // global shadow
        colors[2] = Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
        colors[3] = Color::from_srgb8(24, 24, 27, 255).to_rgba16f();
        colors[4] = Color::from_srgb8(63, 63, 70, 255).to_rgba16f();
        colors[5] = Color::from_srgb8(228, 228, 231, 255).to_rgba16f();
        self.write_theme(&colors)
    }

    /// Current target size.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.target.width, self.target.height)
    }

    /// Resize the offscreen target (and every dependent image view).
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if width == self.target.width && height == self.target.height {
            return Ok(());
        }
        self.gpu.wait_idle();
        // SAFETY: the device is idle.
        unsafe { self.target.destroy(&self.gpu) };
        self.target = RenderTarget::new(&self.gpu, width, height)?;
        Ok(())
    }

    fn ensure_pool_capacity(&mut self, len: usize, which: Which) -> Result<(), GpuError> {
        let (handle, cap) = match which {
            Which::Primitives => (self.prim_handle, self.prim_capacity),
            Which::Transforms => (self.transform_handle, self.transform_capacity),
            Which::Clips => (self.clip_handle, self.clip_capacity),
            Which::Payloads => (self.payload_handle, self.payload_capacity),
        };
        let needed = (len as u64).max(1);
        if needed <= cap {
            return Ok(());
        }
        // Double until it fits so growth is amortised.
        let mut new_cap = cap.max(4096);
        while new_cap < needed {
            new_cap *= 2;
        }
        let epoch = self.frame_index + self.gpu.frames_in_flight() as u64;
        let new_handle = self
            .pool
            .grow(handle, new_cap as u32, epoch)
            .map_err(|e| GpuError::Allocation(e.to_string()))?;
        match which {
            Which::Primitives => {
                self.prim_handle = new_handle;
                self.prim_capacity = new_cap;
            }
            Which::Transforms => {
                self.transform_handle = new_handle;
                self.transform_capacity = new_cap;
            }
            Which::Clips => {
                self.clip_handle = new_handle;
                self.clip_capacity = new_cap;
            }
            Which::Payloads => {
                self.payload_handle = new_handle;
                self.payload_capacity = new_cap;
            }
        }
        // The arrays moved: force a full re-upload next frame.
        self.last_generation = u64::MAX;
        Ok(())
    }

    /// Render one layer pass offscreen and return its statistics.
    ///
    /// # Errors
    /// Propagates Vulkan failures (including device loss).
    pub fn render(&mut self, scene: &SceneUpload<'_>) -> Result<FrameStats, GpuError> {
        let frame_count = self.gpu.frames_in_flight();
        let frame_slot = (self.frame_index % u64::from(frame_count)) as usize;
        let cmd = self.gpu.frames[frame_slot].command_buffer;
        {
            let gpu = &self.gpu;
            let sync = &gpu.frames[frame_slot];
            // SAFETY: the fence, command buffer and queue belong to this
            // renderer; waiting on our own fence cannot deadlock.
            unsafe {
                gpu.device
                    .wait_for_fences(&[sync.in_flight], true, u64::MAX)?;
                gpu.device.reset_fences(&[sync.in_flight])?;
                gpu.device.reset_command_buffer(
                    cmd,
                    vk::CommandBufferResetFlags::empty(),
                )?;
            }
        }

        self.stats = FrameStats {
            submitted: scene.primitives.len() as u32,
            ..Default::default()
        };

        let upload = self.prepare_upload(scene)?;
        self.stats.patches = upload.patch_count;
        self.stats.bytes_uploaded = upload.bytes;

        let timeline_value = self.frame_index + 1;
        // SAFETY: the command buffer is exclusively ours until the submit below
        // and the fence guarantees the previous submission has retired.
        unsafe {
            self.gpu.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        self.record_frame(cmd, scene, &upload, frame_slot)?;

        {
            let gpu = &self.gpu;
            let sync = &gpu.frames[frame_slot];
            // SAFETY: the command buffer was just closed and is submitted once.
            unsafe {
                gpu.device.end_command_buffer(cmd)?;
                // Offscreen: nothing signals `image_available`, so waiting on it
                // would deadlock. Swapchain presentation supplies that wait.
                let signal_semaphores = [gpu.timeline.semaphore];
                let values = [timeline_value];
                let mut timeline_info =
                    vk::TimelineSemaphoreSubmitInfo::default().signal_semaphore_values(&values);
                let cmds = [cmd];
                let submit = vk::SubmitInfo::default()
                    .command_buffers(&cmds)
                    .signal_semaphores(&signal_semaphores)
                    .push_next(&mut timeline_info);
                gpu.device
                    .queue_submit(gpu.graphics_queue, &[submit], sync.in_flight)?;
            }
        }
        self.frame_index += 1;
        Ok(self.stats)
    }

    /// Reset the visibility counter and hand back the resolved addresses.
    fn record_frame(
        &mut self,
        cmd: vk::CommandBuffer,
        scene: &SceneUpload<'_>,
        upload: &UploadPlan,
        _frame_slot: usize,
    ) -> Result<(), GpuError> {
        let gpu = &self.gpu;
        let device = &gpu.device;

        // ---- clear counters -------------------------------------------------
        // SAFETY: our own command buffer, 64 bytes inside a 64-byte buffer.
        unsafe { device.cmd_fill_buffer(cmd, self.counters.raw, 0, 64, 0) };

        // ---- 1/2. scatter ----------------------------------------------------
        if upload.patch_count > 0 {
            let mut pc = PushConstants {
                prim_count: upload.patch_count,
                scratch_bda: self.patches.address,
                scratch_bda2: self.pool_buffer.address,
                scratch_bda3: self.ring_buffer.address,
                ..Default::default()
            };
            self.dispatch(
                cmd,
                crate::pipeline::ComputeStage::Scatter,
                &mut pc,
                upload.patch_count.div_ceil(64),
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
            // SAFETY: recording into our own command buffer.
            unsafe {
                device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&barriers),
                )
            };
        }

        // ---- 3. cull ---------------------------------------------------------
        let n = scene.primitives.len() as u32;
        // Unused slots start at the maximum key so they sort to the very end,
        // where `indirect.comp` stops. This must happen before the cull pass,
        // which overwrites slots [0, visible).
        self.clear_keys_tail(cmd, n)?;
        let barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE);
        let barriers = [barrier];
        // SAFETY: recording into our own command buffer.
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

        // ---- 4. radix sort ---------------------------------------------------
        // Sort the *whole* key array: entries beyond the visible count were
        // filled with the maximum key by the clear below, so they stay at the
        // end. `n` is the maximum possible visible count.
        if n > 0 {
            let barrier = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .dst_access_mask(
                    vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE,
                );
            let barriers = [barrier];
            // SAFETY: recording into our own command buffer.
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
                self.dispatch(cmd, crate::pipeline::ComputeStage::SortHist, &mut hist, blocks);
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

        // ---- 5. indirect -----------------------------------------------------
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

        // The compute stage wrote the draw commands and the run count; both must
        // be made visible to the *indirect* stage specifically. A
        // COMPUTE→COMPUTE barrier (what `pipeline_barrier` emits) is not enough
        // and silently yields zero instances.
        {
            let barrier = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
                .dst_stage_mask(
                    vk::PipelineStageFlags2::DRAW_INDIRECT
                        | vk::PipelineStageFlags2::VERTEX_SHADER,
                )
                .dst_access_mask(
                    vk::AccessFlags2::INDIRECT_COMMAND_READ
                        | vk::AccessFlags2::SHADER_READ,
                );
            let barriers = [barrier];
            // SAFETY: recording into our own command buffer.
            unsafe {
                device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(&barriers),
                )
            };
        }

        // ---- 6/7. graphics ---------------------------------------------------
        self.begin_rendering(cmd);
        self.record_graphics(cmd, scene);
        self.end_rendering(cmd);

        Ok(())
    }

    fn pipeline_barrier(&self, cmd: vk::CommandBuffer) {
        let barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::SHADER_WRITE);
        let barriers = [barrier];
        // SAFETY: recording into our own command buffer.
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
        // SAFETY: recording into our own command buffer; `pc` is 128B.
        unsafe {
            let device = &self.gpu.device;
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipelines.compute[stage as usize]);
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
            primitives_bda: self.pool_buffer.address + u64::from(self.pool_offset(self.prim_handle)),
            transforms_bda: self.pool_buffer.address
                + u64::from(self.pool_offset(self.transform_handle)),
            clips_bda: self.pool_buffer.address + u64::from(self.pool_offset(self.clip_handle)),
            theme_lut_bda: self.lut_buffer.address,
            payloads_bda: self.pool_buffer.address + u64::from(self.pool_offset(self.payload_handle)),
            camera_chunk: scene.camera.chunk,
            camera_local: scene.camera.local,
            viewport_size: [self.target.width as f32, self.target.height as f32],
            camera_zoom: scene.camera.zoom,
            dpi_scale: scene.camera.dpi_scale,
        }
    }

    fn pool_offset(&self, handle: PoolHandle) -> u32 {
        self.pool.offset_of(handle).unwrap_or(0)
    }

    fn clear_keys_tail(&self, cmd: vk::CommandBuffer, n: u32) -> Result<(), GpuError> {
        // Keys are sorted ascending and unused slots must land at the end, so
        // they are filled with 0xFFFF…; the matching index slots are zeroed.
        // `vkCmdFillBuffer` writes 32-bit patterns, so fill the whole range with
        // 0xFFFFFFFF — that is exactly the maximum u64 key.
        let bytes = u64::from(n) * 8;
        // SAFETY: range is inside the buffer (capacity_primitives >= n).
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
        // SAFETY: recording; the target is exclusively owned.
        unsafe {
            let to_attachment = [image_barrier(
                self.target.color,
                self.target.layout,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE,
                vk::AccessFlags2::NONE,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                false,
            )];
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
            self.target.layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;

            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.07, 0.07, 0.08, 1.0],
                },
            }];
            let depth_clear = vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    // Reverse-Z: the far plane is 0.
                    depth: 0.0,
                    stencil: 0,
                },
            };
            let color_attachment = [vk::RenderingAttachmentInfo::default()
                .image_view(self.target.color_view)
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
                    width: self.target.width,
                    height: self.target.height,
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
        // SAFETY: matches `begin_rendering`.
        unsafe {
            self.gpu.device.cmd_end_rendering(cmd);
        }
    }

    fn record_graphics(&mut self, cmd: vk::CommandBuffer, scene: &SceneUpload<'_>) {
        let device = &self.gpu.device;
        let extent = vk::Extent2D {
            width: self.target.width,
            height: self.target.height,
        };
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

        // SAFETY: recording; all handles are alive and owned by `self`.
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

            // Pass A: opaque core, front-to-back, depth only. Uses the sorted
            // (back-to-front) list with GREATER_OR_EQUAL, so the nearest opaque
            // core wins the depth buffer and everything behind it Early-Z dies.
            draw(self.pipelines.opaque_core);

            // Pass B: the single blended uber draw.
            draw(self.pipelines.uber_normal);

            self.stats.draw_calls = 2;
        }
    }

    /// Copy the colour target into a host buffer and return tightly packed
    /// BGRA8 rows. Used by the golden-image tests and the screenshot command.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn read_pixels(&mut self) -> Result<Vec<u8>, GpuError> {
        let width = self.target.width;
        let height = self.target.height;
        let row_bytes = u64::from(width) * 4;
        let size = row_bytes * u64::from(height);
        let device = &self.gpu.device;
        // SAFETY: the GPU is idle after `wait_idle`, so the copy cannot race.
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

            // The GPU is idle, so the counters are final: surface them.
            let counters = core::slice::from_raw_parts(self.counters.mapped.cast::<u32>(), 4);
            self.stats.visible = counters[2];
            self.stats.draw_calls = counters[1];
            Ok(out)
        }
    }

    /// Upload the scene into the persistent pool if (and only if) it changed.
    fn prepare_upload(&mut self, scene: &SceneUpload<'_>) -> Result<UploadPlan, GpuError> {
        self.ensure_pool_capacity(scene.primitives.len(), Which::Primitives)?;
        self.ensure_pool_capacity(scene.transforms.len(), Which::Transforms)?;
        self.ensure_pool_capacity(scene.clips.len(), Which::Clips)?;
        self.ensure_pool_capacity(scene.payloads.len(), Which::Payloads)?;

        let mut plan = UploadPlan::default();
        if self.last_generation == scene.generation {
            return Ok(plan);
        }
        self.last_generation = scene.generation;

        // Rotate the ring *before* allocating: the cursor belongs to the frame
        // slot that `begin_frame` selects.
        self.ring.begin_frame(self.frame_index);

        let mut bytes: Vec<u8> = Vec::new();
        let mut spans: Vec<(u32, u32, u32)> = Vec::new(); // (dst, src_rel, len)
        let mut stage = |dst: u32, data: &[u8], bytes: &mut Vec<u8>, spans: &mut Vec<(u32, u32, u32)>| {
            if data.is_empty() {
                return;
            }
            let src_rel = bytes.len() as u32;
            bytes.extend_from_slice(data);
            spans.push((dst, src_rel, data.len() as u32));
        };

        stage(
            self.pool_offset(self.prim_handle),
            bytemuck::cast_slice(scene.primitives),
            &mut bytes,
            &mut spans,
        );
        stage(
            self.pool_offset(self.transform_handle),
            bytemuck::cast_slice(scene.transforms),
            &mut bytes,
            &mut spans,
        );
        stage(
            self.pool_offset(self.clip_handle),
            bytemuck::cast_slice(scene.clips),
            &mut bytes,
            &mut spans,
        );
        stage(
            self.pool_offset(self.payload_handle),
            bytemuck::cast_slice(scene.payloads),
            &mut bytes,
            &mut spans,
        );

        if spans.is_empty() {
            return Ok(plan);
        }

        // Reserve the payload bytes in the transient ring. The ring is
        // host-visible, so the copy below needs no staging buffer at all.
        let ring_slot = self.ring.allocate(bytes.len() as u32, 256).ok_or_else(|| {
            GpuError::Allocation(format!(
                "transient ring exhausted: {} bytes needed, {} available",
                bytes.len(),
                self.ring.slot_capacity()
            ))
        })?;

        // SAFETY: both buffers are host visible, mapped for their whole
        // lifetime, and the ranges were just reserved.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ring_buffer
                    .host_ptr(u64::from(ring_slot))
                    .ok_or_else(|| GpuError::Allocation("ring not mapped".into()))?,
                bytes.len(),
            );
            let patches: Vec<GpuScatterPatch> = spans
                .iter()
                .map(|(dst, src_rel, len)| GpuScatterPatch {
                    dst_offset: *dst,
                    src_offset: ring_slot + *src_rel,
                    len: *len,
                    _pad: 0,
                })
                .collect();
            core::ptr::copy_nonoverlapping(
                patches.as_ptr().cast::<u8>(),
                self.patches
                    .host_ptr(0)
                    .ok_or_else(|| GpuError::Allocation("patches not mapped".into()))?,
                patches.len() * 16,
            );
            plan.patch_count = patches.len() as u32;
        }

        plan.bytes = bytes.len() as u32;
        plan.ring_offset = ring_slot;
        Ok(plan)
    }

    /// Release every GPU resource. Consumes the renderer.
    pub fn destroy(mut self) {
        self.gpu.wait_idle();
        // SAFETY: the device is idle and `self` is consumed.
        unsafe {
            self.target.destroy(&self.gpu);
            self.pipelines.destroy(&self.gpu);
            for b in [
                &self.pool_buffer,
                &self.ring_buffer,
                &self.lut_buffer,
                &self.keys_a,
                &self.keys_b,
                &self.idx_a,
                &self.idx_b,
                &self.sort_scratch,
                &self.commands,
                &self.counters,
                &self.patches,
                &self.staging,
            ] {
                self.gpu.destroy_buffer(b);
            }
        }
        self.gpu.destroy();
    }
}

#[derive(Copy, Clone, Debug)]
enum Which {
    Primitives,
    Transforms,
    Clips,
    Payloads,
}

#[derive(Copy, Clone, Debug, Default)]
struct UploadPlan {
    patch_count: u32,
    bytes: u32,
    ring_offset: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{PrimFlags, SubElement};

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
        let prims = vec![quad(16.0, 16.0, 96.0, 96.0, 2)];
        let transforms = vec![GpuTransform::IDENTITY];
        renderer.write_theme(&{
            let mut c = vec![[0u16; 4]; 512];
            c[1] = unbound_math::Color::from_srgb8(0, 0, 0, 40).to_rgba16f();
            c[2] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
            c
        }).unwrap();

        renderer
            .render(&scene(&prims, &transforms, 1))
            .expect("render");
        let pixels = renderer.read_pixels().expect("readback");
        assert_eq!(pixels.len(), 128 * 128 * 4);


        // Centre of the quad must be the fill token colour (white).
        let idx = (64 * 128 + 64) * 4;
        assert!(
            pixels[idx] > 200 && pixels[idx + 1] > 200 && pixels[idx + 2] > 200,
            "centre pixel was {:?}",
            &pixels[idx..idx + 4]
        );
        // A pixel outside the quad must be the clear colour (dark).
        // A pixel outside the quad keeps the clear colour, which is the dark
        // background [0.07, 0.07, 0.08] sRGB-encoded (~75).
        let outside = (2 * 128 + 2) * 4;
        let clear = pixels[outside];
        assert!(
            (60..=90).contains(&clear),
            "corner pixel was {:?}",
            &pixels[outside..outside + 4]
        );

        assert_eq!(renderer.stats.visible, 1, "the quad must survive culling");
        assert_eq!(renderer.stats.draw_calls, 1);
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

        // Same generation, different camera: zero bytes must move.
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
        let prims = vec![quad(10_000.0, 10_000.0, 10.0, 10.0, 2)];
        let transforms = vec![GpuTransform::IDENTITY];
        renderer
            .render(&scene_at(&prims, &transforms, 1, (64.0, 64.0)))
            .unwrap();
        // The culling shader rejects it; the draw command count drops to zero.
        assert_eq!(renderer.stats.submitted, 1);
        let pixels = renderer.read_pixels().unwrap();
        let idx = (32 * 64 + 32) * 4;
        // Clear colour only: the primitive was rejected before rasterisation.
        assert!(
            (60..=90).contains(&pixels[idx]),
            "culled primitive painted: {:?}",
            &pixels[idx..idx + 4]
        );
        assert_eq!(renderer.stats.visible, 0, "culling must report 0 visible");
        renderer.destroy();
    }
}

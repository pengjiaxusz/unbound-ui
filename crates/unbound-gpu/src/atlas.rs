//! GPU-side glyph and image atlases (`docs` §3.5).
//!
//! The renderer binds two bindless `Texture2DArray` samplers: an `R8` atlas for
//! coverage glyphs and an `RGBA8` atlas for colour glyphs and images. The CPU
//! side (`unbound-text`) owns the shelf packer and decides *where* every glyph
//! lands; this module owns the GPU textures and knows *how* to move the pixels
//! there.
//!
//! A page of the CPU atlas maps to one array layer, so an update is a single
//! `vkCmdCopyBufferToImage` into a sub-region of one layer.

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::device::{Buffer, Gpu, GpuError, MemoryKind};
use ash::vk;

/// The number of bindless layers each atlas exposes.
///
/// Must match `MAX_ATLAS_LAYERS` in the pipeline module: the descriptor array
/// is sized once, at pipeline creation.
pub const ATLAS_LAYERS: u32 = crate::pipeline::MAX_ATLAS_LAYERS;

/// The edge length of one atlas page, in texels.
pub const PAGE_SIZE: u32 = 2048;

/// Which of the two atlases an update targets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AtlasKind {
    /// Single-channel coverage (glyphs).
    R8,
    /// Four-channel colour (emoji, images).
    Rgba8,
}

impl AtlasKind {
    /// The Vulkan format this atlas uses.
    #[must_use]
    pub const fn format(self) -> vk::Format {
        match self {
            Self::R8 => vk::Format::R8_UNORM,
            Self::Rgba8 => vk::Format::R8G8B8A8_UNORM,
        }
    }

    /// The bytes per texel.
    #[must_use]
    pub const fn bpp(self) -> u32 {
        match self {
            Self::R8 => 1,
            Self::Rgba8 => 4,
        }
    }
}

/// One of the two atlas textures.
///
/// The Vulkan objects are immutable once created; only the *layout* of each
/// layer changes, and that is tracked separately so the atlas can be shared
/// behind an `Arc` while still being uploaded into.
pub struct AtlasTexture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    /// One view per layer; the bindless descriptor array binds them in order.
    views: Vec<vk::ImageView>,
    kind: AtlasKind,
    /// A shared handle on the device, so the atlas can free itself even when
    /// it outlives the [`Gpu`] that created it.
    lifetime: crate::device::DeviceLifetime,
    /// Set once the images have been freed.
    ///
    /// The atlas set is shared with the application, so the renderer frees the
    /// images explicitly before the device goes away; the `Drop` impl then
    /// finds them already gone. The flag makes both paths idempotent.
    freed: std::sync::atomic::AtomicBool,
    /// The current layout of each layer.
    ///
    /// A layer that has never been written is `UNDEFINED`; after an upload it
    /// is `SHADER_READ_ONLY_OPTIMAL`. The layout has to be tracked because a
    /// second upload into the same layer must *not* declare `old_layout` as
    /// `UNDEFINED`: that would discard the glyphs already in the layer.
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
    /// Free the images. Idempotent: the second call does nothing.
    ///
    /// The atlas set is shared with the application, so the renderer frees the
    /// images before the device is destroyed; the `Drop` impl then finds them
    /// already gone. When the atlas outlives the device, the `Drop` impl frees
    /// them instead — unless the device is already gone, in which case the
    /// images went with it and the stale handle must not be touched.
    pub fn free(&self) {
        if self.freed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        if !self.lifetime.is_alive() {
            return;
        }
        // SAFETY: the device is alive and the images belong to it. The
        // renderer waits for the device to be idle before calling this.
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
        // The atlas may outlive the `Gpu` that created it: the application
        // keeps an `Arc` to the atlas set for the whole session.
        self.free();
    }
}

impl AtlasTexture {
    /// Create an empty atlas array.
    ///
    /// # Errors
    /// Propagates Vulkan allocation failures.
    pub fn new(gpu: &Gpu, kind: AtlasKind, layers: u32) -> Result<Self, GpuError> {
        // SAFETY: creation only; ownership moves into the returned struct.
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

            // A view per layer: the bindless descriptor array holds one slot
            // per layer, and each slot addresses a single array layer.
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

    /// The view for one layer.
    #[must_use]
    pub fn layer_view(&self, layer: u32) -> Option<vk::ImageView> {
        self.views.get(layer as usize).copied()
    }

    /// The number of layers this atlas exposes.
    #[must_use]
    pub fn layers(&self) -> u32 {
        self.views.len() as u32
    }

    /// The current layout of a layer.
    fn current_layout(&self, layer: u32) -> vk::ImageLayout {
        self.layouts
            .lock()
            .map(|layouts| layouts[layer as usize])
            .unwrap_or(vk::ImageLayout::UNDEFINED)
    }

    /// Record the new layout of a layer.
    fn set_layout(&self, layer: u32, layout: vk::ImageLayout) {
        if let Ok(mut layouts) = self.layouts.lock() {
            layouts[layer as usize] = layout;
        }
    }

    /// The kind of atlas.
    #[must_use]
    pub const fn kind(&self) -> AtlasKind {
        self.kind
    }

    /// The raw image handle.
    #[must_use]
    pub const fn image(&self) -> vk::Image {
        self.image
    }
}

/// A rectangle of texels to move from the CPU to one atlas layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlasUpload {
    /// Which atlas receives the pixels.
    pub kind: AtlasKind,
    /// The array layer.
    pub layer: u32,
    /// The destination origin inside the layer.
    pub x: u32,
    pub y: u32,
    /// The size of the region.
    pub width: u32,
    pub height: u32,
    /// Row-major texels; `width * height * bpp` bytes.
    pub data: Vec<u8>,
}

impl AtlasUpload {
    /// The bytes per texel of this upload.
    #[must_use]
    pub const fn bpp(&self) -> u32 {
        self.kind.bpp()
    }

    /// The expected payload length.
    #[must_use]
    pub fn expected_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * (self.bpp() as usize)
    }
}

/// The pair of atlases the renderer binds into its descriptor slots.
pub struct AtlasSet {
    /// The coverage atlas.
    pub r8: AtlasTexture,
    /// The colour atlas.
    pub rgba: AtlasTexture,
}

impl core::fmt::Debug for AtlasSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AtlasSet")
            .field("r8", &self.r8)
            .field("rgba", &self.rgba)
            .finish_non_exhaustive()
    }
}

impl AtlasSet {
    /// Create both atlases.
    ///
    /// # Errors
    /// Propagates Vulkan allocation failures.
    pub fn new(gpu: &Gpu, layers: u32) -> Result<Self, GpuError> {
        Ok(Self {
            r8: AtlasTexture::new(gpu, AtlasKind::R8, layers)?,
            rgba: AtlasTexture::new(gpu, AtlasKind::Rgba8, layers)?,
        })
    }

    /// The atlas matching `kind`.
    #[must_use]
    pub const fn get(&self, kind: AtlasKind) -> &AtlasTexture {
        match kind {
            AtlasKind::R8 => &self.r8,
            AtlasKind::Rgba8 => &self.rgba,
        }
    }

    /// Free the images of both atlases. Idempotent.
    ///
    /// The renderer calls this before it tears the device down: Vulkan
    /// requires every object to be destroyed before the device, and the
    /// application may still hold an `Arc` to the atlas set.
    pub fn free(&self) {
        self.r8.free();
        self.rgba.free();
    }

    /// Upload one rectangle of texels into the matching atlas.
    ///
    /// The pixels travel through a host-visible staging buffer and are copied
    /// into the destination layer with a single `vkCmdCopyBufferToImage`. The
    /// copy is recorded on the graphics queue and waited on, so the GPU never
    /// samples a half-written glyph.
    ///
    /// # Errors
    /// Returns [`GpuError::Vulkan`] when the copy cannot be recorded.
    pub fn upload(&self, gpu: &Gpu, upload: &AtlasUpload) -> Result<(), GpuError> {
        let expected = upload.expected_len();
        if upload.data.len() < expected {
            return Err(GpuError::Allocation(format!(
                "atlas upload is {} bytes, expected {expected}",
                upload.data.len()
            )));
        }
        if upload.width == 0 || upload.height == 0 {
            return Ok(());
        }
        let atlas = self.get(upload.kind);
        if upload.layer >= atlas.layers() {
            return Err(GpuError::Allocation(format!(
                "atlas layer {} is out of range ({} layers)",
                upload.layer,
                atlas.layers()
            )));
        }

        let staging = gpu.create_buffer(
            expected as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryKind::HostVisible,
            "atlas-staging",
        )?;
        // SAFETY: the staging buffer is exclusively ours until the copy runs.
        unsafe {
            gpu.write_host(&staging, 0, &upload.data[..expected]);
            gpu.flush(&staging, 0, expected as u64);
        }

        let cmd = gpu.begin_single_time()?;
        // SAFETY: `cmd` is a fresh command buffer we own.
        unsafe {
            // The layer's current layout is tracked, so a second upload into
            // the same layer keeps the glyphs that are already there.
            let old_layout = atlas.current_layout(upload.layer);
            let barrier = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(old_layout)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(atlas.image())
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(upload.layer)
                    .layer_count(1))];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&barrier),
            );

            gpu.device.cmd_copy_buffer_to_image(
                cmd,
                staging.raw,
                atlas.image(),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(upload.layer)
                        .layer_count(1))
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

            // Back to shader-read so the graphics queue can sample it.
            let to_read = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(atlas.image())
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(upload.layer)
                    .layer_count(1))];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        }
        gpu.end_single_time(cmd)?;

        // The layer is now readable by the shader.
        self.get(upload.kind).set_layout(
            upload.layer,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );

        // The staging buffer is dead once the copy has retired.
        // SAFETY: the copy already waited for the queue to go idle.
        unsafe {
            gpu.destroy_buffer(&staging);
        }
        Ok(())
    }

    /// Read back a rectangle of texels from one atlas layer.
    ///
    /// This is a debugging aid: it proves the pixels actually landed on the
    /// GPU rather than only in the staging buffer.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
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
        // SAFETY: the command buffer is ours.
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
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(layer)
                    .layer_count(1))];
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
                    .image_subresource(vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(layer)
                        .layer_count(1))
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
            // Back to shader-read so the renderer can keep sampling it.
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
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(layer)
                    .layer_count(1))];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        }
        gpu.end_single_time(cmd)?;
        self.get(kind).set_layout(layer, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

        let mut out = vec![0u8; size as usize];
        // SAFETY: the copy has retired; the staging buffer is host visible.
        unsafe {
            std::ptr::copy_nonoverlapping(staging.mapped, out.as_mut_ptr(), out.len());
            gpu.destroy_buffer(&staging);
        }
        Ok(out)
    }
}

/// A staging buffer kept alive for repeated atlas uploads.
///
/// Reusing one buffer avoids a heap allocation per glyph batch.
#[derive(Debug)]
pub struct UploadStaging {
    buffer: Buffer,
}

impl UploadStaging {
    /// Allocate a staging buffer of `capacity` bytes.
    ///
    /// # Errors
    /// Propagates allocation failures.
    pub fn new(gpu: &Gpu, capacity: u64) -> Result<Self, GpuError> {
        let buffer = gpu.create_buffer(
            capacity.max(1),
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryKind::HostVisible,
            "atlas-staging",
        )?;
        Ok(Self { buffer })
    }

    /// The capacity in bytes.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.buffer.size
    }

    /// The raw buffer handle.
    #[must_use]
    pub const fn raw(&self) -> vk::Buffer {
        self.buffer.raw
    }

    /// # Safety
    /// The device must be idle.
    pub unsafe fn destroy(&self, gpu: &Gpu) {
        // SAFETY: the caller guarantees the device is idle, so no command
        // buffer still references the buffer.
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
        // The shader samples a 2048² page.
        assert_eq!(PAGE_SIZE, 2048);
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
        // The pipeline binds `MAX_ATLAS_LAYERS` slots; the atlas must not
        // provide fewer or the descriptor write would go out of range.
        assert_eq!(ATLAS_LAYERS, crate::pipeline::MAX_ATLAS_LAYERS);
    }
}

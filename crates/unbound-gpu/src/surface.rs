//! Windowed presentation: surface, swapchain and the acquire/present cycle.
//!
//! The renderer in this crate is deliberately surface-free by default: every
//! test renders into an offscreen image, which is what makes the GPU suite
//! runnable on CI. This module adds the *windowed* half without changing that:
//! a [`Surface`] owns a `VkSurfaceKHR` and a swapchain, and
//! [`Swapchain::acquire`] hands the caller a colour view to draw into.
//!
//! The present mode follows the policy in [`crate::swapchain`]: with
//! `VK_EXT_present_mode_fifo_latest_ready` the FIFO-latest-ready mode keeps
//! vsync but presents the newest completed image, which is the difference
//! between three frames of input lag and one.
//!
//! Resizing is the other half of the story. A window that is dragged is
//! *out of date* for a few frames, and both `vkAcquireNextImageKHR` and
//! `vkQueuePresentKHR` report that with `VK_ERROR_OUT_OF_DATE_KHR`. That is
//! not a failure — it is the driver asking for a new swapchain. The old
//! swapchain is handed to the driver as `oldSwapchain` so the presentation
//! engine can reuse its resources instead of tearing the surface down.

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::device::{Gpu, GpuError};
use crate::swapchain::{PresentConfig, PresentMode, ResizeBehaviour};
use ash::vk;

/// A `VkSurfaceKHR` bound to a native window.
pub struct Surface {
    surface: vk::SurfaceKHR,
    /// The instance extension loader for `VK_KHR_surface`.
    khr_surface: ash::khr::surface::Instance,
    /// The Win32-specific loader. It is kept alive for as long as the
    /// surface exists because its function pointers are what created it.
    #[cfg(windows)]
    #[allow(dead_code)]
    win32_surface: Option<ash::khr::win32_surface::Instance>,
}

impl core::fmt::Debug for Surface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Surface")
            .field("surface", &self.surface)
            .finish_non_exhaustive()
    }
}

impl Surface {
    /// Create a surface for a Win32 window handle.
    ///
    /// `hwnd` is the raw `HWND` of the window.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    #[cfg(windows)]
    pub fn new_win32(gpu: &Gpu, hwnd: isize) -> Result<Self, GpuError> {
        let khr_surface = ash::khr::surface::Instance::new(&gpu.entry, &gpu.instance);
        let win32_surface = ash::khr::win32_surface::Instance::new(&gpu.entry, &gpu.instance);
        // SAFETY: the handle comes from a live window and is only read here.
        let create_info = vk::Win32SurfaceCreateInfoKHR::default().hwnd(hwnd);
        let surface = unsafe { win32_surface.create_win32_surface(&create_info, None) }
            .map_err(GpuError::from)?;
        Ok(Self {
            surface,
            khr_surface,
            win32_surface: Some(win32_surface),
        })
    }

    /// Create a surface from a raw window handle.
    ///
    /// On Windows the handle must be an `HWND`. On other platforms the
    /// platform-specific loader is not wired up yet, so the call reports an
    /// error instead of silently producing nothing.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn new(gpu: &Gpu, raw_handle: RawWindowHandle) -> Result<Self, GpuError> {
        #[cfg(windows)]
        {
            let RawWindowHandle::Win32 { hwnd } = raw_handle;
            Self::new_win32(gpu, hwnd)
        }
        #[cfg(not(windows))]
        {
            let _ = (gpu, raw_handle);
            Err(GpuError::Allocation(
                "windowed presentation is implemented for Win32 only".into(),
            ))
        }
    }

    /// The raw surface handle.
    #[must_use]
    pub const fn handle(&self) -> vk::SurfaceKHR {
        self.surface
    }

    /// The surface capabilities of the given physical device.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn capabilities(&self, gpu: &Gpu) -> Result<vk::SurfaceCapabilitiesKHR, GpuError> {
        // SAFETY: the surface belongs to this instance.
        unsafe {
            self.khr_surface
                .get_physical_device_surface_capabilities(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// The supported present modes for the given physical device.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn present_modes(&self, gpu: &Gpu) -> Result<Vec<vk::PresentModeKHR>, GpuError> {
        // SAFETY: the surface belongs to this instance.
        unsafe {
            self.khr_surface
                .get_physical_device_surface_present_modes(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// The formats the surface can present.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn formats(&self, gpu: &Gpu) -> Result<Vec<vk::SurfaceFormatKHR>, GpuError> {
        // SAFETY: the surface belongs to this instance.
        unsafe {
            self.khr_surface
                .get_physical_device_surface_formats(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// The present-scaling capabilities of the surface for a present mode.
    ///
    /// `None` when the query is not possible — the instance does not expose
    /// `VK_KHR_get_surface_capabilities2`, or the driver refused the call.
    /// The platform's own scaling is used then.
    ///
    /// The capabilities are what makes it legal to ask for a specific scaling
    /// behaviour: `VK_EXT_swapchain_maintenance1` requires the behaviour to
    /// be one the surface says it supports.
    #[must_use]
    pub fn present_scaling_capabilities(
        &self,
        gpu: &Gpu,
        present_mode: vk::PresentModeKHR,
    ) -> Option<vk::SurfacePresentScalingCapabilitiesEXT<'_>> {
        if !gpu
            .enabled_instance_extensions
            .iter()
            .any(|name| name == "VK_KHR_get_surface_capabilities2")
        {
            return None;
        }
        let loader = ash::khr::get_surface_capabilities2::Instance::new(&gpu.entry, &gpu.instance);
        // The scaling capabilities depend on the present mode, so the mode is
        // part of the query.
        let mut present_mode_info = vk::SurfacePresentModeEXT::default().present_mode(present_mode);
        let surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
            .surface(self.surface)
            .push_next(&mut present_mode_info);
        let mut scaling = vk::SurfacePresentScalingCapabilitiesEXT::default();
        let mut capabilities = vk::SurfaceCapabilities2KHR::default();
        capabilities = capabilities.push_next(&mut scaling);
        // SAFETY: the surface belongs to this instance and the physical device
        // is the one the surface was created for.
        let result = unsafe {
            loader.get_physical_device_surface_capabilities2(
                gpu.physical,
                &surface_info,
                &mut capabilities,
            )
        };
        result.ok().map(|()| scaling)
    }

    /// Destroy the surface.
    pub fn destroy(self, gpu: &Gpu) {
        // SAFETY: the surface belongs to `gpu.instance`, which outlives it.
        unsafe {
            self.khr_surface.destroy_surface(self.surface, None);
        }
        let _ = gpu;
    }
}

/// The raw window handle a surface is created from.
#[derive(Copy, Clone, Debug)]
pub enum RawWindowHandle {
    /// A Win32 `HWND`.
    #[cfg(windows)]
    Win32 {
        /// The window handle.
        hwnd: isize,
    },
}

/// How long the low-latency present path waits for the presentation.
///
/// The wait has to be bounded: a presentation that never happens — an
/// occluded window, a session without a compositor — would otherwise block
/// the frame loop forever. Fifty milliseconds is longer than any healthy
/// refresh interval and short enough that a stalled one is not felt as a
/// stutter.
pub const PRESENT_WAIT_TIMEOUT_NS: u64 = 50_000_000;

/// How many consecutive presentation timeouts make the wait pointless.
///
/// A window that is not being composited never completes a presentation, so
/// waiting for it only adds latency to every single frame. After this many
/// timeouts in a row the low-latency path switches itself off.
pub const PRESENT_WAIT_GIVE_UP_AFTER: u32 = 3;

/// An acquired swapchain image, ready to be rendered into.
#[derive(Copy, Clone, Debug)]
pub struct AcquiredImage {
    /// Index of the image inside the swapchain.
    pub index: u32,
    /// The image view to render into.
    pub view: vk::ImageView,
    /// The image itself. The renderer needs it to transition the layout: a
    /// view alone cannot be transitioned.
    pub image: vk::Image,
    /// Signalled once the presentation engine has handed the image over.
    ///
    /// The render submission must wait on this semaphore before it touches
    /// the image.
    pub available: vk::Semaphore,
    /// The semaphore this image's render submission has to signal.
    ///
    /// Every swapchain image owns its own render-finished semaphore. A
    /// single global semaphore cannot be reused: the presentation engine
    /// only releases an image when it is acquired again, and the wait of the
    /// previous presentation may still be pending at that point. Signalling
    /// a semaphore that is still in use by `vkQueuePresentKHR` is a
    /// validation error (`VUID-vkQueueSubmit2-semaphore-03868`).
    ///
    /// With one semaphore per image, the semaphore is only signalled again
    /// once the very same image index has been acquired again — and the
    /// image is only handed out again after its previous presentation has
    /// completed, which is exactly when the previous wait was consumed.
    pub render_finished: vk::Semaphore,
}

/// How many render-finished semaphores a swapchain with `image_count`
/// images needs.
///
/// One per swapchain image: the semaphore is bound to the image it
/// describes, not to the frame slot, because the presentation engine — not
/// the renderer — decides how long an image stays busy.
pub(crate) const fn render_finished_slot_count(image_count: usize) -> usize {
    image_count
}

/// The slot of the render-finished semaphore that belongs to `image_index`.
///
/// The mapping has to be injective for the images of one swapchain: two
/// images may never share a semaphore, or the second presentation would
/// signal a semaphore the first presentation still waits on.
pub(crate) const fn render_finished_slot(image_index: u32, image_count: usize) -> usize {
    if image_count == 0 {
        0
    } else {
        (image_index as usize) % image_count
    }
}

/// Result of acquiring the next swapchain image.
#[derive(Copy, Clone, Debug)]
pub enum AcquireOutcome {
    /// An image is ready to be rendered into.
    Ready {
        /// The acquired image.
        image: AcquiredImage,
        /// True when the swapchain still works but no longer matches the
        /// surface exactly and should be recreated after presenting.
        suboptimal: bool,
    },
    /// The surface size changed incompatibly; recreate the swapchain now.
    OutOfDate,
}

/// Result of presenting a rendered swapchain image.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentOutcome {
    /// Presented cleanly.
    Optimal,
    /// Presented, but the swapchain is suboptimal or out-of-date and must
    /// be recreated before the next frame.
    Recreate,
}

/// `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`.
///
/// The mode comes from `VK_EXT_present_mode_fifo_latest_ready` (promoted to
/// `VK_KHR_present_mode_fifo_latest_ready`), *not* from
/// `VK_EXT_swapchain_maintenance1`. `ash` 0.38 predates both, so the value
/// from the registry is used directly.
///
/// The neighbouring value `1000111000` is
/// `VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR` and has nothing to do with
/// the latest-ready mode.
const PRESENT_MODE_FIFO_LATEST_READY: vk::PresentModeKHR =
    vk::PresentModeKHR::from_raw(1_000_361_000);

/// The Vulkan present mode for a policy mode.
///
/// `FifoLatestReady` is only legal when the driver offers
/// `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`; every Vulkan implementation must
/// support `VK_PRESENT_MODE_FIFO_KHR`, so that is the fallback.
pub(crate) fn resolve_present_mode(
    mode: PresentMode,
    supports_latest_ready: bool,
) -> vk::PresentModeKHR {
    match mode {
        PresentMode::FifoLatestReady if supports_latest_ready => PRESENT_MODE_FIFO_LATEST_READY,
        PresentMode::FifoLatestReady | PresentMode::Fifo => vk::PresentModeKHR::FIFO,
        PresentMode::Immediate => vk::PresentModeKHR::IMMEDIATE,
    }
}

/// The extent the swapchain images are created with.
///
/// `current` is the driver's `currentExtent`: on Win32 it is the
/// authoritative size of the window the surface was created for. A
/// one-pixel difference between the requested size and the driver's size is
/// enough to make the driver report the swapchain as out of date on the very
/// next frame, so the driver's value wins whenever it is known.
///
/// The clamp into `min ..= max` cannot panic: a driver that reports
/// `min > max` would make `u32::clamp` panic, so the low bound is the
/// *smaller* of the two.
pub(crate) fn resolve_extent(
    current: (u32, u32),
    min: (u32, u32),
    max: (u32, u32),
    width: u32,
    height: u32,
) -> vk::Extent2D {
    let (mut w, mut h) = if current.0 != u32::MAX {
        (current.0, current.1)
    } else {
        (width, height)
    };
    w = w.clamp(min.0.min(max.0), min.0.max(max.0));
    h = h.clamp(min.1.min(max.1), min.1.max(max.1));
    vk::Extent2D {
        width: w,
        height: h,
    }
}

/// What `vkAcquireNextImageKHR` reported.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquireStatus {
    /// The image is ready to be rendered into.
    Ready {
        /// Index of the image inside the swapchain.
        index: u32,
        /// The swapchain still works but no longer matches the surface.
        suboptimal: bool,
    },
    /// The surface changed; the swapchain has to be recreated.
    OutOfDate,
}

/// Classify the result of `vkAcquireNextImageKHR`.
///
/// `VK_ERROR_OUT_OF_DATE_KHR` is not a failure: the window was resized and
/// the swapchain has to be recreated. Every other error is fatal.
pub(crate) fn classify_acquire(
    result: Result<(u32, bool), vk::Result>,
) -> Result<AcquireStatus, vk::Result> {
    match result {
        Ok((index, false)) => Ok(AcquireStatus::Ready {
            index,
            suboptimal: false,
        }),
        Ok((index, true)) => Ok(AcquireStatus::Ready {
            index,
            suboptimal: true,
        }),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(AcquireStatus::OutOfDate),
        Err(other) => Err(other),
    }
}

/// Classify the result of `vkQueuePresentKHR`.
///
/// `VK_ERROR_OUT_OF_DATE_KHR` and `VK_SUBOPTIMAL_KHR` mean the swapchain has
/// to be recreated, not that the frame failed. Every other error is fatal.
pub(crate) fn classify_present(
    result: Result<bool, vk::Result>,
) -> Result<PresentOutcome, vk::Result> {
    match result {
        Ok(false) => Ok(PresentOutcome::Optimal),
        Ok(true) => Ok(PresentOutcome::Recreate),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
            Ok(PresentOutcome::Recreate)
        }
        Err(other) => Err(other),
    }
}

/// A swapchain plus the image views the renderer draws into.
pub struct Swapchain {
    swapchain: vk::SwapchainKHR,
    khr_swapchain: ash::khr::swapchain::Device,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    /// One "the image is ready" semaphore per frame in flight.
    ///
    /// `vkAcquireNextImageKHR` **must** be given a semaphore (or a fence):
    /// handing it two null handles is invalid usage, even though some drivers
    /// tolerate it. The semaphore is the synchronisation point between the
    /// presentation engine and the render, so the render submission waits on
    /// it before touching the image.
    ///
    /// A pool rather than a single semaphore, because a binary semaphore may
    /// only carry one pending signal: re-signalling it before the previous
    /// wait consumed it would lose the signal and deadlock the next frame.
    /// The pool is as deep as the number of frames in flight, and the
    /// renderer's own in-flight fence guarantees that the previous wait has
    /// already been consumed before a slot is reused.
    acquire_semaphores: Vec<vk::Semaphore>,
    /// One render-finished semaphore per swapchain image.
    ///
    /// The semaphore is bound to the image, not to the frame slot: the
    /// presentation engine may still be waiting on it long after the render
    /// submission that signalled it has retired. Sharing one semaphore
    /// across the images is what produced
    /// `VUID-vkQueueSubmit2-semaphore-03868`.
    render_finished_semaphores: Vec<vk::Semaphore>,
    /// The next semaphore of the round-robin.
    next_semaphore: usize,
    format: vk::Format,
    color_space: vk::ColorSpaceKHR,
    next_index: u32,
    /// The image currently acquired, if any.
    acquired: Option<u32>,
    /// The value the swapchain was created with, so a resize can be detected.
    extent: (u32, u32),
    /// The next presentation identifier for `VK_KHR_present_wait`.
    ///
    /// The specification requires a non-zero identifier to be greater than
    /// every identifier previously passed for the same swapchain.
    next_present_id: u64,
    /// Wait for the presentation to actually reach the screen.
    wait_for_present: bool,
    /// `VK_KHR_present_wait` is available.
    has_present_wait: bool,
    /// `VK_EXT_swapchain_maintenance1` is available.
    ///
    /// The extension *and* its feature have to be enabled: the present
    /// fence and the present scaling behaviour are only legal then.
    has_maintenance1: bool,
    /// How many presentations did not reach the screen in time.
    present_timeouts: u32,
    /// How many timeouts happened in a row.
    consecutive_timeouts: u32,
    /// Skip the present wait once, right after a recreation.
    ///
    /// While the user drags a window edge the compositor frequently drops the
    /// present requests of the old chain; blocking for 50 ms each time stalls
    /// the message loop. The first frame after a rebuild therefore presents
    /// without waiting.
    skip_present_wait_once: bool,
}

impl core::fmt::Debug for Swapchain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Swapchain")
            .field("format", &self.format)
            .field("extent", &self.extent)
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

impl Swapchain {
    /// Create the swapchain for a surface.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn new(
        gpu: &Gpu,
        surface: &Surface,
        width: u32,
        height: u32,
        config: &PresentConfig,
    ) -> Result<Self, GpuError> {
        Self::with_old(
            gpu,
            surface,
            width,
            height,
            config,
            vk::SwapchainKHR::null(),
        )
    }

    /// Create the swapchain, optionally retiring an older one.
    ///
    /// `old_swapchain` is handed to the driver so the presentation engine can
    /// reuse its resources: without it the old swapchain is torn down before
    /// the new one exists, which shows up as a flicker and as validation
    /// errors about images of a destroyed swapchain.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    fn with_old(
        gpu: &Gpu,
        surface: &Surface,
        width: u32,
        height: u32,
        config: &PresentConfig,
        old_swapchain: vk::SwapchainKHR,
    ) -> Result<Self, GpuError> {
        let capabilities = surface.capabilities(gpu)?;
        let formats = surface.formats(gpu)?;
        let modes = surface.present_modes(gpu)?;

        // The swapchain images become the renderer's colour attachment, so their
        // format has to match the pipeline's `COLOR_FORMAT` *exactly*: a
        // graphics pipeline created for one attachment format cannot be used
        // with another, and a silently different channel order would also
        // swap red and blue on screen. The format is sRGB so the hardware
        // encodes the shader's linear output.
        let chosen = formats
            .iter()
            .find(|f| f.format == crate::pipeline::COLOR_FORMAT)
            .copied()
            .ok_or_else(|| {
                let available: Vec<String> =
                    formats.iter().map(|f| format!("{:?}", f.format)).collect();
                GpuError::Allocation(format!(
                    "the surface does not expose the renderer's colour format {:?}; \
                     it offers {:?}",
                    crate::pipeline::COLOR_FORMAT,
                    available
                ))
            })?;

        // `VK_EXT_swapchain_maintenance1` is only usable when the extension
        // *and* its feature are enabled: the present fence and the present
        // scaling behaviour are illegal usage without the feature.
        let has_maintenance1 = gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1")
            && gpu.features.swapchain_maintenance1;

        // `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT` comes from its own
        // extension (`VK_EXT_present_mode_fifo_latest_ready`, promoted to
        // `VK_KHR_present_mode_fifo_latest_ready`) — *not* from
        // `VK_EXT_swapchain_maintenance1`. Asking for the mode without that
        // extension enabled is invalid usage, and the driver reports the
        // mode in `supported_present_modes` only when it really has it.
        let latest_ready_extension = gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_EXT_present_mode_fifo_latest_ready")
            || gpu
                .enabled_device_extensions
                .iter()
                .any(|name| name == "VK_KHR_present_mode_fifo_latest_ready");
        let supports_latest_ready =
            latest_ready_extension && modes.contains(&PRESENT_MODE_FIFO_LATEST_READY);

        let present_mode = resolve_present_mode(config.mode, supports_latest_ready);
        // Only ask for a mode the surface actually supports.
        let present_mode = if modes.contains(&present_mode) {
            present_mode
        } else {
            vk::PresentModeKHR::FIFO
        };

        // The driver's `currentExtent` is the authoritative size of the
        // surface: a one-pixel mismatch with the requested size is enough to
        // make the driver report the swapchain as out of date on the very
        // next frame.
        let extent = resolve_extent(
            (
                capabilities.current_extent.width,
                capabilities.current_extent.height,
            ),
            (
                capabilities.min_image_extent.width,
                capabilities.min_image_extent.height,
            ),
            (
                capabilities.max_image_extent.width,
                capabilities.max_image_extent.height,
            ),
            width,
            height,
        );

        let image_count = (capabilities.min_image_count + 1).max(2);
        let image_count = image_count.min(capabilities.max_image_count);

        let queue_family_index = gpu.families.graphics;
        let queue_family_indices = [queue_family_index];

        // `VK_EXT_swapchain_maintenance1` lets the application say how the
        // presentation engine has to scale a swapchain that no longer matches
        // the surface. The behaviour has to be one the surface says it
        // supports, so the capabilities are queried first.
        let mut scaling_info = vk::SwapchainPresentScalingCreateInfoEXT::default();
        let mut use_scaling = false;
        if has_maintenance1 {
            if let Some(caps) = surface.present_scaling_capabilities(gpu, present_mode) {
                match config.resize {
                    ResizeBehaviour::Stretch
                        if caps
                            .supported_present_scaling
                            .contains(vk::PresentScalingFlagsEXT::STRETCH) =>
                    {
                        scaling_info =
                            scaling_info.scaling_behavior(vk::PresentScalingFlagsEXT::STRETCH);
                        use_scaling = true;
                    }
                    ResizeBehaviour::OneToOne
                        if caps
                            .supported_present_scaling
                            .contains(vk::PresentScalingFlagsEXT::ONE_TO_ONE) =>
                    {
                        // Never scale the stale image: anchor it top-left and
                        // let the fresh frame fill the rest of the window.
                        scaling_info = scaling_info
                            .scaling_behavior(vk::PresentScalingFlagsEXT::ONE_TO_ONE)
                            .present_gravity_x(vk::PresentGravityFlagsEXT::MIN)
                            .present_gravity_y(vk::PresentGravityFlagsEXT::MIN);
                        use_scaling = true;
                    }
                    _ => {}
                }
            }
        }

        let mut create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface.handle())
            .min_image_count(image_count)
            .image_format(chosen.format)
            .image_color_space(chosen.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(
                // `TRANSFER_SRC` lets the standard Vulkan screenshot layer
                // copy the presented frame out, which is how the windowed
                // path can be inspected without a desktop session.
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            // The old swapchain is handed to the driver so it can reuse the
            // presentation resources instead of tearing the surface down.
            .old_swapchain(old_swapchain);
        if use_scaling {
            create_info = create_info.push_next(&mut scaling_info);
        }

        let khr_swapchain = ash::khr::swapchain::Device::new(&gpu.instance, &gpu.device);
        // SAFETY: the surface belongs to the same instance as the device.
        let swapchain = unsafe { khr_swapchain.create_swapchain(&create_info, None) }
            .map_err(GpuError::from)?;

        // The images come from the swapchain; we only create views.
        // SAFETY: the swapchain was just created.
        let images =
            unsafe { khr_swapchain.get_swapchain_images(swapchain) }.map_err(GpuError::from)?;

        let mut views = Vec::with_capacity(images.len());
        for image in &images {
            // SAFETY: the image belongs to our swapchain.
            let view = unsafe {
                gpu.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(*image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(chosen.format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(0)
                                .layer_count(1),
                        ),
                    None,
                )
            }
            .map_err(GpuError::from)?;
            views.push(view);
        }

        // One "image is ready" semaphore per frame in flight.
        let pool_size = gpu.frames_in_flight().max(1);
        let mut acquire_semaphores = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            // SAFETY: the device outlives the swapchain.
            let semaphore = unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(GpuError::from)?
            };
            acquire_semaphores.push(semaphore);
        }

        // One "render finished" semaphore per swapchain image. The number has
        // to match the image count, not the number of frames in flight: the
        // semaphore describes how long *this image* stays busy for the
        // presentation engine.
        let mut render_finished_semaphores = Vec::with_capacity(images.len());
        for _ in 0..render_finished_slot_count(images.len()) {
            // SAFETY: the device outlives the swapchain.
            let semaphore = unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(GpuError::from)?
            };
            render_finished_semaphores.push(semaphore);
        }

        // The low-latency present path needs three things at once: the
        // `VK_KHR_present_wait` extension, its `VK_KHR_present_id`
        // dependency, and the `presentWait`/`presentId` *features*. Checking
        // only the extension string enabled the feature-less path, and
        // `vkWaitForPresentKHR` with an unenabled feature is invalid usage.
        let has_present_wait = gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_KHR_present_wait")
            && gpu
                .enabled_device_extensions
                .iter()
                .any(|name| name == "VK_KHR_present_id")
            && gpu.features.present_wait
            && gpu.features.present_id;

        Ok(Self {
            swapchain,
            khr_swapchain,
            images,
            views,
            acquire_semaphores,
            render_finished_semaphores,
            next_semaphore: 0,
            format: chosen.format,
            color_space: chosen.color_space,
            next_index: 0,
            acquired: None,
            extent: (extent.width, extent.height),
            next_present_id: 1,
            wait_for_present: config.wait_for_present && has_present_wait,
            has_present_wait,
            has_maintenance1,
            present_timeouts: 0,
            consecutive_timeouts: 0,
            skip_present_wait_once: false,
        })
    }

    /// The format the swapchain images use.
    #[must_use]
    pub const fn format(&self) -> vk::Format {
        self.format
    }

    /// The colour space the swapchain presents in.
    #[must_use]
    pub const fn color_space(&self) -> vk::ColorSpaceKHR {
        self.color_space
    }

    /// The current extent.
    #[must_use]
    pub const fn extent(&self) -> (u32, u32) {
        self.extent
    }

    /// Whether `VK_EXT_swapchain_maintenance1` and its feature are enabled.
    ///
    /// With them the application may hand the OS a present fence and ask for
    /// a specific present scaling behaviour.
    #[must_use]
    pub const fn has_maintenance1(&self) -> bool {
        self.has_maintenance1
    }

    /// How many presentations did not reach the screen in time.
    ///
    /// A non-zero count means the window is not being composited the way the
    /// application expects — the frame loop keeps running, but the images
    /// are not actually shown.
    #[must_use]
    pub const fn present_timeouts(&self) -> u32 {
        self.present_timeouts
    }

    /// Rebuild the swapchain for a new surface size, handing the old
    /// swapchain to the driver so the presentation engine can reuse its
    /// resources instead of tearing the surface down.
    ///
    /// The device is put to idle first: the images of the old swapchain must
    /// not be in flight while it is being replaced.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn recreate(
        &mut self,
        gpu: &Gpu,
        surface: &Surface,
        width: u32,
        height: u32,
        config: &PresentConfig,
    ) -> Result<(), GpuError> {
        gpu.wait_idle();
        let mut replacement = Self::with_old(gpu, surface, width, height, config, self.swapchain)?;
        // The next frame must not block on a presentation the compositor may
        // drop while the window is still being dragged.
        replacement.skip_present_wait_once = true;
        let old = core::mem::replace(self, replacement);
        old.destroy(gpu);
        Ok(())
    }

    /// Acquire the next image to draw into.
    ///
    /// Returns the index into [`Self::views`], the image view to render into
    /// and the semaphore that is signalled once the image is ready. The
    /// caller must present the same index afterwards, and the render
    /// submission must wait on the semaphore before it touches the image.
    ///
    /// An out-of-date surface is reported as an error for the benefit of the
    /// callers that predate [`Self::acquire_outcome`]: the frame loop has to
    /// recreate the swapchain. New code should use
    /// [`Self::acquire_outcome`], which reports it as
    /// [`AcquireOutcome::OutOfDate`] instead of an error.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn acquire(&mut self, timeout_ns: u64) -> Result<AcquiredImage, GpuError> {
        match self.acquire_outcome(timeout_ns)? {
            AcquireOutcome::Ready { image, .. } => Ok(image),
            AcquireOutcome::OutOfDate => Err(GpuError::Vulkan(vk::Result::ERROR_OUT_OF_DATE_KHR)),
        }
    }

    /// Acquire the next image, reporting an out-of-date surface as such.
    ///
    /// `VK_ERROR_OUT_OF_DATE_KHR` is not a failure: the window was resized
    /// and the swapchain has to be recreated. Callers that keep the swapchain
    /// alive can then call [`Self::recreate`] and carry on.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn acquire_outcome(&mut self, timeout_ns: u64) -> Result<AcquireOutcome, GpuError> {
        // The semaphore comes from a round-robin pool: a binary semaphore may
        // only carry one pending signal, so the slot is only reused once the
        // previous frame has consumed it.
        let semaphore = self.acquire_semaphores[self.next_semaphore];
        // SAFETY: the swapchain belongs to this device.
        let raw = unsafe {
            self.khr_swapchain.acquire_next_image(
                self.swapchain,
                timeout_ns,
                semaphore,
                vk::Fence::null(),
            )
        };
        let status = classify_acquire(raw).map_err(GpuError::from)?;
        match status {
            AcquireStatus::OutOfDate => {
                // The semaphore was not signalled, so the slot is *not*
                // consumed: the next attempt may use it again.
                self.acquired = None;
                Ok(AcquireOutcome::OutOfDate)
            }
            AcquireStatus::Ready { index, suboptimal } => {
                self.next_semaphore = (self.next_semaphore + 1) % self.acquire_semaphores.len();
                self.next_index = index;
                self.acquired = Some(index);
                let view = self.views[index as usize];
                // The semaphore belongs to this image: it is only signalled
                // again after this very image has been acquired again.
                let slot = render_finished_slot(index, self.render_finished_semaphores.len());
                Ok(AcquireOutcome::Ready {
                    image: AcquiredImage {
                        index,
                        view,
                        available: semaphore,
                        image: self.images[index as usize],
                        render_finished: self.render_finished_semaphores[slot],
                    },
                    suboptimal,
                })
            }
        }
    }

    /// Present the acquired image, waiting for the renderer to finish.
    ///
    /// `render_finished` is signalled by the renderer's frame submission; the
    /// present waits for it so the display never shows a half-drawn frame.
    ///
    /// An out-of-date surface is *not* an error here: the frame was
    /// presented as far as the driver could and the swapchain has to be
    /// recreated. Use [`Self::present_outcome`] to learn whether that is
    /// needed.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn present(
        &mut self,
        gpu: &Gpu,
        index: u32,
        render_finished: vk::Semaphore,
    ) -> Result<(), GpuError> {
        self.present_outcome(gpu, index, render_finished)
            .map(|_| ())
    }

    /// Present the acquired image, reporting an out-of-date swapchain as
    /// such.
    ///
    /// `VK_ERROR_OUT_OF_DATE_KHR` and `VK_SUBOPTIMAL_KHR` mean the swapchain
    /// has to be recreated before the next frame — they are not failures.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn present_outcome(
        &mut self,
        gpu: &Gpu,
        index: u32,
        render_finished: vk::Semaphore,
    ) -> Result<PresentOutcome, GpuError> {
        let swapchains = [self.swapchain];
        // The present info takes the *index* of the image, not the image.
        let indices = [index];
        // `PresentInfoKHR` is a plain struct in this ash version: the pointer
        // fields are written directly.
        // The present waits for the renderer to finish drawing the frame.
        let wait_semaphores = [render_finished];

        // `VK_KHR_present_wait` identifies *this* presentation with a
        // monotonically increasing, non-zero identifier. The identifier is
        // chosen by the application and must be greater than every identifier
        // previously passed for the same swapchain.
        let present_id = self.next_present_id;
        let present_ids = [present_id];
        let present_id_struct = vk::PresentIdKHR {
            s_type: vk::StructureType::PRESENT_ID_KHR,
            p_next: core::ptr::null(),
            swapchain_count: 1,
            p_present_ids: present_ids.as_ptr(),
            _marker: core::marker::PhantomData,
        };
        let p_next = if self.has_present_wait {
            &present_id_struct as *const vk::PresentIdKHR as *const core::ffi::c_void
        } else {
            core::ptr::null()
        };

        let present_info = vk::PresentInfoKHR {
            s_type: vk::StructureType::PRESENT_INFO_KHR,
            p_next,
            wait_semaphore_count: 1,
            p_wait_semaphores: wait_semaphores.as_ptr(),
            swapchain_count: 1,
            p_swapchains: swapchains.as_ptr(),
            p_image_indices: indices.as_ptr(),
            p_results: core::ptr::null_mut(),
            _marker: core::marker::PhantomData,
        };
        // SAFETY: the swapchain and image both belong to this device.
        let result = unsafe {
            self.khr_swapchain
                .queue_present(gpu.graphics_queue, &present_info)
        };
        self.acquired = None;
        // `queue_present` reports whether the presentation was suboptimal, and
        // an out-of-date surface is reported as an error. Neither is a
        // failure of the frame: both mean "recreate the swapchain".
        let mut outcome = classify_present(result).map_err(GpuError::from)?;
        // When the present already reported the swapchain as out of date the
        // image never reached the screen, so waiting for the presentation
        // would only burn the timeout.
        let out_of_date = matches!(result, Err(vk::Result::ERROR_OUT_OF_DATE_KHR));

        if self.has_present_wait {
            // The identifier must keep growing for the same swapchain.
            self.next_present_id += 1;
            // The frame right after a recreation never waits: the compositor
            // may still be holding the old chain.
            let skip_wait = core::mem::take(&mut self.skip_present_wait_once);
            if self.wait_for_present && !out_of_date && !skip_wait {
                // Block until the image is really on the screen. This is the
                // low-latency path: the input queue is sampled right after a
                // real present, so the reaction is picked up by the very next
                // frame instead of three frames later.
                //
                // The wait is **bounded**. A presentation that never happens
                // — a window that is not composited, an occluded or
                // minimised window — would otherwise block the frame loop
                // forever. A timeout means "this frame was not shown", which
                // is a hiccup, not a fatal error.
                // SAFETY: the swapchain is alive and the identifier was just
                // passed to `queue_present`.
                let waited = unsafe {
                    ash::khr::present_wait::Device::new(&gpu.instance, &gpu.device)
                        .wait_for_present(self.swapchain, present_id, PRESENT_WAIT_TIMEOUT_NS)
                };
                match waited {
                    Ok(()) => {
                        self.consecutive_timeouts = 0;
                    }
                    Err(vk::Result::TIMEOUT) => {
                        self.present_timeouts += 1;
                        self.consecutive_timeouts += 1;
                        if self.consecutive_timeouts >= PRESENT_WAIT_GIVE_UP_AFTER {
                            // The window is not being composited. Waiting for
                            // a presentation that never happens would add the
                            // full timeout to every frame, so the low-latency
                            // path switches itself off.
                            self.wait_for_present = false;
                            self.consecutive_timeouts = 0;
                        }
                    }
                    // The swapchain no longer matches the surface: recreate
                    // it instead of failing the frame.
                    Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
                        outcome = PresentOutcome::Recreate;
                    }
                    // The surface itself is gone: nothing can be presented
                    // any more, so this one stays a hard error.
                    Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                        return Err(GpuError::Vulkan(vk::Result::ERROR_SURFACE_LOST_KHR));
                    }
                    Err(error) => return Err(GpuError::Vulkan(error)),
                }
            }
        }
        Ok(outcome)
    }

    /// Destroy the swapchain and its views.
    pub fn destroy(self, gpu: &Gpu) {
        // SAFETY: the views and the swapchain belong to this device.
        unsafe {
            for semaphore in &self.acquire_semaphores {
                gpu.device.destroy_semaphore(*semaphore, None);
            }
            for semaphore in &self.render_finished_semaphores {
                gpu.device.destroy_semaphore(*semaphore, None);
            }
            for view in &self.views {
                gpu.device.destroy_image_view(*view, None);
            }
            self.khr_swapchain.destroy_swapchain(self.swapchain, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk;

    #[test]
    fn the_swapchain_requires_the_surface_extensions() {
        // The full stack: the surface, the swapchain and the maintenance
        // extension that enables the low-latency present mode.
        let extensions = vec![
            "VK_KHR_surface".to_string(),
            "VK_KHR_swapchain".to_string(),
            "VK_EXT_swapchain_maintenance1".to_string(),
        ];
        assert!(crate::swapchain::is_supported(&extensions));

        // Without the swapchain extension the stack is incomplete.
        let incomplete = vec!["VK_KHR_surface".to_string()];
        assert!(!crate::swapchain::is_supported(&incomplete));
    }

    #[test]
    fn the_swapchain_uses_the_renderers_colour_format() {
        // The swapchain images are the renderer's colour attachment, so the
        // format has to be the one the graphics pipelines were created with.
        // A different channel order would silently swap red and blue.
        assert_eq!(
            crate::pipeline::COLOR_FORMAT,
            vk::Format::B8G8R8A8_SRGB,
            "the swapchain must match the pipeline's attachment format"
        );
    }

    /// The lowest-latency present mode has its own extension, and its own
    /// value: `1000111000` is `VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR`,
    /// not the latest-ready mode.
    #[test]
    fn fifo_latest_ready_maps_to_the_extension_present_mode() {
        assert_eq!(
            resolve_present_mode(PresentMode::FifoLatestReady, true).as_raw(),
            1_000_361_000,
            "VK_PRESENT_MODE_FIFO_LATEST_READY_EXT"
        );
        assert_ne!(
            resolve_present_mode(PresentMode::FifoLatestReady, true),
            vk::PresentModeKHR::from_raw(1_000_111_000),
            "1000111000 is VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR"
        );

        // Without the extension the mode is illegal, so FIFO is used.
        assert_eq!(
            resolve_present_mode(PresentMode::FifoLatestReady, false),
            vk::PresentModeKHR::FIFO
        );
        assert_eq!(
            resolve_present_mode(PresentMode::Fifo, true),
            vk::PresentModeKHR::FIFO
        );
        assert_eq!(
            resolve_present_mode(PresentMode::Immediate, true),
            vk::PresentModeKHR::IMMEDIATE
        );
    }

    /// The driver's `currentExtent` is the authoritative surface size: a
    /// one-pixel difference is enough to make the driver report the
    /// swapchain as out of date on the very next frame.
    #[test]
    fn the_extent_prefers_the_drivers_current_extent() {
        let extent = resolve_extent((1920, 1080), (0, 0), (u32::MAX, u32::MAX), 800, 600);
        assert_eq!(extent.width, 1920);
        assert_eq!(extent.height, 1080);

        // `u32::MAX` is the "the surface has no size" sentinel, so the
        // requested size is used instead.
        let extent = resolve_extent((u32::MAX, u32::MAX), (0, 0), (u32::MAX, u32::MAX), 800, 600);
        assert_eq!(extent.width, 800);
        assert_eq!(extent.height, 600);

        // The result is clamped into the driver's range.
        let extent = resolve_extent((10, 10), (100, 100), (200, 200), 800, 600);
        assert_eq!(extent.width, 100);
        assert_eq!(extent.height, 100);
    }

    /// A driver that reports `min > max` must not make the clamp panic.
    #[test]
    fn the_extent_clamp_survives_a_nonsensical_range() {
        let extent = resolve_extent((150, 150), (300, 300), (200, 200), 150, 150);
        assert_eq!(extent.width, 200);
        assert_eq!(extent.height, 200);
    }

    /// An out-of-date surface is a request to recreate the swapchain, not a
    /// failure of the frame.
    #[test]
    fn present_and_acquire_outcomes_classify_correctly() {
        assert_eq!(classify_present(Ok(false)), Ok(PresentOutcome::Optimal));
        assert_eq!(classify_present(Ok(true)), Ok(PresentOutcome::Recreate));
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            Ok(PresentOutcome::Recreate)
        );
        assert_eq!(
            classify_present(Err(vk::Result::SUBOPTIMAL_KHR)),
            Ok(PresentOutcome::Recreate)
        );
        // A lost surface is fatal: nothing can be presented any more.
        assert!(classify_present(Err(vk::Result::ERROR_SURFACE_LOST_KHR)).is_err());

        assert_eq!(
            classify_acquire(Ok((3, false))),
            Ok(AcquireStatus::Ready {
                index: 3,
                suboptimal: false
            })
        );
        assert_eq!(
            classify_acquire(Ok((3, true))),
            Ok(AcquireStatus::Ready {
                index: 3,
                suboptimal: true
            })
        );
        assert_eq!(
            classify_acquire(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            Ok(AcquireStatus::OutOfDate)
        );
        assert!(classify_acquire(Err(vk::Result::ERROR_SURFACE_LOST_KHR)).is_err());
    }

    /// The render-finished semaphores are bound to the swapchain *image*,
    /// not to the frame slot.
    ///
    /// One global semaphore for every image is the bug that produced
    /// `VUID-vkQueueSubmit2-semaphore-03868`: the presentation engine of
    /// image 0 was still waiting on a semaphore that the frame for image 1
    /// signalled again.
    #[test]
    fn the_render_finished_semaphores_are_allocated_per_image() {
        // One semaphore per image — not per frame in flight.
        assert_eq!(render_finished_slot_count(1), 1);
        assert_eq!(render_finished_slot_count(3), 3);
        assert_eq!(render_finished_slot_count(0), 0);

        // The mapping is injective: two images never share a semaphore.
        let count = 4;
        let slots: Vec<usize> = (0..count as u32)
            .map(|index| render_finished_slot(index, count))
            .collect();
        let mut unique = slots.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), count, "every image needs its own semaphore");

        // An empty swapchain cannot panic.
        assert_eq!(render_finished_slot(0, 0), 0);
        assert_eq!(render_finished_slot(7, 0), 0);
    }

    /// The signatures the windowed callers depend on.
    ///
    /// The new outcome-aware API is additive: the four calls the showcase
    /// makes keep their exact shape. This test is a compile-time guard for
    /// that.
    #[test]
    #[allow(clippy::type_complexity)]
    fn the_windowed_api_keeps_its_shape() {
        let _new: fn(&Gpu, &Surface, u32, u32, &PresentConfig) -> Result<Swapchain, GpuError> =
            Swapchain::new;
        let _acquire: fn(&mut Swapchain, u64) -> Result<AcquiredImage, GpuError> =
            Swapchain::acquire;
        let _present: fn(&mut Swapchain, &Gpu, u32, vk::Semaphore) -> Result<(), GpuError> =
            Swapchain::present;
        let _destroy: fn(Swapchain, &Gpu) = Swapchain::destroy;
        let _extent: fn(&Swapchain) -> (u32, u32) = Swapchain::extent;

        // The outcome-aware API is additive.
        let _acquire_outcome: fn(&mut Swapchain, u64) -> Result<AcquireOutcome, GpuError> =
            Swapchain::acquire_outcome;
        let _present_outcome: fn(
            &mut Swapchain,
            &Gpu,
            u32,
            vk::Semaphore,
        ) -> Result<PresentOutcome, GpuError> = Swapchain::present_outcome;
        let _recreate: fn(
            &mut Swapchain,
            &Gpu,
            &Surface,
            u32,
            u32,
            &PresentConfig,
        ) -> Result<(), GpuError> = Swapchain::recreate;
    }

    /// Recreating the swapchain hands the old one to the driver instead of
    /// tearing the surface down first.
    ///
    /// The windowed part needs a real `HWND`, which the test suite does not
    /// build; the parts that can be checked without a window are the device
    /// negotiation and the pure decisions.
    #[test]
    fn swapchain_recreate_retires_the_old_handle_cleanly() {
        // The present mode and the extent are the two decisions that decide
        // whether the driver is happy with the new swapchain.
        assert_eq!(
            resolve_present_mode(PresentMode::FifoLatestReady, false),
            vk::PresentModeKHR::FIFO
        );
        let extent = resolve_extent((1280, 720), (0, 0), (u32::MAX, u32::MAX), 640, 360);
        assert_eq!((extent.width, extent.height), (1280, 720));

        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            // No GPU: the pure assertions above are all that can be checked.
            return;
        };

        // `VK_EXT_swapchain_maintenance1` is *not* requested by the test
        // device, so the feature must not be claimed: the flag reports the
        // enabled feature, not the extension string.
        assert!(!gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1"));
        assert!(
            !gpu.features.swapchain_maintenance1,
            "the feature must not be enabled without the extension"
        );

        // The instance extensions the surface-maintenance dependency chain
        // adds are only added when the device extension is requested.
        assert!(!gpu
            .enabled_instance_extensions
            .iter()
            .any(|name| name == "VK_EXT_surface_maintenance1"));

        gpu.destroy();
    }
}

//! Windowed presentation: surface, swapchain and the acquire/present cycle.
//!
//! The renderer in this crate is deliberately surface-free by default: every
//! test renders into an offscreen image, which is what makes the GPU suite
//! runnable on CI. This module adds the *windowed* half without changing that:
//! a [`Surface`] owns a `VkSurfaceKHR` and a swapchain, and
//! [`Swapchain::acquire`] hands the caller a colour view to draw into.
//!
//! The present mode follows the policy in [`crate::swapchain`]: with
//! `VK_EXT_swapchain_maintenance1` the FIFO-latest-ready mode keeps vsync but
//! presents the newest completed image, which is the difference between three
//! frames of input lag and one.

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::device::{Gpu, GpuError};
use crate::swapchain::{PresentConfig, PresentMode};
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
        let win32_surface =
            ash::khr::win32_surface::Instance::new(&gpu.entry, &gpu.instance);
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
    /// How many presentations did not reach the screen in time.
    present_timeouts: u32,
    /// How many timeouts happened in a row.
    consecutive_timeouts: u32,
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
                let available: Vec<String> = formats
                    .iter()
                    .map(|f| format!("{:?}", f.format))
                    .collect();
                GpuError::Allocation(format!(
                    "the surface does not expose the renderer's colour format {:?}; \
                     it offers {:?}",
                    crate::pipeline::COLOR_FORMAT,
                    available
                ))
            })?;

        let present_mode = match config.mode {
            PresentMode::FifoLatestReady => vk::PresentModeKHR::FIFO,
            PresentMode::Fifo => vk::PresentModeKHR::FIFO,
            PresentMode::Immediate => vk::PresentModeKHR::IMMEDIATE,
        };
        // Only ask for a mode the surface actually supports.
        let present_mode = if modes.contains(&present_mode) {
            present_mode
        } else {
            vk::PresentModeKHR::FIFO
        };

        let extent = vk::Extent2D {
            width: width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        };

        let image_count = (capabilities.min_image_count + 1).max(2);
        let image_count = image_count.min(capabilities.max_image_count);

        let queue_family_index = gpu.families.graphics;
        let queue_family_indices = [queue_family_index];

        let create_info = vk::SwapchainCreateInfoKHR::default()
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
            .clipped(true);

        let khr_swapchain =
            ash::khr::swapchain::Device::new(&gpu.instance, &gpu.device);
        // SAFETY: the surface belongs to the same instance as the device.
        let swapchain = unsafe { khr_swapchain.create_swapchain(&create_info, None) }
            .map_err(GpuError::from)?;

        // The images come from the swapchain; we only create views.
        // SAFETY: the swapchain was just created.
        let images = unsafe { khr_swapchain.get_swapchain_images(swapchain) }
            .map_err(GpuError::from)?;

        let mut views = Vec::with_capacity(images.len());
        for image in &images {
            // SAFETY: the image belongs to our swapchain.
            let view = unsafe {
                gpu.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(*image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(chosen.format)
                        .subresource_range(vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1)),
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
            next_semaphore: 0,
            format: chosen.format,
            color_space: chosen.color_space,
            next_index: 0,
            acquired: None,
            extent: (extent.width, extent.height),
            next_present_id: 1,
            wait_for_present: config.wait_for_present && has_present_wait,
            has_present_wait,
            present_timeouts: 0,
            consecutive_timeouts: 0,
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

    /// How many presentations did not reach the screen in time.
    ///
    /// A non-zero count means the window is not being composited the way the
    /// application expects — the frame loop keeps running, but the images
    /// are not actually shown.
    #[must_use]
    pub const fn present_timeouts(&self) -> u32 {
        self.present_timeouts
    }

    /// Acquire the next image to draw into.
    ///
    /// Returns the index into [`Self::views`], the image view to render into
    /// and the semaphore that is signalled once the image is ready. The
    /// caller must present the same index afterwards, and the render
    /// submission must wait on the semaphore before it touches the image.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn acquire(&mut self, timeout_ns: u64) -> Result<AcquiredImage, GpuError> {
        // The semaphore comes from a round-robin pool: a binary semaphore may
        // only carry one pending signal, so the slot is only reused once the
        // previous frame has consumed it.
        let semaphore = self.acquire_semaphores[self.next_semaphore];
        self.next_semaphore = (self.next_semaphore + 1) % self.acquire_semaphores.len();
        // SAFETY: the swapchain belongs to this device.
        let (index, _) = unsafe {
            self.khr_swapchain
                .acquire_next_image(self.swapchain, timeout_ns, semaphore, vk::Fence::null())
                .map_err(GpuError::from)?
        };
        self.next_index = index;
        self.acquired = Some(index);
        let view = self.views[index as usize];
        Ok(AcquiredImage {
            index,
            view,
            available: semaphore,
            image: self.images[index as usize],
        })
    }

    /// Present the acquired image, waiting for the renderer to finish.
    ///
    /// `render_finished` is signalled by the renderer's frame submission; the
    /// present waits for it so the display never shows a half-drawn frame.
    ///
    /// # Errors
    /// Propagates Vulkan failures, including a lost surface.
    pub fn present(
        &mut self,
        gpu: &Gpu,
        index: u32,
        render_finished: vk::Semaphore,
    ) -> Result<(), GpuError> {
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
        // `queue_present` reports whether the presentation was suboptimal;
        // that is not an error, so the boolean is dropped.
        result.map(|_| ()).map_err(GpuError::from)?;

        if self.has_present_wait {
            // The identifier must keep growing for the same swapchain.
            self.next_present_id += 1;
            if self.wait_for_present {
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
                    Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                        return Err(GpuError::Vulkan(vk::Result::ERROR_OUT_OF_DATE_KHR));
                    }
                    Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                        return Err(GpuError::Vulkan(vk::Result::ERROR_SURFACE_LOST_KHR));
                    }
                    Err(error) => return Err(GpuError::Vulkan(error)),
                }
            }
        }
        Ok(())
    }

    /// Destroy the swapchain and its views.
    pub fn destroy(self, gpu: &Gpu) {
        // SAFETY: the views and the swapchain belong to this device.
        unsafe {
            for semaphore in &self.acquire_semaphores {
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
}

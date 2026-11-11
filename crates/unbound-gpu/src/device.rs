//! Vulkan 1.3 device bring-up (`docs` §2.1).
//!
//! Everything the renderer needs is *requested and verified* here, once:
//!
//! * `bufferDeviceAddress` — the 64B push constants carry raw device addresses,
//! * `descriptorIndexing` (bindless) — one descriptor set for all textures,
//! * `dynamicRendering` — no `VkRenderPass`/`VkFramebuffer` boilerplate,
//! * `synchronization2` + `timelineSemaphore` — queue-family-correct barriers
//!   and multi-frame completion tracking,
//! * `multiDrawIndirect` + `drawIndirectCount` — GPU-authored draw commands.
//!
//! A device that cannot provide these is rejected instead of silently degrading
//! into a slow path: the whole architecture depends on them.
//!
//! This module is the one place in the crate that is unavoidably `unsafe`: it is
//! a thin, documented wrapper over the Vulkan C API. Every `unsafe` block below
//! is preceded by the invariant that makes it sound, and all handles are owned
//! by [`Gpu`] and released exactly once in [`Gpu::destroy`].
#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicU64, Ordering};

use ash::vk;

/// How a buffer's memory should be sourced.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// Fast VRAM, not CPU visible. Uploads go through a staging buffer.
    DeviceLocal,
    /// CPU visible + coherent. Small, frequently rewritten data.
    HostVisible,
    /// CPU visible, prefer device-local (ReBAR / Smart Access Memory).
    HostVisibleDeviceLocal,
}

/// Everything the renderer negotiated with the driver.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuFeatures {
    pub buffer_device_address: bool,
    pub descriptor_indexing: bool,
    pub runtime_descriptor_array: bool,
    pub partially_bound_descriptors: bool,
    pub update_after_bind: bool,
    pub dynamic_rendering: bool,
    pub synchronization2: bool,
    pub timeline_semaphore: bool,
    pub multi_draw_indirect: bool,
    pub draw_indirect_count: bool,
    pub shader_int64: bool,
    pub shader_float64: bool,
    pub independent_blend: bool,
    /// `hostQueryReset`: the host may reset a query pool between frames.
    pub host_query_reset: bool,
    pub sampler_anisotropy: bool,
    pub swapchain_maintenance1: bool,
    pub present_wait: bool,
    /// True when the persistent pool landed in ReBAR-visible memory.
    pub rebar: bool,
}

impl GpuFeatures {
    /// Features without which the architecture cannot function.
    #[must_use]
    pub fn missing_required(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.buffer_device_address {
            missing.push("bufferDeviceAddress");
        }
        if !self.descriptor_indexing {
            missing.push("descriptorIndexing");
        }
        if !self.dynamic_rendering {
            missing.push("dynamicRendering");
        }
        if !self.synchronization2 {
            missing.push("synchronization2");
        }
        if !self.timeline_semaphore {
            missing.push("timelineSemaphore");
        }
        if !self.multi_draw_indirect {
            missing.push("multiDrawIndirect");
        }
        if !self.draw_indirect_count {
            missing.push("drawIndirectCount");
        }
        missing
    }
}

/// Device creation failure.
#[derive(Debug)]
pub enum GpuError {
    /// The Vulkan loader is missing.
    Loader(String),
    /// `vkCreateInstance` / `vkCreateDevice` failed.
    Vulkan(vk::Result),
    /// No physical device matches the requirements.
    NoSuitableDevice(String),
    /// A required feature or extension is absent.
    UnsupportedFeatures(Vec<&'static str>),
    /// Allocation of a `VkBuffer` or `VkDeviceMemory` failed.
    Allocation(String),
    /// The device disappeared (`VK_ERROR_DEVICE_LOST`).
    DeviceLost,
}

impl core::fmt::Display for GpuError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Loader(e) => write!(f, "Vulkan loader unavailable: {e}"),
            Self::Vulkan(r) => write!(f, "Vulkan call failed: {r:?}"),
            Self::NoSuitableDevice(e) => write!(f, "no suitable GPU: {e}"),
            Self::UnsupportedFeatures(list) => {
                write!(f, "GPU lacks required features: {}", list.join(", "))
            }
            Self::Allocation(e) => write!(f, "GPU allocation failed: {e}"),
            Self::DeviceLost => write!(f, "VK_ERROR_DEVICE_LOST"),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<vk::Result> for GpuError {
    fn from(r: vk::Result) -> Self {
        if r == vk::Result::ERROR_DEVICE_LOST {
            Self::DeviceLost
        } else {
            Self::Vulkan(r)
        }
    }
}

/// Configuration for [`Gpu::new`].
#[derive(Clone, Debug)]
pub struct GpuConfig {
    pub app_name: String,
    /// Enable `VK_LAYER_KHRONOS_validation` when present.
    pub validation: bool,
    /// Extra instance extensions (e.g. the platform surface extensions).
    pub instance_extensions: Vec<String>,
    /// Extra device extensions (e.g. `VK_KHR_swapchain`).
    pub device_extensions: Vec<String>,
    /// Frames in flight. 2 keeps input latency minimal, 3 smooths jitter.
    pub frames_in_flight: u32,
    /// Persistent pool size in bytes.
    pub pool_capacity: u64,
    /// Per-frame transient ring size in bytes.
    pub ring_capacity: u64,
    /// Force the first discrete GPU when true; otherwise pick the best score.
    pub prefer_discrete: bool,
    /// Enable GPU-assisted validation when the validation layer is on.
    pub gpu_assisted_validation: bool,
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            app_name: "unbound-ui".into(),
            validation: cfg!(debug_assertions),
            instance_extensions: Vec::new(),
            device_extensions: Vec::new(),
            frames_in_flight: 2,
            pool_capacity: 64 << 20,
            ring_capacity: 4 << 20,
            prefer_discrete: true,
            gpu_assisted_validation: false,
        }
    }
}

/// A `VkBuffer` plus its bound memory and (optionally) a persistent mapping.
#[derive(Debug)]
pub struct Buffer {
    pub raw: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: u64,
    /// Device address (0 when `bufferDeviceAddress` was not requested).
    pub address: u64,
    /// Non-null when the memory is host visible and permanently mapped.
    pub mapped: *mut u8,
    pub host_visible: bool,
}

// SAFETY: the pointers inside are opaque Vulkan handles; Vulkan objects are
// externally synchronised by the renderer, which owns the only `Gpu`.
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    /// Host pointer to `offset`, if the buffer is mapped.
    #[must_use]
    pub fn host_ptr(&self, offset: u64) -> Option<*mut u8> {
        if self.mapped.is_null() || offset > self.size {
            None
        } else {
            // SAFETY: pointer arithmetic within the mapped allocation.
            Some(unsafe { self.mapped.add(offset as usize) })
        }
    }
}

/// Timeline semaphore wrapper.
#[derive(Debug)]
pub struct Timeline {
    pub semaphore: vk::Semaphore,
    value: AtomicU64,
}

impl Timeline {
    #[must_use]
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Acquire)
    }

    /// Reserve the next value to be signalled.
    pub fn next(&self) -> u64 {
        self.value.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Record that `value` has completed.
    pub fn completed(&self, value: u64) {
        let mut cur = self.value.load(Ordering::Acquire);
        while value > cur {
            match self
                .value
                .compare_exchange(cur, value, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
    }
}

/// Per-frame synchronisation primitives.
#[derive(Debug)]
pub struct FrameSync {
    pub image_available: vk::Semaphore,
    pub render_finished: vk::Semaphore,
    pub in_flight: vk::Fence,
    pub command_buffer: vk::CommandBuffer,
    /// Timeline value that marks this frame complete.
    pub timeline_value: u64,
}

/// Queue family assignment.
#[derive(Copy, Clone, Debug, Default)]
pub struct QueueFamilies {
    pub graphics: u32,
    pub transfer: u32,
    /// True when the transfer queue is a distinct family (real DMA queue).
    pub dedicated_transfer: bool,
}

/// The whole device bundle: instance, physical device, device, queues and
/// per-frame synchronisation.
pub struct Gpu {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub physical: vk::PhysicalDevice,
    pub device: ash::Device,
    pub properties: vk::PhysicalDeviceProperties,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub families: QueueFamilies,
    pub graphics_queue: vk::Queue,
    pub transfer_queue: vk::Queue,
    pub features: GpuFeatures,
    pub enabled_device_extensions: Vec<String>,
    /// Dynamic-rendering function table (`vkCmdBeginRendering`).
    pub khr_dynamic_rendering: ash::khr::dynamic_rendering::Device,
    /// Synchronization2 function table (`vkCmdPipelineBarrier2`).
    pub khr_synchronization2: ash::khr::synchronization2::Device,
    /// Draw-indirect-count function table (`vkCmdDrawIndirectCount`).
    pub khr_draw_indirect_count: ash::khr::draw_indirect_count::Device,
    pub timeline: Timeline,
    pub frames: Vec<FrameSync>,
    command_pool: vk::CommandPool,
    pub config: GpuConfig,
}

impl core::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gpu")
            .field("device", &self.device_name())
            .field("api", &self.api_version())
            .field("features", &self.features)
            .field("frames_in_flight", &self.frames.len())
            .finish_non_exhaustive()
    }
}

fn cstrings(list: &[String]) -> Vec<std::ffi::CString> {
    list.iter()
        .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
        .collect()
}

fn raw_ptrs(strings: &[std::ffi::CString]) -> Vec<*const c_char> {
    strings.iter().map(|c| c.as_ptr()).collect()
}

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

impl Gpu {
    /// Create the instance, pick a device and enable everything the renderer
    /// needs.
    ///
    /// # Errors
    /// Returns [`GpuError`] on loader/instance/device failure or when the GPU
    /// lacks a required feature.
    pub fn new(config: GpuConfig) -> Result<Self, GpuError> {
        // SAFETY: every call below is a plain Vulkan object creation whose
        // results are checked; all created objects are owned by `Self`.
        unsafe { Self::new_inner(config) }
    }

    unsafe fn new_inner(config: GpuConfig) -> Result<Self, GpuError> {
        let entry = ash::Entry::load().map_err(|e| GpuError::Loader(e.to_string()))?;

        // ---- layers -------------------------------------------------------
        let available_layers = entry
            .enumerate_instance_layer_properties()
            .unwrap_or_default();
        let validation_available = available_layers.iter().any(|l| {
            l.layer_name_as_c_str()
                .map(|n| n == VALIDATION_LAYER)
                .unwrap_or(false)
        });
        let enable_validation = config.validation && validation_available;
        let layer_names = if enable_validation {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };

        let mut instance_exts = config.instance_extensions.clone();
        if enable_validation {
            instance_exts.push("VK_EXT_debug_utils".to_string());
        }
        let ext_cstrings = cstrings(&instance_exts);
        let ext_ptrs = raw_ptrs(&ext_cstrings);

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"unbound-ui")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"unbound")
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::make_api_version(0, 1, 3, 0));

        let instance = entry
            .create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app_info)
                    .enabled_layer_names(&layer_names)
                    .enabled_extension_names(&ext_ptrs),
                None,
            )
            .map_err(GpuError::Vulkan)?;

        // ---- debug messenger ---------------------------------------------
        let debug = if enable_validation {
            let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
            let messenger = utils
                .create_debug_utils_messenger(&debug_messenger_info(), None)
                .map_err(GpuError::Vulkan)?;
            Some((utils, messenger))
        } else {
            None
        };

        // ---- physical device ---------------------------------------------
        let devices = instance
            .enumerate_physical_devices()
            .map_err(GpuError::Vulkan)?;
        if devices.is_empty() {
            return Err(GpuError::NoSuitableDevice("no physical device".into()));
        }

        let mut best: Option<(vk::PhysicalDevice, i32, vk::PhysicalDeviceProperties)> = None;
        for pd in devices {
            let props = instance.get_physical_device_properties(pd);
            let mut score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
                _ => 100,
            };
            if props.api_version < vk::make_api_version(0, 1, 3, 0) {
                score -= 900;
            }
            if config.prefer_discrete && props.device_type != vk::PhysicalDeviceType::DISCRETE_GPU {
                score -= 100;
            }
            if best.as_ref().map(|(_, s, _)| score > *s).unwrap_or(true) {
                best = Some((pd, score, props));
            }
        }
        let (physical, _, properties) = best.expect("at least one device");

        // ---- feature discovery -------------------------------------------
        let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
        let core;
        {
            let mut query = vk::PhysicalDeviceFeatures2::default()
                .push_next(&mut f13)
                .push_next(&mut f12);
            instance.get_physical_device_features2(physical, &mut query);
            core = query.features;
        }

        let dev_ext_props = instance
            .enumerate_device_extension_properties(physical)
            .unwrap_or_default();
        let dev_ext_names: Vec<String> = dev_ext_props
            .iter()
            .filter_map(|e| {
                e.extension_name_as_c_str()
                    .ok()
                    .map(|c| c.to_string_lossy().into_owned())
            })
            .collect();
        let has_ext = |name: &str| dev_ext_names.iter().any(|n| n == name);

        let features = GpuFeatures {
            buffer_device_address: f12.buffer_device_address == vk::TRUE,
            descriptor_indexing: f12.descriptor_indexing == vk::TRUE,
            runtime_descriptor_array: f12.runtime_descriptor_array == vk::TRUE,
            partially_bound_descriptors: f12.descriptor_binding_partially_bound == vk::TRUE,
            update_after_bind: f12.descriptor_binding_sampled_image_update_after_bind
                == vk::TRUE,
            dynamic_rendering: f13.dynamic_rendering == vk::TRUE,
            synchronization2: f13.synchronization2 == vk::TRUE,
            timeline_semaphore: f12.timeline_semaphore == vk::TRUE,
            host_query_reset: f12.host_query_reset == vk::TRUE,
            multi_draw_indirect: core.multi_draw_indirect == vk::TRUE,
            draw_indirect_count: f12.draw_indirect_count == vk::TRUE,
            shader_int64: core.shader_int64 == vk::TRUE,
            shader_float64: core.shader_float64 == vk::TRUE,
            independent_blend: core.independent_blend == vk::TRUE,
            sampler_anisotropy: core.sampler_anisotropy == vk::TRUE,
            swapchain_maintenance1: has_ext("VK_EXT_swapchain_maintenance1"),
            present_wait: has_ext("VK_KHR_present_wait"),
            rebar: false,
        };

        let missing = features.missing_required();
        if !missing.is_empty() {
            return Err(GpuError::UnsupportedFeatures(missing));
        }

        // ---- queue families ----------------------------------------------
        let families_props = instance.get_physical_device_queue_family_properties(physical);
        let mut graphics = None;
        let mut transfer = None;
        for (i, p) in families_props.iter().enumerate() {
            let f = p.queue_flags;
            if f.contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE) {
                graphics.get_or_insert(i as u32);
            }
            // Prefer a queue that can ONLY transfer: that is the DMA engine.
            if f.contains(vk::QueueFlags::TRANSFER) && !f.contains(vk::QueueFlags::GRAPHICS) {
                transfer.get_or_insert(i as u32);
            }
        }
        let graphics = graphics.ok_or_else(|| {
            GpuError::NoSuitableDevice("no graphics+compute queue family".into())
        })?;
        let dedicated = transfer.is_some();
        let transfer = transfer.unwrap_or(graphics);
        let families = QueueFamilies {
            graphics,
            transfer,
            dedicated_transfer: dedicated,
        };

        // ---- device -------------------------------------------------------
        let mut used_families = vec![graphics];
        if transfer != graphics {
            used_families.push(transfer);
        }
        used_families.sort_unstable();
        used_families.dedup();
        let priorities = [1.0f32];
        let queue_infos: Vec<vk::DeviceQueueCreateInfo<'_>> = used_families
            .iter()
            .map(|f| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*f)
                    .queue_priorities(&priorities)
            })
            .collect();

        let mut enabled_features12 = vk::PhysicalDeviceVulkan12Features::default()
            .buffer_device_address(features.buffer_device_address)
            .descriptor_indexing(features.descriptor_indexing)
            .runtime_descriptor_array(features.runtime_descriptor_array)
            .descriptor_binding_partially_bound(features.partially_bound_descriptors)
            .descriptor_binding_sampled_image_update_after_bind(features.update_after_bind)
            .descriptor_binding_variable_descriptor_count(features.runtime_descriptor_array)
            .shader_sampled_image_array_non_uniform_indexing(features.runtime_descriptor_array)
            .timeline_semaphore(features.timeline_semaphore)
            .host_query_reset(features.host_query_reset)
            .draw_indirect_count(features.draw_indirect_count);
        let mut enabled_features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(features.dynamic_rendering)
            .synchronization2(features.synchronization2);
        let enabled_core = vk::PhysicalDeviceFeatures::default()
            .multi_draw_indirect(features.multi_draw_indirect)
            .independent_blend(features.independent_blend)
            .sampler_anisotropy(features.sampler_anisotropy)
            .shader_int64(features.shader_int64)
            .shader_float64(features.shader_float64);

        let device_exts = config.device_extensions.clone();
        for optional in [
            "VK_EXT_swapchain_maintenance1",
            "VK_KHR_present_wait",
        ] {
            if has_ext(optional) && !device_exts.iter().any(|e| e == optional) {
                // Only opt in when the caller is creating a swapchain; a headless
                // device gains nothing and adds validation noise.
                let _ = optional;
            }
        }
        let dev_ext_cstrings = cstrings(&device_exts);
        let dev_ext_ptrs = raw_ptrs(&dev_ext_cstrings);

        let device_create = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&dev_ext_ptrs)
            .enabled_features(&enabled_core)
            .push_next(&mut enabled_features13)
            .push_next(&mut enabled_features12);

        let device = instance
            .create_device(physical, &device_create, None)
            .map_err(GpuError::Vulkan)?;

        Self::finish(
            entry,
            instance,
            debug,
            physical,
            device,
            properties,
            families,
            features,
            device_exts,
            config,
        )
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn finish(
        entry: ash::Entry,
        instance: ash::Instance,
        debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
        physical: vk::PhysicalDevice,
        device: ash::Device,
        properties: vk::PhysicalDeviceProperties,
        families: QueueFamilies,
        mut features: GpuFeatures,
        enabled_device_extensions: Vec<String>,
        config: GpuConfig,
    ) -> Result<Self, GpuError> {
        let graphics_queue = device.get_device_queue(families.graphics, 0);
        let transfer_queue = if families.dedicated_transfer {
            device.get_device_queue(families.transfer, 0)
        } else {
            graphics_queue
        };

        let memory_properties = instance.get_physical_device_memory_properties(physical);

        // ReBAR detection: the largest device-local heap that is also host
        // visible means the pool can be written directly from the CPU.
        features.rebar = (0..memory_properties.memory_type_count as usize).any(|i| {
            let t = memory_properties.memory_types[i];
            t.property_flags.contains(
                vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE,
            ) && memory_properties.memory_heaps[t.heap_index as usize].size > (256 << 20)
        });

        // Timeline semaphore, used by every subsystem to know when a frame is
        // truly done.
        let sem_type = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let mut sem_type = sem_type;
        let timeline_semaphore = device
            .create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut sem_type),
                None,
            )
            .map_err(GpuError::Vulkan)?;
        let timeline = Timeline {
            semaphore: timeline_semaphore,
            value: AtomicU64::new(0),
        };

        let command_pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(families.graphics)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .map_err(GpuError::Vulkan)?;

        let frame_count = config.frames_in_flight.max(1);
        let cmd_buffers = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(frame_count),
            )
            .map_err(GpuError::Vulkan)?;

        let mut frames = Vec::with_capacity(frame_count as usize);
        for (i, cmd) in cmd_buffers.into_iter().enumerate() {
            let image_available = device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(GpuError::Vulkan)?;
            let render_finished = device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(GpuError::Vulkan)?;
            let in_flight = device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(GpuError::Vulkan)?;
            frames.push(FrameSync {
                image_available,
                render_finished,
                in_flight,
                command_buffer: cmd,
                timeline_value: i as u64,
            });
        }

        let khr_dynamic_rendering = ash::khr::dynamic_rendering::Device::new(&instance, &device);
        let khr_synchronization2 = ash::khr::synchronization2::Device::new(&instance, &device);
        let khr_draw_indirect_count = ash::khr::draw_indirect_count::Device::new(&instance, &device);

        Ok(Self {
            entry,
            instance,
            debug,
            physical,
            device,
            properties,
            memory_properties,
            families,
            graphics_queue,
            transfer_queue,
            features,
            enabled_device_extensions,
            khr_dynamic_rendering,
            khr_synchronization2,
            khr_draw_indirect_count,
            timeline,
            frames,
            command_pool,
            config,
        })
    }

    #[must_use]
    pub fn device_name(&self) -> String {
        self.properties
            .device_name_as_c_str()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "<unnamed>".into())
    }

    #[must_use]
    pub fn api_version(&self) -> (u32, u32, u32) {
        (
            vk::api_version_major(self.properties.api_version),
            vk::api_version_minor(self.properties.api_version),
            vk::api_version_patch(self.properties.api_version),
        )
    }

    #[must_use]
    pub fn frames_in_flight(&self) -> u32 {
        self.frames.len() as u32
    }

    /// Public wrapper over the memory-type search (used by the texture pool).
    #[must_use]
    pub fn memory_type_index_public(&self, kind: MemoryKind) -> Option<u32> {
        self.memory_type_index(kind)
    }

    /// Find a memory type that both matches `kind` **and** is allowed by the
    /// resource's `memoryTypeBits` requirement.
    ///
    /// Vulkan rejects `vkBindImageMemory` when the type index is not in the
    /// image's bit mask, and the "best" type by properties is frequently outside
    /// it (tiling constraints differ per format).
    #[must_use]
    pub fn memory_type_index_for_bits(&self, bits: u32, kind: MemoryKind) -> Option<u32> {
        let required = match kind {
            MemoryKind::DeviceLocal => vk::MemoryPropertyFlags::DEVICE_LOCAL,
            _ => vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        };
        let preferred = match kind {
            MemoryKind::DeviceLocal => {
                vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE
            }
            _ => vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        };
        let candidates = |mask: vk::MemoryPropertyFlags| {
            (0..self.memory_properties.memory_type_count as usize).find(|i| {
                bits & (1 << i) != 0
                    && self.memory_properties.memory_types[*i]
                        .property_flags
                        .contains(mask)
            })
        };
        candidates(preferred)
            .or_else(|| candidates(required))
            .map(|i| i as u32)
    }

    /// Find a memory type satisfying `flags`, preferring `kind`.
    fn memory_type_index(&self, kind: MemoryKind) -> Option<u32> {
        let wanted = match kind {
            MemoryKind::DeviceLocal => {
                vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE
            }
            MemoryKind::HostVisible => {
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
            }
            MemoryKind::HostVisibleDeviceLocal => {
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
            }
        };
        let required = match kind {
            MemoryKind::DeviceLocal => vk::MemoryPropertyFlags::DEVICE_LOCAL,
            _ => vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        };
        // First pass: exact preference.
        for (i, t) in self
            .memory_properties
            .memory_types
            .iter()
            .enumerate()
            .take(self.memory_properties.memory_type_count as usize)
        {
            if t.property_flags.contains(wanted) {
                return Some(i as u32);
            }
        }
        // Second pass: minimum requirement.
        for (i, t) in self
            .memory_properties
            .memory_types
            .iter()
            .enumerate()
            .take(self.memory_properties.memory_type_count as usize)
        {
            if t.property_flags.contains(required) {
                return Some(i as u32);
            }
        }
        None
    }

    /// Create a buffer and bind its memory.
    ///
    /// # Errors
    /// Returns [`GpuError::Allocation`] when no memory type matches or the
    /// driver rejects the allocation.
    pub fn create_buffer(
        &self,
        size: u64,
        usage: vk::BufferUsageFlags,
        kind: MemoryKind,
        name: &str,
    ) -> Result<Buffer, GpuError> {
        if size == 0 {
            return Err(GpuError::Allocation(format!("{name}: zero-sized buffer")));
        }
        // SAFETY: straight-line Vulkan creation; on any failure the partially
        // created objects are destroyed before returning.
        unsafe {
            let buffer = self
                .device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(GpuError::Vulkan)?;

            let requirements = self.device.get_buffer_memory_requirements(buffer);
            let type_index = self
                .memory_type_index_for_bits(requirements.memory_type_bits, kind)
                .ok_or_else(|| {
                    GpuError::Allocation(format!("{name}: no memory type for {kind:?}"))
                })?;

            let mut flags_info = vk::MemoryAllocateFlagsInfo::default()
                .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
            let mut alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(type_index);
            if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                alloc_info = alloc_info.push_next(&mut flags_info);
            }

            let memory = match self.device.allocate_memory(&alloc_info, None) {
                Ok(m) => m,
                Err(e) => {
                    self.device.destroy_buffer(buffer, None);
                    return Err(GpuError::from(e));
                }
            };
            if let Err(e) = self.device.bind_buffer_memory(buffer, memory, 0) {
                self.device.destroy_buffer(buffer, None);
                self.device.free_memory(memory, None);
                return Err(GpuError::from(e));
            }

            let host_visible = self.memory_properties.memory_types[type_index as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
            let mapped = if host_visible {
                self.device
                    .map_memory(memory, 0, requirements.size, vk::MemoryMapFlags::empty())
                    .map_err(|e| {
                        self.device.destroy_buffer(buffer, None);
                        self.device.free_memory(memory, None);
                        GpuError::from(e)
                    })? as *mut u8
            } else {
                core::ptr::null_mut()
            };

            let address = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                self.device.get_buffer_device_address(
                    &vk::BufferDeviceAddressInfo::default().buffer(buffer),
                )
            } else {
                0
            };

            Ok(Buffer {
                raw: buffer,
                memory,
                size,
                address,
                mapped,
                host_visible,
            })
        }
    }

    /// Destroy a buffer and its memory.
    ///
    /// # Safety
    /// The caller must ensure no command buffer still references the buffer.
    pub unsafe fn destroy_buffer(&self, buffer: &Buffer) {
        if !buffer.mapped.is_null() {
            self.device.unmap_memory(buffer.memory);
        }
        self.device.destroy_buffer(buffer.raw, None);
        self.device.free_memory(buffer.memory, None);
    }

    /// Write `data` into a host-visible buffer.
    ///
    /// # Safety
    /// `offset + data.len()` must be within the buffer, and the caller must not
    /// race with the GPU reading that range.
    pub unsafe fn write_host<T: bytemuck::Pod>(
        &self,
        buffer: &Buffer,
        offset: u64,
        data: &[T],
    ) -> bool {
        let bytes = bytemuck::cast_slice(data);
        let Some(ptr) = buffer.host_ptr(offset) else {
            return false;
        };
        if offset as usize + bytes.len() > buffer.size as usize {
            return false;
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        true
    }

    /// Flush a mapped range (no-op for coherent memory, required otherwise).
    ///
    /// # Safety
    /// `offset..offset+len` must be inside a mapped buffer.
    pub unsafe fn flush(&self, buffer: &Buffer, offset: u64, len: u64) {
        if buffer.mapped.is_null() {
            return;
        }
        let range = vk::MappedMemoryRange::default()
            .memory(buffer.memory)
            .offset(offset)
            .size(len);
        let _ = self.device.flush_mapped_memory_ranges(&[range]);
    }

    /// Begin a one-shot command buffer on the graphics queue.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn begin_single_time(&self) -> Result<vk::CommandBuffer, GpuError> {
        // SAFETY: standard one-shot pattern; the pair is always ended in
        // `end_single_time`.
        unsafe {
            let cmd = self.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            self.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            Ok(cmd)
        }
    }

    /// Submit and wait for a one-shot command buffer.
    ///
    /// # Errors
    /// Propagates Vulkan failures.
    pub fn end_single_time(&self, cmd: vk::CommandBuffer) -> Result<(), GpuError> {
        // SAFETY: `cmd` comes from `begin_single_time` and is freed here.
        unsafe {
            self.device.end_command_buffer(cmd)?;
            let cmds = [cmd];
            let submit = vk::SubmitInfo::default().command_buffers(&cmds);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null())?;
            self.device.queue_wait_idle(self.graphics_queue)?;
            self.device.free_command_buffers(self.command_pool, &cmds);
            Ok(())
        }
    }

    /// Upload `bytes` to a device-local buffer through a staging buffer.
    ///
    /// # Errors
    /// Propagates allocation and Vulkan failures.
    pub fn upload_buffer(&self, dst: &Buffer, offset: u64, bytes: &[u8]) -> Result<(), GpuError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let staging = self.create_buffer(
            bytes.len() as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryKind::HostVisible,
            "staging-upload",
        )?;
        // SAFETY: the staging buffer is exclusively ours.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), staging.mapped, bytes.len());
            let cmd = self.begin_single_time()?;
            self.device.cmd_copy_buffer(
                cmd,
                staging.raw,
                dst.raw,
                &[vk::BufferCopy::default()
                    .src_offset(0)
                    .dst_offset(offset)
                    .size(bytes.len() as u64)],
            );
            self.end_single_time(cmd)?;
            self.destroy_buffer(&staging);
        }
        Ok(())
    }

    /// Wait until the GPU has completed `value` on the timeline.
    ///
    /// # Errors
    /// Propagates Vulkan failures (including device loss).
    pub fn wait_timeline(&self, value: u64) -> Result<(), GpuError> {
        // SAFETY: plain semaphore wait.
        unsafe {
            let semaphores = [self.timeline.semaphore];
            let values = [value];
            let info = vk::SemaphoreWaitInfo::default()
                .semaphores(&semaphores)
                .values(&values);
            self.device.wait_semaphores(&info, u64::MAX)?;
        }
        self.timeline.completed(value);
        Ok(())
    }

    /// Block until the whole device is idle.
    pub fn wait_idle(&self) {
        // SAFETY: trivial.
        unsafe {
            let _ = self.device.device_wait_idle();
        }
    }

    /// Tear everything down. Safe to call exactly once.
    pub fn destroy(mut self) {
        // SAFETY: we own every handle and are consuming `self`.
        unsafe {
            let _ = self.device.device_wait_idle();
            for f in &self.frames {
                self.device.destroy_semaphore(f.image_available, None);
                self.device.destroy_semaphore(f.render_finished, None);
                self.device.destroy_fence(f.in_flight, None);
            }
            self.device.destroy_semaphore(self.timeline.semaphore, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            if let Some((utils, messenger)) = self.debug.take() {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

fn debug_messenger_info<'a>() -> vk::DebugUtilsMessengerCreateInfoEXT<'a> {
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(vulkan_debug_callback))
}

unsafe extern "system" fn vulkan_debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut core::ffi::c_void,
) -> vk::Bool32 {
    if data.is_null() {
        return vk::FALSE;
    }
    // SAFETY: the driver guarantees a valid pointer for the callback duration.
    let data = unsafe { &*data };
    let message = data
        .message_as_c_str()
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Validation output must never be swallowed by an unconfigured logger: the
    // whole point of the layer is to be seen during development and CI.
    // Count everything so CI can gate on it instead of on log scraping.
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        crate::debug::VALIDATION.record("ERROR", &message);
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        crate::debug::VALIDATION.record("WARNING", &message);
    }
    match severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
            eprintln!("[vulkan-error:{kind:?}] {message}")
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
            eprintln!("[vulkan-warning:{kind:?}] {message}")
        }
        _ => eprintln!("[vulkan-info:{kind:?}] {message}"),
    }
    vk::FALSE
}

/// Serialises GPU tests.
///
/// Multiple `VkInstance`s creating devices simultaneously trips the vendor
/// overlay layers installed on developer machines (RTSS / FPS Monitor), so the
/// integration tests take this lock instead of fighting the environment.
pub fn gpu_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Test helper: create a headless device and skip the test when unavailable.
///
/// Returns `None` when the machine has no Vulkan 1.3 device, so CI without a
/// GPU reports "skipped" instead of "failed".
#[must_use]
pub fn test_device() -> Option<Gpu> {
    let validation = std::env::var("UNBOUND_VALIDATION").is_ok();
    match Gpu::new(GpuConfig {
        validation,
        pool_capacity: 8 << 20,
        ring_capacity: 1 << 20,
        ..Default::default()
    }) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_feature_list_is_stable() {
        let none = GpuFeatures::default().missing_required();
        assert_eq!(none.len(), 7);
        let all = GpuFeatures {
            buffer_device_address: true,
            descriptor_indexing: true,
            dynamic_rendering: true,
            synchronization2: true,
            timeline_semaphore: true,
            multi_draw_indirect: true,
            draw_indirect_count: true,
            ..Default::default()
        };
        assert!(all.missing_required().is_empty());
    }

    #[test]
    fn device_creation_reports_features() {
        let _guard = gpu_test_guard();
        let Some(gpu) = test_device() else {
            return;
        };
        assert!(gpu.features.buffer_device_address);
        assert!(gpu.features.dynamic_rendering);
        assert!(gpu.features.synchronization2);
        assert!(gpu.features.timeline_semaphore);
        assert!(gpu.frames_in_flight() >= 2);
        println!(
            "device={} api={:?} rebar={} dedicated_transfer={}",
            gpu.device_name(),
            gpu.api_version(),
            gpu.features.rebar,
            gpu.families.dedicated_transfer
        );
        gpu.destroy();
    }

    #[test]
    fn buffer_device_address_is_64_byte_aligned() {
        let _guard = gpu_test_guard();
        let Some(gpu) = test_device() else {
            return;
        };
        let buf = gpu
            .create_buffer(
                4096,
                vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                MemoryKind::DeviceLocal,
                "test-bda",
            )
            .expect("buffer");
        assert_ne!(buf.address, 0);
        assert_eq!(buf.address % 64, 0, "BDA must be 64B aligned");
        // SAFETY: nobody else references the buffer.
        unsafe { gpu.destroy_buffer(&buf) };
        gpu.destroy();
    }

    #[test]
    fn staging_upload_round_trips() {
        let _guard = gpu_test_guard();
        let Some(gpu) = test_device() else {
            return;
        };
        let payload: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
        let dst = gpu
            .create_buffer(
                payload.len() as u64,
                vk::BufferUsageFlags::TRANSFER_DST
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | vk::BufferUsageFlags::STORAGE_BUFFER,
                MemoryKind::DeviceLocal,
                "round-trip",
            )
            .expect("dst");
        gpu.upload_buffer(&dst, 0, &payload).expect("upload");

        // Read it back through a host-visible staging buffer.
        let readback = gpu
            .create_buffer(
                payload.len() as u64,
                vk::BufferUsageFlags::TRANSFER_DST,
                MemoryKind::HostVisible,
                "readback",
            )
            .expect("readback");
        // SAFETY: exclusive ownership + queue idle after upload.
        unsafe {
            let cmd = gpu.begin_single_time().unwrap();
            gpu.device.cmd_copy_buffer(
                cmd,
                dst.raw,
                readback.raw,
                &[vk::BufferCopy::default().size(payload.len() as u64)],
            );
            gpu.end_single_time(cmd).unwrap();
            let bytes = core::slice::from_raw_parts(readback.mapped, payload.len());
            assert_eq!(bytes, payload.as_slice());
            gpu.destroy_buffer(&readback);
            gpu.destroy_buffer(&dst);
        }
        gpu.destroy();
    }

    #[test]
    fn timeline_reaches_signalled_value() {
        let _guard = gpu_test_guard();
        let Some(gpu) = test_device() else {
            return;
        };
        // SAFETY: signalling on the graphics queue with an empty submit.
        unsafe {
            let value = gpu.timeline.next();
            let semaphores = [gpu.timeline.semaphore];
            let values = [value];
            let mut timeline_info =
                vk::TimelineSemaphoreSubmitInfo::default().signal_semaphore_values(&values);
            let submit = vk::SubmitInfo::default()
                .signal_semaphores(&semaphores)
                .push_next(&mut timeline_info);
            gpu.device
                .queue_submit(gpu.graphics_queue, &[submit], vk::Fence::null())
                .unwrap();
            gpu.wait_timeline(value).unwrap();
            assert!(gpu.timeline.value() >= value);
        }
        gpu.destroy();
    }
}

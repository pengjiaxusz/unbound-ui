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

/// Pick a memory type for `kind` out of the device's `memory_types`.
///
/// `bits` is the resource's `memoryTypeBits`: a type outside that mask cannot
/// back the allocation at all, whatever its properties say.
///
/// The ladders matter more than they look. A discrete GPU reports three kinds
/// of heap: real VRAM (`DEVICE_LOCAL`), system memory (`HOST_VISIBLE`), and a
/// small window of VRAM the CPU can also map — on NVIDIA the "BAR" window is a
/// couple of hundred megabytes, and it is the only type that is both device
/// local and host visible. Preferring it for [`MemoryKind::DeviceLocal`] does
/// not make anything faster: it caps every pool, sort key buffer and atlas
/// image at the size of that window, and the failure mode is a
/// `vkAllocateMemory` that returns `ERROR_OUT_OF_DEVICE_MEMORY` with most of
/// the card still free. `DeviceLocal` means fast VRAM the CPU never touches, so
/// a plain device-local type always wins; the window is the fallback for parts
/// that expose nothing else.
fn choose_memory_type(types: &[vk::MemoryType], bits: u32, kind: MemoryKind) -> Option<u32> {
    use vk::MemoryPropertyFlags as Flags;

    let coherent = Flags::HOST_VISIBLE | Flags::HOST_COHERENT;
    let local = Flags::DEVICE_LOCAL;

    let find = |test: &dyn Fn(Flags) -> bool| {
        types.iter().enumerate().find_map(|(index, ty)| {
            let allowed = bits & (1 << index) != 0;
            (allowed && test(ty.property_flags)).then_some(index as u32)
        })
    };

    match kind {
        // Fast VRAM, never mapped on the host.
        MemoryKind::DeviceLocal => {
            find(&|flags| flags.contains(local) && !flags.contains(coherent))
                .or_else(|| find(&|flags| flags.contains(local)))
        }
        // ReBAR / Smart Access Memory: VRAM the CPU may write directly.
        MemoryKind::HostVisibleDeviceLocal => {
            find(&|flags| flags.contains(local) && flags.contains(coherent))
                .or_else(|| find(&|flags| flags.contains(coherent)))
        }
        // Staging: system memory, because a mapped write there does not steal
        // from the VRAM window.
        MemoryKind::HostVisible => {
            find(&|flags| flags.contains(coherent) && !flags.contains(local))
                .or_else(|| find(&|flags| flags.contains(coherent)))
        }
    }
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
    /// `drawIndirectFirstInstance`: the host may set a non-zero
    /// `firstInstance` in an indirect command.
    ///
    /// Without it the driver is allowed to ignore `firstInstance`, which
    /// makes every draw run after the first redraw the *first* run's
    /// primitives. The renderer's MDI compaction emits one run per
    /// `(layer, barrier, variant)` group, so the feature is required.
    pub draw_indirect_first_instance: bool,
    pub shader_int64: bool,
    pub shader_float64: bool,
    pub independent_blend: bool,
    /// `shaderDemoteToHelperInvocation`: the fragment shaders' `discard`
    /// compiles to `OpDemoteToHelperInvocation` under
    /// `--target-env=vulkan1.3`. Creating such a module without the feature
    /// enabled is invalid usage (`VUID-VkShaderModuleCreateInfo-pCode-08740`).
    ///
    /// On a Vulkan 1.2 device the feature comes from
    /// `VK_EXT_shader_demote_to_helper_invocation` instead of core 1.3.
    pub shader_demote_to_helper_invocation: bool,
    /// `hostQueryReset`: the host may reset a query pool between frames.
    pub host_query_reset: bool,
    pub sampler_anisotropy: bool,
    /// `swapchainMaintenance1`: the application may hand the OS a fence so a
    /// destroyed swapchain is not reused while it is still scanned out, and
    /// may ask for a specific present scaling behaviour.
    ///
    /// True only when the extension *and* the feature are enabled — the
    /// extension string alone says nothing about the feature.
    pub swapchain_maintenance1: bool,
    pub present_wait: bool,
    /// `presentId`: the application may tag a presentation with an id.
    ///
    /// Required by `VK_KHR_present_wait`, which the low-latency present path
    /// uses to wait for a specific presentation.
    pub present_id: bool,
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
        if !self.draw_indirect_first_instance {
            missing.push("drawIndirectFirstInstance");
        }
        if !self.multi_draw_indirect {
            missing.push("multiDrawIndirect");
        }
        if !self.draw_indirect_count {
            missing.push("drawIndirectCount");
        }
        if !self.shader_demote_to_helper_invocation {
            missing.push("shaderDemoteToHelperInvocation");
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
    /// A caller supplied an argument the renderer cannot accept.
    InvalidArgument(String),
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
            Self::InvalidArgument(e) => write!(f, "invalid argument: {e}"),
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

/// A shared handle on the Vulkan device, so that objects which outlive the
/// [`Gpu`] can still free themselves.
///
/// Vulkan requires every child object to be destroyed before the device. The
/// atlases are handed to the application as an `Arc` and may well outlive the
/// renderer that owns the `Gpu` — the showcase keeps one for the whole
/// session. Giving them a clone of the device handle lets them free their
/// images whenever the last reference goes away, instead of leaking them or
/// touching a device that has already been destroyed.
#[derive(Clone)]
pub struct DeviceLifetime {
    device: ash::Device,
    /// Set to false when the owning [`Gpu`] tears the device down.
    ///
    /// The atlas checks this before freeing: after `Gpu::destroy` the device
    /// handle is stale, and Vulkan forbids using it.
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl DeviceLifetime {
    /// The device handle.
    #[must_use]
    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    /// True while the device is still alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Mark the device as destroyed. Called by `Gpu::destroy`.
    fn invalidate(&self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

impl core::fmt::Debug for DeviceLifetime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceLifetime")
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

/// A `VkBuffer` plus its bound memory and (optionally) a persistent mapping.
#[derive(Debug)]
pub struct Buffer {
    pub raw: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: u64,
    /// The byte size of the bound `VkDeviceMemory` allocation.
    ///
    /// The allocator rounds the request up, so this is not always `size`; the
    /// mapped-range bounds check has to use the allocation, not the buffer.
    pub allocation_size: u64,
    /// Device address (0 when `bufferDeviceAddress` was not requested).
    pub address: u64,
    /// Non-null when the memory is host visible and permanently mapped.
    pub mapped: *mut u8,
    pub host_visible: bool,
    /// True when the bound memory type carries `HOST_COHERENT`.
    ///
    /// A flush is a no-op there, so the hot path skips the driver call.
    pub host_coherent: bool,
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
    /// The instance extensions that were actually enabled.
    ///
    /// The list is resolved, not the requested one: the surface-maintenance
    /// dependency chain adds extensions the application never asked for.
    pub enabled_instance_extensions: Vec<String>,
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
    /// A shared handle on the device, handed to the objects that may outlive
    /// the `Gpu` (the atlases).
    pub lifetime: DeviceLifetime,
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

fn push_unique(extensions: &mut Vec<String>, name: &str) {
    if !extensions.iter().any(|e| e == name) {
        extensions.push(name.to_string());
    }
}

/// Resolve the *instance*-extension dependencies of the requested device
/// extensions.
///
/// Two chains have their root at `VK_KHR_surface`:
///
/// ```text
/// VK_KHR_swapchain              -> VK_KHR_surface (instance)
/// VK_EXT_swapchain_maintenance1 -> VK_EXT_surface_maintenance1 (instance)
///                              -> VK_KHR_get_surface_capabilities2 (instance)
///                              -> VK_KHR_surface (instance)
/// ```
///
/// Enabling a device extension without its instance dependencies makes
/// `vkCreateDevice` invalid usage
/// (`VUID-vkCreateDevice-ppEnabledExtensionNames-01387`); enabling an
/// instance extension without its own dependencies makes `vkCreateInstance`
/// invalid (`VUID-vkCreateInstance-ppEnabledExtensionNames-01388`). The
/// instance has to be created before the device, so both chains are resolved
/// here, from the request, instead of trusting every caller to know them.
///
/// The platform counterpart (`VK_KHR_win32_surface` on Windows, …) cannot be
/// inferred from a device extension: it comes from the window's display handle
/// through [`crate::surface_extensions::required_instance_extensions`].
fn resolve_instance_extension_dependencies(
    instance_exts: &mut Vec<String>,
    device_exts: &[String],
) {
    let wants_swapchain = device_exts.iter().any(|e| e == "VK_KHR_swapchain");
    let wants_maintenance = device_exts
        .iter()
        .any(|e| e == "VK_EXT_swapchain_maintenance1");
    let has_surface_query = instance_exts
        .iter()
        .any(|e| e == "VK_KHR_get_surface_capabilities2" || e == "VK_EXT_surface_maintenance1");
    if !wants_swapchain && !wants_maintenance && !has_surface_query {
        return;
    }
    // The root of both chains. The surface module expects it to lead the list.
    if !instance_exts.iter().any(|e| e == "VK_KHR_surface") {
        instance_exts.insert(0, "VK_KHR_surface".to_string());
    }
    if wants_maintenance || has_surface_query {
        push_unique(instance_exts, "VK_KHR_get_surface_capabilities2");
    }
    if wants_maintenance {
        push_unique(instance_exts, "VK_EXT_surface_maintenance1");
    }
}

/// Device extensions the renderer can run without.
///
/// They only improve presentation (frame pacing, tearing control, present
/// fences), so a missing one is a warning, not a failure.
#[must_use]
pub fn is_optional_device_extension(name: &str) -> bool {
    matches!(
        name,
        "VK_EXT_swapchain_maintenance1" | "VK_KHR_present_wait" | "VK_KHR_present_id"
    )
}

/// The device extensions a physical device exposes.
fn device_extension_names(instance: &ash::Instance, device: vk::PhysicalDevice) -> Vec<String> {
    // SAFETY: `instance` is alive and `device` was enumerated from it.
    let properties =
        unsafe { instance.enumerate_device_extension_properties(device) }.unwrap_or_default();
    properties
        .iter()
        .filter_map(|e| {
            e.extension_name_as_c_str()
                .ok()
                .map(|c| c.to_string_lossy().into_owned())
        })
        .collect()
}

fn cstrings(list: &[String]) -> Vec<std::ffi::CString> {
    list.iter()
        .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
        .collect()
}

fn raw_ptrs(strings: &[std::ffi::CString]) -> Vec<*const c_char> {
    strings.iter().map(|c| c.as_ptr()).collect()
}

/// Turn a requested `offset..offset+len` into a range that satisfies
/// `VUID-VkMappedMemoryRange-size-00685`.
///
/// Returns `(offset, size)` where `offset` is atom aligned, `size` is either
/// atom aligned or ends exactly at `allocation`, and `offset + size` never
/// exceeds the allocation. `None` means there is nothing to flush.
#[must_use]
pub fn align_mapped_range(offset: u64, len: u64, atom: u64, allocation: u64) -> Option<(u64, u64)> {
    if len == 0 || allocation == 0 {
        return None;
    }
    let atom = atom.max(1);
    let aligned_offset = offset & !(atom - 1);
    if aligned_offset >= allocation {
        return None;
    }
    // The *unrounded* end, so a misaligned start does not lose its tail.
    let end = offset.saturating_add(len).min(allocation);
    if end <= aligned_offset {
        return None;
    }
    let aligned_end = end.div_ceil(atom) * atom;
    let size = if aligned_end <= allocation {
        aligned_end - aligned_offset
    } else {
        allocation - aligned_offset
    };
    (size > 0).then_some((aligned_offset, size))
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
        resolve_instance_extension_dependencies(&mut instance_exts, &config.device_extensions);
        let ext_cstrings = cstrings(&instance_exts);
        let ext_ptrs = raw_ptrs(&ext_cstrings);

        // The loader may support less than 1.3 (Windows 10 ships an older
        // loader). Asking for 1.3 unconditionally makes `vkCreateInstance`
        // fail; asking for what the loader knows keeps the app running and the
        // device selection below takes care of the 1.2 path.
        let max_api = entry
            .try_enumerate_instance_version()
            .ok()
            .flatten()
            .unwrap_or(vk::make_api_version(0, 1, 0, 0));
        let api_version = max_api.min(vk::make_api_version(0, 1, 3, 0));
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"unbound-ui")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"unbound")
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(api_version);

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
        //
        // A candidate has to be probed *before* it is chosen. Windows 10
        // drivers commonly expose Vulkan 1.2 only: the 1.3 core features the
        // renderer uses are also available as `VK_KHR_dynamic_rendering` and
        // `VK_KHR_synchronization2`, so a 1.2 device is still usable — but
        // only if those two extensions are present. Selecting by device type
        // alone used to pick a 1.2 GPU and fail at `vkCreateDevice`.
        let devices = instance
            .enumerate_physical_devices()
            .map_err(GpuError::Vulkan)?;
        let mut best: Option<(
            vk::PhysicalDevice,
            i32,
            vk::PhysicalDeviceProperties,
            Vec<String>,
        )> = None;
        let mut rejected: Vec<String> = Vec::new();
        for pd in devices {
            let props = instance.get_physical_device_properties(pd);
            let name = props
                .device_name_as_c_str()
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "unnamed".into());
            let ext_names = device_extension_names(&instance, pd);
            let missing: Vec<&str> = config
                .device_extensions
                .iter()
                .filter(|extension| !is_optional_device_extension(extension))
                .map(String::as_str)
                .filter(|extension| !ext_names.iter().any(|n| n == extension))
                .collect();
            if !missing.is_empty() {
                rejected.push(format!("{name}: no {}", missing.join(", ")));
                continue;
            }
            let has_12_extensions = ext_names.iter().any(|n| n == "VK_KHR_dynamic_rendering")
                && ext_names.iter().any(|n| n == "VK_KHR_synchronization2");
            if props.api_version < vk::make_api_version(0, 1, 3, 0) && !has_12_extensions {
                rejected.push(format!("{name}: Vulkan 1.2 without the 1.3 back-ports"));
                continue;
            }
            let mut score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
                _ => 100,
            };
            // Prefer a driver that speaks 1.3 natively, then one that also
            // exposes the optional present extensions.
            if props.api_version >= vk::make_api_version(0, 1, 3, 0) {
                score += 400;
            }
            score += config
                .device_extensions
                .iter()
                .filter(|extension| is_optional_device_extension(extension))
                .filter(|extension| ext_names.iter().any(|n| *n == **extension))
                .count() as i32
                * 10;
            if config.prefer_discrete && props.device_type != vk::PhysicalDeviceType::DISCRETE_GPU {
                score -= 100;
            }
            if best.as_ref().map(|(_, s, _, _)| score > *s).unwrap_or(true) {
                best = Some((pd, score, props, ext_names));
            }
        }
        let Some((physical, _, properties, dev_ext_names)) = best else {
            return Err(GpuError::NoSuitableDevice(if rejected.is_empty() {
                "no physical device".into()
            } else {
                rejected.join("; ")
            }));
        };

        // ---- extension dependencies ----------------------------------------
        //
        // Vulkan requires the dependencies of an enabled extension to be
        // enabled too, and a device may simply not offer an optional one. The
        // old code enabled every requested string unconditionally, so a
        // driver without `VK_KHR_present_wait` made `vkCreateDevice` fail with
        // `VK_ERROR_EXTENSION_NOT_PRESENT` — an error the application could
        // not diagnose.
        //
        //   VK_EXT_swapchain_maintenance1 -> VK_EXT_surface_maintenance1
        //                                       (an *instance* extension)
        //   VK_KHR_present_wait            -> VK_KHR_present_id
        //                                       (a *device* extension)
        let has_ext = |name: &str| dev_ext_names.iter().any(|n| n == name);
        let mut device_exts: Vec<String> = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();
        for extension in &config.device_extensions {
            if has_ext(extension) {
                device_exts.push(extension.clone());
            } else if is_optional_device_extension(extension) {
                skipped.push(extension.as_str());
            } else {
                // The selection loop already rejected devices without the
                // required extensions, so this is only a safety net.
                return Err(GpuError::UnsupportedFeatures(vec!["device extension"]));
            }
        }
        for extension in skipped {
            eprintln!("GPU: {extension} is not supported by this driver; continuing without it");
        }
        // A 1.2 driver exposes the Vulkan 1.3 features as back-port extensions.
        let needs_13_backports = properties.api_version < vk::make_api_version(0, 1, 3, 0);
        if needs_13_backports {
            for extension in ["VK_KHR_dynamic_rendering", "VK_KHR_synchronization2"] {
                push_unique(&mut device_exts, extension);
            }
            // `discard` compiles to `OpDemoteToHelperInvocation`; on a 1.2
            // driver the feature comes from this extension rather than core
            // 1.3. Without it the shader module is invalid usage.
            if has_ext("VK_EXT_shader_demote_to_helper_invocation") {
                push_unique(
                    &mut device_exts,
                    "VK_EXT_shader_demote_to_helper_invocation",
                );
            }
        }
        if device_exts.iter().any(|e| e == "VK_KHR_present_wait")
            && !device_exts.iter().any(|e| e == "VK_KHR_present_id")
        {
            device_exts.push("VK_KHR_present_id".to_string());
        }
        // The extension has to be *requested* to be enabled: a driver that
        // merely exposes it does not enable the feature.
        let maintenance1_requested = device_exts
            .iter()
            .any(|e| e == "VK_EXT_swapchain_maintenance1");

        // ---- feature discovery -------------------------------------------
        let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
        // Only one of these is chained: `VkPhysicalDeviceVulkan13Features`
        // is invalid below API 1.3, and querying it there is undefined.
        let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut f_dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default();
        let mut f_synchronization2 = vk::PhysicalDeviceSynchronization2Features::default();
        // The present features live in their own structures, so they have to
        // be chained into the query as well. An unknown structure type in the
        // chain makes the query fail, hence the extension checks.
        let mut present_wait_query = vk::PhysicalDevicePresentWaitFeaturesKHR::default();
        let mut present_id_query = vk::PhysicalDevicePresentIdFeaturesKHR::default();
        let mut maintenance1_query = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default();
        let mut demote_query =
            vk::PhysicalDeviceShaderDemoteToHelperInvocationFeaturesEXT::default();
        let core;
        {
            let mut query = vk::PhysicalDeviceFeatures2::default().push_next(&mut f12);
            if needs_13_backports {
                query = query
                    .push_next(&mut f_dynamic_rendering)
                    .push_next(&mut f_synchronization2);
                if has_ext("VK_EXT_shader_demote_to_helper_invocation") {
                    query = query.push_next(&mut demote_query);
                }
            } else {
                query = query.push_next(&mut f13);
            }
            if has_ext("VK_KHR_present_id") {
                query = query.push_next(&mut present_id_query);
            }
            if has_ext("VK_KHR_present_wait") {
                query = query.push_next(&mut present_wait_query);
            }
            if has_ext("VK_EXT_swapchain_maintenance1") {
                query = query.push_next(&mut maintenance1_query);
            }
            instance.get_physical_device_features2(physical, &mut query);
            core = query.features;
        }
        let dynamic_rendering = if needs_13_backports {
            f_dynamic_rendering.dynamic_rendering == vk::TRUE
        } else {
            f13.dynamic_rendering == vk::TRUE
        };
        let synchronization2 = if needs_13_backports {
            f_synchronization2.synchronization2 == vk::TRUE
        } else {
            f13.synchronization2 == vk::TRUE
        };

        let features = GpuFeatures {
            buffer_device_address: f12.buffer_device_address == vk::TRUE,
            descriptor_indexing: f12.descriptor_indexing == vk::TRUE,
            runtime_descriptor_array: f12.runtime_descriptor_array == vk::TRUE,
            partially_bound_descriptors: f12.descriptor_binding_partially_bound == vk::TRUE,
            update_after_bind: f12.descriptor_binding_sampled_image_update_after_bind == vk::TRUE,
            dynamic_rendering,
            synchronization2,
            timeline_semaphore: f12.timeline_semaphore == vk::TRUE,
            host_query_reset: f12.host_query_reset == vk::TRUE,
            multi_draw_indirect: core.multi_draw_indirect == vk::TRUE,
            draw_indirect_count: f12.draw_indirect_count == vk::TRUE,
            draw_indirect_first_instance: core.draw_indirect_first_instance == vk::TRUE,
            shader_int64: core.shader_int64 == vk::TRUE,
            shader_float64: core.shader_float64 == vk::TRUE,
            independent_blend: core.independent_blend == vk::TRUE,
            sampler_anisotropy: core.sampler_anisotropy == vk::TRUE,
            shader_demote_to_helper_invocation: if needs_13_backports {
                demote_query.shader_demote_to_helper_invocation == vk::TRUE
            } else {
                f13.shader_demote_to_helper_invocation == vk::TRUE
            },
            // The feature has to be *queried* and then enabled: the extension
            // string alone says nothing about the feature.
            swapchain_maintenance1: maintenance1_requested
                && maintenance1_query.swapchain_maintenance1 == vk::TRUE,
            present_wait: has_ext("VK_KHR_present_wait")
                && present_wait_query.present_wait == vk::TRUE,
            present_id: has_ext("VK_KHR_present_id") && present_id_query.present_id == vk::TRUE,
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
        let graphics = graphics
            .ok_or_else(|| GpuError::NoSuitableDevice("no graphics+compute queue family".into()))?;
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
            .synchronization2(features.synchronization2)
            .shader_demote_to_helper_invocation(features.shader_demote_to_helper_invocation);
        let mut enabled_dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default()
            .dynamic_rendering(features.dynamic_rendering);
        let mut enabled_synchronization2 = vk::PhysicalDeviceSynchronization2Features::default()
            .synchronization2(features.synchronization2);
        let enabled_core = vk::PhysicalDeviceFeatures::default()
            .multi_draw_indirect(features.multi_draw_indirect)
            // The MDI compaction emits one run per (layer, barrier, variant)
            // group, and every run but the first carries a non-zero
            // `firstInstance`. Without this feature the driver may ignore it
            // and redraw the first run's primitives instead.
            .draw_indirect_first_instance(features.draw_indirect_first_instance)
            .independent_blend(features.independent_blend)
            .sampler_anisotropy(features.sampler_anisotropy)
            .shader_int64(features.shader_int64)
            .shader_float64(features.shader_float64);

        let dev_ext_cstrings = cstrings(&device_exts);
        let dev_ext_ptrs = raw_ptrs(&dev_ext_cstrings);

        // `VK_KHR_present_wait` and `VK_KHR_present_id` are two separate
        // *features*. Without them `vkWaitForPresentKHR` and a non-zero
        // `VkPresentIdKHR` are invalid usage, even though the extensions are
        // enabled.
        let present_wait_enabled = device_exts.iter().any(|e| e == "VK_KHR_present_wait");
        let mut present_wait_features = vk::PhysicalDevicePresentWaitFeaturesKHR::default()
            .present_wait(present_wait_enabled && features.present_wait);
        let mut present_id_features = vk::PhysicalDevicePresentIdFeaturesKHR::default()
            .present_id(present_wait_enabled && features.present_id);

        // `VK_EXT_swapchain_maintenance1` has a feature of its own: the
        // present fence and the present scaling behaviour are only legal
        // when `swapchainMaintenance1` is enabled. The structure may only be
        // chained when the extension itself is enabled.
        let mut maintenance1_features =
            vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default()
                .swapchain_maintenance1(features.swapchain_maintenance1);
        // Vulkan 1.2 keeps the demote feature in its own extension structure;
        // chaining it on a 1.3 device would duplicate the core 1.3 feature.
        let mut demote_features =
            vk::PhysicalDeviceShaderDemoteToHelperInvocationFeaturesEXT::default()
                .shader_demote_to_helper_invocation(features.shader_demote_to_helper_invocation);

        let mut device_create = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&dev_ext_ptrs)
            .enabled_features(&enabled_core)
            .push_next(&mut present_id_features)
            .push_next(&mut present_wait_features)
            .push_next(&mut enabled_features12);
        if needs_13_backports {
            // Vulkan 1.2: the features come from the KHR extensions, and the
            // core 1.3 structure must not be chained.
            device_create = device_create
                .push_next(&mut enabled_dynamic_rendering)
                .push_next(&mut enabled_synchronization2);
            if features.shader_demote_to_helper_invocation {
                device_create = device_create.push_next(&mut demote_features);
            }
        } else {
            device_create = device_create.push_next(&mut enabled_features13);
        }
        if features.swapchain_maintenance1 {
            device_create = device_create.push_next(&mut maintenance1_features);
        }

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
            instance_exts,
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
        enabled_instance_extensions: Vec<String>,
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
            let in_flight = device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(GpuError::Vulkan)?;
            frames.push(FrameSync {
                in_flight,
                command_buffer: cmd,
                timeline_value: i as u64,
            });
        }

        let khr_dynamic_rendering = ash::khr::dynamic_rendering::Device::new(&instance, &device);
        let khr_synchronization2 = ash::khr::synchronization2::Device::new(&instance, &device);
        let khr_draw_indirect_count =
            ash::khr::draw_indirect_count::Device::new(&instance, &device);

        let lifetime = DeviceLifetime {
            device: device.clone(),
            alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };

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
            enabled_instance_extensions,
            khr_dynamic_rendering,
            khr_synchronization2,
            khr_draw_indirect_count,
            timeline,
            frames,
            command_pool,
            config,
            lifetime,
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
        choose_memory_type(self.memory_types(), u32::MAX, kind)
    }

    /// The memory types the device actually reports.
    fn memory_types(&self) -> &[vk::MemoryType] {
        &self.memory_properties.memory_types[..self.memory_properties.memory_type_count as usize]
    }

    /// Find a memory type that both matches `kind` **and** is allowed by the
    /// resource's `memoryTypeBits` requirement.
    ///
    /// Vulkan rejects `vkBindImageMemory` when the type index is not in the
    /// image's bit mask, and the "best" type by properties is frequently outside
    /// it (tiling constraints differ per format).
    #[must_use]
    pub fn memory_type_index_for_bits(&self, bits: u32, kind: MemoryKind) -> Option<u32> {
        choose_memory_type(self.memory_types(), bits, kind)
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

            let mem_flags = self.memory_properties.memory_types[type_index as usize].property_flags;
            let host_visible = mem_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
            let host_coherent = mem_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);
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
                allocation_size: requirements.size,
                address,
                mapped,
                host_visible,
                host_coherent,
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
        // The memory may not be host-coherent — the driver is free to fall
        // back to a non-coherent type when the preferred one is unavailable —
        // and the GPU would then read stale bytes. Flushing is a no-op for
        // coherent memory.
        self.flush(buffer, offset, bytes.len() as u64);
        true
    }

    /// Flush a mapped range (no-op for coherent memory, required otherwise).
    ///
    /// Vulkan's `VUID-VkMappedMemoryRange-size-00685` requires the range to
    /// stay inside the allocation, `offset` to be a multiple of
    /// `nonCoherentAtomSize`, and `size` to be either a multiple of that value
    /// or to reach exactly the end of the allocation.
    ///
    /// # Safety
    /// `offset..offset+len` must be inside a mapped buffer.
    pub unsafe fn flush(&self, buffer: &Buffer, offset: u64, len: u64) {
        if buffer.mapped.is_null() || buffer.host_coherent || len == 0 {
            return;
        }
        let atom = self.properties.limits.non_coherent_atom_size.max(1);
        let Some((aligned_offset, size)) =
            align_mapped_range(offset, len, atom, buffer.allocation_size)
        else {
            return;
        };
        let range = vk::MappedMemoryRange::default()
            .memory(buffer.memory)
            .offset(aligned_offset)
            .size(size);
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
            self.device
                .queue_submit(self.graphics_queue, &[submit], vk::Fence::null())?;
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
        // The objects that may outlive the `Gpu` (the atlases) must not touch
        // the device after this point, so the shared lifetime is invalidated
        // before the device handle goes away.
        self.lifetime.invalidate();
        // SAFETY: we own every handle and are consuming `self`.
        unsafe {
            let _ = self.device.device_wait_idle();
            for f in &self.frames {
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
    fn the_surface_maintenance_dependency_chain_is_complete() {
        let mut extensions = vec!["VK_KHR_surface".to_string()];
        resolve_instance_extension_dependencies(
            &mut extensions,
            &["VK_EXT_swapchain_maintenance1".to_string()],
        );
        assert!(extensions
            .iter()
            .any(|e| e == "VK_KHR_get_surface_capabilities2"));
        assert!(extensions
            .iter()
            .any(|e| e == "VK_EXT_surface_maintenance1"));

        // A second pass must not duplicate anything.
        let before = extensions.len();
        resolve_instance_extension_dependencies(
            &mut extensions,
            &["VK_EXT_swapchain_maintenance1".to_string()],
        );
        assert_eq!(extensions.len(), before);

        // Without any surface-related request the instance list stays untouched.
        let mut plain = Vec::new();
        resolve_instance_extension_dependencies(&mut plain, &[]);
        assert!(plain.is_empty());
    }

    /// `VUID-vkCreateDevice-ppEnabledExtensionNames-01387`: a device that
    /// enables `VK_KHR_swapchain` needs an instance with `VK_KHR_surface`.
    /// The windowed runner forgot to request the instance extensions at all,
    /// which turned the missing dependency into a validation error and then
    /// into an unresolved `vkCreateWin32SurfaceKHR`.
    #[test]
    fn the_swapchain_brings_its_instance_extensions() {
        let device = vec!["VK_KHR_swapchain".to_string()];

        let mut bare = Vec::new();
        resolve_instance_extension_dependencies(&mut bare, &device);
        assert_eq!(bare, vec!["VK_KHR_surface".to_string()]);

        // A caller that supplied the platform extension but forgot the root
        // gets the root inserted, exactly once.
        let mut platform_only = vec!["VK_KHR_win32_surface".to_string()];
        resolve_instance_extension_dependencies(&mut platform_only, &device);
        assert_eq!(
            platform_only,
            vec![
                "VK_KHR_surface".to_string(),
                "VK_KHR_win32_surface".to_string()
            ]
        );

        // The fully derived list is left alone.
        let mut complete = vec![
            "VK_KHR_surface".to_string(),
            "VK_KHR_win32_surface".to_string(),
        ];
        resolve_instance_extension_dependencies(&mut complete, &device);
        assert_eq!(complete.len(), 2);
    }

    #[test]
    fn a_surface_query_extension_pulls_in_its_dependencies() {
        let mut extensions = vec!["VK_EXT_surface_maintenance1".to_string()];
        resolve_instance_extension_dependencies(&mut extensions, &[]);
        assert_eq!(extensions[0], "VK_KHR_surface");
        assert!(extensions
            .iter()
            .any(|e| e == "VK_KHR_get_surface_capabilities2"));
    }

    #[test]
    fn required_feature_list_is_stable() {
        let none = GpuFeatures::default().missing_required();
        assert_eq!(none.len(), 9);
        assert!(none.contains(&"shaderDemoteToHelperInvocation"));
        let all = GpuFeatures {
            buffer_device_address: true,
            descriptor_indexing: true,
            dynamic_rendering: true,
            synchronization2: true,
            timeline_semaphore: true,
            multi_draw_indirect: true,
            draw_indirect_first_instance: true,
            draw_indirect_count: true,
            shader_demote_to_helper_invocation: true,
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
        assert!(
            gpu.features.shader_demote_to_helper_invocation,
            "the fragment shaders use OpDemoteToHelperInvocation"
        );
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
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
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

    /// The memory layout a discrete NVIDIA card reports: two plain VRAM types,
    /// a block of system memory, and — as type 5, at the end — the small window
    /// of VRAM that is also host visible (the Resizable BAR window).
    fn discrete_memory_types() -> Vec<vk::MemoryType> {
        use vk::MemoryPropertyFlags as Flags;
        let ty = |heap_index, property_flags| vk::MemoryType {
            heap_index,
            property_flags,
        };
        vec![
            ty(1, Flags::empty()),
            ty(0, Flags::DEVICE_LOCAL),
            ty(0, Flags::DEVICE_LOCAL),
            ty(1, Flags::HOST_VISIBLE | Flags::HOST_COHERENT),
            ty(
                1,
                Flags::HOST_VISIBLE | Flags::HOST_COHERENT | Flags::HOST_CACHED,
            ),
            ty(
                2,
                Flags::DEVICE_LOCAL | Flags::HOST_VISIBLE | Flags::HOST_COHERENT,
            ),
        ]
    }

    #[test]
    fn device_local_memory_stays_out_of_the_cpu_visible_window() {
        let types = discrete_memory_types();
        // Plain VRAM is type 1. Type 5 is VRAM too, but it is the cramped
        // window the CPU can map: putting the primitive pool, the sort keys or
        // an atlas image there caps them at the size of that window.
        assert_eq!(
            choose_memory_type(&types, u32::MAX, MemoryKind::DeviceLocal),
            Some(1)
        );
        // When the resource may only live in the window, take it anyway.
        assert_eq!(
            choose_memory_type(&types, 1 << 5, MemoryKind::DeviceLocal),
            Some(5)
        );
    }

    #[test]
    fn host_visible_device_local_memory_prefers_the_window() {
        let types = discrete_memory_types();
        assert_eq!(
            choose_memory_type(&types, u32::MAX, MemoryKind::HostVisibleDeviceLocal),
            Some(5)
        );
    }

    #[test]
    fn host_visible_memory_stays_in_system_ram() {
        let types = discrete_memory_types();
        assert_eq!(
            choose_memory_type(&types, u32::MAX, MemoryKind::HostVisible),
            Some(3)
        );
    }

    /// The reported range used to put the *end* offset into `size`, so the
    /// second frame of a 4 MiB ring flushed `2 MiB + 64 B` starting at `2 MiB`
    /// and the validation layer rejected it (`VUID-...-00685`).
    #[test]
    fn mapped_memory_flush_range_never_exceeds_allocation_bounds() {
        let allocation = 4_194_304;
        let atom = 64;
        for len in [64, 128, 4096] {
            let (offset, size) =
                align_mapped_range(2_097_152, len, atom, allocation).expect("a flushable range");
            assert_eq!(offset % atom, 0, "offset must be atom aligned");
            assert_eq!(size % atom, 0, "a rounded size must be atom aligned");
            assert!(
                offset + size <= allocation,
                "flush {offset}+{size} must stay inside {allocation}"
            );
            assert!(offset + size >= 2_097_152 + len, "the tail must be flushed");
        }
        // A misaligned start rounds down without losing the requested tail.
        let (offset, size) = align_mapped_range(100, 10, 64, 1024).expect("a flushable range");
        assert_eq!(offset, 64);
        assert!(offset + size >= 110);
        // A range that ends exactly at the allocation may stop unaligned.
        let (offset, size) = align_mapped_range(1000, 24, 64, 1024).expect("a flushable range");
        assert_eq!(offset, 960);
        assert_eq!(offset + size, 1024);
        // Degenerate ranges are ignored instead of producing an invalid one.
        assert_eq!(align_mapped_range(0, 0, atom, allocation), None);
        assert_eq!(align_mapped_range(allocation, 16, atom, allocation), None);
    }

    #[test]
    fn a_device_without_vram_still_gets_device_local_memory() {
        // Integrated parts report a single type that is both device local and
        // host visible; refusing it would leave nothing to allocate from.
        let types = vec![vk::MemoryType {
            heap_index: 0,
            property_flags: vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT,
        }];
        assert_eq!(
            choose_memory_type(&types, u32::MAX, MemoryKind::DeviceLocal),
            Some(0)
        );
    }

    #[test]
    fn the_real_device_keeps_device_local_out_of_the_cpu_visible_window() {
        let _guard = gpu_test_guard();
        let Some(gpu) = test_device() else {
            return;
        };
        let types = gpu.memory_types();
        let index = gpu
            .memory_type_index_for_bits(u32::MAX, MemoryKind::DeviceLocal)
            .expect("a device always offers device-local memory");
        let chosen = types[index as usize].property_flags;
        let plain_exists = types.iter().any(|ty| {
            ty.property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                && !ty
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        });
        assert!(
            !plain_exists || !chosen.contains(vk::MemoryPropertyFlags::HOST_VISIBLE),
            "device-local memory landed in the CPU-visible VRAM window (type {index}) \
             while a plain VRAM type exists"
        );
        gpu.destroy();
    }
}

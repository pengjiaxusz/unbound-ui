//! Vulkan 1.3 设备初始化。
//!
//! 渲染器所需的全部能力在此一次性请求并校验：
//!
//! * `bufferDeviceAddress` — Push Constant 直接携带设备物理地址（BDA）；
//! * `descriptorIndexing`（Bindless）— 全局单个描述符集管理所有纹理；
//! * `dynamicRendering` — 免去 `VkRenderPass`/`VkFramebuffer` 样板代码；
//! * `synchronization2` + `timelineSemaphore` — 队列族屏障与多帧完成度追踪；
//! * `multiDrawIndirect` + `drawIndirectCount` — GPU 生成并驱动间接绘制命令。
//!
//! 不满足上述能力的设备会被直接拒绝，而非静默退化到慢速路径。
//!
//! 本模块是 crate 中唯一不可避免包含大量 `unsafe` 的地方：它是对 Vulkan C API 的
//! 薄封装。每个 `unsafe` 块前均注明其安全性不变量，所有句柄归 [`Gpu`] 所有，
//! 并在 [`Gpu::destroy`] 中严格释放一次。
#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicU64, Ordering};

use ash::vk;

/// 缓冲区内存的分配来源策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// 高速显存（CPU 不可见），上传需经暂存缓冲区。
    DeviceLocal,
    /// CPU 可见且一致的内存，适用于频繁重写的小型数据。
    HostVisible,
    /// CPU 可见且优先设备本地的内存（ReBAR / Smart Access Memory）。
    HostVisibleDeviceLocal,
}

/// 从设备的 `memory_types` 中为 `kind` 挑选合适的内存类型。
///
/// `bits` 为资源的 `memoryTypeBits`：不在该掩码内的类型无论属性如何均不可用。
///
/// 优先级阶梯非常关键：独显通常报告三类堆——纯显存（`DEVICE_LOCAL`）、
/// 系统内存（`HOST_VISIBLE`）以及 CPU 可映射的小块显存窗口（NVIDIA 上约两百 MB，
/// 同时具备设备本地与宿主可见属性）。若将 [`MemoryKind::DeviceLocal`] 优先分配到
/// 该窗口，会把所有内存池、排序键缓冲区和图集限制在该窗口大小内，导致显存明明充足
/// 却报 `ERROR_OUT_OF_DEVICE_MEMORY`。因此纯 `DEVICE_LOCAL` 始终优先，仅在无其他
/// 选择时才回退到可映射显存窗口。
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
        // 高速纯显存，不在宿主端映射。
        MemoryKind::DeviceLocal => {
            find(&|flags| flags.contains(local) && !flags.contains(coherent))
                .or_else(|| find(&|flags| flags.contains(local)))
        }
        // ReBAR / SAM：CPU 可直接写入的显存。
        MemoryKind::HostVisibleDeviceLocal => {
            find(&|flags| flags.contains(local) && flags.contains(coherent))
                .or_else(|| find(&|flags| flags.contains(coherent)))
        }
        // 暂存区：优先系统内存，避免占用有限的 ReBAR 显存映射窗口。
        MemoryKind::HostVisible => {
            find(&|flags| flags.contains(coherent) && !flags.contains(local))
                .or_else(|| find(&|flags| flags.contains(coherent)))
        }
    }
}

/// 渲染器与驱动协商启用的全部能力快照。
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
    /// `drawIndirectFirstInstance`：允许间接绘制命令使用非零 `firstInstance`。
    ///
    /// 若未启用，驱动可忽略 `firstInstance`，导致首个批次之后的每个绘制批次
    /// 都重复绘制第一批的图元。MDI 压缩为每个 `(layer, barrier, variant)` 组
    /// 生成独立批次，因此该特性为硬性必需。
    pub draw_indirect_first_instance: bool,
    pub shader_int64: bool,
    pub shader_float64: bool,
    pub independent_blend: bool,
    /// `shaderDemoteToHelperInvocation`：在 `--target-env=vulkan1.3` 下，
    /// 片元着色器中的 `discard` 会编译为 `OpDemoteToHelperInvocation`。
    /// 未启用该特性就创建此类模块属于非法用法
    /// （`VUID-VkShaderModuleCreateInfo-pCode-08740`）。
    ///
    /// Vulkan 1.2 设备上该特性来自
    /// `VK_EXT_shader_demote_to_helper_invocation`，而非 1.3 核心。
    pub shader_demote_to_helper_invocation: bool,
    /// `hostQueryReset`：允许宿主端在帧间直接重置查询池。
    pub host_query_reset: bool,
    pub sampler_anisotropy: bool,
    /// `swapchainMaintenance1`：允许向操作系统提交呈现围栏并指定呈现缩放行为。
    ///
    /// 仅当扩展与特性同时启用时为 true——仅有扩展名并不代表特性已开启。
    pub swapchain_maintenance1: bool,
    pub present_wait: bool,
    /// `presentId`：允许为每次呈现标记递增 ID。
    ///
    /// 低延迟呈现路径 `VK_KHR_present_wait` 依赖此特性。
    pub present_id: bool,
    /// 当持久化池位于 ReBAR 宿主可见显存时为 true。
    pub rebar: bool,
}

impl GpuFeatures {
    /// 架构正常运行不可或缺的核心特性集合。
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

/// Vulkan 初始化与资源创建阶段的详细诊断快照。
///
/// 当 Windows 10 旧驱动、缺失 Vulkan Runtime 或特性不满足导致初始化失败时，
/// 该结构记录精确的失败阶段、加载器版本、实例/设备扩展协商结果与候选物理设备详情，
/// 供 [`GpuError::diagnostic_report`] 输出完整的定位信息。
#[derive(Clone, Debug, Default)]
pub struct GpuInitDiagnostics {
    /// 当前执行或失败所处的阶段名称（如 `"vkCreateInstance"`、`"vkCreateDevice"`）。
    pub stage: &'static str,
    /// Vulkan 加载器报告的最高实例 API 版本 `(major, minor, patch)`。
    pub loader_api_version: Option<(u32, u32, u32)>,
    /// 实际请求创建的实例 API 版本 `(major, minor, patch)`。
    pub requested_api_version: Option<(u32, u32, u32)>,
    /// 调用方与依赖链推导出的目标实例扩展。
    pub requested_instance_extensions: Vec<String>,
    /// 经 `vkEnumerateInstanceExtensionProperties` 过滤后实际启用的实例扩展。
    pub enabled_instance_extensions: Vec<String>,
    /// 因加载器或驱动不支持而被安全跳过的可选实例扩展。
    pub skipped_instance_extensions: Vec<String>,
    /// 缺失的必需实例扩展。
    pub missing_required_instance_extensions: Vec<String>,
    /// 选中的物理设备名称。
    pub selected_device_name: Option<String>,
    /// 选中的物理设备类型（如 `DISCRETE_GPU`、`INTEGRATED_GPU`）。
    pub selected_device_type: Option<String>,
    /// 选中的物理设备 API 版本 `(major, minor, patch)`。
    pub selected_device_api_version: Option<(u32, u32, u32)>,
    /// 选中的物理设备驱动原始版本号。
    pub selected_device_driver_version: Option<u32>,
    /// 选中的物理设备厂商 ID 与设备 ID `(vendor_id, device_id)`。
    pub selected_device_ids: Option<(u32, u32)>,
    /// 调用方请求的设备扩展。
    pub requested_device_extensions: Vec<String>,
    /// 最终协商启用的设备扩展。
    pub enabled_device_extensions: Vec<String>,
    /// 因驱动或实例不支持而被安全跳过的可选设备扩展（含原因说明）。
    pub skipped_device_extensions: Vec<String>,
    /// 被淘汰的候选设备及其详细原因。
    pub rejected_devices: Vec<String>,
    /// 缺失的必需硬件特性。
    pub missing_required_features: Vec<&'static str>,
}

static LAST_INIT_DIAGNOSTICS: std::sync::Mutex<Option<GpuInitDiagnostics>> =
    std::sync::Mutex::new(None);

fn store_init_diagnostics(diag: &GpuInitDiagnostics) {
    if let Ok(mut guard) = LAST_INIT_DIAGNOSTICS.lock() {
        *guard = Some(diag.clone());
    }
}

/// 获取最近一次 Vulkan 设备初始化记录的诊断快照。
#[must_use]
pub fn last_init_diagnostics() -> Option<GpuInitDiagnostics> {
    LAST_INIT_DIAGNOSTICS
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

/// 将 `vk::Result` 转换为通俗易懂的故障原因说明。
#[must_use]
pub fn explain_vk_result(result: vk::Result) -> &'static str {
    match result {
        vk::Result::ERROR_EXTENSION_NOT_PRESENT => {
            "请求的 Vulkan 扩展在当前系统加载器（vulkan-1.dll）或显卡驱动中不存在（常见于 Windows 10 旧版显卡驱动）"
        }
        vk::Result::ERROR_FEATURE_NOT_PRESENT => {
            "显卡驱动不支持所请求的某项 Vulkan 硬件特性"
        }
        vk::Result::ERROR_INCOMPATIBLE_DRIVER => {
            "显卡驱动与请求的 Vulkan API 版本不兼容（需支持 Vulkan 1.2+ 的显卡驱动）"
        }
        vk::Result::ERROR_INITIALIZATION_FAILED => {
            "Vulkan 底层初始化失败（可能是驱动安装损坏、虚拟显卡环境受限或安全软件拦截）"
        }
        vk::Result::ERROR_LAYER_NOT_PRESENT => {
            "请求的 Vulkan 校验层（如 VK_LAYER_KHRONOS_validation）未安装"
        }
        vk::Result::ERROR_OUT_OF_HOST_MEMORY => "系统内存（RAM）不足，无法完成 Vulkan 分配",
        vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => "显卡显存（VRAM）不足，无法完成 Vulkan 分配",
        vk::Result::ERROR_DEVICE_LOST => {
            "GPU 设备已丢失（驱动重置/TDR 超时/显卡硬件断开）"
        }
        vk::Result::ERROR_SURFACE_LOST_KHR => "窗口渲染表面（VkSurfaceKHR）已失效",
        vk::Result::ERROR_NATIVE_WINDOW_IN_USE_KHR => {
            "原生窗口句柄（HWND）已被另一个未销毁的 VkSurfaceKHR 占用"
        }
        vk::Result::ERROR_OUT_OF_DATE_KHR => "交换链尺寸与窗口当前尺寸不一致，需重建交换链",
        _ => "底层 Vulkan API 调用返回非预期错误码",
    }
}

/// 设备创建与操作错误类型。
#[derive(Debug)]
pub enum GpuError {
    /// 缺少 Vulkan 加载器。
    Loader(String),
    /// `vkCreateInstance` 或 `vkCreateDevice` 调用失败。
    Vulkan(vk::Result),
    /// 没有满足要求的物理设备。
    NoSuitableDevice(String),
    /// 缺少必需的特性或扩展。
    UnsupportedFeatures(Vec<&'static str>),
    /// `VkBuffer` 或 `VkDeviceMemory` 分配失败。
    Allocation(String),
    /// 调用方传入了渲染器无法接受的非法参数。
    InvalidArgument(String),
    /// 设备已丢失（`VK_ERROR_DEVICE_LOST`）。
    DeviceLost,
}

impl GpuError {
    /// 生成包含失败阶段、驱动/扩展协商详情与排障建议的结构化诊断报告。
    #[must_use]
    pub fn diagnostic_report(&self) -> String {
        use core::fmt::Write as _;

        let mut out = String::new();
        let _ = writeln!(out, "=== Unbound UI Vulkan 诊断报告 ===");
        let _ = writeln!(out, "错误摘要: {self}");

        if let Self::Vulkan(res) = self {
            let _ = writeln!(
                out,
                "错误码解析: {res:?} (code = {}) — {}",
                res.as_raw(),
                explain_vk_result(*res)
            );
        }

        if let Some(diag) = last_init_diagnostics() {
            if !diag.stage.is_empty() {
                let _ = writeln!(out, "失败阶段: {}", diag.stage);
            }
            if let Some((maj, min, pat)) = diag.loader_api_version {
                let req = diag
                    .requested_api_version
                    .map(|(rmaj, rmin, rpat)| format!("{rmaj}.{rmin}.{rpat}"))
                    .unwrap_or_else(|| "unknown".into());
                let _ = writeln!(
                    out,
                    "Vulkan Loader 版本: {maj}.{min}.{pat} (请求实例版本: {req})"
                );
            }
            if !diag.requested_instance_extensions.is_empty() {
                let _ = writeln!(
                    out,
                    "请求的实例扩展: [{}]",
                    diag.requested_instance_extensions.join(", ")
                );
                let _ = writeln!(
                    out,
                    "启用的实例扩展: [{}]",
                    diag.enabled_instance_extensions.join(", ")
                );
            }
            if !diag.skipped_instance_extensions.is_empty() {
                let _ = writeln!(
                    out,
                    "已跳过的可选实例扩展: [{}]",
                    diag.skipped_instance_extensions.join(", ")
                );
            }
            if !diag.missing_required_instance_extensions.is_empty() {
                let _ = writeln!(
                    out,
                    "缺失的必需实例扩展: [{}]",
                    diag.missing_required_instance_extensions.join(", ")
                );
            }
            if let Some(name) = &diag.selected_device_name {
                let dev_type = diag.selected_device_type.as_deref().unwrap_or("UNKNOWN");
                let api = diag
                    .selected_device_api_version
                    .map(|(a, b, c)| format!("{a}.{b}.{c}"))
                    .unwrap_or_else(|| "unknown".into());
                let drv = diag
                    .selected_device_driver_version
                    .map(|d| format!("0x{d:08X}"))
                    .unwrap_or_else(|| "unknown".into());
                let ids = diag
                    .selected_device_ids
                    .map(|(v, d)| format!("vendor=0x{v:04X}, device=0x{d:04X}"))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "目标物理设备: {name} (type={dev_type}, api={api}, driver={drv}, {ids})"
                );
            }
            if !diag.requested_device_extensions.is_empty() {
                let _ = writeln!(
                    out,
                    "请求的设备扩展: [{}]",
                    diag.requested_device_extensions.join(", ")
                );
                let _ = writeln!(
                    out,
                    "启用的设备扩展: [{}]",
                    diag.enabled_device_extensions.join(", ")
                );
            }
            if !diag.skipped_device_extensions.is_empty() {
                let _ = writeln!(
                    out,
                    "已跳过的可选设备扩展: [{}]",
                    diag.skipped_device_extensions.join("; ")
                );
            }
            if !diag.missing_required_features.is_empty() {
                let _ = writeln!(
                    out,
                    "缺失的必需硬件特性: [{}]",
                    diag.missing_required_features.join(", ")
                );
            }
            if !diag.rejected_devices.is_empty() {
                let _ = writeln!(out, "被拒绝的候选设备:");
                for item in &diag.rejected_devices {
                    let _ = writeln!(out, "  - {item}");
                }
            }
        }

        let _ = writeln!(out, "排障建议:");
        match self {
            Self::Loader(_) => {
                let _ = writeln!(
                    out,
                    "  1. 请确认系统已安装显卡驱动以及 Vulkan Runtime (vulkan-1.dll)。"
                );
                let _ = writeln!(
                    out,
                    "  2. 可前往显卡厂商官网（NVIDIA / AMD / Intel）或 https://vulkan.lunarg.com 安装最新运行时。"
                );
            }
            Self::NoSuitableDevice(_) => {
                let _ = writeln!(
                    out,
                    "  1. 渲染器需要支持 Vulkan 1.3，或支持 Vulkan 1.2 且具备 VK_KHR_dynamic_rendering 与 VK_KHR_synchronization2 扩展的显卡驱动。"
                );
                let _ = writeln!(
                    out,
                    "  2. 在 Windows 10 机器上，请将独立显卡/核显驱动升级至支持 Vulkan 1.2+ 的较新版本。"
                );
            }
            Self::UnsupportedFeatures(missing) => {
                let _ = writeln!(
                    out,
                    "  1. 当前显卡驱动缺少关键渲染特性: {}。",
                    missing.join(", ")
                );
                let _ = writeln!(
                    out,
                    "  2. 请更新显卡驱动；若机器存在双显卡（核显+独显），请在 Windows 图形设置中将程序指定为“高性能”独立显卡运行。"
                );
            }
            Self::Vulkan(vk::Result::ERROR_EXTENSION_NOT_PRESENT) => {
                let _ = writeln!(
                    out,
                    "  1. 驱动或 Vulkan Loader 拒绝了某项扩展请求。请检查上方“缺失的必需实例扩展”或“启用的设备扩展”列表。"
                );
                let _ = writeln!(
                    out,
                    "  2. 在 Windows 10 旧驱动环境下，建议更新显卡驱动并检查是否存在过期的第三方 Vulkan 覆盖层（如旧版 RTSS / OBS Vulkan Hook）。"
                );
            }
            Self::Vulkan(_) => {
                let _ = writeln!(
                    out,
                    "  1. 请参考上方失败阶段与错误码解析排查驱动状态，必要时可设置环境变量 UNBOUND_VALIDATION=1 获取校验层详细输出。"
                );
            }
            Self::Allocation(_) => {
                let _ = writeln!(
                    out,
                    "  1. 显存或主机内存分配失败，请检查是否有其他大型 3D 程序占满 VRAM，或调低 GpuConfig::{{pool_capacity, ring_capacity}}。"
                );
            }
            Self::InvalidArgument(_) => {
                let _ = writeln!(
                    out,
                    "  1. 请检查调用渲染器 API 时传入的槽位索引或缓冲区参数。"
                );
            }
            Self::DeviceLost => {
                let _ = writeln!(
                    out,
                    "  1. GPU 设备发生重置（VK_ERROR_DEVICE_LOST），框架将尝试触发自动设备恢复流程。"
                );
            }
        }
        out
    }
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

/// [`Gpu::new`] 的初始化配置。
#[derive(Clone, Debug)]
pub struct GpuConfig {
    pub app_name: String,
    /// 若存在则启用 `VK_LAYER_KHRONOS_validation` 校验层。
    pub validation: bool,
    /// 额外的实例扩展（如平台表面扩展）。
    pub instance_extensions: Vec<String>,
    /// 额外的设备扩展（如 `VK_KHR_swapchain`）。
    pub device_extensions: Vec<String>,
    /// 最大在飞帧数：2 可压低输入延迟，3 可平滑帧间抖动。
    pub frames_in_flight: u32,
    /// 持久化池大小（字节）。
    pub pool_capacity: u64,
    /// 逐帧瞬态环形缓冲区大小（字节）。
    pub ring_capacity: u64,
    /// 为 true 时强制选用首个独立 GPU；否则按评分择优。
    pub prefer_discrete: bool,
    /// 开启校验层时同步启用 GPU 辅助校验。
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

/// Vulkan 设备的共享句柄，使生命周期长于 [`Gpu`] 的对象仍能安全释放自身。
///
/// Vulkan 要求所有子对象必须在设备之前销毁。图集以 `Arc` 形式交给应用层，
/// 可能比拥有 `Gpu` 的渲染器存活更久。共享该句柄可让图集在最后一个引用释放时
/// 自行销毁图像，或感知设备已被销毁而避免触碰悬空句柄。
#[derive(Clone)]
pub struct DeviceLifetime {
    device: ash::Device,
    /// 当所属 [`Gpu`] 销毁设备时置为 false。
    ///
    /// 图集在释放前检查该标志：在 `Gpu::destroy` 之后设备句柄已失效，严禁再使用。
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl DeviceLifetime {
    /// 底层 Vulkan 设备句柄。
    #[must_use]
    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    /// 当设备仍处于存活状态时为 true。
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    /// 标记设备已销毁（由 `Gpu::destroy` 调用）。
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

/// `VkBuffer` 及其绑定的设备内存与可选的持久映射指针。
#[derive(Debug)]
pub struct Buffer {
    pub raw: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: u64,
    /// 绑定的 `VkDeviceMemory` 实际分配字节数。
    ///
    /// 分配器可能会向上取整，故此值不一定等于 `size`；映射范围边界检查须以此为准。
    pub allocation_size: u64,
    /// 设备物理地址（未请求 `bufferDeviceAddress` 时为 0）。
    pub address: u64,
    /// 当内存宿主可见且已持久映射时非空。
    pub mapped: *mut u8,
    pub host_visible: bool,
    /// 当绑定的内存类型具备 `HOST_COHERENT` 属性时为 true。
    ///
    /// 一致性内存无需刷写，热路径可跳过对应驱动调用。
    pub host_coherent: bool,
}

// SAFETY: 内部指针为不透明 Vulkan 句柄；由唯一持有 `Gpu` 的渲染器负责外部同步。
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    /// 若缓冲区已映射，返回指向 `offset` 处的宿主指针。
    #[must_use]
    pub fn host_ptr(&self, offset: u64) -> Option<*mut u8> {
        if self.mapped.is_null() || offset > self.size {
            None
        } else {
            // SAFETY: 在已映射内存分配范围内的指针算术。
            Some(unsafe { self.mapped.add(offset as usize) })
        }
    }
}

/// 时间线信号量封装。
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

    /// 预留下一个待触发的时间线值。
    pub fn next(&self) -> u64 {
        self.value.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 记录 `value` 已完成。
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

/// 逐帧同步原语。
#[derive(Debug)]
pub struct FrameSync {
    pub in_flight: vk::Fence,
    pub command_buffer: vk::CommandBuffer,
    /// 标记本帧完成的时间线值。
    pub timeline_value: u64,
}

/// 队列族分配结果。
#[derive(Copy, Clone, Debug, Default)]
pub struct QueueFamilies {
    pub graphics: u32,
    pub transfer: u32,
    /// 当传输队列属于独立队列族（真实 DMA 引擎）时为 true。
    pub dedicated_transfer: bool,
}

/// 完整设备集合：实例、物理设备、逻辑设备、队列与逐帧同步原语。
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
    /// 实际启用的实例扩展列表。
    ///
    /// 这是解析依赖链并按系统实际支持情况过滤后的最终列表。
    pub enabled_instance_extensions: Vec<String>,
    /// 动态渲染函数表（`vkCmdBeginRendering`）。
    pub khr_dynamic_rendering: ash::khr::dynamic_rendering::Device,
    /// Synchronization2 函数表（`vkCmdPipelineBarrier2`）。
    pub khr_synchronization2: ash::khr::synchronization2::Device,
    /// 间接计数绘制函数表（`vkCmdDrawIndirectCount`）。
    pub khr_draw_indirect_count: ash::khr::draw_indirect_count::Device,
    pub timeline: Timeline,
    pub frames: Vec<FrameSync>,
    command_pool: vk::CommandPool,
    pub config: GpuConfig,
    /// 设备的共享句柄，传递给生命周期可能长于 `Gpu` 的对象（如图集）。
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

/// 解析所请求设备扩展对应的*实例*扩展依赖。
///
/// 两条依赖链都以 `VK_KHR_surface` 为根：
///
/// ```text
/// VK_KHR_swapchain              -> VK_KHR_surface（实例）
/// VK_EXT_swapchain_maintenance1 -> VK_EXT_surface_maintenance1（实例）
///                              -> VK_KHR_get_surface_capabilities2（实例）
///                              -> VK_KHR_surface（实例）
/// ```
///
/// 启用设备扩展却缺少其实例依赖会使 `vkCreateDevice` 成为非法用法
/// （`VUID-vkCreateDevice-ppEnabledExtensionNames-01387`）；启用实例扩展却缺少
/// 其自身依赖会使 `vkCreateInstance` 非法
/// （`VUID-vkCreateInstance-ppEnabledExtensionNames-01388`）。实例必须先于设备
/// 创建，因此两条链都在此依据请求推导，而非要求每个调用方自行记住。
///
/// 平台对应的扩展（Windows 上的 `VK_KHR_win32_surface` 等）无法从设备扩展反推，
/// 只能通过窗口显示句柄由
/// [`crate::surface_extensions::required_instance_extensions`] 得到。
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
    // 两条依赖链的共同根；surface 模块要求它位于列表首位。
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

/// 渲染器在缺失时仍可运行的可选实例扩展。
///
/// 在 Windows 10 旧版加载器或旧显卡驱动上，`VK_EXT_surface_maintenance1`、
/// `VK_KHR_get_surface_capabilities2` 或 `VK_EXT_debug_utils` 可能不存在；
/// 必须在调用 `vkCreateInstance` 前依据 `vkEnumerateInstanceExtensionProperties`
/// 过滤掉不支持的可选实例扩展，否则 `vkCreateInstance` 会直接因
/// `VK_ERROR_EXTENSION_NOT_PRESENT` 失败。
#[must_use]
pub fn is_optional_instance_extension(name: &str) -> bool {
    matches!(
        name,
        "VK_EXT_surface_maintenance1" | "VK_KHR_get_surface_capabilities2" | "VK_EXT_debug_utils"
    )
}

/// 实例扩展协商结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstanceExtensionResolution {
    /// 最终传给 `vkCreateInstance` 的实例扩展列表。
    pub enabled: Vec<String>,
    /// 因当前机器不支持而被安全跳过的可选实例扩展。
    pub skipped: Vec<String>,
    /// 当前机器缺失的必需实例扩展（非空时无法创建窗口表面）。
    pub missing_required: Vec<String>,
}

/// 根据加载器实际支持的实例扩展列表 `available` 过滤目标实例扩展 `desired`。
///
/// 保持依赖链合法性：若 `VK_KHR_get_surface_capabilities2` 不可用，则依赖它的
/// `VK_EXT_surface_maintenance1` 也会一并降级跳过。
#[must_use]
pub fn filter_instance_extensions(
    desired: &[String],
    available: &[String],
) -> InstanceExtensionResolution {
    let is_avail = |name: &str| available.iter().any(|a| a == name);
    let mut enabled = Vec::new();
    let mut skipped = Vec::new();
    let mut missing_required = Vec::new();

    for ext in desired {
        if is_avail(ext) {
            push_unique(&mut enabled, ext);
        } else if is_optional_instance_extension(ext) {
            push_unique(&mut skipped, ext);
        } else {
            push_unique(&mut missing_required, ext);
        }
    }

    // `VK_EXT_surface_maintenance1` 强依赖 `VK_KHR_get_surface_capabilities2`；
    // 若后者未启用，必须同步移除前者以避免触发 VUID-vkCreateInstance-ppEnabledExtensionNames-01388。
    if enabled.iter().any(|e| e == "VK_EXT_surface_maintenance1")
        && !enabled
            .iter()
            .any(|e| e == "VK_KHR_get_surface_capabilities2")
    {
        enabled.retain(|e| e != "VK_EXT_surface_maintenance1");
        push_unique(&mut skipped, "VK_EXT_surface_maintenance1");
    }

    InstanceExtensionResolution {
        enabled,
        skipped,
        missing_required,
    }
}

/// 渲染器在缺失时仍可运行的可选设备扩展。
///
/// 它们仅用于改善呈现体验（帧步调、撕裂控制、呈现围栏），缺失时不视为失败。
#[must_use]
pub fn is_optional_device_extension(name: &str) -> bool {
    matches!(
        name,
        "VK_EXT_swapchain_maintenance1" | "VK_KHR_present_wait" | "VK_KHR_present_id"
    )
}

/// Vulkan 1.2 向后移植的 1.3 核心特性扩展。
///
/// 在 Vulkan 1.3+ 驱动上这些特性已晋升为核心 `VkPhysicalDeviceVulkan13Features`；
/// 在 Vulkan 1.2 驱动上则作为必需扩展由 `needs_13_backports` 分支统一启用。
#[must_use]
pub fn is_13_backport_device_extension(name: &str) -> bool {
    matches!(
        name,
        "VK_KHR_dynamic_rendering"
            | "VK_KHR_synchronization2"
            | "VK_EXT_shader_demote_to_helper_invocation"
    )
}

/// 设备扩展协商结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceExtensionResolution {
    /// 最终启用并传给 `vkCreateDevice` 的设备扩展列表。
    pub enabled: Vec<String>,
    /// 被安全跳过的可选设备扩展及原因。
    pub skipped: Vec<String>,
    /// 缺失的必需设备扩展。
    pub missing_required: Vec<String>,
}

/// 结合设备自身暴露的扩展 `available_device_exts`、实例已启用的扩展 `enabled_instance_exts`
/// 与物理设备 API 版本，解析最终要启用的设备扩展集合。
///
/// 严格执行跨层依赖守卫：
/// 1. `VK_EXT_swapchain_maintenance1` 仅在设备支持该扩展**且**实例已启用
///    `VK_EXT_surface_maintenance1` + `VK_KHR_get_surface_capabilities2` 时启用；
/// 2. `VK_KHR_present_wait` 仅在设备同时支持 `VK_KHR_present_wait` 与 `VK_KHR_present_id` 时启用；
/// 3. Vulkan 1.3+ 设备通过 `VkPhysicalDeviceVulkan13Features` 开启核心特性，不重复挂载 1.2 KHR 向后移植扩展。
#[must_use]
pub fn resolve_device_extensions(
    requested_device_exts: &[String],
    available_device_exts: &[String],
    enabled_instance_exts: &[String],
    device_api_version: u32,
) -> DeviceExtensionResolution {
    let has_dev_ext = |name: &str| available_device_exts.iter().any(|n| n == name);
    let has_inst_ext = |name: &str| enabled_instance_exts.iter().any(|n| n == name);
    let needs_13_backports = device_api_version < vk::make_api_version(0, 1, 3, 0);

    let mut enabled = Vec::new();
    let mut skipped = Vec::new();
    let mut missing_required = Vec::new();

    for extension in requested_device_exts {
        let name = extension.as_str();
        if is_13_backport_device_extension(name) {
            if needs_13_backports {
                if has_dev_ext(name) {
                    push_unique(&mut enabled, name);
                } else {
                    push_unique(&mut missing_required, name);
                }
            }
            continue;
        }

        if name == "VK_EXT_swapchain_maintenance1" {
            let inst_ready = has_inst_ext("VK_EXT_surface_maintenance1")
                && has_inst_ext("VK_KHR_get_surface_capabilities2");
            if !has_dev_ext(name) {
                skipped.push(format!("{name} (driver does not expose extension)"));
            } else if !inst_ready {
                skipped.push(format!(
                    "{name} (instance lacks VK_EXT_surface_maintenance1)"
                ));
            } else {
                push_unique(&mut enabled, name);
            }
            continue;
        }

        if name == "VK_KHR_present_wait" {
            if !has_dev_ext("VK_KHR_present_wait") {
                skipped.push("VK_KHR_present_wait (driver does not expose extension)".to_string());
            } else if !has_dev_ext("VK_KHR_present_id") {
                skipped.push(
                    "VK_KHR_present_wait (driver lacks dependency VK_KHR_present_id)".to_string(),
                );
            } else {
                push_unique(&mut enabled, "VK_KHR_present_wait");
                push_unique(&mut enabled, "VK_KHR_present_id");
            }
            continue;
        }

        if name == "VK_KHR_present_id" {
            if has_dev_ext("VK_KHR_present_id") {
                push_unique(&mut enabled, "VK_KHR_present_id");
            } else {
                skipped.push("VK_KHR_present_id (driver does not expose extension)".to_string());
            }
            continue;
        }

        if has_dev_ext(name) {
            push_unique(&mut enabled, name);
        } else if is_optional_device_extension(name) {
            skipped.push(format!("{name} (not supported by driver)"));
        } else {
            push_unique(&mut missing_required, name);
        }
    }

    if needs_13_backports {
        for req_backport in ["VK_KHR_dynamic_rendering", "VK_KHR_synchronization2"] {
            if has_dev_ext(req_backport) {
                push_unique(&mut enabled, req_backport);
            } else {
                push_unique(&mut missing_required, req_backport);
            }
        }
        if has_dev_ext("VK_EXT_shader_demote_to_helper_invocation") {
            push_unique(&mut enabled, "VK_EXT_shader_demote_to_helper_invocation");
        }
    }

    DeviceExtensionResolution {
        enabled,
        skipped,
        missing_required,
    }
}

/// 查询 Vulkan 加载器暴露的所有实例扩展名称。
fn instance_extension_names(entry: &ash::Entry) -> Vec<String> {
    // SAFETY: `entry` 为有效加载的 Vulkan 函数入口。
    let properties =
        unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
    properties
        .iter()
        .filter_map(|e| {
            e.extension_name_as_c_str()
                .ok()
                .map(|c| c.to_string_lossy().into_owned())
        })
        .collect()
}

/// 查询物理设备暴露的所有设备扩展名称。
fn device_extension_names(instance: &ash::Instance, device: vk::PhysicalDevice) -> Vec<String> {
    // SAFETY: `instance` 存活且 `device` 由其枚举得到。
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

/// 将请求的 `offset..offset+len` 调整为满足 `VUID-VkMappedMemoryRange-size-00685` 的范围。
///
/// 返回 `(offset, size)`，其中 `offset` 按原子粒度对齐，`size` 要么对齐原子粒度，
/// 要么恰好到达 `allocation` 末尾；返回 `None` 表示无需刷写。
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
    // 记录未舍入的结束位置，避免起始地址向下对齐时丢失尾部字节。
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
    /// 创建 Vulkan 实例、挑选物理设备并启用渲染器所需的全部特性。
    ///
    /// # Errors
    /// 当加载器/实例/设备创建失败或 GPU 缺少必需特性时返回 [`GpuError`]。
    pub fn new(config: GpuConfig) -> Result<Self, GpuError> {
        // SAFETY: 下方均为标准 Vulkan 对象创建调用且逐一检查返回值；所有对象归 `Self` 所有。
        unsafe { Self::new_inner(config) }
    }

    unsafe fn new_inner(config: GpuConfig) -> Result<Self, GpuError> {
        let mut diag = GpuInitDiagnostics {
            stage: "Entry::load",
            requested_device_extensions: config.device_extensions.clone(),
            ..Default::default()
        };
        store_init_diagnostics(&diag);

        let entry = ash::Entry::load().map_err(|e| {
            let msg = format!("{e} (failed to load vulkan-1.dll / libvulkan)");
            store_init_diagnostics(&diag);
            GpuError::Loader(msg)
        })?;

        // ---- 加载器版本与实例扩展探测 ---------------------------------------
        let max_api = entry
            .try_enumerate_instance_version()
            .ok()
            .flatten()
            .unwrap_or(vk::make_api_version(0, 1, 0, 0));
        let api_version = max_api.min(vk::make_api_version(0, 1, 3, 0));
        diag.loader_api_version = Some((
            vk::api_version_major(max_api),
            vk::api_version_minor(max_api),
            vk::api_version_patch(max_api),
        ));
        diag.requested_api_version = Some((
            vk::api_version_major(api_version),
            vk::api_version_minor(api_version),
            vk::api_version_patch(api_version),
        ));

        let available_layers = entry
            .enumerate_instance_layer_properties()
            .unwrap_or_default();
        let validation_available = available_layers.iter().any(|l| {
            l.layer_name_as_c_str()
                .map(|n| n == VALIDATION_LAYER)
                .unwrap_or(false)
        });
        let want_validation = config.validation && validation_available;

        let mut desired_instance_exts = config.instance_extensions.clone();
        if want_validation {
            push_unique(&mut desired_instance_exts, "VK_EXT_debug_utils");
        }
        resolve_instance_extension_dependencies(
            &mut desired_instance_exts,
            &config.device_extensions,
        );
        diag.requested_instance_extensions = desired_instance_exts.clone();

        let available_instance_exts = instance_extension_names(&entry);
        let inst_resolution =
            filter_instance_extensions(&desired_instance_exts, &available_instance_exts);
        diag.enabled_instance_extensions = inst_resolution.enabled.clone();
        diag.skipped_instance_extensions = inst_resolution.skipped.clone();
        diag.missing_required_instance_extensions = inst_resolution.missing_required.clone();
        diag.stage = "vkCreateInstance";
        store_init_diagnostics(&diag);

        for skipped_ext in &inst_resolution.skipped {
            eprintln!(
                "[unbound-gpu] Instance extension {skipped_ext} is not supported by this system; continuing without it"
            );
        }

        if !inst_resolution.missing_required.is_empty() {
            return Err(GpuError::NoSuitableDevice(format!(
                "Vulkan instance lacks required surface extension(s): {} (available: [{}])",
                inst_resolution.missing_required.join(", "),
                available_instance_exts.join(", ")
            )));
        }

        let instance_exts = inst_resolution.enabled;
        let enable_validation =
            want_validation && instance_exts.iter().any(|e| e == "VK_EXT_debug_utils");
        let layer_names = if enable_validation {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };

        let ext_cstrings = cstrings(&instance_exts);
        let ext_ptrs = raw_ptrs(&ext_cstrings);

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
            .map_err(|err| {
                store_init_diagnostics(&diag);
                GpuError::Vulkan(err)
            })?;

        // ---- 调试信使 -------------------------------------------------------
        let debug = if enable_validation {
            let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
            match utils.create_debug_utils_messenger(&debug_messenger_info(), None) {
                Ok(messenger) => Some((utils, messenger)),
                Err(err) => {
                    eprintln!(
                        "[unbound-gpu] Could not create debug messenger ({err:?}); continuing without it"
                    );
                    None
                }
            }
        } else {
            None
        };

        // ---- 物理设备选择 ---------------------------------------------------
        //
        // 候选设备必须在选中前完整探测：Windows 10 驱动常仅暴露 Vulkan 1.2，
        // 若具备 `VK_KHR_dynamic_rendering` 与 `VK_KHR_synchronization2` 扩展，
        // 1.2 设备依然可用；仅按设备类型选择会导致选中不可用的 1.2 GPU。
        diag.stage = "vkEnumeratePhysicalDevices";
        store_init_diagnostics(&diag);
        let devices = match instance.enumerate_physical_devices() {
            Ok(list) => list,
            Err(err) => {
                if let Some((utils, messenger)) = debug {
                    utils.destroy_debug_utils_messenger(messenger, None);
                }
                instance.destroy_instance(None);
                store_init_diagnostics(&diag);
                return Err(GpuError::Vulkan(err));
            }
        };

        diag.stage = "PhysicalDeviceSelection";
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
            let dev_api = format!(
                "{}.{}.{}",
                vk::api_version_major(props.api_version),
                vk::api_version_minor(props.api_version),
                vk::api_version_patch(props.api_version)
            );
            let ext_names = device_extension_names(&instance, pd);
            let missing: Vec<&str> = config
                .device_extensions
                .iter()
                .filter(|extension| {
                    !is_optional_device_extension(extension)
                        && !is_13_backport_device_extension(extension)
                })
                .map(String::as_str)
                .filter(|extension| !ext_names.iter().any(|n| n == extension))
                .collect();
            if !missing.is_empty() {
                rejected.push(format!(
                    "{name} (api={dev_api}, type={:?}): missing required device extension(s) [{}]",
                    props.device_type,
                    missing.join(", ")
                ));
                continue;
            }
            if props.api_version < vk::make_api_version(0, 1, 2, 0) {
                rejected.push(format!(
                    "{name} (api={dev_api}, type={:?}): Vulkan API < 1.2 is not supported",
                    props.device_type
                ));
                continue;
            }
            let has_12_extensions = ext_names.iter().any(|n| n == "VK_KHR_dynamic_rendering")
                && ext_names.iter().any(|n| n == "VK_KHR_synchronization2");
            if props.api_version < vk::make_api_version(0, 1, 3, 0) && !has_12_extensions {
                rejected.push(format!(
                    "{name} (api={dev_api}, type={:?}): Vulkan 1.2 driver without VK_KHR_dynamic_rendering / VK_KHR_synchronization2",
                    props.device_type
                ));
                continue;
            }
            let mut score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
                _ => 100,
            };
            // 优先选择原生支持 Vulkan 1.3 的驱动，其次优先支持可选呈现扩展的设备。
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
        diag.rejected_devices = rejected.clone();
        let Some((physical, _, properties, dev_ext_names)) = best else {
            if let Some((utils, messenger)) = debug {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            instance.destroy_instance(None);
            store_init_diagnostics(&diag);
            return Err(GpuError::NoSuitableDevice(if rejected.is_empty() {
                "no Vulkan physical device found on this system".into()
            } else {
                rejected.join("; ")
            }));
        };

        let selected_name = properties
            .device_name_as_c_str()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "<unnamed>".into());
        diag.selected_device_name = Some(selected_name);
        diag.selected_device_type = Some(format!("{:?}", properties.device_type));
        diag.selected_device_api_version = Some((
            vk::api_version_major(properties.api_version),
            vk::api_version_minor(properties.api_version),
            vk::api_version_patch(properties.api_version),
        ));
        diag.selected_device_driver_version = Some(properties.driver_version);
        diag.selected_device_ids = Some((properties.vendor_id, properties.device_id));

        // ---- 扩展依赖解析 ---------------------------------------------------
        let dev_resolution = resolve_device_extensions(
            &config.device_extensions,
            &dev_ext_names,
            &instance_exts,
            properties.api_version,
        );
        let mut device_exts = dev_resolution.enabled;
        diag.enabled_device_extensions = device_exts.clone();
        diag.skipped_device_extensions = dev_resolution.skipped.clone();
        store_init_diagnostics(&diag);

        for skipped_reason in &dev_resolution.skipped {
            eprintln!("[unbound-gpu] Optional device extension skipped: {skipped_reason}");
        }
        if !dev_resolution.missing_required.is_empty() {
            if let Some((utils, messenger)) = debug {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            instance.destroy_instance(None);
            return Err(GpuError::UnsupportedFeatures(vec!["device extension"]));
        }

        let needs_13_backports = properties.api_version < vk::make_api_version(0, 1, 3, 0);
        let has_enabled_dev_ext = |name: &str| device_exts.iter().any(|e| e == name);

        // ---- 特性探测 -------------------------------------------------------
        diag.stage = "FeatureVerification";
        let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
        // 两者仅挂载其一：API 1.3 以下挂载 `VkPhysicalDeviceVulkan13Features` 属于未定义行为。
        let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut f_dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default();
        let mut f_synchronization2 = vk::PhysicalDeviceSynchronization2Features::default();
        // 呈现相关特性位于独立结构体中，仅在对应扩展已通过跨层校验后再链入查询。
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
                if has_enabled_dev_ext("VK_EXT_shader_demote_to_helper_invocation") {
                    query = query.push_next(&mut demote_query);
                }
            } else {
                query = query.push_next(&mut f13);
            }
            if has_enabled_dev_ext("VK_KHR_present_id") {
                query = query.push_next(&mut present_id_query);
            }
            if has_enabled_dev_ext("VK_KHR_present_wait") {
                query = query.push_next(&mut present_wait_query);
            }
            if has_enabled_dev_ext("VK_EXT_swapchain_maintenance1") {
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

        // 仅当 `VK_KHR_present_wait` 与 `VK_KHR_present_id` 的扩展与特性位同时为 TRUE 时才启用，
        // 否则从启用扩展列表中剔除，防止向 `vkCreateDevice` 传入未启用特性的扩展。
        let present_wait_supported = has_enabled_dev_ext("VK_KHR_present_wait")
            && has_enabled_dev_ext("VK_KHR_present_id")
            && present_wait_query.present_wait == vk::TRUE
            && present_id_query.present_id == vk::TRUE;
        // 先完成所有基于 `device_exts` 的只读判定，再执行 `retain` 剔除：
        // 闭包 `has_enabled_dev_ext` 持有不可变借用，需在其最后一次调用结束后才能可变借用。
        let swapchain_maintenance1_supported = has_enabled_dev_ext("VK_EXT_swapchain_maintenance1")
            && maintenance1_query.swapchain_maintenance1 == vk::TRUE;
        if !present_wait_supported {
            device_exts.retain(|e| e != "VK_KHR_present_wait" && e != "VK_KHR_present_id");
        }

        if !swapchain_maintenance1_supported {
            device_exts.retain(|e| e != "VK_EXT_swapchain_maintenance1");
        }
        diag.enabled_device_extensions = device_exts.clone();

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
            swapchain_maintenance1: swapchain_maintenance1_supported,
            present_wait: present_wait_supported,
            present_id: present_wait_supported,
            rebar: false,
        };

        let missing = features.missing_required();
        if !missing.is_empty() {
            diag.missing_required_features = missing.clone();
            store_init_diagnostics(&diag);
            if let Some((utils, messenger)) = debug {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            instance.destroy_instance(None);
            return Err(GpuError::UnsupportedFeatures(missing));
        }

        // ---- 队列族选择 -----------------------------------------------------
        diag.stage = "QueueFamilySelection";
        store_init_diagnostics(&diag);
        let families_props = instance.get_physical_device_queue_family_properties(physical);
        let mut graphics = None;
        let mut transfer = None;
        for (i, p) in families_props.iter().enumerate() {
            let f = p.queue_flags;
            if f.contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE) {
                graphics.get_or_insert(i as u32);
            }
            // 优先选择仅支持传输能力的队列族：即专用 DMA 引擎。
            if f.contains(vk::QueueFlags::TRANSFER) && !f.contains(vk::QueueFlags::GRAPHICS) {
                transfer.get_or_insert(i as u32);
            }
        }
        let Some(graphics) = graphics else {
            if let Some((utils, messenger)) = debug {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            instance.destroy_instance(None);
            return Err(GpuError::NoSuitableDevice(
                "no graphics+compute queue family".into(),
            ));
        };
        let dedicated = transfer.is_some();
        let transfer = transfer.unwrap_or(graphics);
        let families = QueueFamilies {
            graphics,
            transfer,
            dedicated_transfer: dedicated,
        };

        // ---- 逻辑设备创建 ---------------------------------------------------
        diag.stage = "vkCreateDevice";
        store_init_diagnostics(&diag);
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
            // MDI 压缩为每个 (layer, barrier, variant) 组生成一个批次，除首批次外均携带
            // 非零 `firstInstance`。若未启用该特性，驱动可能忽略它并重复绘制第一批图元。
            .draw_indirect_first_instance(features.draw_indirect_first_instance)
            .independent_blend(features.independent_blend)
            .sampler_anisotropy(features.sampler_anisotropy)
            .shader_int64(features.shader_int64)
            .shader_float64(features.shader_float64);

        let dev_ext_cstrings = cstrings(&device_exts);
        let dev_ext_ptrs = raw_ptrs(&dev_ext_cstrings);

        // 严禁在未启用对应设备扩展时将 `present_id_features`、`present_wait_features` 或
        // `maintenance1_features` 链入 `VkDeviceCreateInfo::pNext`：Windows 10 旧驱动遇到
        // 未启用扩展的 pNext 结构体将直接报错 `VK_ERROR_EXTENSION_NOT_PRESENT` 或触发未定义行为。
        let mut present_wait_features =
            vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(features.present_wait);
        let mut present_id_features =
            vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(features.present_id);
        let mut maintenance1_features =
            vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default()
                .swapchain_maintenance1(features.swapchain_maintenance1);
        let mut demote_features =
            vk::PhysicalDeviceShaderDemoteToHelperInvocationFeaturesEXT::default()
                .shader_demote_to_helper_invocation(features.shader_demote_to_helper_invocation);

        let mut device_create = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&dev_ext_ptrs)
            .enabled_features(&enabled_core)
            .push_next(&mut enabled_features12);
        if needs_13_backports {
            // Vulkan 1.2 路径：特性来自 KHR 扩展结构体，严禁链入 1.3 核心结构体。
            device_create = device_create
                .push_next(&mut enabled_dynamic_rendering)
                .push_next(&mut enabled_synchronization2);
            if features.shader_demote_to_helper_invocation {
                device_create = device_create.push_next(&mut demote_features);
            }
        } else {
            device_create = device_create.push_next(&mut enabled_features13);
        }
        if features.present_id {
            device_create = device_create.push_next(&mut present_id_features);
        }
        if features.present_wait {
            device_create = device_create.push_next(&mut present_wait_features);
        }
        if features.swapchain_maintenance1 {
            device_create = device_create.push_next(&mut maintenance1_features);
        }

        let device = match instance.create_device(physical, &device_create, None) {
            Ok(dev) => dev,
            Err(err) => {
                if let Some((utils, messenger)) = debug {
                    utils.destroy_debug_utils_messenger(messenger, None);
                }
                instance.destroy_instance(None);
                store_init_diagnostics(&diag);
                return Err(GpuError::Vulkan(err));
            }
        };

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

        // ReBAR 检测：若同时具备设备本地与宿主可见属性的最大堆足够大，说明 CPU 可直接写入显存。
        features.rebar = (0..memory_properties.memory_type_count as usize).any(|i| {
            let t = memory_properties.memory_types[i];
            t.property_flags.contains(
                vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE,
            ) && memory_properties.memory_heaps[t.heap_index as usize].size > (256 << 20)
        });

        // 时间线信号量，供各子系统精确判断某帧何时在 GPU 上真正执行完毕。
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

    /// 内存类型查找的公开封装（供纹理池使用）。
    #[must_use]
    pub fn memory_type_index_public(&self, kind: MemoryKind) -> Option<u32> {
        choose_memory_type(self.memory_types(), u32::MAX, kind)
    }

    /// 返回设备实际报告的所有内存类型。
    fn memory_types(&self) -> &[vk::MemoryType] {
        &self.memory_properties.memory_types[..self.memory_properties.memory_type_count as usize]
    }

    /// 查找既匹配 `kind` 又满足资源 `memoryTypeBits` 掩码的内存类型。
    ///
    /// 若类型索引不在图像的掩码内，`vkBindImageMemory` 会直接失败。
    #[must_use]
    pub fn memory_type_index_for_bits(&self, bits: u32, kind: MemoryKind) -> Option<u32> {
        choose_memory_type(self.memory_types(), bits, kind)
    }

    /// 创建缓冲区并绑定其设备内存。
    ///
    /// # Errors
    /// 若无匹配内存类型或驱动拒绝分配则返回 [`GpuError::Allocation`]。
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
        // SAFETY: 顺序创建 Vulkan 资源；任意步骤失败均会在返回前清理已创建对象。
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

    /// 销毁缓冲区及其绑定的内存。
    ///
    /// # Safety
    /// 调用方须确保已无任何命令缓冲区仍引用该缓冲区。
    pub unsafe fn destroy_buffer(&self, buffer: &Buffer) {
        if !buffer.mapped.is_null() {
            self.device.unmap_memory(buffer.memory);
        }
        self.device.destroy_buffer(buffer.raw, None);
        self.device.free_memory(buffer.memory, None);
    }

    /// 将 `data` 写入宿主可见缓冲区。
    ///
    /// # Safety
    /// `offset + data.len()` 必须位于缓冲区范围内，且不得与 GPU 读取该范围产生数据竞争。
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
        // 所分配内存可能非宿主一致（首选类型不可用时驱动可能回退），
        // 不刷写会导致 GPU 读到陈旧数据；对一致性内存而言刷写为空操作。
        self.flush(buffer, offset, bytes.len() as u64);
        true
    }

    /// 刷写已映射内存范围（一致性内存为空操作，否则必需）。
    ///
    /// 遵循 `VUID-VkMappedMemoryRange-size-00685` 对 `nonCoherentAtomSize` 对齐的要求。
    ///
    /// # Safety
    /// `offset..offset+len` 必须位于已映射缓冲区范围内。
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

    /// 在图形队列上开启一个一次性命令缓冲区。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn begin_single_time(&self) -> Result<vk::CommandBuffer, GpuError> {
        // SAFETY: 标准一次性命令缓冲区模式；总是与 `end_single_time` 配对结束。
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

    /// 提交一次性命令缓冲区并等待执行完成。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn end_single_time(&self, cmd: vk::CommandBuffer) -> Result<(), GpuError> {
        // SAFETY: `cmd` 来自 `begin_single_time` 并在此释放。
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

    /// 通过暂存缓冲区将 `bytes` 上传到设备本地缓冲区。
    ///
    /// # Errors
    /// 透传分配与 Vulkan 调用错误。
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
        // SAFETY: 暂存缓冲区由本调用独占。
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

    /// 阻塞等待直至 GPU 在时间线上完成 `value`。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含设备丢失）。
    pub fn wait_timeline(&self, value: u64) -> Result<(), GpuError> {
        // SAFETY: 标准时间线信号量等待。
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

    /// 阻塞等待整个设备空闲。
    pub fn wait_idle(&self) {
        // SAFETY: 设备存活且无外部约束。
        unsafe {
            let _ = self.device.device_wait_idle();
        }
    }

    /// 销毁全部设备资源。仅可安全调用一次。
    pub fn destroy(mut self) {
        // 生命周期可能长于 `Gpu` 的对象（如图集）在此之后不得再触碰设备，
        // 因此在销毁设备句柄前先使共享状态失效。
        self.lifetime.invalidate();
        // SAFETY: `Self` 拥有全部句柄且正在被消费销毁。
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
    // SAFETY: 驱动保证指针在回调期间有效。
    let data = unsafe { &*data };
    let message = data
        .message_as_c_str()
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default();
    // 校验层输出绝不能被未配置的日志器吞掉；同时统计计数以供 CI 门禁断言。
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

/// 串行化 GPU 测试。
///
/// 多个 `VkInstance` 并发创建设备容易触发开发者机器上的第三方覆盖层（如 RTSS）崩溃，
/// 因此集成测试通过此锁串行执行。
pub fn gpu_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 测试辅助函数：创建无头设备，不可用时跳过测试。
///
/// 当机器无 Vulkan 1.3 设备时返回 `None`，使无 GPU 的 CI 报告跳过而非失败。
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

        // 第二次解析不得产生重复项。
        let before = extensions.len();
        resolve_instance_extension_dependencies(
            &mut extensions,
            &["VK_EXT_swapchain_maintenance1".to_string()],
        );
        assert_eq!(extensions.len(), before);

        // 没有任何 surface 相关请求时，实例扩展列表保持为空。
        let mut plain = Vec::new();
        resolve_instance_extension_dependencies(&mut plain, &[]);
        assert!(plain.is_empty());
    }

    /// `VUID-vkCreateDevice-ppEnabledExtensionNames-01387`：启用
    /// `VK_KHR_swapchain` 的设备要求实例已启用 `VK_KHR_surface`。
    /// 窗口运行器曾完全忘记请求实例扩展，使缺失的依赖先变成校验错误，
    /// 进而导致 `vkCreateWin32SurfaceKHR` 无法解析。
    #[test]
    fn the_swapchain_brings_its_instance_extensions() {
        let device = vec!["VK_KHR_swapchain".to_string()];

        let mut bare = Vec::new();
        resolve_instance_extension_dependencies(&mut bare, &device);
        assert_eq!(bare, vec!["VK_KHR_surface".to_string()]);

        // 调用方若只提供了平台扩展而漏掉根扩展，根扩展应被恰好补入一次。
        let mut platform_only = vec!["VK_KHR_win32_surface".to_string()];
        resolve_instance_extension_dependencies(&mut platform_only, &device);
        assert_eq!(
            platform_only,
            vec![
                "VK_KHR_surface".to_string(),
                "VK_KHR_win32_surface".to_string()
            ]
        );

        // 已完整派生的列表保持原样。
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

    /// 回归测试：Windows 10 旧版加载器/驱动不支持 `VK_EXT_surface_maintenance1` 时，
    /// 实例与设备扩展过滤器必须安全降级跳过可选扩展，绝不让 `vkCreateInstance` 或
    /// `vkCreateDevice` 触发 `ERROR_EXTENSION_NOT_PRESENT`。
    #[test]
    fn win10_older_driver_gracefully_skips_missing_optional_extensions() {
        let mut desired_inst = vec![
            "VK_KHR_surface".to_string(),
            "VK_KHR_win32_surface".to_string(),
        ];
        let requested_dev = vec![
            "VK_KHR_swapchain".to_string(),
            "VK_EXT_swapchain_maintenance1".to_string(),
            "VK_KHR_present_wait".to_string(),
        ];
        resolve_instance_extension_dependencies(&mut desired_inst, &requested_dev);

        // 模拟 Win10 旧加载器：仅提供基础 surface 扩展，不提供 VK_EXT_surface_maintenance1。
        let available_inst = vec![
            "VK_KHR_surface".to_string(),
            "VK_KHR_win32_surface".to_string(),
            "VK_KHR_get_surface_capabilities2".to_string(),
        ];
        let inst_res = filter_instance_extensions(&desired_inst, &available_inst);
        assert!(inst_res.missing_required.is_empty());
        assert!(!inst_res
            .enabled
            .iter()
            .any(|e| e == "VK_EXT_surface_maintenance1"));
        assert!(inst_res
            .skipped
            .iter()
            .any(|e| e == "VK_EXT_surface_maintenance1"));

        // 即使物理设备列表包含 VK_EXT_swapchain_maintenance1，由于实例未启用
        // VK_EXT_surface_maintenance1，设备层也必须自动跳过它；同时若缺少 VK_KHR_present_id，
        // VK_KHR_present_wait 也必须自动跳过。
        let available_dev = vec![
            "VK_KHR_swapchain".to_string(),
            "VK_EXT_swapchain_maintenance1".to_string(),
            "VK_KHR_present_wait".to_string(),
            "VK_KHR_dynamic_rendering".to_string(),
            "VK_KHR_synchronization2".to_string(),
        ];
        let dev_res = resolve_device_extensions(
            &requested_dev,
            &available_dev,
            &inst_res.enabled,
            vk::make_api_version(0, 1, 2, 0),
        );
        assert!(dev_res.missing_required.is_empty());
        assert!(dev_res.enabled.iter().any(|e| e == "VK_KHR_swapchain"));
        assert!(dev_res
            .enabled
            .iter()
            .any(|e| e == "VK_KHR_dynamic_rendering"));
        assert!(dev_res
            .enabled
            .iter()
            .any(|e| e == "VK_KHR_synchronization2"));
        assert!(!dev_res
            .enabled
            .iter()
            .any(|e| e == "VK_EXT_swapchain_maintenance1"));
        assert!(!dev_res.enabled.iter().any(|e| e == "VK_KHR_present_wait"));
        assert!(!dev_res.enabled.iter().any(|e| e == "VK_KHR_present_id"));
    }

    #[test]
    fn diagnostic_report_includes_context_and_troubleshooting_hints() {
        let err = GpuError::Vulkan(vk::Result::ERROR_EXTENSION_NOT_PRESENT);
        let report = err.diagnostic_report();
        assert!(report.contains("ERROR_EXTENSION_NOT_PRESENT"));
        assert!(report.contains("排障建议"));
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
        // SAFETY: 无其他代码引用该缓冲区。
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

        // 通过宿主可见暂存缓冲区回读数据。
        let readback = gpu
            .create_buffer(
                payload.len() as u64,
                vk::BufferUsageFlags::TRANSFER_DST,
                MemoryKind::HostVisible,
                "readback",
            )
            .expect("readback");
        // SAFETY: 独占所有权且上传后队列已空闲。
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
        // SAFETY: 在图形队列上通过空提交触发信号量。
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

    /// 模拟 NVIDIA 独显报告的内存布局：两个纯显存类型、一块系统内存，以及末尾的
    /// 宿主可见显存窗口（Resizable BAR）。
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
        // 类型 1 为纯显存；类型 5 虽也是显存但受限于狭小的 CPU 映射窗口，
        // 不可用于存放持久化池、排序键或图集图像。
        assert_eq!(
            choose_memory_type(&types, u32::MAX, MemoryKind::DeviceLocal),
            Some(1)
        );
        // 当资源掩码仅允许该窗口类型时仍正常选用。
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

    /// 回归测试：过去将结束偏移误填为 `size`，导致 4 MiB 环的第二帧从 `2 MiB` 起
    /// 刷写 `2 MiB + 64 B` 而被校验层拒绝（`VUID-...-00685`）。
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
        // 未对齐的起始偏移向下取整，同时不丢失请求尾部。
        let (offset, size) = align_mapped_range(100, 10, 64, 1024).expect("a flushable range");
        assert_eq!(offset, 64);
        assert!(offset + size >= 110);
        // 恰好结束于分配末尾的范围允许长度不按原子粒度对齐。
        let (offset, size) = align_mapped_range(1000, 24, 64, 1024).expect("a flushable range");
        assert_eq!(offset, 960);
        assert_eq!(offset + size, 1024);
        // 退化空范围直接忽略，避免生成非法刷写范围。
        assert_eq!(align_mapped_range(0, 0, atom, allocation), None);
        assert_eq!(align_mapped_range(allocation, 16, atom, allocation), None);
    }

    #[test]
    fn a_device_without_vram_still_gets_device_local_memory() {
        // 集成显卡仅报告单一同时具备设备本地与宿主可见的内存类型，必须允许选用。
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

//! 窗口化呈现：表面（Surface）、交换链（Swapchain）与获取/呈现循环。
//!
//! 本 crate 的渲染器默认保持无表面设计：所有测试均渲染至离屏图像，从而使 GPU 测试套件
//! 可在 CI 上运行。本模块在此基础上补充窗口化能力：[`Surface`] 持有 `VkSurfaceKHR`
//! 与交换链，[`Swapchain::acquire`] 向调用方返回可绘制的颜色图像视图。
//!
//! 呈现模式遵循 [`crate::swapchain`] 中的策略：配合 `VK_EXT_present_mode_fifo_latest_ready`，
//! FIFO-latest-ready 模式在保持垂直同步的同时呈现最新完成的图像，将输入延迟从三帧降至一帧。
//!
//! 窗口缩放时，拖拽过程中的几帧会处于过期状态，`vkAcquireNextImageKHR` 与
//! `vkQueuePresentKHR` 均会返回 `VK_ERROR_OUT_OF_DATE_KHR`。这并非错误——而是驱动请求
//! 重建交换链。旧交换链会作为 `oldSwapchain` 传给驱动，以便呈现引擎复用资源而无需销毁表面。

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::device::{Gpu, GpuError};
use crate::swapchain::{PresentConfig, PresentMode, ResizeBehaviour};
use ash::vk;

/// 绑定到原生窗口的 `VkSurfaceKHR`。
pub struct Surface {
    surface: vk::SurfaceKHR,
    /// `VK_KHR_surface` 实例扩展加载器。
    khr_surface: ash::khr::surface::Instance,
    /// Win32 专用加载器。需在表面存活期间保持有效，因为表面由其函数指针创建。
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
    /// 为 Win32 窗口句柄创建 Vulkan 表面。
    ///
    /// `hwnd` 为窗口的原始 `HWND`。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    #[cfg(windows)]
    pub fn new_win32(gpu: &Gpu, hwnd: isize) -> Result<Self, GpuError> {
        // ash 在 `Instance::new` 中解析扩展函数指针，缺失时会直接 *panic*。
        // 该 panic 发生在不可展开的 `extern "system"` 加载器中，因此会 abort
        // 整个进程，而不是沿调用方的错误路径展开。此处先检查已启用列表，
        // 把缺失扩展当作普通「不支持的请求」返回错误。
        const REQUIRED: [&str; 2] = ["VK_KHR_surface", "VK_KHR_win32_surface"];
        let missing: Vec<&'static str> = REQUIRED
            .into_iter()
            .filter(|name| {
                !gpu.enabled_instance_extensions
                    .iter()
                    .any(|enabled| enabled == name)
            })
            .collect();
        if !missing.is_empty() {
            return Err(GpuError::UnsupportedFeatures(missing));
        }

        let khr_surface = ash::khr::surface::Instance::new(&gpu.entry, &gpu.instance);
        let win32_surface = ash::khr::win32_surface::Instance::new(&gpu.entry, &gpu.instance);
        // SAFETY: 句柄来自存活窗口，此处仅作读取。
        let create_info = vk::Win32SurfaceCreateInfoKHR::default().hwnd(hwnd);
        let surface = unsafe { win32_surface.create_win32_surface(&create_info, None) }
            .map_err(GpuError::from)?;
        Ok(Self {
            surface,
            khr_surface,
            win32_surface: Some(win32_surface),
        })
    }

    /// 从原始窗口句柄创建表面。
    ///
    /// 在 Windows 上句柄须为 `HWND`。其他平台尚未接入对应加载器，
    /// 因此会显式返回错误而非静默无操作。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
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

    /// 底层 Vulkan 表面句柄。
    #[must_use]
    pub const fn handle(&self) -> vk::SurfaceKHR {
        self.surface
    }

    /// 查询指定物理设备的表面能力。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn capabilities(&self, gpu: &Gpu) -> Result<vk::SurfaceCapabilitiesKHR, GpuError> {
        // SAFETY: 该表面属于当前实例。
        unsafe {
            self.khr_surface
                .get_physical_device_surface_capabilities(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// 查询指定物理设备支持的呈现模式列表。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn present_modes(&self, gpu: &Gpu) -> Result<Vec<vk::PresentModeKHR>, GpuError> {
        // SAFETY: 该表面属于当前实例。
        unsafe {
            self.khr_surface
                .get_physical_device_surface_present_modes(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// 查询表面支持的像素格式列表。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
    pub fn formats(&self, gpu: &Gpu) -> Result<Vec<vk::SurfaceFormatKHR>, GpuError> {
        // SAFETY: 该表面属于当前实例。
        unsafe {
            self.khr_surface
                .get_physical_device_surface_formats(gpu.physical, self.surface)
                .map_err(GpuError::from)
        }
    }

    /// 查询表面在指定呈现模式下的呈现缩放能力。
    ///
    /// 若无法查询（实例未暴露 `VK_KHR_get_surface_capabilities2` 或驱动拒绝调用）
    /// 则返回 `None`，此时沿用平台默认缩放行为。
    ///
    /// `VK_EXT_swapchain_maintenance1` 要求请求的缩放行为必须属于表面声明支持的范围。
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
        // 缩放能力取决于呈现模式，因此将模式一并传入查询。
        let mut present_mode_info = vk::SurfacePresentModeEXT::default().present_mode(present_mode);
        let surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
            .surface(self.surface)
            .push_next(&mut present_mode_info);
        let mut scaling = vk::SurfacePresentScalingCapabilitiesEXT::default();
        let mut capabilities = vk::SurfaceCapabilities2KHR::default();
        capabilities = capabilities.push_next(&mut scaling);
        // SAFETY: 该表面属于当前实例，且物理设备与创建表面时一致。
        let result = unsafe {
            loader.get_physical_device_surface_capabilities2(
                gpu.physical,
                &surface_info,
                &mut capabilities,
            )
        };
        result.ok().map(|()| scaling)
    }

    /// 销毁表面。
    pub fn destroy(self, gpu: &Gpu) {
        // SAFETY: 表面属于 `gpu.instance`，且实例生命周期长于表面。
        unsafe {
            self.khr_surface.destroy_surface(self.surface, None);
        }
        let _ = gpu;
    }
}

/// 用于创建表面的原始窗口句柄。
#[derive(Copy, Clone, Debug)]
pub enum RawWindowHandle {
    /// Win32 `HWND`。
    #[cfg(windows)]
    Win32 {
        /// 窗口句柄。
        hwnd: isize,
    },
}

/// 低延迟呈现路径等待画面上屏的超时时间。
///
/// 等待必须有界：对于永不完成呈现的场景（被遮挡的窗口、无合成器的会话），
/// 无限等待会永久阻塞帧循环。50 ms 长于任何正常刷新间隔，又短到不至于让卡顿感过于明显。
pub const PRESENT_WAIT_TIMEOUT_NS: u64 = 50_000_000;

/// 连续多少次呈现超时后判定等待无意义。
///
/// 未参与合成的窗口永远不会完成呈现通知，继续等待只会给每帧白白增加延迟。
/// 连续超时达到此次数后，低延迟等待路径会自动关闭。
pub const PRESENT_WAIT_GIVE_UP_AFTER: u32 = 3;

/// 已获取并可供渲染的交换链图像。
#[derive(Copy, Clone, Debug)]
pub struct AcquiredImage {
    /// 该图像在交换链中的索引。
    pub index: u32,
    /// 供渲染写入的图像视图。
    pub view: vk::ImageView,
    /// 图像本体句柄。渲染器需要用它执行布局转换：仅凭图像视图无法转换布局。
    pub image: vk::Image,
    /// 当呈现引擎交出图像所有权时触发。
    ///
    /// 渲染提交在访问该图像前必须等待此信号量。
    pub available: vk::Semaphore,
    /// 该图像的渲染提交完成后必须触发的信号量。
    ///
    /// 每张交换链图像独占一个“渲染完成”信号量。不能复用单个全局信号量：
    /// 呈现引擎直到再次获取该图像时才释放它，而上一次呈现的等待可能仍未消费。
    /// 触发仍被 `vkQueuePresentKHR` 使用的信号量会触发校验错误（`VUID-vkQueueSubmit2-semaphore-03868`）。
    ///
    /// 按每张图像分配独立信号量后，只有同一图像索引再次被获取时才会重新触发该信号量——
    /// 而此时前一次呈现必然已完成，其等待也已消费完毕。
    pub render_finished: vk::Semaphore,
}

/// 拥有 `image_count` 张图像的交换链所需的渲染完成信号量数量。
///
/// 每张图像一个：信号量绑定到图像而非帧槽位，因为图像占用时长由呈现引擎而非渲染器决定。
pub(crate) const fn render_finished_slot_count(image_count: usize) -> usize {
    image_count
}

/// 获取 `image_index` 对应的渲染完成信号量槽位。
///
/// 同一交换链内该映射必须为单射：两张图像绝不可共享信号量，
/// 否则第二次呈现会触发第一次呈现仍在等待的信号量。
pub(crate) const fn render_finished_slot(image_index: u32, image_count: usize) -> usize {
    if image_count == 0 {
        0
    } else {
        (image_index as usize) % image_count
    }
}

/// 获取下一张交换链图像的结果。
#[derive(Copy, Clone, Debug)]
pub enum AcquireOutcome {
    /// 图像已就绪，可用于渲染。
    Ready {
        /// 获取到的图像信息。
        image: AcquiredImage,
        /// 为 true 表示交换链仍可用但已不再精确匹配表面，应在呈现后重建。
        suboptimal: bool,
    },
    /// 表面尺寸已不兼容变化；应立即重建交换链。
    OutOfDate,
}

/// 呈现已渲染交换链图像的结果。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentOutcome {
    /// 正常呈现完成。
    Optimal,
    /// 已呈现，但交换链处于次优或过期状态，须在下一帧前重建。
    Recreate,
}

/// `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT` 枚举值。
///
/// 该模式来自 `VK_EXT_present_mode_fifo_latest_ready`（已晋升为
/// `VK_KHR_present_mode_fifo_latest_ready`），而非 `VK_EXT_swapchain_maintenance1`。
/// `ash` 0.38 早于这两个扩展，因此直接使用 Vulkan 注册表中的常量值。
///
/// 相邻值 `1000111000` 是 `VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR`，与本模式无关。
const PRESENT_MODE_FIFO_LATEST_READY: vk::PresentModeKHR =
    vk::PresentModeKHR::from_raw(1_000_361_000);

/// 将策略模式映射为 Vulkan 呈现模式。
///
/// 仅当驱动提供 `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT` 时 `FifoLatestReady` 才合法；
/// 所有 Vulkan 实现必须支持 `VK_PRESENT_MODE_FIFO_KHR`，因此将其作为回退。
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

/// 计算创建交换链图像所使用的尺寸。
///
/// `current` 为驱动报告的 `currentExtent`：在 Win32 上它是表面对应窗口的权威尺寸。
/// 请求尺寸与驱动尺寸哪怕相差 1 像素，也会导致驱动在下一帧立即报告交换链过期，
/// 因此只要驱动提供了有效值就以驱动为准。
///
/// 钳制到 `min ..= max` 不会 panic：若驱动异常返回 `min > max` 会导致 `u32::clamp`
/// panic，因此取两者中较小值作为下界。
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

/// `vkAcquireNextImageKHR` 返回状态的分类。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquireStatus {
    /// 图像已就绪，可用于渲染。
    Ready {
        /// 该图像在交换链中的索引。
        index: u32,
        /// 交换链仍可用但不再精确匹配表面。
        suboptimal: bool,
    },
    /// 表面已变化；必须重建交换链。
    OutOfDate,
}

/// 分类 `vkAcquireNextImageKHR` 的返回结果。
///
/// `VK_ERROR_OUT_OF_DATE_KHR` 并非错误：仅表示窗口尺寸变化需要重建交换链。其余错误均为致命错误。
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

/// 分类 `vkQueuePresentKHR` 的返回结果。
///
/// `VK_ERROR_OUT_OF_DATE_KHR` 与 `VK_SUBOPTIMAL_KHR` 表示需要重建交换链，
/// 并非帧渲染失败。其余错误均为致命错误。
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

/// 交换链及其供渲染器绘制的图像视图集合。
pub struct Swapchain {
    swapchain: vk::SwapchainKHR,
    khr_swapchain: ash::khr::swapchain::Device,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    /// 每个在飞帧槽位对应一个“图像就绪”信号量。
    ///
    /// `vkAcquireNextImageKHR` **必须**接收一个信号量（或围栏）：同时传入两个空句柄属于非法用法。
    /// 该信号量是呈现引擎与渲染提交之间的同步点，渲染提交在访问图像前等待此信号量。
    ///
    /// 使用信号量池而非单个信号量，因为二值信号量同时只能承载一个待消费信号；
    /// 池深度等于最大在飞帧数，渲染器自身的在飞围栏确保槽位复用前先前的等待已消费完毕。
    acquire_semaphores: Vec<vk::Semaphore>,
    /// 每张交换链图像对应一个“渲染完成”信号量。
    ///
    /// 该信号量绑定到图像而非帧槽位：渲染提交退役许久后呈现引擎仍可能还在等待它。
    /// 跨图像共享信号量正是触发 `VUID-vkQueueSubmit2-semaphore-03868` 的原因。
    render_finished_semaphores: Vec<vk::Semaphore>,
    /// 轮转池中下一个可用的信号量索引。
    next_semaphore: usize,
    format: vk::Format,
    color_space: vk::ColorSpaceKHR,
    next_index: u32,
    /// 当前已获取的图像索引（若有）。
    acquired: Option<u32>,
    /// 创建交换链时的尺寸，用于检测窗口尺寸变化。
    extent: (u32, u32),
    /// `VK_KHR_present_wait` 使用的下一个呈现标识符。
    ///
    /// 规范要求非零标识符必须严格大于同一交换链此前传入的所有标识符。
    next_present_id: u64,
    /// 是否等待画面真正呈现到屏幕。
    wait_for_present: bool,
    /// `VK_KHR_present_wait` 扩展及特性可用。
    has_present_wait: bool,
    /// `VK_EXT_swapchain_maintenance1` 扩展及其特性均已启用。
    ///
    /// 仅当扩展与特性同时启用时，呈现围栏和呈现缩放行为才合法。
    has_maintenance1: bool,
    /// 未能按时上屏的呈现超时总次数。
    present_timeouts: u32,
    /// 连续发生的呈现超时次数。
    consecutive_timeouts: u32,
    /// 交换链重建后跳过一次呈现等待。
    ///
    /// 用户拖拽窗口边缘时，合成器常会丢弃旧交换链的呈现请求；若每次阻塞 50 ms
    /// 会卡死消息循环。因此重建后的首帧呈现不执行等待。
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
    /// 为指定表面创建交换链。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
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

    /// 创建交换链，并可选地退役旧交换链。
    ///
    /// 将 `old_swapchain` 传给驱动以便呈现引擎复用资源：若不传递，旧交换链会在新交换链
    /// 创建前被拆除，导致闪烁及引用已销毁交换链图像的校验错误。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
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

        // 交换链图像将作为渲染器的颜色附件，其格式必须与管线的 `COLOR_FORMAT` 严格匹配：
        // 针对某附件格式创建的图形管线不可用于其他格式，通道顺序不同也会导致红蓝反转。
        // 格式为 sRGB，使硬件自动对着色器的线性输出执行编码。
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

        // `VK_EXT_swapchain_maintenance1` 仅在扩展与特性同时启用时可用：未启用特性时使用呈现围栏或呈现缩放属于非法用法。
        let has_maintenance1 = gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1")
            && gpu.features.swapchain_maintenance1;

        // `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT` 来自独立扩展
        // （`VK_EXT_present_mode_fifo_latest_ready` / `VK_KHR_present_mode_fifo_latest_ready`），
        // 而非 `VK_EXT_swapchain_maintenance1`。未启用该扩展时请求此模式属于非法用法。
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
        // 仅请求表面实际支持的呈现模式。
        let present_mode = if modes.contains(&present_mode) {
            present_mode
        } else {
            vk::PresentModeKHR::FIFO
        };

        // 驱动的 `currentExtent` 是表面的权威尺寸：哪怕 1 像素不匹配也会导致驱动在下一帧立即报告交换链过期。
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

        // `VK_EXT_swapchain_maintenance1` 允许指定交换链尺寸与表面不匹配时的缩放行为，
        // 且该行为必须在表面声明的支持范围内，因此需先查询缩放能力。
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
                        // 绝不拉伸旧图像：将其锚定在左上角，由新帧填充窗口其余区域。
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
                // `TRANSFER_SRC` 允许标准 Vulkan 截图层拷出已呈现帧，便于在无桌面会话时检查窗口路径。
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
            // 将旧交换链移交驱动以复用呈现资源，而非先销毁表面。
            .old_swapchain(old_swapchain);
        if use_scaling {
            create_info = create_info.push_next(&mut scaling_info);
        }

        let khr_swapchain = ash::khr::swapchain::Device::new(&gpu.instance, &gpu.device);
        // SAFETY: 表面与设备属于同一 Vulkan 实例。
        let swapchain = unsafe { khr_swapchain.create_swapchain(&create_info, None) }
            .map_err(GpuError::from)?;

        // 图像由交换链拥有；此处仅为其创建视图。
        // SAFETY: 交换链刚刚创建完成。
        let images =
            unsafe { khr_swapchain.get_swapchain_images(swapchain) }.map_err(GpuError::from)?;

        let mut views = Vec::with_capacity(images.len());
        for image in &images {
            // SAFETY: 图像属于本交换链。
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

        // 每个在飞帧对应一个“图像就绪”信号量。
        let pool_size = gpu.frames_in_flight().max(1);
        let mut acquire_semaphores = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            // SAFETY: 设备生命周期长于交换链。
            let semaphore = unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(GpuError::from)?
            };
            acquire_semaphores.push(semaphore);
        }

        // 每张交换链图像对应一个“渲染完成”信号量。数量必须匹配图像总数而非在飞帧数：
        // 该信号量描述的是*该图像*在呈现引擎中保持占用的生命周期。
        let mut render_finished_semaphores = Vec::with_capacity(images.len());
        for _ in 0..render_finished_slot_count(images.len()) {
            // SAFETY: 设备生命周期长于交换链。
            let semaphore = unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(GpuError::from)?
            };
            render_finished_semaphores.push(semaphore);
        }

        // 低延迟呈现路径同时需要三项条件：`VK_KHR_present_wait` 扩展、其依赖的
        // `VK_KHR_present_id`，以及对应的 `presentWait`/`presentId` 特性。
        // 仅检查扩展名而不检查特性会在未启用特性时调用 `vkWaitForPresentKHR`，触发非法用法。
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

    /// 交换链图像使用的像素格式。
    #[must_use]
    pub const fn format(&self) -> vk::Format {
        self.format
    }

    /// 交换链呈现使用的色彩空间。
    #[must_use]
    pub const fn color_space(&self) -> vk::ColorSpaceKHR {
        self.color_space
    }

    /// 当前交换链尺寸。
    #[must_use]
    pub const fn extent(&self) -> (u32, u32) {
        self.extent
    }

    /// 是否已启用 `VK_EXT_swapchain_maintenance1` 及其特性。
    ///
    /// 启用后可向操作系统提交呈现围栏并指定呈现缩放行为。
    #[must_use]
    pub const fn has_maintenance1(&self) -> bool {
        self.has_maintenance1
    }

    /// 未能按时上屏的呈现超时次数。
    ///
    /// 非零计数通常意味着窗口未按预期参与合成——帧循环仍在运行，但图像并未真正显示。
    #[must_use]
    pub const fn present_timeouts(&self) -> u32 {
        self.present_timeouts
    }

    /// 按新表面尺寸重建交换链，并将旧交换链移交驱动以复用资源。
    ///
    /// 重建前先等待设备空闲：替换期间旧交换链的图像不得处于在飞状态。
    ///
    /// # Errors
    /// 透传 Vulkan 调用错误。
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
        // 窗口仍在拖拽期间合成器可能丢弃呈现请求，下一帧不得阻塞等待。
        replacement.skip_present_wait_once = true;
        let old = core::mem::replace(self, replacement);
        old.destroy(gpu);
        Ok(())
    }

    /// 获取下一张待绘制的交换链图像。
    ///
    /// 返回 [`Self::views`] 中的索引、目标图像视图以及图像就绪信号量。
    /// 调用方随后须呈现同一索引，且渲染提交在访问图像前须等待该信号量。
    ///
    /// 为兼容早期调用方，表面过期会作为错误返回；新代码请使用 [`Self::acquire_outcome`]，
    /// 它会将其报告为 [`AcquireOutcome::OutOfDate`]。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含表面丢失）。
    pub fn acquire(&mut self, timeout_ns: u64) -> Result<AcquiredImage, GpuError> {
        match self.acquire_outcome(timeout_ns)? {
            AcquireOutcome::Ready { image, .. } => Ok(image),
            AcquireOutcome::OutOfDate => Err(GpuError::Vulkan(vk::Result::ERROR_OUT_OF_DATE_KHR)),
        }
    }

    /// 获取下一张交换链图像，并将表面过期作为正常状态分支返回。
    ///
    /// `VK_ERROR_OUT_OF_DATE_KHR` 并非失败：表示窗口已缩放需重建交换链，
    /// 调用方可紧接着调用 [`Self::recreate`] 继续运行。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含表面丢失）。
    pub fn acquire_outcome(&mut self, timeout_ns: u64) -> Result<AcquireOutcome, GpuError> {
        // 信号量取自轮转池：二值信号量同时仅能承载一个待消费信号，槽位仅在前一帧消费后复用。
        let semaphore = self.acquire_semaphores[self.next_semaphore];
        // SAFETY: 交换链属于当前设备。
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
                // 信号量未被触发，因此该槽位未消耗：下次尝试可继续复用。
                self.acquired = None;
                Ok(AcquireOutcome::OutOfDate)
            }
            AcquireStatus::Ready { index, suboptimal } => {
                self.next_semaphore = (self.next_semaphore + 1) % self.acquire_semaphores.len();
                self.next_index = index;
                self.acquired = Some(index);
                let view = self.views[index as usize];
                // 该信号量绑定于此图像：仅当同一图像再次被获取后才会重新触发。
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

    /// 呈现已获取的图像，并等待渲染器完成绘制。
    ///
    /// `render_finished` 由渲染器帧提交触发；呈现操作等待它以确保屏幕绝不显示半绘帧。
    ///
    /// 表面过期在此并非致命错误：请使用 [`Self::present_outcome`] 判断是否需要重建交换链。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含表面丢失）。
    pub fn present(
        &mut self,
        gpu: &Gpu,
        index: u32,
        render_finished: vk::Semaphore,
    ) -> Result<(), GpuError> {
        self.present_outcome(gpu, index, render_finished)
            .map(|_| ())
    }

    /// 呈现已获取的图像，并将过期或次优状态作为正常结果返回。
    ///
    /// `VK_ERROR_OUT_OF_DATE_KHR` 与 `VK_SUBOPTIMAL_KHR` 表示下一帧前需重建交换链，而非帧失败。
    ///
    /// # Errors
    /// 透传 Vulkan 错误（含表面丢失）。
    pub fn present_outcome(
        &mut self,
        gpu: &Gpu,
        index: u32,
        render_finished: vk::Semaphore,
    ) -> Result<PresentOutcome, GpuError> {
        let swapchains = [self.swapchain];
        // 呈现信息接收的是图像索引，而非图像句柄本身。
        let indices = [index];
        // 当前 ash 版本中 `PresentInfoKHR` 为普通结构体：可直接写入指针字段。
        // 呈现操作等待渲染器完成本帧绘制。
        let wait_semaphores = [render_finished];

        // `VK_KHR_present_wait` 使用单调递增的非零标识符标识本次呈现；
        // 该值由应用指定，且须严格大于同一交换链此前传入的所有标识符。
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
        // SAFETY: 交换链与图像均属于当前设备。
        let result = unsafe {
            self.khr_swapchain
                .queue_present(gpu.graphics_queue, &present_info)
        };
        self.acquired = None;
        // `queue_present` 通过返回值报告次优状态，通过错误码报告过期状态。
        // 两者均非帧失败，仅意味着“重建交换链”。
        let mut outcome = classify_present(result).map_err(GpuError::from)?;
        // 当呈现已报告交换链过期时，图像并未上屏，继续等待只会白白耗尽超时。
        let out_of_date = matches!(result, Err(vk::Result::ERROR_OUT_OF_DATE_KHR));

        if self.has_present_wait {
            // 同一交换链的呈现标识符必须保持严格递增。
            self.next_present_id += 1;
            // 重建后的首帧从不等待：合成器可能仍持有旧交换链。
            let skip_wait = core::mem::take(&mut self.skip_present_wait_once);
            if self.wait_for_present && !out_of_date && !skip_wait {
                // 阻塞等待直至图像真正上屏。这是低延迟路径的核心：在真实上屏后立即采样输入队列，
                // 使交互响应在紧邻的下一帧生效，而非延迟三帧。
                //
                // 等待**有界**：对于未参与合成、被遮挡或最小化的窗口，超时仅表示
                // “本帧未实际显示”，属于偶发抖动而非致命错误。
                // SAFETY: 交换链仍存活且该标识符刚刚传入 `queue_present`。
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
                            // 窗口未被合成。继续等待永不发生的呈现会给每帧增加完整超时，
                            // 因此自动关闭低延迟等待路径。
                            self.wait_for_present = false;
                            self.consecutive_timeouts = 0;
                        }
                    }
                    // 交换链已不再匹配表面：触发重建而非判为帧失败。
                    Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
                        outcome = PresentOutcome::Recreate;
                    }
                    // 表面本身已丢失：无法再进行任何呈现，保持为硬错误。
                    Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                        return Err(GpuError::Vulkan(vk::Result::ERROR_SURFACE_LOST_KHR));
                    }
                    Err(error) => return Err(GpuError::Vulkan(error)),
                }
            }
        }
        Ok(outcome)
    }

    /// 销毁交换链及其所有图像视图。
    pub fn destroy(self, gpu: &Gpu) {
        // SAFETY: 视图与交换链均属于当前设备。
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
        // 完整呈现栈：表面、交换链以及启用低延迟呈现模式的维护扩展。
        let extensions = vec![
            "VK_KHR_surface".to_string(),
            "VK_KHR_swapchain".to_string(),
            "VK_EXT_swapchain_maintenance1".to_string(),
        ];
        assert!(crate::swapchain::is_supported(&extensions));

        // 缺少交换链扩展时呈现栈不完整。
        let incomplete = vec!["VK_KHR_surface".to_string()];
        assert!(!crate::swapchain::is_supported(&incomplete));
    }

    /// 缺少实例扩展必须是可诊断的错误而非 abort：
    /// 当 `vkCreateWin32SurfaceKHR` 无法解析时，ash 加载器会在
    /// 不可展开的函数内部 panic。
    #[cfg(windows)]
    #[test]
    fn new_win32_reports_missing_instance_extensions() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            // 无 GPU 时，只能校验上面的纯逻辑断言。
            return;
        };
        // 无头测试设备完全不请求 surface 扩展，因此该检查必须在
        // ash 尝试加载函数指针之前触发。
        assert!(
            matches!(
                Surface::new_win32(&gpu, 0),
                Err(GpuError::UnsupportedFeatures(_))
            ),
            "a missing instance extension must be an error, not an abort"
        );
        gpu.destroy();
    }

    #[test]
    fn the_swapchain_uses_the_renderers_colour_format() {
        // 交换链图像用作渲染器的颜色附件，格式必须与图形管线一致；通道顺序不同会导致红蓝反转。
        assert_eq!(
            crate::pipeline::COLOR_FORMAT,
            vk::Format::B8G8R8A8_SRGB,
            "the swapchain must match the pipeline's attachment format"
        );
    }

    /// 最低延迟呈现模式拥有独立扩展与枚举值：`1000111000` 是
    /// `VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR`，并非 latest-ready 模式。
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

        // 缺少该扩展时请求此模式非法，故回退到 FIFO。
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

    /// 驱动的 `currentExtent` 是表面权威尺寸：相差 1 像素即可导致下一帧报告交换链过期。
    #[test]
    fn the_extent_prefers_the_drivers_current_extent() {
        let extent = resolve_extent((1920, 1080), (0, 0), (u32::MAX, u32::MAX), 800, 600);
        assert_eq!(extent.width, 1920);
        assert_eq!(extent.height, 1080);

        // `u32::MAX` 为“表面未指定固定尺寸”哨兵值，此时采用请求尺寸。
        let extent = resolve_extent((u32::MAX, u32::MAX), (0, 0), (u32::MAX, u32::MAX), 800, 600);
        assert_eq!(extent.width, 800);
        assert_eq!(extent.height, 600);

        // 将结果钳制在驱动允许的尺寸范围内。
        let extent = resolve_extent((10, 10), (100, 100), (200, 200), 800, 600);
        assert_eq!(extent.width, 100);
        assert_eq!(extent.height, 100);
    }

    /// 驱动异常报告 `min > max` 时钳制操作不得 panic。
    #[test]
    fn the_extent_clamp_survives_a_nonsensical_range() {
        let extent = resolve_extent((150, 150), (300, 300), (200, 200), 150, 150);
        assert_eq!(extent.width, 200);
        assert_eq!(extent.height, 200);
    }

    /// 表面过期是对重建交换链的请求，而非帧失败。
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
        // 表面丢失为致命错误：无法再继续呈现。
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

    /// 渲染完成信号量绑定到交换链*图像*而非帧槽位。
    ///
    /// 所有图像共用一个全局信号量曾导致 `VUID-vkQueueSubmit2-semaphore-03868`：
    /// 图像 0 的呈现引擎仍在等待信号量时，图像 1 的帧又重新触发了它。
    #[test]
    fn the_render_finished_semaphores_are_allocated_per_image() {
        // 每张图像一个信号量——而非每个在飞帧一个。
        assert_eq!(render_finished_slot_count(1), 1);
        assert_eq!(render_finished_slot_count(3), 3);
        assert_eq!(render_finished_slot_count(0), 0);

        // 映射为单射：任意两张图像绝不共享信号量。
        let count = 4;
        let slots: Vec<usize> = (0..count as u32)
            .map(|index| render_finished_slot(index, count))
            .collect();
        let mut unique = slots.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), count, "every image needs its own semaphore");

        // 空交换链场景下不得 panic。
        assert_eq!(render_finished_slot(0, 0), 0);
        assert_eq!(render_finished_slot(7, 0), 0);
    }

    /// 窗口化调用方依赖的签名守卫。
    ///
    /// 新的 outcome-aware API 为增量新增，原有四个调用保持签名不变，本测试用于编译期保护。
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

        // 新的 outcome-aware API 为增量扩展。
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

    /// 重建交换链时将旧交换链移交驱动而非先销毁表面。
    ///
    /// 窗口部分需要真实 `HWND`，测试套件不构造窗口；此处验证无需窗口的设备协商与纯逻辑决策。
    #[test]
    fn swapchain_recreate_retires_the_old_handle_cleanly() {
        // 呈现模式与尺寸是决定驱动是否接受新交换链的两个核心决策。
        assert_eq!(
            resolve_present_mode(PresentMode::FifoLatestReady, false),
            vk::PresentModeKHR::FIFO
        );
        let extent = resolve_extent((1280, 720), (0, 0), (u32::MAX, u32::MAX), 640, 360);
        assert_eq!((extent.width, extent.height), (1280, 720));

        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            // 无 GPU 环境：仅执行上方的纯逻辑断言即可。
            return;
        };

        // 测试设备未请求 `VK_EXT_swapchain_maintenance1`，因此不得声明该特性已启用：
        // 该标志反映的是特性启用状态，而非扩展名存在与否。
        assert!(!gpu
            .enabled_device_extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1"));
        assert!(
            !gpu.features.swapchain_maintenance1,
            "the feature must not be enabled without the extension"
        );

        // 表面维护扩展依赖链引入的实例扩展，仅在请求该设备扩展时才会被添加。
        assert!(!gpu
            .enabled_instance_extensions
            .iter()
            .any(|name| name == "VK_EXT_surface_maintenance1"));

        gpu.destroy();
    }
}

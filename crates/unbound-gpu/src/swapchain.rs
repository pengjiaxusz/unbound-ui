//! 低延迟窗口呈现的交换链策略（参见 `docs/architecture/gpu-renderer.md`，M4）。
//!
//! 本 crate 的渲染器刻意保持无表面（surface-free）：所有测试均渲染至离屏图像，
//! 从而使 GPU 测试套件可在 CI 上运行。此处存放并供平台层使用的，是呈现策略本身：
//! 哪些设备扩展必需、默认采用何种呈现模式，以及窗口缩放时如何处理以避免黑边或拉伸。
//!
//! `VK_EXT_swapchain_maintenance1` 尤为关键：它允许应用向操作系统提交围栏，
//! 确保已销毁的交换链在仍在扫描输出时不被复用，并允许指定交换链尺寸与窗口不匹配时的
//! 缩放方式。
//!
//! 在保留垂直同步的同时呈现最新完成图像的 `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`
//! 模式来自另一扩展 `VK_EXT_present_mode_fifo_latest_ready`（已晋升为
//! `VK_KHR_present_mode_fifo_latest_ready`）。此处策略仅表达*偏好*；
//! 实际 Vulkan 模式在 [`crate::surface`] 中结合驱动真实能力解析。

#![forbid(unsafe_op_in_unsafe_fn)]

/// 窗口化后端在渲染器基础扩展集之外额外所需的扩展。
pub const REQUIRED_SWAPCHAIN_EXTENSIONS: [&str; 2] =
    ["VK_KHR_swapchain", "VK_EXT_swapchain_maintenance1"];

/// 当交换链尺寸与窗口不再匹配时（重建交换链前的缩放过渡帧），呈现引擎应采用的缩放策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeBehaviour {
    /// 拉伸至窗口尺寸：无黑边，仅单帧存在轻微纵横比失真。
    Stretch,
    /// 保持纵横比：可能出现黑边，无失真。
    PreserveAspect,
    /// 永不缩放：将旧图像锚定在左上角，在交换链重建前由新窗口区域显示清屏色。
    ///
    /// 拖拽缩放时的拉伸帧正是用户看到的“果冻感”；而锚定单帧几乎不可察觉，
    /// 因为紧接着的下一帧已按新尺寸渲染完成。
    OneToOne,
}

/// 请求使用的呈现模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// 无撕裂，配合 `maintenance1` 可获得最低延迟。
    FifoLatestReady,
    /// 无撕裂，但可能排队缓冲多帧。
    Fifo,
    /// 允许撕裂，延迟最低。
    Immediate,
}

impl PresentMode {
    /// 根据表面支持情况决定请求的呈现模式。
    ///
    /// 当维护扩展栈存在时优先选择 `FifoLatestReady`；`Fifo` 是所有 Vulkan 实现
    /// 必须支持的唯一模式，因此作为安全回退。该偏好会在 [`crate::surface`] 中
    /// 结合驱动实际支持的模式解析为具体的 `VkPresentModeKHR`。
    ///
    /// `allow_tearing` 优先于垂直同步：无帧率上限运行（`0` FPS 限制）意味着
    /// 希望受 GPU 性能而非显示器刷新率限制，而 FIFO 会将其静默锁死在 60 Hz。
    #[must_use]
    pub fn choose(has_maintenance1: bool, allow_tearing: bool) -> Self {
        if allow_tearing {
            Self::Immediate
        } else if have_extension(&["VK_EXT_swapchain_maintenance1"], has_maintenance1) {
            Self::FifoLatestReady
        } else {
            Self::Fifo
        }
    }

    /// 用于日志输出的 Vulkan 模式名称。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::FifoLatestReady => "VK_PRESENT_MODE_FIFO_LATEST_READY_EXT",
            Self::Fifo => "VK_PRESENT_MODE_FIFO_KHR",
            Self::Immediate => "VK_PRESENT_MODE_IMMEDIATE_KHR",
        }
    }
}

fn have_extension(_names: &[&str], available: bool) -> bool {
    available
}

/// 根据设备能力推导出的呈现策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentConfig {
    pub mode: PresentMode,
    pub resize: ResizeBehaviour,
    /// 请求呈现围栏，确保仅当 GPU 完成扫描输出后才销毁交换链。
    pub present_fence: bool,
    /// 阻塞等待直至画面真正呈现到屏幕。
    ///
    /// 这是 `VK_KHR_present_wait` 的低延迟路径：CPU 等待真实呈现而非仅等待渲染完成，
    /// 从而在用户刚能看到上一帧的时刻采样输入队列。
    pub wait_for_present: bool,
}

impl PresentConfig {
    /// 根据设备实际暴露的扩展构建呈现策略。
    #[must_use]
    pub fn from_capabilities(extensions: &[String], allow_tearing: bool) -> Self {
        let has_maintenance1 = extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1");
        let has_present_wait = extensions.iter().any(|name| name == "VK_KHR_present_wait");
        Self {
            mode: PresentMode::choose(has_maintenance1, allow_tearing),
            // 在尺寸变更与交换链重建之间的单帧内，将旧图像锚定在左上角而非拉伸铺满
            // 新窗口（参见 `docs/architecture/gpu-renderer.md`）。
            resize: ResizeBehaviour::OneToOne,
            present_fence: has_maintenance1,
            // 阻塞等待呈现会将无上限运行锁死在显示器刷新率上，而传入 `0` 正是为了避免此事。
            wait_for_present: has_present_wait && !allow_tearing,
        }
    }

    /// `extensions` 中仍缺失的扩展列表。
    #[must_use]
    pub fn missing_extensions(extensions: &[String]) -> Vec<&'static str> {
        REQUIRED_SWAPCHAIN_EXTENSIONS
            .iter()
            .copied()
            .filter(|required| !extensions.iter().any(|name| name == required))
            .collect()
    }
}

/// 便捷方法：表面扩展栈是否完全受支持？
#[must_use]
pub fn is_supported(extensions: &[String]) -> bool {
    PresentConfig::missing_extensions(extensions).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exts(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn maintenance1_enables_the_low_latency_mode() {
        let config = PresentConfig::from_capabilities(
            &exts(&["VK_KHR_swapchain", "VK_EXT_swapchain_maintenance1"]),
            false,
        );
        assert_eq!(config.mode, PresentMode::FifoLatestReady);
        assert!(config.present_fence);
        assert_eq!(config.resize, ResizeBehaviour::OneToOne);
    }

    #[test]
    fn without_maintenance1_it_falls_back_to_fifo() {
        let config = PresentConfig::from_capabilities(&exts(&["VK_KHR_swapchain"]), false);
        assert_eq!(config.mode, PresentMode::Fifo);
        assert!(!config.present_fence);
    }

    /// 低延迟呈现路径仅在支持 `VK_KHR_present_wait` 时可用；缺乏该扩展时 CPU 无法获知图像何时上屏。
    #[test]
    fn the_present_wait_path_needs_its_extension() {
        let without = PresentConfig::from_capabilities(&exts(&["VK_KHR_swapchain"]), false);
        assert!(!without.wait_for_present);

        let with = PresentConfig::from_capabilities(
            &exts(&["VK_KHR_swapchain", "VK_KHR_present_wait"]),
            false,
        );
        assert!(with.wait_for_present);
    }

    #[test]
    fn tearing_is_only_used_when_allowed() {
        assert_eq!(PresentMode::choose(false, true), PresentMode::Immediate);
        assert_eq!(PresentMode::choose(false, false), PresentMode::Fifo);
    }

    #[test]
    fn missing_extensions_are_reported() {
        let missing = PresentConfig::missing_extensions(&exts(&["VK_KHR_surface"]));
        assert_eq!(
            missing,
            vec!["VK_KHR_swapchain", "VK_EXT_swapchain_maintenance1"]
        );
        assert!(!is_supported(&exts(&["VK_KHR_surface"])));
    }

    #[test]
    fn a_complete_stack_is_supported() {
        let extensions = exts(&[
            "VK_KHR_surface",
            "VK_KHR_swapchain",
            "VK_EXT_swapchain_maintenance1",
        ]);
        assert!(is_supported(&extensions));
        assert!(PresentConfig::missing_extensions(&extensions).is_empty());
    }

    #[test]
    fn present_mode_names_are_vulkan_strings() {
        assert_eq!(
            PresentMode::FifoLatestReady.name(),
            "VK_PRESENT_MODE_FIFO_LATEST_READY_EXT"
        );
        assert_eq!(PresentMode::Fifo.name(), "VK_PRESENT_MODE_FIFO_KHR");
    }
}

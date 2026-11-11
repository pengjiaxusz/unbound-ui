//! Swapchain policy for low-latency windowed presentation (`docs` §2.3, M4).
//!
//! The renderer in this crate is deliberately surface-free: every test renders
//! into an offscreen image, which is what makes the GPU suite runnable on CI.
//! What *can* live here — and what the platform layer needs — is the policy:
//! which device extensions are mandatory, which present mode is the right
//! default, and how a resize must behave so the user never sees a black border
//! or a stretched frame.
//!
//! `VK_EXT_swapchain_maintenance1` is the important one. It allows
//! `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`, the mode that keeps vsync but
//! presents the newest completed image instead of queueing stale ones — the
//! difference between 3 frames of input lag and 1. It also lets the application
//! hand the OS a fence so destroyed swapchains are not reused while still
//! scanned out.

#![forbid(unsafe_op_in_unsafe_fn)]

/// Extensions the windowed backend requires on top of the renderer's own set.
pub const REQUIRED_SWAPCHAIN_EXTENSIONS: [&str; 2] =
    ["VK_KHR_swapchain", "VK_EXT_swapchain_maintenance1"];

/// How the presentation engine must scale a swapchain whose size no longer
/// matches the window (the resize frame, before the swapchain is recreated).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeBehaviour {
    /// Scale to the window: no black bars, slight aspect distortion for one frame.
    Stretch,
    /// Keep the aspect ratio: letterboxing, no distortion.
    PreserveAspect,
}

/// Which present mode to ask for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// Tear-free, lowest latency available with `maintenance1`.
    FifoLatestReady,
    /// Tear-free, but may queue frames.
    Fifo,
    /// Tearing, lowest latency.
    Immediate,
}

impl PresentMode {
    /// The mode to request given what the surface reports.
    ///
    /// `FifoLatestReady` is preferred whenever `VK_EXT_swapchain_maintenance1` is
    /// present; `Fifo` is the only mode every Vulkan implementation must support,
    /// so it is the safe fallback.
    #[must_use]
    pub fn choose(has_maintenance1: bool, allow_tearing: bool) -> Self {
        if have_extension(&["VK_EXT_swapchain_maintenance1"], has_maintenance1) {
            Self::FifoLatestReady
        } else if allow_tearing {
            Self::Immediate
        } else {
            Self::Fifo
        }
    }

    /// Vulkan name, for logs.
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

/// Presentation policy derived from device capabilities.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentConfig {
    pub mode: PresentMode,
    pub resize: ResizeBehaviour,
    /// Ask for a present fence so a swapchain is only destroyed once the GPU is
    /// done scanning it out.
    pub present_fence: bool,
    /// Block until the presentation has actually reached the screen.
    ///
    /// This is the low-latency path of `VK_KHR_present_wait`: the CPU waits
    /// for the real present instead of for the render, so the input queue is
    /// sampled at the moment the user can see the frame.
    pub wait_for_present: bool,
}

impl PresentConfig {
    /// Build the policy from the extensions the device actually exposes.
    #[must_use]
    pub fn from_capabilities(extensions: &[String], allow_tearing: bool) -> Self {
        let has_maintenance1 = extensions
            .iter()
            .any(|name| name == "VK_EXT_swapchain_maintenance1");
        let has_present_wait = extensions.iter().any(|name| name == "VK_KHR_present_wait");
        Self {
            mode: PresentMode::choose(has_maintenance1, allow_tearing),
            // Stretch during a resize: a letterboxed frame reads as a bug to
            // users, a 1-frame aspect change does not (`docs` §2.3).
            resize: ResizeBehaviour::Stretch,
            present_fence: has_maintenance1,
            wait_for_present: has_present_wait,
        }
    }

    /// Extensions still missing from `extensions`.
    #[must_use]
    pub fn missing_extensions(extensions: &[String]) -> Vec<&'static str> {
        REQUIRED_SWAPCHAIN_EXTENSIONS
            .iter()
            .copied()
            .filter(|required| !extensions.iter().any(|name| name == required))
            .collect()
    }
}

/// Convenience: is the surface stack fully supported?
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
        assert_eq!(config.resize, ResizeBehaviour::Stretch);
    }

    #[test]
    fn without_maintenance1_it_falls_back_to_fifo() {
        let config = PresentConfig::from_capabilities(&exts(&["VK_KHR_swapchain"]), false);
        assert_eq!(config.mode, PresentMode::Fifo);
        assert!(!config.present_fence);
    }

    /// The low-latency present path is only possible with
    /// `VK_KHR_present_wait`; without it the CPU cannot know when the image
    /// reached the screen.
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

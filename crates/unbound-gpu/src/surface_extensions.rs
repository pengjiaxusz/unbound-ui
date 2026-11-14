//! 根据显示句柄推导窗口系统所需的 Vulkan 实例扩展。
//!
//! `VK_KHR_surface` 始终必需；对应的平台扩展完全取决于窗口的创建方式。
//! 过去硬编码 Windows 扩展对导致旧外壳只能在 Windows 运行——因为在窗口
//! 显示句柄尚未提供信息前就创建了实例。
//!
//! [`required_instance_extensions`] 是仅依赖句柄的纯函数，因此可在包括无显示服务器
//! 的 CI 在内的所有平台上测试。

use raw_window_handle::RawDisplayHandle;

/// 向 `display` 呈现画面时实例必须启用的扩展列表。
///
/// 返回列表具有确定性且已去重，且始终以 `VK_KHR_surface` 开头。
#[must_use]
pub fn required_instance_extensions(display: RawDisplayHandle) -> Vec<&'static str> {
    let mut extensions = vec![name(ash::khr::surface::NAME)];
    let platform = match display {
        RawDisplayHandle::Windows(_) => Some(ash::khr::win32_surface::NAME),
        RawDisplayHandle::Xlib(_) => Some(ash::khr::xlib_surface::NAME),
        RawDisplayHandle::Xcb(_) => Some(ash::khr::xcb_surface::NAME),
        RawDisplayHandle::Wayland(_) => Some(ash::khr::wayland_surface::NAME),
        RawDisplayHandle::Android(_) => Some(ash::khr::android_surface::NAME),
        RawDisplayHandle::AppKit(_) => Some(ash::ext::metal_surface::NAME),
        _ => None,
    };
    if let Some(platform) = platform.map(name) {
        if !extensions.contains(&platform) {
            extensions.push(platform);
        }
    }
    extensions
}

/// Ash 将扩展名暴露为 `&'static CStr`；公开 API 统一使用字符串切片。
fn name(raw: &'static core::ffi::CStr) -> &'static str {
    raw.to_str().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use core::ptr::NonNull;

    use super::*;
    use raw_window_handle::{
        AndroidDisplayHandle, AppKitDisplayHandle, WaylandDisplayHandle, WebDisplayHandle,
        WindowsDisplayHandle, XcbDisplayHandle, XlibDisplayHandle,
    };

    #[test]
    fn windows_asks_for_the_win32_surface_extension() {
        let extensions =
            required_instance_extensions(RawDisplayHandle::Windows(WindowsDisplayHandle::new()));
        assert_eq!(extensions, vec!["VK_KHR_surface", "VK_KHR_win32_surface"]);
    }

    #[test]
    fn every_unix_backend_gets_its_own_extension() {
        assert!(
            required_instance_extensions(RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0)))
                .contains(&"VK_KHR_xlib_surface")
        );
        assert!(
            required_instance_extensions(RawDisplayHandle::Xcb(XcbDisplayHandle::new(None, 0)))
                .contains(&"VK_KHR_xcb_surface")
        );
        assert!(required_instance_extensions(RawDisplayHandle::Wayland(
            WaylandDisplayHandle::new(NonNull::dangling())
        ))
        .contains(&"VK_KHR_wayland_surface"));
    }

    #[test]
    fn macos_and_android_are_covered() {
        assert!(
            required_instance_extensions(RawDisplayHandle::AppKit(AppKitDisplayHandle::new()))
                .contains(&"VK_EXT_metal_surface")
        );
        assert!(required_instance_extensions(RawDisplayHandle::Android(
            AndroidDisplayHandle::new()
        ))
        .contains(&"VK_KHR_android_surface"));
    }

    #[test]
    fn a_web_display_still_gets_the_base_surface_extension() {
        let extensions =
            required_instance_extensions(RawDisplayHandle::Web(WebDisplayHandle::new()));
        assert_eq!(extensions, vec!["VK_KHR_surface"]);
    }
}

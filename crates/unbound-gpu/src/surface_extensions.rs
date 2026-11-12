//! Window-system instance extensions, derived from a display handle.
//!
//! `VK_KHR_surface` is always needed; the platform counterpart depends entirely
//! on how the window was created. Hard-coding the Windows pair is what made the
//! old shell Windows-only — the instance was created before the window's
//! display handle had a chance to vote.
//!
//! [`required_instance_extensions`] is a pure function of the handle so it can
//! be tested on every platform, including CI with no display server.

use raw_window_handle::RawDisplayHandle;

/// The extensions an instance must enable to present to `display`.
///
/// The list is deterministic and deduplicated, and always begins with
/// `VK_KHR_surface`.
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

/// Ash exposes extension names as `&'static CStr`; the public API is text.
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

//! 跨平台操作系统「减弱动画（Reduced Motion）」无障碍偏好查询。
//!
//! * Windows：调用 `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, ...)` 读取系统
//!   「显示动画」开关；关闭（`0`）时返回 `true`（偏好减弱动画）。
//! * Linux：支持环境变量 `UNBOUND_REDUCED_MOTION` 显式覆写（`1`/`true` 开启、`0`/`false` 关闭），
//!   未设置时尝试读取 GNOME `org.gnome.desktop.interface enable-animations` 偏好并缓存结果，
//!   不可用时默认返回 `false`（正常播放动画）。
//! * 其它平台：默认返回 `false`。

/// 解析环境变量 `UNBOUND_REDUCED_MOTION` 取值。
#[must_use]
pub fn parse_reduced_motion_env(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// 解析 GNOME `gsettings get org.gnome.desktop.interface enable-animations` 输出。
///
/// 返回 `Some(true)` 表示用户偏好减弱动画（即 `enable-animations == false`）；
/// 返回 `Some(false)` 表示允许正常动画（`enable-animations == true`）。
#[must_use]
pub fn parse_gnome_enable_animations(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

/// 查询当前操作系统是否开启了「减弱动画（Reduced Motion）」无障碍偏好。
///
/// 返回 `true` 时，框架级主题过渡与动效应自动降级为瞬切。
#[must_use]
pub fn prefers_reduced_motion() -> bool {
    if let Ok(raw) = std::env::var("UNBOUND_REDUCED_MOTION") {
        if let Some(flag) = parse_reduced_motion_env(&raw) {
            return flag;
        }
    }

    #[cfg(windows)]
    {
        win32::prefers_reduced_motion()
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        linux::prefers_reduced_motion()
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        false
    }
}

#[cfg(windows)]
mod win32 {
    use core::ffi::c_void;

    use ::windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    pub(super) fn prefers_reduced_motion() -> bool {
        let mut client_area_animation: i32 = 1;
        // SAFETY: `client_area_animation` 为栈上有效 `BOOL`（`i32`），满足
        // `SPI_GETCLIENTAREAANIMATION` 的输出缓冲区要求。
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&raw mut client_area_animation).cast::<c_void>()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        ok.is_ok() && client_area_animation == 0
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod linux {
    use std::process::Command;
    use std::sync::OnceLock;

    use super::parse_gnome_enable_animations;

    pub(super) fn prefers_reduced_motion() -> bool {
        static CACHED: OnceLock<bool> = OnceLock::new();
        *CACHED.get_or_init(|| {
            let output = Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "enable-animations"])
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    parse_gnome_enable_animations(&stdout).unwrap_or(false)
                }
                _ => false,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reduced_motion_env_handles_common_boolean_spellings() {
        assert_eq!(parse_reduced_motion_env("1"), Some(true));
        assert_eq!(parse_reduced_motion_env("true"), Some(true));
        assert_eq!(parse_reduced_motion_env("ON"), Some(true));
        assert_eq!(parse_reduced_motion_env("0"), Some(false));
        assert_eq!(parse_reduced_motion_env("false"), Some(false));
        assert_eq!(parse_reduced_motion_env("off"), Some(false));
        assert_eq!(parse_reduced_motion_env("unknown"), None);
    }

    #[test]
    fn parse_gnome_enable_animations_inverts_flag_for_reduced_motion() {
        assert_eq!(parse_gnome_enable_animations("false\n"), Some(true));
        assert_eq!(parse_gnome_enable_animations("true\n"), Some(false));
        assert_eq!(parse_gnome_enable_animations("invalid"), None);
    }
}

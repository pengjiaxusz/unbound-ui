//! Windows 构建号探测。
//!
//! Win11 原生云母的启用门槛依赖真实构建号（`22621` 及以上），因此这里走
//! `ntdll!RtlGetVersion`：它报告内核真实版本，不像 `GetVersionEx` 那样受
//! 应用兼容性清单影响而撒谎。非 Windows 环境返回“未知”。

#[cfg(windows)]
fn probe() -> Option<u32> {
    use ::windows::Wdk::System::SystemServices::RtlGetVersion;
    use ::windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: core::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` 是有效且完整初始化的结构体，大小字段已按 API 要求填写；
    // `RtlGetVersion` 只写入该结构体，不会保存指针。
    let status = unsafe { RtlGetVersion(&raw mut info) };
    if status.0 >= 0 {
        Some(info.dwBuildNumber)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn probe() -> Option<u32> {
    None
}

/// 返回当前 Windows 的构建号；非 Windows 或探测失败时返回 `None`。
///
/// 例如 Windows 11 22H2 返回 `Some(22621)`，Windows 10 21H2 返回 `Some(19044)`。
#[must_use]
pub fn windows_build() -> Option<u32> {
    probe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_probe_reports_a_real_build_number() {
        let build = windows_build().expect("RtlGetVersion must report the build number on Windows");
        assert!(
            build >= 10,
            "the reported Windows build {build} must be a plausible build number"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_probe_reports_unknown() {
        assert_eq!(
            windows_build(),
            None,
            "non-Windows hosts must report an unknown build number"
        );
    }
}

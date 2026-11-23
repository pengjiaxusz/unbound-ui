//! 操作系统主题强调色读取（Windows DWM / Accent 注册表与跨平台暗色回退）。
//!
//! * Windows 平台优先从 `HKCU\Software\Microsoft\Windows\DWM\AccentColor`（`0xAABBGGRR`）读取
//!   用户在系统「个性化 → 颜色」中选择的精确强调色，失败时回退至同路径下的
//!   `ColorizationColor`（`0xAARRGGBB`）。
//! * 非 Windows 平台或系统未配置有效强调色时，提供符合 Win11 暗色主题可读性的默认强调色回退
//!   （[`DEFAULT_SYSTEM_ACCENT_SRGB`] = `#60CDFF`）。

use unbound_math::color::Color;

/// 非 Windows 平台或读取失败时的默认暗色主题强调色（sRGB `#60CDFF`）。
pub const DEFAULT_SYSTEM_ACCENT_SRGB: [u8; 4] = [96, 205, 255, 255];

/// 将 Windows 注册表 `DWM\AccentColor` 的 `0xAABBGGRR` 整数解码为不透明 sRGB8 RGBA。
#[must_use]
pub const fn decode_abgr_accent(abgr: u32) -> [u8; 4] {
    let r = (abgr & 0xFF) as u8;
    let g = ((abgr >> 8) & 0xFF) as u8;
    let b = ((abgr >> 16) & 0xFF) as u8;
    [r, g, b, 255]
}

/// 将 Windows `DWM\ColorizationColor` 的 `0xAARRGGBB` 整数解码为不透明 sRGB8 RGBA。
#[must_use]
pub const fn decode_argb_colorization(argb: u32) -> [u8; 4] {
    let r = ((argb >> 16) & 0xFF) as u8;
    let g = ((argb >> 8) & 0xFF) as u8;
    let b = (argb & 0xFF) as u8;
    [r, g, b, 255]
}

/// 尝试读取当前操作系统的主题强调色（线性 RGBA）。
///
/// - Windows：读取 `HKCU\Software\Microsoft\Windows\DWM` 下的 `AccentColor`，
///   若不可用则回退读取 `ColorizationColor`；若值为 `0` 或读取失败返回 `None`。
/// - 非 Windows：恒返回 `None`（调用方可使用 [`system_accent_color`] 获取默认回退色）。
#[must_use]
pub fn read_system_accent_color() -> Option<Color> {
    #[cfg(windows)]
    {
        win32::read_dwm_accent_srgb8().map(|[r, g, b, a]| Color::from_srgb8(r, g, b, a))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 返回当前系统主题强调色；在非 Windows 平台或读取失败时返回合理暗色主题默认回退色。
#[must_use]
pub fn system_accent_color() -> Color {
    read_system_accent_color().unwrap_or_else(|| {
        let [r, g, b, a] = DEFAULT_SYSTEM_ACCENT_SRGB;
        Color::from_srgb8(r, g, b, a)
    })
}

#[cfg(windows)]
mod win32 {
    use core::ffi::c_void;

    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    use super::{decode_abgr_accent, decode_argb_colorization};

    fn read_dwm_dword(value_name: PCWSTR) -> Option<u32> {
        let mut data: u32 = 0;
        let mut size = core::mem::size_of::<u32>() as u32;
        // SAFETY: `HKEY_CURRENT_USER` 与静态宽字符串字面量均有效；`data` 与 `size` 指向栈上有效内存，
        // `RRF_RT_REG_DWORD` 保证内核最多写入 4 字节。
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\DWM"),
                value_name,
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut data).cast::<c_void>()),
                Some(&raw mut size),
            )
        };
        if status.is_ok() && data != 0 {
            Some(data)
        } else {
            None
        }
    }

    pub(super) fn read_dwm_accent_srgb8() -> Option<[u8; 4]> {
        if let Some(abgr) = read_dwm_dword(w!("AccentColor")) {
            return Some(decode_abgr_accent(abgr));
        }
        if let Some(argb) = read_dwm_dword(w!("ColorizationColor")) {
            return Some(decode_argb_colorization(argb));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_abgr_accent_extracts_rgb_with_opaque_alpha() {
        // Windows AccentColor 典型紫罗兰值 0xFF_E5_9A_DB => R=0xDB, G=0x9A, B=0xE5
        assert_eq!(decode_abgr_accent(0xFFE5_9ADB), [0xDB, 0x9A, 0xE5, 255]);
        // 即使源高位 alpha 为 0 也强制输出不透明 alpha=255
        assert_eq!(decode_abgr_accent(0x0011_2233), [0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn decode_argb_colorization_extracts_rgb_with_opaque_alpha() {
        // DWM ColorizationColor 0xC4_00_78_D4 => R=0x00, G=0x78, B=0xD4
        assert_eq!(
            decode_argb_colorization(0xC400_78D4),
            [0x00, 0x78, 0xD4, 255]
        );
    }

    #[test]
    fn system_accent_color_returns_finite_opaque_color() {
        let color = system_accent_color();
        assert!(color.is_finite());
        assert!((color.a - 1.0).abs() < 1e-4);
        #[cfg(not(windows))]
        {
            assert!(read_system_accent_color().is_none());
            let [r, g, b, a] = DEFAULT_SYSTEM_ACCENT_SRGB;
            assert_eq!(color, Color::from_srgb8(r, g, b, a));
        }
    }
}

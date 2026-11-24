//! `xdg-desktop-portal` 协议解析与外观快照纯逻辑（跨平台编译，无 D-Bus 即可单测）。

use std::path::PathBuf;

/// `org.freedesktop.appearance` 的 `color-scheme` 枚举值。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PortalColorScheme {
    /// 未指定偏好（值 `0` 或未知值）。
    #[default]
    NoPreference,
    /// 偏好深色主题（值 `1`）。
    PreferDark,
    /// 偏好浅色主题（值 `2`）。
    PreferLight,
}

impl PortalColorScheme {
    /// 转换为布尔深色标志（`NoPreference` 返回 `None`）。
    #[must_use]
    pub const fn is_dark(self) -> Option<bool> {
        match self {
            Self::NoPreference => None,
            Self::PreferDark => Some(true),
            Self::PreferLight => Some(false),
        }
    }
}

/// 解析 `org.freedesktop.portal.Settings` 返回的 `color-scheme` 无符号整数。
#[must_use]
pub const fn parse_portal_color_scheme(raw: u32) -> PortalColorScheme {
    match raw {
        1 => PortalColorScheme::PreferDark,
        2 => PortalColorScheme::PreferLight,
        _ => PortalColorScheme::NoPreference,
    }
}

/// 解析 `org.freedesktop.portal.Settings` 返回的 `accent-color` `(f64, f64, f64)` 浮点 RGB。
///
/// 任一分量非有限值或超出 `[0.0, 1.0]` 区间时返回 `None`，避免畸形 portal 响应污染主题。
#[must_use]
pub fn parse_portal_accent_color(r: f64, g: f64, b: f64) -> Option<[u8; 4]> {
    for c in [r, g, b] {
        if !c.is_finite() || !(0.0..=1.0).contains(&c) {
            return None;
        }
    }
    let to_u8 = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Some([to_u8(r), to_u8(g), to_u8(b), 255])
}

/// `xdg-desktop-portal` 文件选择器过滤器项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalFileFilter {
    /// 过滤器显示名称，例如 `"PNG Image (*.png)"`。
    pub name: String,
    /// `(规则类型, 通配模式)` 列表；类型 `0` 表示 glob 通配符。
    pub patterns: Vec<(u32, String)>,
}

/// 将 [`crate::FileDialogConfig::filters`] 转换为 `xdg-desktop-portal` `FileChooser` 过滤器列表。
///
/// 支持分号分隔的多模式字符串（如 `"*.png;*.jpg"`），跳过空白模式。
#[must_use]
pub fn convert_dialog_filters(filters: &[(String, String)]) -> Vec<PortalFileFilter> {
    filters
        .iter()
        .filter_map(|(name, raw_patterns)| {
            let patterns: Vec<(u32, String)> = raw_patterns
                .split(';')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| (0u32, p.to_owned()))
                .collect();
            if patterns.is_empty() {
                None
            } else {
                Some(PortalFileFilter {
                    name: name.clone(),
                    patterns,
                })
            }
        })
        .collect()
}

/// 十六进制 ASCII 字符转半字节值。
const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 对 URI 路径段执行百分号解码（支持空格 `%20` 与多字节 UTF-8 中文路径）。
#[must_use]
pub fn percent_decode_bytes(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// 将 `file://` URI 解码为本地文件系统路径；非本地 URI 或空路径返回 `None`。
#[must_use]
pub fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    let rest = trimmed.strip_prefix("file://")?;
    let path_part = if let Some(after_local) = rest.strip_prefix("localhost/") {
        format!("/{after_local}")
    } else if rest.starts_with('/') {
        rest.to_owned()
    } else {
        // 非本机主机名的远程 file:// URI 不作为本地路径处理
        return None;
    };
    let decoded_bytes = percent_decode_bytes(&path_part);
    let decoded = String::from_utf8(decoded_bytes).ok()?;
    if decoded.is_empty() {
        None
    } else {
        Some(PathBuf::from(decoded))
    }
}

/// Linux 桌面外观缓存快照（由后台 D-Bus 线程刷新，UI 线程零阻塞读取）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LinuxAppearanceSnapshot {
    /// 系统深色模式偏好（`None` 表示未知或无偏好）。
    pub dark_mode: Option<bool>,
    /// 系统强调色不透明 sRGB8（`None` 表示未配置或无 portal）。
    pub accent_srgb8: Option<[u8; 4]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_color_scheme_parses_standard_values() {
        assert_eq!(
            parse_portal_color_scheme(0),
            PortalColorScheme::NoPreference
        );
        assert_eq!(parse_portal_color_scheme(1), PortalColorScheme::PreferDark);
        assert_eq!(parse_portal_color_scheme(2), PortalColorScheme::PreferLight);
        assert_eq!(
            parse_portal_color_scheme(99),
            PortalColorScheme::NoPreference
        );
        assert_eq!(PortalColorScheme::PreferDark.is_dark(), Some(true));
        assert_eq!(PortalColorScheme::PreferLight.is_dark(), Some(false));
        assert_eq!(PortalColorScheme::NoPreference.is_dark(), None);
    }

    #[test]
    fn portal_accent_color_validates_range_and_converts_to_srgb8() {
        assert_eq!(
            parse_portal_accent_color(0.0, 0.5, 1.0),
            Some([0, 128, 255, 255])
        );
        assert_eq!(parse_portal_accent_color(-0.1, 0.5, 0.5), None);
        assert_eq!(parse_portal_accent_color(0.2, 1.2, 0.5), None);
        assert_eq!(parse_portal_accent_color(f64::NAN, 0.5, 0.5), None);
    }

    #[test]
    fn decode_file_uri_handles_spaces_cjk_localhost_and_rejects_non_file() {
        assert_eq!(
            decode_file_uri("file:///home/user/My%20Documents/%E6%B5%8B%E8%AF%95.png"),
            Some(PathBuf::from("/home/user/My Documents/测试.png")) // lang-ok: CJK UTF-8 test fixture
        );
        assert_eq!(
            decode_file_uri("file://localhost/tmp/demo.txt"),
            Some(PathBuf::from("/tmp/demo.txt"))
        );
        assert_eq!(decode_file_uri("https://example.com/a.png"), None);
        assert_eq!(decode_file_uri("file://remote-host/share/a.txt"), None);
    }

    #[test]
    fn convert_dialog_filters_splits_semicolon_patterns() {
        let filters = vec![
            ("Images".to_string(), "*.png; *.jpg; ".to_string()),
            ("Empty".to_string(), "   ".to_string()),
        ];
        let out = convert_dialog_filters(&filters);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Images");
        assert_eq!(
            out[0].patterns,
            vec![(0, "*.png".to_string()), (0, "*.jpg".to_string())]
        );
    }
}

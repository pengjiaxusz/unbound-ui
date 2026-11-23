//! 桌面壁纸读取：路径解析、解码缓存与主屏逻辑尺寸。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use image::{DynamicImage, ImageBuffer, Rgba};

/// 显式指定自绘云母壁纸路径的环境变量名（Linux 三级回退的第一级）。
pub const WALLPAPER_ENV_VAR: &str = "UNBOUND_WALLPAPER_PATH";

/// Linux / WSLg 未注入主屏几何时的默认主屏物理尺寸（1080p）。
#[cfg(not(target_os = "windows"))]
const DEFAULT_NON_WINDOWS_PRIMARY_SCREEN: (u32, u32) = (1920, 1080);

/// 内置程序化渐变底图的稳定来源指纹。
#[cfg(not(target_os = "windows"))]
const PROCEDURAL_WALLPAPER_STAMP: WallpaperStamp = WallpaperStamp {
    path_hash: 0x554E_424F_554E_4431,
    modified_secs: 1,
    modified_nanos: 0,
    len: 1280 * 720 * 4,
};

/// 外部窗口宿主（如 Wayland `wl_output`）注入的主屏物理尺寸与缩放因子。
#[derive(Copy, Clone, Debug, PartialEq)]
struct PrimaryScreenMetrics {
    width: u32,
    height: u32,
    scale: f32,
}

static PRIMARY_SCREEN_OVERRIDE: OnceLock<Mutex<Option<PrimaryScreenMetrics>>> = OnceLock::new();

/// 注入主屏物理尺寸与缩放因子（供 Wayland `wl_output` 等跨平台窗口宿主回填）。
///
/// 传入无效尺寸或非正缩放时忽略更新；调用 [`clear_primary_screen_metrics`] 可恢复默认行为。
pub fn set_primary_screen_metrics(physical_width: u32, physical_height: u32, scale: f32) {
    if physical_width == 0 || physical_height == 0 || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let lock = PRIMARY_SCREEN_OVERRIDE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(PrimaryScreenMetrics {
            width: physical_width,
            height: physical_height,
            scale,
        });
    }
}

/// 清除外部注入的主屏几何覆盖，回退至平台默认查询或内置默认尺寸。
pub fn clear_primary_screen_metrics() {
    let lock = PRIMARY_SCREEN_OVERRIDE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = None;
    }
}

fn injected_primary_screen_metrics() -> Option<PrimaryScreenMetrics> {
    let lock = PRIMARY_SCREEN_OVERRIDE.get_or_init(|| Mutex::new(None));
    lock.lock().ok().and_then(|guard| *guard)
}

/// 已解码壁纸。按「文件修改时间 + 大小」失效，避免反复解码 4K JPEG。
struct CachedWallpaper {
    modified: SystemTime,
    len: u64,
    image: Arc<DynamicImage>,
}

static CACHE: OnceLock<Mutex<Option<CachedWallpaper>>> = OnceLock::new();
static PROCEDURAL_FALLBACK: OnceLock<Arc<DynamicImage>> = OnceLock::new();

/// Linux 壁纸三级回退的解析来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WallpaperSourceKind {
    /// 第一级：由 `UNBOUND_WALLPAPER_PATH` 环境变量显式指定的文件。
    EnvVar(PathBuf),
    /// 第二级：从 GNOME / KDE 桌面配置读取到的壁纸文件。
    DesktopConfig(PathBuf),
    /// 第三级：无可用桌面壁纸文件时使用的内置程序化渐变底图。
    ProceduralFallback,
}

/// 按三级优先级解析壁纸来源（纯逻辑接缝，便于无头测试）：
/// 1. 环境变量路径（存在且为文件）；
/// 2. 桌面环境配置路径（存在且为文件）；
/// 3. 内置程序化渐变底图。
#[must_use]
pub fn resolve_wallpaper_source_with<F>(
    env_candidate: Option<PathBuf>,
    desktop_candidate: Option<PathBuf>,
    is_file: F,
) -> WallpaperSourceKind
where
    F: Fn(&Path) -> bool,
{
    if let Some(path) = env_candidate {
        if !path.as_os_str().is_empty() && is_file(&path) {
            return WallpaperSourceKind::EnvVar(path);
        }
    }
    if let Some(path) = desktop_candidate {
        if !path.as_os_str().is_empty() && is_file(&path) {
            return WallpaperSourceKind::DesktopConfig(path);
        }
    }
    WallpaperSourceKind::ProceduralFallback
}

/// 解析 GNOME `gsettings` 返回的 `picture-uri` / `picture-uri-dark` 文本为本地路径。
#[must_use]
pub fn parse_gnome_picture_uri(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim().trim_matches('\'').trim_matches('"').trim();
    if trimmed.is_empty() || trimmed == "none" {
        return None;
    }
    let stripped = trimmed
        .strip_prefix("file://")
        .map(percent_decode_path)
        .unwrap_or_else(|| trimmed.to_owned());
    let is_posix_abs = stripped.starts_with('/');
    let path = PathBuf::from(stripped);
    (path.is_absolute() || is_posix_abs).then_some(path)
}

/// 解析 KDE Plasma `plasma-org.kde.plasma.desktop-appletsrc` 中的 `Image=` 壁纸路径。
#[must_use]
pub fn parse_kde_wallpaper_config(content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("Image=") else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let stripped = value
            .strip_prefix("file://")
            .map(percent_decode_path)
            .unwrap_or_else(|| value.to_owned());
        let is_posix_abs = stripped.starts_with('/');
        let path = PathBuf::from(stripped);
        if path.is_absolute() || is_posix_abs {
            return Some(path);
        }
    }
    None
}

fn percent_decode_path(input: &str) -> String {
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
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 生成并缓存内置的轻量程序化柔和渐变底图（1280×720）。
///
/// 采用多瓣柔和色彩渐变（冷蓝、青蓝与暖紫光晕），在经过 `sigma = 16.0` 的云母磨砂
/// 与主题明度压缩后，仍保留清晰的明暗层次与色相过渡，确保 WSLg 无壁纸环境开箱可用。
#[must_use]
pub fn procedural_fallback_wallpaper() -> Arc<DynamicImage> {
    Arc::clone(PROCEDURAL_FALLBACK.get_or_init(|| {
        const W: u32 = 1280;
        const H: u32 = 720;
        let mut buf = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(W, H);
        let inv_w = 1.0 / f32::from(W as u16 - 1);
        let inv_h = 1.0 / f32::from(H as u16 - 1);
        for y in 0..H {
            let v = y as f32 * inv_h;
            for x in 0..W {
                let u = x as f32 * inv_w;
                // 基底斜向冷色调渐变
                let diag = (u * 0.65 + v * 0.35).clamp(0.0, 1.0);
                let mut r = 24.0 + 48.0 * diag;
                let mut g = 46.0 + 72.0 * (1.0 - v * 0.5);
                let mut b = 98.0 + 96.0 * (1.0 - diag * 0.4);

                // 左上青蓝主光晕
                let d1 = (u - 0.28).powi(2) * 1.4 + (v - 0.32).powi(2) * 2.2;
                let lobe1 = (-d1 * 4.5).exp();
                r += 36.0 * lobe1;
                g += 118.0 * lobe1;
                b += 132.0 * lobe1;

                // 右下暖紫/玫红辅光晕，拉出跨屏幕色相与明度带宽
                let d2 = (u - 0.76).powi(2) * 1.6 + (v - 0.68).powi(2) * 2.0;
                let lobe2 = (-d2 * 4.0).exp();
                r += 148.0 * lobe2;
                g += 44.0 * lobe2;
                b += 108.0 * lobe2;

                // 顶部柔和亮带
                let d3 = (u - 0.55).powi(2) * 2.2 + (v - 0.16).powi(2) * 3.5;
                let lobe3 = (-d3 * 5.0).exp();
                r += 64.0 * lobe3;
                g += 86.0 * lobe3;
                b += 92.0 * lobe3;

                buf.put_pixel(
                    x,
                    y,
                    Rgba([
                        r.clamp(0.0, 255.0).round() as u8,
                        g.clamp(0.0, 255.0).round() as u8,
                        b.clamp(0.0, 255.0).round() as u8,
                        255,
                    ]),
                );
            }
        }
        Arc::new(DynamicImage::ImageRgba8(buf))
    }))
}

#[cfg(not(target_os = "windows"))]
fn linux_desktop_wallpaper_path() -> Option<PathBuf> {
    // 1. GNOME（先深色再默认）
    for key in ["picture-uri-dark", "picture-uri"] {
        if let Ok(output) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.background", key])
            .output()
        {
            if output.status.success() {
                if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                    if let Some(path) = parse_gnome_picture_uri(stdout) {
                        if path.is_file() {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }

    // 2. KDE Plasma
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if let Some(cfg_dir) = config_home {
        let kde_rc = cfg_dir.join("plasma-org.kde.plasma.desktop-appletsrc");
        if let Ok(content) = std::fs::read_to_string(kde_rc) {
            if let Some(path) = parse_kde_wallpaper_config(&content) {
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// 当前桌面壁纸的静态文件路径。
///
/// Windows 读取系统壁纸注册/SPI 路径；Linux 按 `UNBOUND_WALLPAPER_PATH` → GNOME/KDE
/// 桌面配置顺序探测。返回 `None` 时，Linux 在 [`load_wallpaper`] 中自动回退至内置程序化渐变底图。
#[must_use]
pub fn wallpaper_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_wallpaper_path()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let env_candidate = std::env::var_os(WALLPAPER_ENV_VAR).map(PathBuf::from);
        if let Some(ref p) = env_candidate {
            if !p.as_os_str().is_empty() && p.is_file() {
                return Some(p.clone());
            }
        }
        let desktop_candidate = linux_desktop_wallpaper_path();
        match resolve_wallpaper_source_with(env_candidate, desktop_candidate, Path::is_file) {
            WallpaperSourceKind::EnvVar(p) | WallpaperSourceKind::DesktopConfig(p) => Some(p),
            WallpaperSourceKind::ProceduralFallback => None,
        }
    }
}

/// 壁纸源指纹：路径、修改时间与文件大小。
///
/// 只用于判断“是否需要重新生成”，不参与纹理内容。幻灯片或动态壁纸拿不到静态
/// 文件时为 `None`，此时与另一个 `None` 视为同一来源。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WallpaperStamp {
    /// 路径哈希。
    pub path_hash: u64,
    /// 修改时间（Unix 纪元秒）。
    pub modified_secs: u64,
    /// 修改时间（纳秒部分）。
    pub modified_nanos: u32,
    /// 文件字节数。
    pub len: u64,
}

/// 读取当前壁纸源指纹；Linux 无静态壁纸文件时返回稳定的内置渐变底图指纹。
#[must_use]
pub fn wallpaper_stamp() -> Option<WallpaperStamp> {
    use std::hash::{Hash, Hasher};

    let Some(path) = wallpaper_path() else {
        #[cfg(not(target_os = "windows"))]
        {
            return Some(PROCEDURAL_WALLPAPER_STAMP);
        }
        #[cfg(target_os = "windows")]
        {
            return None;
        }
    };
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    let since_epoch = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    Some(WallpaperStamp {
        path_hash: hasher.finish(),
        modified_secs: since_epoch.as_secs(),
        modified_nanos: since_epoch.subsec_nanos(),
        len: metadata.len(),
    })
}

/// 读取并解码当前壁纸，命中缓存时零成本返回。
///
/// 解码在锁外进行：4K 壁纸在 debug 构建下可达数百毫秒，不能阻塞同时到达的
/// 第二个请求（主题切换与窗口缩放可能并发触发重建）。
/// 在非 Windows 平台，若壁纸文件缺失或解码失败，自动回退至内置程序化柔和渐变底图。
///
/// # Errors
/// Windows 下路径不可用、读盘失败或图像解码失败时返回可读的错误文本。
pub fn load_wallpaper() -> Result<(Arc<DynamicImage>, SystemTime, u64), String> {
    let Some(path) = wallpaper_path() else {
        #[cfg(not(target_os = "windows"))]
        {
            return Ok((procedural_fallback_wallpaper(), SystemTime::UNIX_EPOCH, 0));
        }
        #[cfg(target_os = "windows")]
        {
            return Err("wallpaper path unavailable".to_string());
        }
    };

    match load_wallpaper_from_path(&path) {
        Ok(result) => Ok(result),
        Err(err) => {
            #[cfg(not(target_os = "windows"))]
            {
                let _ = err;
                Ok((procedural_fallback_wallpaper(), SystemTime::UNIX_EPOCH, 0))
            }
            #[cfg(target_os = "windows")]
            {
                Err(err)
            }
        }
    }
}

fn load_wallpaper_from_path(path: &Path) -> Result<(Arc<DynamicImage>, SystemTime, u64), String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("wallpaper metadata failed: {error}"))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("wallpaper modified time failed: {error}"))?;
    let len = metadata.len();

    let cache = CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = cache
            .lock()
            .map_err(|_| "wallpaper cache poisoned".to_string())?;
        if let Some(entry) = guard.as_ref() {
            if entry.modified == modified && entry.len == len {
                return Ok((Arc::clone(&entry.image), modified, len));
            }
        }
    }

    let bytes = std::fs::read(path)
        .map_err(|error| format!("reading wallpaper [{}] failed: {error}", path.display()))?;
    // `TranscodedWallpaper` 等文件没有扩展名，按内容嗅探格式。
    let decoded = image::load_from_memory(&bytes)
        .map_err(|error| format!("decoding wallpaper [{}] failed: {error}", path.display()))?;

    let mut guard = cache
        .lock()
        .map_err(|_| "wallpaper cache poisoned".to_string())?;
    *guard = Some(CachedWallpaper {
        modified,
        len,
        image: Arc::new(decoded),
    });
    let entry = guard.as_ref().ok_or("wallpaper cache write failed")?;
    Ok((Arc::clone(&entry.image), modified, len))
}

/// 主屏在屏幕坐标系中的物理矩形。
///
/// 主屏原点固定为 `(0, 0)`；返回的是物理像素，与 `WM_MOVE` / `ClientToScreen`
/// 使用的坐标系一致，可直接和窗口客户区矩形求交。
#[must_use]
pub fn primary_screen_rect() -> Option<crate::plan::PixelRect> {
    if let Some(metrics) = injected_primary_screen_metrics() {
        return Some(crate::plan::PixelRect::new(
            0,
            0,
            metrics.width,
            metrics.height,
        ));
    }
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        // SAFETY: 两个调用都是无参数的全局指标查询，不涉及指针或句柄。
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(0) as u32;
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(0) as u32;
        (width > 0 && height > 0).then(|| crate::plan::PixelRect::new(0, 0, width, height))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let (width, height) = DEFAULT_NON_WINDOWS_PRIMARY_SCREEN;
        Some(crate::plan::PixelRect::new(0, 0, width, height))
    }
}

/// 系统 DPI 缩放（`1.0` 对应 96 DPI）。
///
/// 主屏逻辑构图随缩放变化；自绘纹理把该值纳入失效条件。
#[must_use]
pub fn primary_screen_scale() -> f32 {
    if let Some(metrics) = injected_primary_screen_metrics() {
        return metrics.scale;
    }
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::HiDpi::GetDpiForSystem;

        // SAFETY: 无参数的系统指标查询，不涉及指针或句柄。
        let dpi = unsafe { GetDpiForSystem() };
        if dpi > 0 {
            dpi as f32 / 96.0
        } else {
            1.0
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        1.0
    }
}

/// 主屏逻辑尺寸（按系统 DPI 归一化，不含每窗口缩放）。
///
/// Mica 的壁纸取样以**主屏**为参照：Win11 的云母同样只跟随主屏壁纸，
/// 多屏时副屏窗口不会为各自壁纸重新取样。
#[must_use]
pub fn primary_screen_logical_size() -> Option<(u32, u32)> {
    if let Some(metrics) = injected_primary_screen_metrics() {
        let width = (f64::from(metrics.width) / f64::from(metrics.scale)).round() as u32;
        let height = (f64::from(metrics.height) / f64::from(metrics.scale)).round() as u32;
        return (width > 0 && height > 0).then_some((width, height));
    }
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::HiDpi::GetDpiForSystem;
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        // SAFETY: both calls are infallible queries of system-wide metrics and
        // take no pointers or handles.
        let scale = unsafe { GetDpiForSystem() } as f64 / 96.0;
        if scale <= 0.0 {
            return None;
        }
        // SAFETY: `SM_CXSCREEN` / `SM_CYSCREEN` return the primary display size in
        // physical pixels; no pointers are involved.
        let width = (unsafe { GetSystemMetrics(SM_CXSCREEN) } as f64 / scale).round() as u32;
        let height = (unsafe { GetSystemMetrics(SM_CYSCREEN) } as f64 / scale).round() as u32;
        (width > 0 && height > 0).then_some((width, height))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(DEFAULT_NON_WINDOWS_PRIMARY_SCREEN)
    }
}

#[cfg(target_os = "windows")]
fn windows_wallpaper_path() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut buffer = vec![0u16; 2048];
    // SAFETY: `buffer` is a writable UTF-16 buffer of `buffer.len()` elements;
    // SPI_GETDESKWALLPAPER writes at most that many code units and reports failure
    // through the returned `Result`.
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    let path = PathBuf::from(OsString::from_wide(&buffer[..end]));
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mica::{DEFAULT_MAX_WIDTH, DEFAULT_SIGMA};
    use crate::selection::MaterialKind;
    use crate::stats::{texture_statistics, StatisticsRequest};

    #[test]
    fn a_wallpaper_path_is_absolute_when_available() {
        if let Some(path) = wallpaper_path() {
            assert!(path.is_absolute(), "wallpaper path must be absolute");
        }
    }

    #[test]
    fn the_primary_screen_has_a_positive_size_on_windows() {
        #[cfg(target_os = "windows")]
        {
            let (width, height) = primary_screen_logical_size().expect("primary screen size");
            assert!(width > 0 && height > 0);
        }
    }

    #[test]
    fn three_tier_wallpaper_resolution_respects_priority() {
        let env_p = PathBuf::from("/tmp/env-wall.png");
        let desk_p = PathBuf::from("/tmp/desk-wall.jpg");

        // 1. 环境变量存在且有效时优先命中
        assert_eq!(
            resolve_wallpaper_source_with(Some(env_p.clone()), Some(desk_p.clone()), |_| true),
            WallpaperSourceKind::EnvVar(env_p.clone())
        );

        // 2. 环境变量文件不存在时回退至桌面配置
        assert_eq!(
            resolve_wallpaper_source_with(Some(env_p.clone()), Some(desk_p.clone()), |p| p
                == desk_p),
            WallpaperSourceKind::DesktopConfig(desk_p.clone())
        );

        // 3. 均不可用时回退至内置程序化渐变底图
        assert_eq!(
            resolve_wallpaper_source_with(Some(env_p), Some(desk_p), |_| false),
            WallpaperSourceKind::ProceduralFallback
        );
    }

    #[test]
    fn desktop_config_parsers_handle_gnome_and_kde_formats() {
        assert_eq!(
            parse_gnome_picture_uri("'file:///home/user/My%20Pictures/wall.png'\n"),
            Some(PathBuf::from("/home/user/My Pictures/wall.png"))
        );
        assert_eq!(parse_gnome_picture_uri("'none'"), None);

        let kde_rc = "[Containments][1][Wallpaper][org.kde.image][General]\nImage=file:///usr/share/wallpapers/Next/contents/images/1920x1080.png\n";
        assert_eq!(
            parse_kde_wallpaper_config(kde_rc),
            Some(PathBuf::from(
                "/usr/share/wallpapers/Next/contents/images/1920x1080.png"
            ))
        );
    }

    #[test]
    fn procedural_fallback_passes_texture_statistics_for_mica_and_mica_alt() {
        let wallpaper = procedural_fallback_wallpaper();
        for material in [MaterialKind::Mica, MaterialKind::MicaAlt] {
            for dark in [false, true] {
                let stats = texture_statistics(&StatisticsRequest {
                    wallpaper: &wallpaper,
                    screen: (1920, 1080),
                    max_width: DEFAULT_MAX_WIDTH,
                    sigma: DEFAULT_SIGMA,
                    dark,
                    material,
                })
                .expect("procedural fallback must produce valid texture statistics");
                assert!(
                    stats.spatial_gradient_energy > 1.0,
                    "expected non-flat gradient energy for {material:?} dark={dark}, got {}",
                    stats.spatial_gradient_energy
                );
                assert!(
                    stats.luminance_bandwidth > 1.0,
                    "expected visible luminance bandwidth for {material:?} dark={dark}, got {}",
                    stats.luminance_bandwidth
                );
                assert!(
                    stats.hue_fidelity > 0.5,
                    "expected preserved hue fidelity for {material:?} dark={dark}, got {}",
                    stats.hue_fidelity
                );
            }
        }
    }

    #[test]
    fn injected_primary_screen_metrics_override_and_clear_cleanly() {
        set_primary_screen_metrics(2560, 1440, 1.25);
        assert_eq!(
            primary_screen_rect(),
            Some(crate::plan::PixelRect::new(0, 0, 2560, 1440))
        );
        assert!((primary_screen_scale() - 1.25).abs() < 1e-4);
        assert_eq!(primary_screen_logical_size(), Some((2048, 1152)));
        clear_primary_screen_metrics();
    }
}

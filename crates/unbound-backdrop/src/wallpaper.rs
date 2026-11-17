//! 桌面壁纸读取：路径解析、解码缓存与主屏逻辑尺寸。

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use image::DynamicImage;

/// 已解码壁纸。按「文件修改时间 + 大小」失效，避免反复解码 4K JPEG。
struct CachedWallpaper {
    modified: SystemTime,
    len: u64,
    image: Arc<DynamicImage>,
}

static CACHE: OnceLock<Mutex<Option<CachedWallpaper>>> = OnceLock::new();

/// 当前桌面壁纸的静态文件路径。
///
/// 幻灯片或动态壁纸可能拿不到静态文件，此时返回 `None`（调用方回退纯色底）。
#[must_use]
pub fn wallpaper_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_wallpaper_path()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// 读取并解码当前壁纸，命中缓存时零成本返回。
///
/// 解码在锁外进行：4K 壁纸在 debug 构建下可达数百毫秒，不能阻塞同时到达的
/// 第二个请求（主题切换与窗口缩放可能并发触发重建）。
///
/// # Errors
/// 路径不可用、读盘失败或图像解码失败时返回可读的错误文本。
pub fn load_wallpaper() -> Result<(Arc<DynamicImage>, SystemTime, u64), String> {
    let path = wallpaper_path().ok_or_else(|| "wallpaper path unavailable".to_string())?;
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("wallpaper metadata failed: {error}"))?;
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

    let bytes = std::fs::read(&path)
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
        None
    }
}

/// 主屏逻辑尺寸（按系统 DPI 归一化，不含每窗口缩放）。
///
/// Mica 的壁纸取样以**主屏**为参照：Win11 的云母同样只跟随主屏壁纸，
/// 多屏时副屏窗口不会为各自壁纸重新取样。
#[must_use]
pub fn primary_screen_logical_size() -> Option<(u32, u32)> {
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
        None
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
}

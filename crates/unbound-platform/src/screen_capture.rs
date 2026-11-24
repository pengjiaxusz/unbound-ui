//! 屏幕设备上下文捕获通道。
//!
//! 渲染器回读只能看到本进程提交的交换链内容，看不到桌面合成器把系统材质
//! （云母、亚克力）与壁纸合成之后的结果。本模块直接从屏幕设备上下文取样，
//! 因此原生路径与自绘路径可以在同一窗口位置下并排对照。
//!
//! 坐标约定：
//!
//! * 捕获矩形与它携带的原点都使用屏幕物理像素，原点可以不是 `(0, 0)`；
//! * 自绘后景纹理以主屏矩形为画布，屏幕像素映射到纹理 UV 前必须减去主屏原点，
//!   见 [`screen_rect_uv`]；
//! * 越界部分按主屏边界钳制；完全无交集时明确报错，绝不伪造像素。
//!
//! 非 Windows 平台返回 [`CaptureError::Unsupported`]，而不是崩溃或空白图像。

use crate::window::ScreenRect;

/// 屏幕捕获的失败原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// 当前平台或运行环境不提供屏幕设备上下文。
    Unsupported(String),
    /// 系统调用失败或捕获区域无效。
    Failed(String),
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported(reason) | Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for CaptureError {}

/// 一次屏幕捕获的结果。
///
/// `origin` 是实际取样区域在屏幕坐标系中的左上角，`width` / `height` 是同一区域的
/// 物理像素尺寸。`bgra` 为行优先的 BGRA 字节，长度等于 `width * height * 4`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedScreen {
    /// 捕获区域的屏幕物理矩形（已按主屏钳制）。
    pub origin: ScreenRect,
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
    /// 行优先 BGRA 字节。
    pub bgra: Vec<u8>,
}

impl CapturedScreen {
    /// 是否没有捕获到任何像素。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.bgra.is_empty()
    }

    /// 某个屏幕物理像素在捕获图像中的下标（`None` 表示不在捕获区域内）。
    #[must_use]
    pub fn index_of(&self, x: i32, y: i32) -> Option<usize> {
        let local_x = x.checked_sub(self.origin.x)?;
        let local_y = y.checked_sub(self.origin.y)?;
        if local_x < 0
            || local_y < 0
            || local_x as u32 >= self.width
            || local_y as u32 >= self.height
        {
            return None;
        }
        Some((local_y as usize * self.width as usize + local_x as usize) * 4)
    }
}

/// 把请求矩形与屏幕矩形求交。
///
/// 部分越界时按屏幕边界钳制；任一矩形为空或完全无交集时返回 `None`。
/// 该函数只做整数运算，不假设任何一个矩形的原点为 `(0, 0)`。
#[must_use]
pub fn clamp_to_screen(requested: ScreenRect, screen: ScreenRect) -> Option<ScreenRect> {
    if requested.is_empty() || screen.is_empty() {
        return None;
    }
    let left = i64::from(requested.x).max(i64::from(screen.x));
    let top = i64::from(requested.y).max(i64::from(screen.y));
    let right = (i64::from(requested.x) + i64::from(requested.width))
        .min(i64::from(screen.x) + i64::from(screen.width));
    let bottom = (i64::from(requested.y) + i64::from(requested.height))
        .min(i64::from(screen.y) + i64::from(screen.height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(ScreenRect::new(
        i32::try_from(left).ok()?,
        i32::try_from(top).ok()?,
        u32::try_from(right - left).ok()?,
        u32::try_from(bottom - top).ok()?,
    ))
}

/// 把屏幕物理矩形映射为自绘纹理的归一化 UV 子矩形。
///
/// 纹理以主屏矩形为画布，因此分子必须减去**屏幕矩形自己的原点**，不能假设
/// 屏幕从 `(0, 0)` 开始。部分越界按边界钳制；完全无交集时退回整张纹理，
/// 保证采样区域始终有效。
#[must_use]
pub fn screen_rect_uv(rect: ScreenRect, screen: ScreenRect) -> [f32; 4] {
    let Some(clamped) = clamp_to_screen(rect, screen) else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    [
        (clamped.x - screen.x) as f32 / screen.width as f32,
        (clamped.y - screen.y) as f32 / screen.height as f32,
        clamped.width as f32 / screen.width as f32,
        clamped.height as f32 / screen.height as f32,
    ]
}

/// 将 X11 32 位 `ZPixmap`（BGRX）字节流规范化为不透明 BGRA（每像素 alpha 置为 `0xFF`）。
#[must_use]
pub fn normalize_x11_bgrx_to_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    out
}

/// 捕获主屏上的指定区域。
///
/// 请求会先按主屏矩形钳制，因此返回的 [`CapturedScreen::origin`] 可能与请求不同。
/// 自绘后景纹理只覆盖主屏，所以捕获也以主屏为界，两者才能落到同一套 UV 坐标。
///
/// # Errors
/// Wayland 会话或不支持截屏的平台返回 [`CaptureError::Unsupported`]；
/// 拿不到屏幕上下文、捕获区域无交集、位块复制失败或读回字节数不符时返回 [`CaptureError::Failed`]。
pub fn capture_screen_region(requested: ScreenRect) -> Result<CapturedScreen, CaptureError> {
    #[cfg(windows)]
    {
        capture_impl(requested)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        capture_linux_impl(requested)
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = requested;
        Err(CaptureError::Unsupported(
            "screen capture is not implemented on this platform".to_string(),
        ))
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
fn capture_linux_impl(requested: ScreenRect) -> Result<CapturedScreen, CaptureError> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
    use x11rb::xcb_ffi::XCBConnection;

    if crate::linux_backend::active_linux_backend()
        == Some(crate::linux_backend::LinuxBackendKind::Wayland)
    {
        return Err(CaptureError::Unsupported(
            "screen capture is unsupported on Wayland compositors without portal authorization"
                .to_string(),
        ));
    }

    let (conn, screen_num) = XCBConnection::connect(None).map_err(|e| {
        CaptureError::Unsupported(format!(
            "X11 screen capture unavailable (no X11 display or libxcb): {e}"
        ))
    })?;
    let screen = &conn.setup().roots[screen_num];
    let primary = ScreenRect::new(
        0,
        0,
        u32::from(screen.width_in_pixels),
        u32::from(screen.height_in_pixels),
    );
    let region = clamp_to_screen(requested, primary).ok_or_else(|| {
        CaptureError::Failed(format!(
            "capture region {}x{} at ({}, {}) does not intersect the primary screen",
            requested.width, requested.height, requested.x, requested.y
        ))
    })?;
    let x = i16::try_from(region.x)
        .map_err(|_| CaptureError::Failed("capture x is out of X11 range".to_string()))?;
    let y = i16::try_from(region.y)
        .map_err(|_| CaptureError::Failed("capture y is out of X11 range".to_string()))?;
    let w = u16::try_from(region.width)
        .map_err(|_| CaptureError::Failed("capture width is out of X11 range".to_string()))?;
    let h = u16::try_from(region.height)
        .map_err(|_| CaptureError::Failed("capture height is out of X11 range".to_string()))?;

    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, x, y, w, h, !0u32)
        .map_err(|e| CaptureError::Failed(format!("X11 GetImage request failed: {e}")))?
        .reply()
        .map_err(|e| CaptureError::Failed(format!("X11 GetImage reply failed: {e}")))?;

    let expected_bytes = region.width as usize * region.height as usize * 4;
    if reply.data.len() != expected_bytes {
        return Err(CaptureError::Failed(format!(
            "X11 GetImage returned {} bytes, expected {expected_bytes}",
            reply.data.len()
        )));
    }
    let bgra = normalize_x11_bgrx_to_bgra(&reply.data);
    Ok(CapturedScreen {
        origin: region,
        width: region.width,
        height: region.height,
        bgra,
    })
}

#[cfg(windows)]
fn capture_impl(requested: ScreenRect) -> Result<CapturedScreen, CaptureError> {
    use core::mem::size_of;

    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HGDIOBJ, RGBQUAD, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    // SAFETY: 两个调用都是无参数的全局指标查询，不涉及指针或句柄。
    let screen = {
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(0) as u32;
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(0) as u32;
        ScreenRect::new(0, 0, width, height)
    };
    let region = clamp_to_screen(requested, screen).ok_or_else(|| {
        CaptureError::Failed(format!(
            "capture region {}x{} at ({}, {}) does not intersect the primary screen",
            requested.width, requested.height, requested.x, requested.y
        ))
    })?;
    let width = i32::try_from(region.width)
        .map_err(|_| CaptureError::Failed("capture width is out of range".to_string()))?;
    let height = i32::try_from(region.height)
        .map_err(|_| CaptureError::Failed("capture height is out of range".to_string()))?;

    // SAFETY: 传入空窗口句柄表示整个屏幕；返回的 DC 在使用后统一释放。
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        return Err(CaptureError::Unsupported(
            "no screen device context is available".to_string(),
        ));
    }
    // SAFETY: 屏幕 DC 有效，创建兼容 DC；失败时返回无效句柄。
    let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if memory_dc.is_invalid() {
        // SAFETY: 只释放本函数取得的屏幕 DC。
        unsafe { ReleaseDC(None, screen_dc) };
        return Err(CaptureError::Failed(
            "could not create a compatible device context".to_string(),
        ));
    }
    // SAFETY: 两个参数都是本函数持有的有效 DC 与正尺寸。
    let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, width, height) };
    if bitmap.is_invalid() {
        // SAFETY: 释放本函数创建的资源。
        unsafe {
            let _ = DeleteDC(memory_dc);
            ReleaseDC(None, screen_dc);
        }
        return Err(CaptureError::Failed(
            "could not create a compatible bitmap".to_string(),
        ));
    }
    // SAFETY: 位图与 DC 都有效；原对象在收尾时选回，避免泄漏。
    let previous = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
    // SAFETY: 源 DC 是屏幕 DC，目标坐标从 0 开始，源坐标是钳制后的屏幕原点。
    let blit = unsafe {
        BitBlt(
            memory_dc,
            0,
            0,
            width,
            height,
            Some(screen_dc),
            region.x,
            region.y,
            SRCCOPY,
        )
    };

    let mut bgra = vec![0_u8; region.width as usize * region.height as usize * 4];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // 负高度表示自上而下的行序，与 PNG 的行优先顺序一致。
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default()],
    };
    let copied = if blit.is_ok() {
        // SAFETY: 缓冲区长度与位图尺寸一致，格式结构体已填好。
        unsafe {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                region.height,
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        }
    } else {
        0
    };

    // SAFETY: 只释放本函数创建的对象；选回原对象后再删除位图与 DC。
    unsafe {
        if !previous.is_invalid() {
            SelectObject(memory_dc, previous);
        }
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
        ReleaseDC(None, screen_dc);
    }

    blit.map_err(|error| CaptureError::Failed(format!("BitBlt failed: {error}")))?;
    if copied as u32 != region.height {
        return Err(CaptureError::Failed(format!(
            "GetDIBits copied {copied} of {} rows",
            region.height
        )));
    }
    Ok(CapturedScreen {
        origin: region,
        width: region.width,
        height: region.height,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: ScreenRect = ScreenRect::new(0, 0, 1920, 1080);

    #[test]
    fn origin_is_subtracted_from_uv() {
        let screen = ScreenRect::new(1920, 0, 1920, 1080);
        let rect = ScreenRect::new(2020, 100, 400, 300);
        let uv = screen_rect_uv(rect, screen);
        assert!((uv[0] - 100.0 / 1920.0).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[1] - 100.0 / 1080.0).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[2] - 400.0 / 1920.0).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[3] - 300.0 / 1080.0).abs() < 1e-6, "uv = {uv:?}");
    }

    #[test]
    fn partial_overflow_is_clamped_before_mapping() {
        let rect = ScreenRect::new(-200, -100, 800, 400);
        let clamped = clamp_to_screen(rect, SCREEN).expect("partial overlap should clamp");
        assert_eq!(clamped, ScreenRect::new(0, 0, 600, 300));
        let uv = screen_rect_uv(rect, SCREEN);
        assert!((uv[0]).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[1]).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[2] - 600.0 / 1920.0).abs() < 1e-6, "uv = {uv:?}");
        assert!((uv[3] - 300.0 / 1080.0).abs() < 1e-6, "uv = {uv:?}");
    }

    #[test]
    fn rect_on_another_monitor_is_clamped_into_the_screen() {
        let screen = ScreenRect::new(1920, 0, 1920, 1080);
        let rect = ScreenRect::new(1700, 500, 800, 300);
        let clamped = clamp_to_screen(rect, screen).expect("cross-screen overlap should clamp");
        assert_eq!(clamped, ScreenRect::new(1920, 500, 580, 300));
    }

    #[test]
    fn rect_outside_the_screen_has_no_intersection() {
        assert_eq!(
            clamp_to_screen(ScreenRect::new(2000, 0, 100, 100), SCREEN),
            None
        );
        assert_eq!(
            clamp_to_screen(ScreenRect::new(-500, 0, 400, 100), SCREEN),
            None
        );
        assert_eq!(
            screen_rect_uv(ScreenRect::new(2000, 0, 100, 100), SCREEN),
            [0.0, 0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn captured_pixel_lookup_uses_the_capture_origin() {
        let captured = CapturedScreen {
            origin: ScreenRect::new(1920, 0, 10, 10),
            width: 10,
            height: 10,
            bgra: vec![0; 10 * 10 * 4],
        };
        assert_eq!(captured.index_of(1920, 0), Some(0));
        assert_eq!(captured.index_of(1925, 5), Some((5 * 10 + 5) * 4));
        assert_eq!(captured.index_of(1919, 0), None);
        assert_eq!(captured.index_of(1930, 0), None);
    }

    #[test]
    fn normalize_x11_bgrx_to_bgra_sets_opaque_alpha() {
        let raw = vec![10, 20, 30, 0, 40, 50, 60, 128];
        let bgra = normalize_x11_bgrx_to_bgra(&raw);
        assert_eq!(bgra, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[test]
    fn wayland_backend_screen_capture_returns_unsupported_error() {
        let prev = crate::linux_backend::active_linux_backend();
        crate::linux_backend::set_active_linux_backend(Some(
            crate::linux_backend::LinuxBackendKind::Wayland,
        ));
        let res = capture_screen_region(ScreenRect::new(0, 0, 64, 64));
        crate::linux_backend::set_active_linux_backend(prev);
        assert!(matches!(res, Err(CaptureError::Unsupported(_))));
    }
}

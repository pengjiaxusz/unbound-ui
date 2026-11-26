//! 自定义 RGBA 位图光标注册表与跨平台句柄缓存。
//!
//! 维护进程级线程安全的光标注册表，为应用层提供 `register_custom_cursor` /
//! `unregister_custom_cursor` / `lookup_custom_cursor_bitmap` 能力，并按上限淘汰旧条目：
//! - **Windows**：延迟创建 32-bit BGRA `HCURSOR`（经 `CreateDIBSection` + `CreateIconIndirect`），
//!   在条目注销、容量淘汰或释放时自动调用 `DestroyIcon` 回收 OS 资源；
//! - **Wayland**：通过 `lookup_custom_cursor_bitmap` 读取原始 RGBA 位图并写入 `wl_shm` 光标表面；
//! - **X11 / 无头环境**：无对应原生位图通道时由后端安全降级至 `CursorIcon::Default`。

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use unbound_input::CustomCursorId;

/// 自定义位图光标单边最大像素尺寸。
pub const MAX_CUSTOM_CURSOR_SIZE: u32 = 256;

/// 注册表允许同时缓存的最大自定义光标条目数（超出后按 FIFO 顺序淘汰最旧条目）。
pub const MAX_CUSTOM_CURSORS: usize = 256;

/// 已校验的自定义 RGBA 位图光标数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomCursorBitmap {
    /// 行优先 32-bit RGBA8 像素数据（长度恒为 `width * height * 4`）。
    pub rgba: Vec<u8>,
    /// 光标位图宽度（像素，范围 `1..=256`）。
    pub width: u32,
    /// 光标位图高度（像素，范围 `1..=256`）。
    pub height: u32,
    /// 热点 X 坐标（已钳制至 `0..width`）。
    pub hotspot_x: u32,
    /// 热点 Y 坐标（已钳制至 `0..height`）。
    pub hotspot_y: u32,
}

struct CustomCursorEntry {
    bitmap: CustomCursorBitmap,
    #[cfg(target_os = "windows")]
    win32_hcursor: Option<isize>,
}

impl Drop for CustomCursorEntry {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        if let Some(raw) = self.win32_hcursor.take() {
            destroy_win32_hcursor(raw);
        }
    }
}

/// 有界自定义位图光标注册表。
///
/// 分配 `1..=u16::MAX` 范围内的非零 [`CustomCursorId`]，在达到容量上限时按 FIFO
/// 淘汰最早注册的条目并自动释放其关联的原生光标句柄。
pub struct CustomCursorRegistry {
    entries: HashMap<u16, CustomCursorEntry>,
    order: VecDeque<u16>,
    next_id: u16,
    max_capacity: usize,
}

impl Default for CustomCursorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomCursorRegistry {
    /// 创建默认容量（[`MAX_CUSTOM_CURSORS`]）的空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(MAX_CUSTOM_CURSORS)
    }

    /// 创建指定容量上限的空注册表（最小为 1）。
    #[must_use]
    pub fn with_capacity(max_capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            next_id: 1,
            max_capacity: max_capacity.max(1),
        }
    }

    /// 注册一张 RGBA8 位图光标并返回分配的 [`CustomCursorId`]。
    ///
    /// 若宽高为 0、超过 [`MAX_CUSTOM_CURSOR_SIZE`] 或缓冲长度与 `width * height * 4` 不符，
    /// 返回 `None`。热点坐标会自动钳制到 `[0, width - 1]` 与 `[0, height - 1]`。
    pub fn register(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        hotspot_x: u32,
        hotspot_y: u32,
    ) -> Option<CustomCursorId> {
        if width == 0
            || height == 0
            || width > MAX_CUSTOM_CURSOR_SIZE
            || height > MAX_CUSTOM_CURSOR_SIZE
        {
            return None;
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if rgba.len() != expected_len {
            return None;
        }

        while self.entries.len() >= self.max_capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }

        let id = self.allocate_id()?;
        let bitmap = CustomCursorBitmap {
            rgba: rgba.to_vec(),
            width,
            height,
            hotspot_x: hotspot_x.min(width - 1),
            hotspot_y: hotspot_y.min(height - 1),
        };
        self.entries.insert(
            id,
            CustomCursorEntry {
                bitmap,
                #[cfg(target_os = "windows")]
                win32_hcursor: None,
            },
        );
        self.order.push_back(id);
        Some(CustomCursorId(id))
    }

    /// 注销指定 [`CustomCursorId`] 并释放其缓存位图与平台句柄。
    ///
    /// 若条目存在并成功移除返回 `true`，否则返回 `false`。
    pub fn unregister(&mut self, id: CustomCursorId) -> bool {
        if id.0 == 0 {
            return false;
        }
        if self.entries.remove(&id.0).is_some() {
            self.order.retain(|&existing| existing != id.0);
            true
        } else {
            false
        }
    }

    /// 按 [`CustomCursorId`] 查询已注册的位图副本。
    #[must_use]
    pub fn lookup_bitmap(&self, id: CustomCursorId) -> Option<CustomCursorBitmap> {
        if id.0 == 0 {
            return None;
        }
        self.entries.get(&id.0).map(|entry| entry.bitmap.clone())
    }

    /// 当前已缓存的自定义光标条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 注册表是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 查找或延迟创建对应的 Win32 `HCURSOR` 原始句柄并缓存。
    #[cfg(target_os = "windows")]
    pub fn lookup_or_create_win32_hcursor(&mut self, id: u16) -> Option<isize> {
        if id == 0 {
            return None;
        }
        let entry = self.entries.get_mut(&id)?;
        if let Some(raw) = entry.win32_hcursor {
            return Some(raw);
        }
        let raw = create_win32_hcursor(&entry.bitmap)?;
        entry.win32_hcursor = Some(raw);
        Some(raw)
    }

    fn allocate_id(&mut self) -> Option<u16> {
        for _ in 0..u16::MAX {
            let candidate = if self.next_id == 0 { 1 } else { self.next_id };
            self.next_id = if candidate == u16::MAX {
                1
            } else {
                candidate + 1
            };
            if !self.entries.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

static GLOBAL_REGISTRY: LazyLock<Mutex<CustomCursorRegistry>> =
    LazyLock::new(|| Mutex::new(CustomCursorRegistry::new()));

/// 向进程全局注册表注册自定义 RGBA8 位图光标。
#[must_use]
pub fn register_custom_cursor(
    rgba: &[u8],
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
) -> Option<CustomCursorId> {
    GLOBAL_REGISTRY
        .lock()
        .ok()?
        .register(rgba, width, height, hotspot_x, hotspot_y)
}

/// 从进程全局注册表注销指定自定义光标并释放其关联的 OS 句柄。
pub fn unregister_custom_cursor(id: CustomCursorId) -> bool {
    GLOBAL_REGISTRY
        .lock()
        .map(|mut guard| guard.unregister(id))
        .unwrap_or(false)
}

/// 从进程全局注册表查询已注册的自定义光标位图副本（供 Wayland 等后端消费）。
#[must_use]
pub fn lookup_custom_cursor_bitmap(id: CustomCursorId) -> Option<CustomCursorBitmap> {
    GLOBAL_REGISTRY.lock().ok()?.lookup_bitmap(id)
}

/// 查询或延迟创建进程全局注册表中对应条目的 Win32 `HCURSOR` 句柄。
#[cfg(target_os = "windows")]
#[must_use]
pub fn lookup_or_create_win32_hcursor(id: u16) -> Option<isize> {
    GLOBAL_REGISTRY
        .lock()
        .ok()?
        .lookup_or_create_win32_hcursor(id)
}

/// 将 RGBA8 位图转换为 Win32 32-bit BGRA `HCURSOR`（调用方负责通过 `DestroyIcon` 释放）。
#[cfg(target_os = "windows")]
#[must_use]
pub fn create_win32_hcursor(bitmap: &CustomCursorBitmap) -> Option<isize> {
    use ::windows::core::BOOL;
    use ::windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    use core::ffi::c_void;
    use core::mem::size_of;

    let expected_len = (bitmap.width as usize)
        .checked_mul(bitmap.height as usize)?
        .checked_mul(4)?;
    if bitmap.width == 0
        || bitmap.height == 0
        || bitmap.width > MAX_CUSTOM_CURSOR_SIZE
        || bitmap.height > MAX_CUSTOM_CURSOR_SIZE
        || bitmap.rgba.len() != expected_len
    {
        return None;
    }

    let width_i32 = i32::try_from(bitmap.width).ok()?;
    let height_i32 = i32::try_from(bitmap.height).ok()?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width_i32,
            biHeight: -height_i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut c_void = core::ptr::null_mut();
    // SAFETY: `info` 为栈上已初始化的顶向下 32-bit BGRA `BITMAPINFO`，`bits_ptr` 接收 GDI 分配的像素缓冲指针。
    let hbm_color =
        unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits_ptr, None, 0) }.ok()?;
    if hbm_color.is_invalid() || bits_ptr.is_null() {
        if !hbm_color.is_invalid() {
            // SAFETY: `hbm_color` 为刚创建的 GDI 位图句柄，释放以防泄漏。
            unsafe {
                let _ = DeleteObject(HGDIOBJ(hbm_color.0));
            }
        }
        return None;
    }

    // 将 RGBA 逐像素转换为 Win32 32-bit BGRA 像素缓冲。
    // SAFETY: `CreateDIBSection` 已按 `width * height * 4` 字节分配了连续可写像素缓冲。
    let dst = unsafe { core::slice::from_raw_parts_mut(bits_ptr.cast::<u8>(), expected_len) };
    for (src_px, dst_px) in bitmap.rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        dst_px[0] = src_px[2];
        dst_px[1] = src_px[1];
        dst_px[2] = src_px[0];
        dst_px[3] = src_px[3];
    }

    // 创建 1bpp 单色掩码位图（全 0 表示透明度由 32-bit 彩色位图的 Alpha 通道控制）。
    // SAFETY: `width_i32` 与 `height_i32` 均在 1..=256 范围内，`None` 表示不传入初始位图数据。
    let hbm_mask = unsafe { CreateBitmap(width_i32, height_i32, 1, 1, None) };
    if hbm_mask.is_invalid() {
        // SAFETY: 释放已创建的彩色 DIB 位图句柄。
        unsafe {
            let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        }
        return None;
    }

    let icon_info = ICONINFO {
        fIcon: BOOL(0),
        xHotspot: bitmap.hotspot_x.min(bitmap.width - 1),
        yHotspot: bitmap.hotspot_y.min(bitmap.height - 1),
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    // SAFETY: `icon_info` 持有两个有效 GDI 位图句柄，调用结束后立即释放临时位图句柄。
    let hicon = unsafe { CreateIconIndirect(&icon_info) }.ok();
    // SAFETY: `CreateIconIndirect` 内部会复制位图数据，此处立即释放两个临时 GDI 位图。
    unsafe {
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
    }

    hicon.and_then(|icon| (!icon.is_invalid()).then_some(icon.0 as isize))
}

#[cfg(target_os = "windows")]
fn destroy_win32_hcursor(raw: isize) {
    if raw == 0 {
        return;
    }
    use ::windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
    use core::ffi::c_void;
    // SAFETY: `raw` 由 `CreateIconIndirect` 创建且每个条目独占，销毁后不再使用。
    unsafe {
        let _ = DestroyIcon(HICON(raw as *mut c_void));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32_window::CursorIcon;

    fn solid_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let count = (width as usize) * (height as usize);
        let mut buf = Vec::with_capacity(count * 4);
        for _ in 0..count {
            buf.extend_from_slice(&rgba);
        }
        buf
    }

    #[test]
    fn register_lookup_and_unregister_roundtrip() {
        let mut registry = CustomCursorRegistry::new();
        let pixels = solid_rgba(16, 16, [255, 128, 64, 255]);
        let id = registry
            .register(&pixels, 16, 16, 4, 8)
            .expect("valid cursor bitmap should register");
        assert_ne!(id.raw(), 0);
        assert_eq!(registry.len(), 1);

        let found = registry
            .lookup_bitmap(id)
            .expect("registered cursor should be retrievable");
        assert_eq!(found.width, 16);
        assert_eq!(found.height, 16);
        assert_eq!(found.hotspot_x, 4);
        assert_eq!(found.hotspot_y, 8);
        assert_eq!(found.rgba, pixels);

        assert!(registry.unregister(id));
        assert!(!registry.unregister(id));
        assert!(registry.lookup_bitmap(id).is_none());
        assert!(registry.is_empty());
    }

    #[test]
    fn rejects_invalid_dimensions_and_buffer_lengths() {
        let mut registry = CustomCursorRegistry::new();
        assert!(registry.register(&[], 0, 16, 0, 0).is_none());
        assert!(registry.register(&[], 16, 0, 0, 0).is_none());
        assert!(registry
            .register(&solid_rgba(4, 4, [0, 0, 0, 255]), 257, 1, 0, 0)
            .is_none());
        assert!(registry
            .register(&solid_rgba(4, 4, [0, 0, 0, 255]), 1, 257, 0, 0)
            .is_none());
        assert!(registry.register(&[255; 12], 2, 2, 0, 0).is_none());
        assert!(registry.is_empty());
    }

    #[test]
    fn clamps_hotspot_to_bitmap_bounds() {
        let mut registry = CustomCursorRegistry::new();
        let pixels = solid_rgba(8, 12, [10, 20, 30, 255]);
        let id = registry
            .register(&pixels, 8, 12, 999, 500)
            .expect("out-of-bounds hotspot should be clamped");
        let found = registry.lookup_bitmap(id).expect("bitmap should exist");
        assert_eq!(found.hotspot_x, 7);
        assert_eq!(found.hotspot_y, 11);
    }

    #[test]
    fn evicts_oldest_entry_when_capacity_exceeded() {
        let mut registry = CustomCursorRegistry::with_capacity(3);
        let pixels = solid_rgba(4, 4, [255, 255, 255, 255]);
        let id1 = registry.register(&pixels, 4, 4, 0, 0).unwrap();
        let id2 = registry.register(&pixels, 4, 4, 1, 1).unwrap();
        let id3 = registry.register(&pixels, 4, 4, 2, 2).unwrap();
        assert_eq!(registry.len(), 3);

        let id4 = registry.register(&pixels, 4, 4, 3, 3).unwrap();
        assert_eq!(registry.len(), 3);
        assert!(registry.lookup_bitmap(id1).is_none());
        assert!(registry.lookup_bitmap(id2).is_some());
        assert!(registry.lookup_bitmap(id3).is_some());
        assert!(registry.lookup_bitmap(id4).is_some());
    }

    #[test]
    fn cursor_icon_u32_roundtrip_covers_standard_and_custom() {
        let standard = [
            CursorIcon::Default,
            CursorIcon::Pointer,
            CursorIcon::Text,
            CursorIcon::Grab,
            CursorIcon::Grabbing,
            CursorIcon::Crosshair,
            CursorIcon::Move,
            CursorIcon::NsResize,
            CursorIcon::EwResize,
            CursorIcon::NwseResize,
            CursorIcon::NeswResize,
            CursorIcon::NotAllowed,
        ];
        for icon in standard {
            let encoded = icon.to_u32();
            assert_eq!(CursorIcon::from_u32(encoded), icon);
        }

        let custom = CursorIcon::Custom(1234);
        assert_eq!(custom.to_u32(), 0x1_0000 | 1234);
        assert_eq!(CursorIcon::from_u32(custom.to_u32()), custom);
        assert_eq!(CursorIcon::from_u32(0xDEAD_BEEF), CursorIcon::Default);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn win32_create_and_destroy_custom_hcursor() {
        let mut registry = CustomCursorRegistry::new();
        let pixels = solid_rgba(16, 16, [255, 0, 0, 255]);
        let id = registry
            .register(&pixels, 16, 16, 8, 8)
            .expect("register should succeed");
        let hcursor = registry
            .lookup_or_create_win32_hcursor(id.raw())
            .expect("CreateIconIndirect should produce a non-null HCURSOR");
        assert_ne!(hcursor, 0);
        // 再次查询应命中缓存并返回相同句柄。
        assert_eq!(
            registry.lookup_or_create_win32_hcursor(id.raw()),
            Some(hcursor)
        );
        // 注销时自动调用 DestroyIcon。
        assert!(registry.unregister(id));
    }
}

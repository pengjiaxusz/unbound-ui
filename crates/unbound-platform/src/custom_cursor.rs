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

/// 自定义矢量光标按 5° 步进量化后的分桶数量（`0..72` 覆盖 `0°..355°`）。
pub const VECTOR_CURSOR_STEP_DEG: f32 = 5.0;
const ROTATION_CURSOR_BUCKETS: usize = 72;
const SCALE_CURSOR_BUCKETS: usize = 36;
const VECTOR_CURSOR_SIZE: u32 = 32;

static ROTATION_CURSOR_CACHE: LazyLock<Mutex<[Option<CustomCursorId>; ROTATION_CURSOR_BUCKETS]>> =
    LazyLock::new(|| Mutex::new([None; ROTATION_CURSOR_BUCKETS]));
static SCALE_CURSOR_CACHE: LazyLock<Mutex<[Option<CustomCursorId>; SCALE_CURSOR_BUCKETS]>> =
    LazyLock::new(|| Mutex::new([None; SCALE_CURSOR_BUCKETS]));

/// 将任意角度（度）量化到最近的 5° 步进角，返回 `(规范化度数, 0..72 桶索引)`。
#[must_use]
pub fn quantize_angle_5deg(angle_deg: f32) -> (f32, usize) {
    let wrapped = angle_deg.rem_euclid(360.0);
    let bucket = ((wrapped / VECTOR_CURSOR_STEP_DEG).round() as usize) % ROTATION_CURSOR_BUCKETS;
    (bucket as f32 * VECTOR_CURSOR_STEP_DEG, bucket)
}

#[inline]
fn dist_point_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let len_sq = vx * vx + vy * vy;
    let t = if len_sq > 1e-6 {
        ((wx * vx + wy * vy) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = px - (ax + t * vx);
    let dy = py - (ay + t * vy);
    (dx * dx + dy * dy).sqrt()
}

#[inline]
fn blend_vector_cursor_pixel(dst: &mut [u8], sd: f32, fill_half: f32, outline_half: f32) {
    // 使用 1px 抗锯齿过渡带：外层白描边，内层深色填充，确保浅/深画布均清晰可见。
    let aa = 1.0;
    let outer_a = (outline_half + 0.5 - sd).clamp(0.0, aa);
    if outer_a <= 0.0 {
        return;
    }
    let fill_a = (fill_half + 0.5 - sd).clamp(0.0, aa);
    let r = (255.0 * (1.0 - fill_a) + 24.0 * fill_a).round() as u8;
    let g = (255.0 * (1.0 - fill_a) + 24.0 * fill_a).round() as u8;
    let b = (255.0 * (1.0 - fill_a) + 27.0 * fill_a).round() as u8;
    let a = (outer_a * 255.0).round() as u8;
    if a > dst[3] {
        dst[0] = r;
        dst[1] = g;
        dst[2] = b;
        dst[3] = a;
    }
}

/// 栅格化 5° 步进旋转手柄矢量光标位图（32x32 RGBA8，热点居中）。
#[must_use]
pub fn rasterize_rotation_cursor_rgba(angle_deg: f32) -> CustomCursorBitmap {
    let (quantized_deg, _) = quantize_angle_5deg(angle_deg);
    let rad = quantized_deg.to_radians();
    let (sin_a, cos_a) = rad.sin_cos();
    let size = VECTOR_CURSOR_SIZE;
    let half = size as f32 * 0.5;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let arc_r = 8.2_f32;
    let arc_half_span = 1.15_f32; // ~66 deg half-span -> 132 deg curved arc
    let head_len = 3.6_f32;
    let head_spread = 2.6_f32;

    for y in 0..size {
        for x in 0..size {
            let rx = x as f32 + 0.5 - half;
            let ry = y as f32 + 0.5 - half;
            // 反向旋转像素坐标，在局部坐标系求值曲线双向箭头 SDF。
            let lx = rx * cos_a + ry * sin_a;
            let ly = -rx * sin_a + ry * cos_a;

            let r = (lx * lx + ly * ly).sqrt();
            let theta = ly.atan2(lx);
            let mut sd = if theta.abs() <= arc_half_span {
                (r - arc_r).abs()
            } else {
                let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
                let ex = arc_r * arc_half_span.cos();
                let ey = sign * arc_r * arc_half_span.sin();
                ((lx - ex).powi(2) + (ly - ey).powi(2)).sqrt()
            };

            // 两端切线箭头（+arc_half_span 与 -arc_half_span）。
            for sign in [-1.0_f32, 1.0_f32] {
                let a = sign * arc_half_span;
                let tip_x = arc_r * a.cos();
                let tip_y = arc_r * a.sin();
                // 外指切向
                let tx = -sign * a.sin();
                let ty = sign * a.cos();
                // 径向法线
                let nx = a.cos();
                let ny = a.sin();
                let bx = tip_x - tx * head_len;
                let by = tip_y - ty * head_len;
                let d1 = dist_point_to_segment(
                    lx,
                    ly,
                    tip_x,
                    tip_y,
                    bx + nx * head_spread,
                    by + ny * head_spread,
                );
                let d2 = dist_point_to_segment(
                    lx,
                    ly,
                    tip_x,
                    tip_y,
                    bx - nx * head_spread,
                    by - ny * head_spread,
                );
                sd = sd.min(d1).min(d2);
            }

            let idx = ((y * size + x) * 4) as usize;
            blend_vector_cursor_pixel(&mut rgba[idx..idx + 4], sd, 1.05, 2.2);
        }
    }

    CustomCursorBitmap {
        rgba,
        width: size,
        height: size,
        hotspot_x: size / 2,
        hotspot_y: size / 2,
    }
}

/// 栅格化任意角度对称缩放矢量光标位图（32x32 RGBA8，5° 步进，热点居中）。
#[must_use]
pub fn rasterize_scale_cursor_rgba(angle_deg: f32) -> CustomCursorBitmap {
    let (quantized_deg, _) = quantize_angle_5deg(angle_deg);
    let rad = quantized_deg.to_radians();
    let (sin_a, cos_a) = rad.sin_cos();
    let size = VECTOR_CURSOR_SIZE;
    let half = size as f32 * 0.5;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let shaft_half = 8.5_f32;
    let head_len = 4.0_f32;
    let head_spread = 3.2_f32;

    for y in 0..size {
        for x in 0..size {
            let rx = x as f32 + 0.5 - half;
            let ry = y as f32 + 0.5 - half;
            let lx = rx * cos_a + ry * sin_a;
            let ly = -rx * sin_a + ry * cos_a;

            let mut sd = dist_point_to_segment(lx, ly, -shaft_half, 0.0, shaft_half, 0.0);
            for sign in [-1.0_f32, 1.0_f32] {
                let tip_x = sign * (shaft_half + 1.5);
                let base_x = tip_x - sign * head_len;
                let d1 = dist_point_to_segment(lx, ly, tip_x, 0.0, base_x, head_spread);
                let d2 = dist_point_to_segment(lx, ly, tip_x, 0.0, base_x, -head_spread);
                sd = sd.min(d1).min(d2);
            }

            let idx = ((y * size + x) * 4) as usize;
            blend_vector_cursor_pixel(&mut rgba[idx..idx + 4], sd, 1.05, 2.2);
        }
    }

    CustomCursorBitmap {
        rgba,
        width: size,
        height: size,
        hotspot_x: size / 2,
        hotspot_y: size / 2,
    }
}

/// 获取或惰性生成缓存的 5° 步进旋转手柄自定义光标 ID。
#[must_use]
pub fn rotation_cursor_for_angle_deg(angle_deg: f32) -> Option<CustomCursorId> {
    let (quantized, bucket) = quantize_angle_5deg(angle_deg);
    let mut cache = ROTATION_CURSOR_CACHE.lock().ok()?;
    if let Some(id) = cache[bucket] {
        if lookup_custom_cursor_bitmap(id).is_some() {
            return Some(id);
        }
    }
    let bmp = rasterize_rotation_cursor_rgba(quantized);
    let id = register_custom_cursor(
        &bmp.rgba,
        bmp.width,
        bmp.height,
        bmp.hotspot_x,
        bmp.hotspot_y,
    )?;
    cache[bucket] = Some(id);
    Some(id)
}

/// 获取或惰性生成缓存的 5° 步进对称缩放自定义光标 ID。
#[must_use]
pub fn scale_cursor_for_angle_deg(angle_deg: f32) -> Option<CustomCursorId> {
    let (quantized, raw_bucket) = quantize_angle_5deg(angle_deg);
    let bucket = raw_bucket % SCALE_CURSOR_BUCKETS;
    let mut cache = SCALE_CURSOR_CACHE.lock().ok()?;
    if let Some(id) = cache[bucket] {
        if lookup_custom_cursor_bitmap(id).is_some() {
            return Some(id);
        }
    }
    let bmp = rasterize_scale_cursor_rgba(quantized);
    let id = register_custom_cursor(
        &bmp.rgba,
        bmp.width,
        bmp.height,
        bmp.hotspot_x,
        bmp.hotspot_y,
    )?;
    cache[bucket] = Some(id);
    Some(id)
}

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

    #[test]
    fn rotation_and_scale_cursors_quantize_to_5_deg_and_cache() {
        assert_eq!(quantize_angle_5deg(0.0), (0.0, 0));
        assert_eq!(quantize_angle_5deg(2.4), (0.0, 0));
        assert_eq!(quantize_angle_5deg(2.6), (5.0, 1));
        assert_eq!(quantize_angle_5deg(-5.0), (355.0, 71));

        let rot1 = rotation_cursor_for_angle_deg(12.2).expect("rotation cursor");
        let rot2 = rotation_cursor_for_angle_deg(11.9).expect("same 5deg bucket");
        assert_eq!(rot1, rot2);

        let bmp = lookup_custom_cursor_bitmap(rot1).expect("cached bitmap");
        assert_eq!(bmp.width, 32);
        assert_eq!(bmp.height, 32);
        assert_eq!(bmp.hotspot_x, 16);
        assert_eq!(bmp.hotspot_y, 16);
        assert!(bmp.rgba.chunks_exact(4).any(|px| px[3] > 0));

        let sc1 = scale_cursor_for_angle_deg(45.0).expect("scale cursor");
        let sc2 = scale_cursor_for_angle_deg(225.0).expect("180deg symmetric bucket");
        assert_eq!(sc1, sc2);
    }
}

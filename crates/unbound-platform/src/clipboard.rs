//! 剪贴板抽象层（`docs` §6.3）。
//!
//! 剪贴板属于平台服务，因此将策略层与系统实现层分离：
//!
//!
//! * [`ClipboardProvider`] 是编辑器交互的 trait。
//! * [`MemoryClipboard`] 是测试与无头运行使用的确定性实现。
//! * [`SystemClipboard`] 是 Windows 实现；在其他平台上返回
//!   [`ClipboardError::Unavailable`] 而非静默无操作。
//!
//!
//! 每次读取均受限于 [`MAX_CLIPBOARD_BYTES`]。粘贴操作绝不能在应用有机会
//! 拒绝之前将恶意的 2 GB 剪贴板载荷变成 4 GB 的 `String`——上限检查在
//! 任何大内存分配前执行，错误中会报告被拒绝的字节大小。
//!

/// 引擎可接受的最大剪贴板载荷（以 UTF-8 字节计）。
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

/// 剪贴板操作失败原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// 剪贴板服务不可用（不支持的平台，或剪贴板被其他进程锁定）。
    ///
    Unavailable,
    /// 载荷大小超出 [`MAX_CLIPBOARD_BYTES`]。
    TooLarge {
        /// 剪贴板报告的字节大小。
        bytes: usize,
        /// 配置的上限。
        limit: usize,
    },
    /// 平台调用因需要上报的原因而失败。
    Os,
}

impl core::fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("clipboard unavailable"),
            Self::TooLarge { bytes, limit } => {
                write!(formatter, "clipboard payload {bytes} exceeds {limit}")
            }
            Self::Os => formatter.write_str("clipboard operation failed"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// 文本控件与编辑器使用的剪贴板契约。
pub trait ClipboardProvider {
    /// 以 UTF-8 文本形式读取剪贴板。`Ok(None)` 表示为空。
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;

    /// 将剪贴板内容替换为 `text`。
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;

    /// 清空剪贴板。
    fn clear(&mut self) -> Result<(), ClipboardError>;
}

/// 确定性的内存剪贴板实现。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryClipboard {
    text: Option<String>,
    image: Option<(u32, u32, Vec<u8>)>,
    files: Vec<std::path::PathBuf>,
    max_bytes: usize,
}

impl Default for MemoryClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: None,
            image: None,
            files: Vec::new(),
            max_bytes: MAX_CLIPBOARD_BYTES,
        }
    }

    /// 读取已复制的文件路径列表。
    #[must_use]
    pub fn read_file_paths(&self) -> Vec<std::path::PathBuf> {
        self.files.clone()
    }

    /// 在剪贴板中设置文件路径列表。
    pub fn write_file_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        self.files = paths;
    }

    /// 读取已复制的 RGBA 图像。
    #[must_use]
    pub fn read_image_rgba(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.image.clone()
    }

    /// 在剪贴板中设置 RGBA 图像。
    pub fn write_image_rgba(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        self.image = Some((width, height, rgba));
    }

    /// 使用更小字节预算的同类内存剪贴板（便于测试）。
    #[must_use]
    pub fn with_limit(max_bytes: usize) -> Self {
        Self {
            text: None,
            image: None,
            files: Vec::new(),
            max_bytes,
        }
    }

    /// 不经由 trait 直接查看剪贴板内容。
    #[must_use]
    pub fn peek(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

impl ClipboardProvider for MemoryClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        Ok(self.text.clone())
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        if text.len() > self.max_bytes {
            return Err(ClipboardError::TooLarge {
                bytes: text.len(),
                limit: self.max_bytes,
            });
        }
        self.text = Some(text.to_owned());
        Ok(())
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.text = None;
        Ok(())
    }
}

/// 系统剪贴板实现。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemClipboard {
    max_bytes: usize,
}

impl SystemClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_bytes: MAX_CLIPBOARD_BYTES,
        }
    }

    /// 从剪贴板读取文件路径列表（例如从文件资源管理器复制的文件）。
    pub fn read_file_paths(&self) -> Result<Vec<std::path::PathBuf>, ClipboardError> {
        #[cfg(windows)]
        {
            win32::read_files()
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    /// 从剪贴板读取 RGBA 图像。
    pub fn read_image_rgba(&self) -> Result<Option<(u32, u32, Vec<u8>)>, ClipboardError> {
        #[cfg(windows)]
        {
            win32::read_image()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    /// 将 RGBA 图像写入剪贴板。
    pub fn write_image_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), ClipboardError> {
        #[cfg(windows)]
        {
            win32::write_image(width, height, rgba)
        }
        #[cfg(not(windows))]
        {
            let _ = (width, height, rgba);
            Err(ClipboardError::Unavailable)
        }
    }
}

#[cfg(windows)]
#[allow(
    non_snake_case,
    non_camel_case_types,
    clippy::upper_case_acronyms,
    dead_code
)]
mod win32 {
    //! 最小化 Win32 剪贴板绑定。
    //!
    //! 本 crate 禁止在 `unsafe fn` 内部执行 unsafe 操作；此处每处 FFI 调用均
    //! 包裹在显式 `unsafe` 块中，且句柄生命周期严格受限于下方的打开/关闭作用域。
    //!

    use core::ffi::c_void;

    type Bool = i32;
    type Handle = *mut c_void;

    const CF_UNICODETEXT: u32 = 13;
    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[repr(C)]
    struct BITMAPINFOHEADER {
        biSize: u32,
        biWidth: i32,
        biHeight: i32,
        biPlanes: u16,
        biBitCount: u16,
        biCompression: u32,
        biSizeImage: u32,
        biXPelsPerMeter: i32,
        biYPelsPerMeter: i32,
        biClrUsed: u32,
        biClrImportant: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> Bool;
        fn CloseClipboard() -> Bool;
        fn EmptyClipboard() -> Bool;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
        fn IsClipboardFormatAvailable(format: u32) -> Bool;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn DragQueryFileW(hDrop: Handle, iFile: u32, lpszFile: *mut u16, cch: u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalFree(memory: Handle) -> Handle;
        fn GlobalLock(memory: Handle) -> *mut c_void;
        fn GlobalUnlock(memory: Handle) -> Bool;
        fn lstrlenW(text: *const u16) -> i32;
    }

    pub(super) fn read(max_bytes: usize) -> Result<Option<String>, super::ClipboardError> {
        // SAFETY: 剪贴板在此作用域内打开、读取并关闭，指针仅在剪贴板持有
        // 所有权期间使用。
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let handle = GetClipboardData(CF_UNICODETEXT);
            if handle.is_null() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let pointer = GlobalLock(handle).cast::<u16>();
            if pointer.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let units = lstrlenW(pointer);
            if units < 0 {
                let _ = GlobalUnlock(handle);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let units = units as usize;
            // 每个 UTF-16 码元最多需要 4 个 UTF-8 字节；在解码前按最坏情况检查，
            // 从而在不分配内存的情况下拒绝超大粘贴。
            //
            if units.saturating_mul(4) > max_bytes {
                let _ = GlobalUnlock(handle);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::TooLarge {
                    bytes: units.saturating_mul(4),
                    limit: max_bytes,
                });
            }
            let slice = core::slice::from_raw_parts(pointer, units);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(handle);
            let _ = CloseClipboard();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text))
            }
        }
    }

    pub(super) fn write(text: &str, max_bytes: usize) -> Result<(), super::ClipboardError> {
        if text.len() > max_bytes {
            return Err(super::ClipboardError::TooLarge {
                bytes: text.len(),
                limit: max_bytes,
            });
        }
        let wide: Vec<u16> = text.encode_utf16().chain(core::iter::once(0)).collect();
        let bytes = core::mem::size_of_val(wide.as_slice());
        // SAFETY: 在所有权移交剪贴板之前完成缓冲区分配与拷贝；失败时释放内存。
        //
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return Err(super::ClipboardError::Unavailable);
            }
            if EmptyClipboard() == 0 {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if memory.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let destination = GlobalLock(memory).cast::<u16>();
            if destination.is_null() {
                let _ = GlobalFree(memory);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            core::ptr::copy_nonoverlapping(wide.as_ptr(), destination, wide.len());
            let _ = GlobalUnlock(memory);
            if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
                let _ = GlobalFree(memory);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let _ = CloseClipboard();
        }
        Ok(())
    }

    pub(super) fn read_files() -> Result<Vec<std::path::PathBuf>, super::ClipboardError> {
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_HDROP) == 0 {
                let _ = CloseClipboard();
                return Ok(Vec::new());
            }
            let handle = GetClipboardData(CF_HDROP);
            if handle.is_null() {
                let _ = CloseClipboard();
                return Ok(Vec::new());
            }
            let count = DragQueryFileW(handle, 0xFFFFFFFF, core::ptr::null_mut(), 0);
            let mut out = Vec::new();
            let mut buffer = vec![0u16; 1024];
            for i in 0..count {
                let len = DragQueryFileW(handle, i, core::ptr::null_mut(), 0) as usize;
                if len >= buffer.len() {
                    buffer.resize(len + 1, 0);
                }
                DragQueryFileW(handle, i, buffer.as_mut_ptr(), buffer.len() as u32);
                let path_str = String::from_utf16_lossy(&buffer[..len]);
                out.push(std::path::PathBuf::from(path_str));
            }
            let _ = CloseClipboard();
            Ok(out)
        }
    }

    pub(super) fn read_image() -> Result<Option<(u32, u32, Vec<u8>)>, super::ClipboardError> {
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_DIB) == 0 {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let handle = GetClipboardData(CF_DIB);
            if handle.is_null() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let pointer = GlobalLock(handle).cast::<u8>();
            if pointer.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let header = &*(pointer as *const BITMAPINFOHEADER);
            let width = header.biWidth.unsigned_abs();
            let height = header.biHeight.unsigned_abs();
            let bpp = header.biBitCount;

            if (bpp != 24 && bpp != 32)
                || width == 0
                || height == 0
                || width > 8192
                || height > 8192
            {
                let _ = GlobalUnlock(handle);
                let _ = CloseClipboard();
                return Ok(None);
            }

            let pixel_offset = header.biSize as usize;
            let pixel_bytes = pointer.add(pixel_offset);
            let row_stride = (((width * bpp as u32 + 31) & !31) / 8) as usize;
            let mut rgba = vec![0u8; (width * height * 4) as usize];

            let is_top_down = header.biHeight < 0;

            for y in 0..height {
                let src_y = if is_top_down { y } else { height - 1 - y };
                let src_row = pixel_bytes.add(src_y as usize * row_stride);
                let dst_row = &mut rgba[(y * width * 4) as usize..((y + 1) * width * 4) as usize];

                if bpp == 32 {
                    for x in 0..width as usize {
                        let b = *src_row.add(x * 4);
                        let g = *src_row.add(x * 4 + 1);
                        let r = *src_row.add(x * 4 + 2);
                        let a = *src_row.add(x * 4 + 3);
                        dst_row[x * 4] = r;
                        dst_row[x * 4 + 1] = g;
                        dst_row[x * 4 + 2] = b;
                        dst_row[x * 4 + 3] = if a == 0 { 255 } else { a };
                    }
                } else if bpp == 24 {
                    for x in 0..width as usize {
                        let b = *src_row.add(x * 3);
                        let g = *src_row.add(x * 3 + 1);
                        let r = *src_row.add(x * 3 + 2);
                        dst_row[x * 4] = r;
                        dst_row[x * 4 + 1] = g;
                        dst_row[x * 4 + 2] = b;
                        dst_row[x * 4 + 3] = 255;
                    }
                }
            }

            let _ = GlobalUnlock(handle);
            let _ = CloseClipboard();
            Ok(Some((width, height, rgba)))
        }
    }

    pub(super) fn write_image(
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), super::ClipboardError> {
        if rgba.len() != (width * height * 4) as usize {
            return Err(super::ClipboardError::Os);
        }
        let header_size = core::mem::size_of::<BITMAPINFOHEADER>();
        let image_size = (width * height * 4) as usize;
        let total_size = header_size + image_size;

        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return Err(super::ClipboardError::Unavailable);
            }
            if EmptyClipboard() == 0 {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let memory = GlobalAlloc(GMEM_MOVEABLE, total_size);
            if memory.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let dest = GlobalLock(memory).cast::<u8>();
            if dest.is_null() {
                let _ = GlobalFree(memory);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }

            let header = BITMAPINFOHEADER {
                biSize: header_size as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                biSizeImage: image_size as u32,
                biXPelsPerMeter: 3780, // ~96 DPI
                biYPelsPerMeter: 3780,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            core::ptr::copy_nonoverlapping(&header as *const _ as *const u8, dest, header_size);

            let dest_pixels = dest.add(header_size);
            for i in 0..(width * height) as usize {
                let r = rgba[i * 4];
                let g = rgba[i * 4 + 1];
                let b = rgba[i * 4 + 2];
                let a = rgba[i * 4 + 3];
                *dest_pixels.add(i * 4) = b;
                *dest_pixels.add(i * 4 + 1) = g;
                *dest_pixels.add(i * 4 + 2) = r;
                *dest_pixels.add(i * 4 + 3) = a;
            }

            let _ = GlobalUnlock(memory);
            if SetClipboardData(CF_DIB, memory).is_null() {
                let _ = GlobalFree(memory);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let _ = CloseClipboard();
            Ok(())
        }
    }
}

impl ClipboardProvider for SystemClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        #[cfg(windows)]
        {
            win32::read(self.max_bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = self.max_bytes;
            Err(ClipboardError::Unavailable)
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        #[cfg(windows)]
        {
            win32::write(text, self.max_bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = (text, self.max_bytes);
            Err(ClipboardError::Unavailable)
        }
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.write_text("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_clipboard_round_trips_text() {
        let mut clipboard = MemoryClipboard::new();
        assert_eq!(clipboard.read_text(), Ok(None));
        clipboard.write_text("你好, unbound").unwrap();
        assert_eq!(
            clipboard.read_text().unwrap().as_deref(),
            Some("你好, unbound")
        );
        clipboard.clear().unwrap();
        assert_eq!(clipboard.read_text(), Ok(None));
    }

    #[test]
    fn an_oversized_payload_is_refused_before_it_is_stored() {
        let mut clipboard = MemoryClipboard::with_limit(8);
        let error = clipboard.write_text("more than eight bytes").unwrap_err();
        assert_eq!(
            error,
            ClipboardError::TooLarge {
                bytes: 21,
                limit: 8
            }
        );
        assert_eq!(clipboard.peek(), None);
    }

    #[test]
    fn a_system_clipboard_is_constructed_with_the_default_limit() {
        let mut clipboard = SystemClipboard::new();
        // 在无头或锁屏桌面上允许读取失败；绝对不能发生的是 panic 或无界内存分配。
        //
        match clipboard.read_text() {
            Ok(Some(text)) => assert!(text.len() <= MAX_CLIPBOARD_BYTES),
            Ok(None) | Err(ClipboardError::Unavailable) | Err(ClipboardError::Os) => {}
            Err(ClipboardError::TooLarge { limit, .. }) => assert_eq!(limit, MAX_CLIPBOARD_BYTES),
        }
    }

    #[test]
    fn memory_clipboard_supports_files_and_images() {
        let mut clipboard = MemoryClipboard::new();
        assert_eq!(
            clipboard.read_file_paths(),
            Vec::<std::path::PathBuf>::new()
        );
        assert_eq!(clipboard.read_image_rgba(), None);

        let files = vec![
            std::path::PathBuf::from(r"C:\test\sample.png"),
            std::path::PathBuf::from(r"C:\test\document.pdf"),
        ];
        clipboard.write_file_paths(files.clone());
        assert_eq!(clipboard.read_file_paths(), files);

        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
        clipboard.write_image_rgba(2, 1, rgba.clone());
        assert_eq!(clipboard.read_image_rgba(), Some((2, 1, rgba)));
    }
}

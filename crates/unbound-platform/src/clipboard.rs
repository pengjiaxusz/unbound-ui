//! 剪贴板抽象层。
//!
//! 剪贴板属于平台服务，因此将策略层与系统实现层分离：
//!
//! * [`ClipboardProvider`] 是编辑器交互的 trait。
//! * [`MemoryClipboard`] 是测试与无头运行使用的确定性实现。
//! * [`SystemClipboard`] 是基于 `windows` crate（0.62.2）的 Windows 实现；在其他平台上返回
//!   [`ClipboardError::Unavailable`] 而非静默无操作。
//!
//! 每次读取均受限于 [`MAX_CLIPBOARD_BYTES`]。粘贴操作绝不能在应用有机会
//! 拒绝之前将恶意的 2 GB 剪贴板载荷变成 4 GB 的 `String`——上限检查在
//! 任何大内存分配前执行，错误中会报告被拒绝的字节大小。

/// 引擎可接受的最大剪贴板载荷（以 UTF-8 字节计）。
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

/// GNOME / Freedesktop clipboard MIME type for file copy/cut operations.
pub const GNOME_COPIED_FILES_MIME: &str = "x-special/gnome-copied-files";

/// File transfer semantics attached to clipboard file-list operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClipboardFileDropEffect {
    /// Copy the referenced files to the paste destination.
    #[default]
    Copy,
    /// Move (cut) the referenced files to the paste destination.
    Move,
}

impl ClipboardFileDropEffect {
    /// Map to Win32 `DROPEFFECT_*` bitmask (`DROPEFFECT_COPY = 1`, `DROPEFFECT_MOVE = 2`).
    #[must_use]
    pub const fn win32_drop_effect_mask(self) -> u32 {
        match self {
            Self::Copy => crate::ole_drag::DROPEFFECT_COPY_MASK,
            Self::Move => crate::ole_drag::DROPEFFECT_MOVE_MASK,
        }
    }

    /// Parse a Win32 `Preferred DropEffect` bitmask into [`ClipboardFileDropEffect`].
    #[must_use]
    pub const fn from_win32_drop_effect_mask(mask: u32) -> Option<Self> {
        if (mask & crate::ole_drag::DROPEFFECT_MOVE_MASK) != 0 {
            Some(Self::Move)
        } else if (mask & crate::ole_drag::DROPEFFECT_COPY_MASK) != 0 {
            Some(Self::Copy)
        } else {
            None
        }
    }

    /// Return the action header token used by `x-special/gnome-copied-files`.
    #[must_use]
    pub const fn gnome_action_name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "cut",
        }
    }
}

/// Encode file paths and drop effect into the `x-special/gnome-copied-files` format.
///
/// Format: `<copy|cut>\nfile:///path/one\nfile:///path/two`
#[must_use]
pub fn encode_gnome_copied_files(
    paths: &[std::path::PathBuf],
    effect: ClipboardFileDropEffect,
) -> String {
    let uri_list = crate::linux_dnd::encode_uri_list(paths);
    let mut out = String::with_capacity(uri_list.len() + 8);
    out.push_str(effect.gnome_action_name());
    for line in uri_list.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            out.push('\n');
            out.push_str(trimmed);
        }
    }
    out
}

/// Decode an `x-special/gnome-copied-files` payload into its drop effect and local file paths.
#[must_use]
pub fn decode_gnome_copied_files(
    raw: &str,
) -> Option<(ClipboardFileDropEffect, Vec<std::path::PathBuf>)> {
    let mut lines = raw.lines();
    let header = lines.next()?.trim();
    let effect = match header {
        "copy" => ClipboardFileDropEffect::Copy,
        "cut" => ClipboardFileDropEffect::Move,
        _ => return None,
    };
    let rest: String = lines.collect::<Vec<_>>().join("\n");
    let paths = crate::linux_dnd::decode_uri_list(&rest);
    Some((effect, paths))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HeadlessFileClipboard {
    paths: Vec<std::path::PathBuf>,
    effect: ClipboardFileDropEffect,
}

static HEADLESS_FILE_LOOPBACK: std::sync::OnceLock<
    std::sync::Mutex<Option<HeadlessFileClipboard>>,
> = std::sync::OnceLock::new();

/// Enable or disable the in-memory file clipboard loopback for headless and test environments.
pub fn set_headless_clipboard_loopback(enabled: bool) {
    let lock = HEADLESS_FILE_LOOPBACK.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        if enabled {
            *guard = Some(HeadlessFileClipboard::default());
        } else {
            *guard = None;
        }
    }
}

/// Return whether the in-memory headless file clipboard loopback is currently enabled.
#[must_use]
pub fn is_headless_clipboard_loopback_enabled() -> bool {
    let lock = HEADLESS_FILE_LOOPBACK.get_or_init(|| std::sync::Mutex::new(None));
    lock.lock().ok().is_some_and(|guard| guard.is_some())
}

/// 剪贴板操作失败原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// 剪贴板服务不可用（不支持的平台，或剪贴板被其他进程锁定）。
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
    file_effect: ClipboardFileDropEffect,
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
            file_effect: ClipboardFileDropEffect::Copy,
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
        self.file_effect = ClipboardFileDropEffect::Copy;
    }

    /// Store file paths together with a copy/move drop effect in memory.
    pub fn write_file_paths_with_effect(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        effect: ClipboardFileDropEffect,
    ) {
        self.files = paths;
        self.file_effect = effect;
    }

    /// Read the stored file drop effect from the in-memory clipboard.
    #[must_use]
    pub const fn read_file_drop_effect(&self) -> ClipboardFileDropEffect {
        self.file_effect
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
            file_effect: ClipboardFileDropEffect::Copy,
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

    /// Create a system clipboard instance with a custom byte-size limit.
    #[must_use]
    pub const fn with_limit(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    fn effective_limit(&self) -> usize {
        if self.max_bytes == 0 {
            MAX_CLIPBOARD_BYTES
        } else {
            self.max_bytes
        }
    }

    /// 从剪贴板读取文件路径列表（例如从文件资源管理器复制的文件）。
    pub fn read_file_paths(&self) -> Result<Vec<std::path::PathBuf>, ClipboardError> {
        if let Some(lock) = HEADLESS_FILE_LOOPBACK.get() {
            if let Ok(guard) = lock.lock() {
                if let Some(loopback) = guard.as_ref() {
                    return Ok(loopback.paths.clone());
                }
            }
        }
        #[cfg(windows)]
        {
            win32::read_files()
        }
        #[cfg(not(windows))]
        {
            Ok(crate::wayland_logic::read_wayland_clipboard_file_paths().unwrap_or_default())
        }
    }

    /// Read the preferred file drop effect (`Copy` or `Move`) associated with clipboard files.
    pub fn read_file_drop_effect(&self) -> Result<Option<ClipboardFileDropEffect>, ClipboardError> {
        if let Some(lock) = HEADLESS_FILE_LOOPBACK.get() {
            if let Ok(guard) = lock.lock() {
                if let Some(loopback) = guard.as_ref() {
                    return Ok((!loopback.paths.is_empty()).then_some(loopback.effect));
                }
            }
        }
        #[cfg(windows)]
        {
            win32::read_preferred_drop_effect()
        }
        #[cfg(not(windows))]
        {
            Ok(crate::wayland_logic::read_wayland_clipboard_file_drop_effect())
        }
    }

    /// Write file paths and their copy/move drop effect to the system clipboard.
    ///
    /// - **Windows**: writes `CF_HDROP` (`DROPFILES` UTF-16LE) and `Preferred DropEffect`.
    /// - **Linux**: publishes `text/uri-list` and `x-special/gnome-copied-files` through the active Wayland/X11 clipboard bridge.
    /// - **Headless loopback**: when [`set_headless_clipboard_loopback`] is enabled, stores paths and effect in memory for deterministic testing.
    pub fn write_file_paths(
        &self,
        paths: &[std::path::PathBuf],
        effect: ClipboardFileDropEffect,
    ) -> Result<(), ClipboardError> {
        let hdrop_payload =
            crate::ole_drag::build_cf_hdrop_payload(paths).map_err(|_| ClipboardError::Os)?;
        let limit = self.effective_limit();
        if hdrop_payload.len() > limit {
            return Err(ClipboardError::TooLarge {
                bytes: hdrop_payload.len(),
                limit,
            });
        }
        if let Some(lock) = HEADLESS_FILE_LOOPBACK.get() {
            if let Ok(mut guard) = lock.lock() {
                if let Some(loopback) = guard.as_mut() {
                    loopback.paths = paths.to_vec();
                    loopback.effect = effect;
                    return Ok(());
                }
            }
        }
        #[cfg(windows)]
        {
            win32::write_files(&hdrop_payload, effect)
        }
        #[cfg(not(windows))]
        {
            let uri_list = crate::linux_dnd::encode_uri_list(paths);
            let gnome_payload = encode_gnome_copied_files(paths, effect);
            if uri_list.len() > limit || gnome_payload.len() > limit {
                return Err(ClipboardError::TooLarge {
                    bytes: uri_list.len().max(gnome_payload.len()),
                    limit,
                });
            }
            if crate::wayland_logic::write_wayland_clipboard_file_paths(
                paths,
                effect,
                &uri_list,
                &gnome_payload,
            ) {
                Ok(())
            } else {
                Err(ClipboardError::Unavailable)
            }
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
mod win32 {
    //! 基于官方 `windows = "0.62.2"` crate 的 Win32 剪贴板实现。
    //!
    //! 句柄生命周期严格受限于每次打开/关闭剪贴板的作用域。

    use ::windows::core::w;
    use ::windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use ::windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
    use ::windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use ::windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use ::windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    const CF_DIB: u32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;

    pub(super) fn read(max_bytes: usize) -> Result<Option<String>, super::ClipboardError> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let Ok(handle) = GetClipboardData(CF_UNICODETEXT) else {
                let _ = CloseClipboard();
                return Ok(None);
            };
            if handle.is_invalid() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let hglobal = HGLOBAL(handle.0);
            let max_units = GlobalSize(hglobal) / core::mem::size_of::<u16>();
            let pointer = GlobalLock(hglobal).cast::<u16>();
            if pointer.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let mut units = 0usize;
            while units < max_units && *pointer.add(units) != 0 {
                units += 1;
            }
            // 每个 UTF-16 码元最多需要 4 个 UTF-8 字节；在解码前按最坏情况检查，
            // 从而在不分配内存的情况下拒绝超大粘贴。
            if units.saturating_mul(4) > max_bytes {
                let _ = GlobalUnlock(hglobal);
                let _ = CloseClipboard();
                return Err(super::ClipboardError::TooLarge {
                    bytes: units.saturating_mul(4),
                    limit: max_bytes,
                });
            }
            let slice = core::slice::from_raw_parts(pointer, units);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(hglobal);
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
        unsafe {
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if EmptyClipboard().is_err() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            };
            let destination = GlobalLock(memory).cast::<u16>();
            if destination.is_null() {
                let _ = GlobalFree(Some(memory));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            core::ptr::copy_nonoverlapping(wide.as_ptr(), destination, wide.len());
            let _ = GlobalUnlock(memory);
            if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(memory));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let _ = CloseClipboard();
        }
        Ok(())
    }

    pub(super) fn read_files() -> Result<Vec<std::path::PathBuf>, super::ClipboardError> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_HDROP).is_err() {
                let _ = CloseClipboard();
                return Ok(Vec::new());
            }
            let Ok(handle) = GetClipboardData(CF_HDROP) else {
                let _ = CloseClipboard();
                return Ok(Vec::new());
            };
            if handle.is_invalid() {
                let _ = CloseClipboard();
                return Ok(Vec::new());
            }
            let hdrop = HDROP(handle.0);
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            let mut out = Vec::new();
            let mut buffer = vec![0u16; 1024];
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                if len >= buffer.len() {
                    buffer.resize(len + 1, 0);
                }
                let copied = DragQueryFileW(hdrop, i, Some(&mut buffer)) as usize;
                if copied > 0 {
                    let path_str = String::from_utf16_lossy(&buffer[..copied]);
                    out.push(std::path::PathBuf::from(path_str));
                }
            }
            let _ = CloseClipboard();
            Ok(out)
        }
    }

    pub(super) fn read_preferred_drop_effect(
    ) -> Result<Option<super::ClipboardFileDropEffect>, super::ClipboardError> {
        unsafe {
            let format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if format == 0 {
                return Ok(None);
            }
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(format).is_err() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let Ok(handle) = GetClipboardData(format) else {
                let _ = CloseClipboard();
                return Ok(None);
            };
            if handle.is_invalid() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let hglobal = HGLOBAL(handle.0);
            if GlobalSize(hglobal) < core::mem::size_of::<u32>() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let pointer = GlobalLock(hglobal).cast::<u8>();
            if pointer.is_null() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let mut raw = [0u8; 4];
            core::ptr::copy_nonoverlapping(pointer, raw.as_mut_ptr(), 4);
            let _ = GlobalUnlock(hglobal);
            let _ = CloseClipboard();
            let mask = u32::from_le_bytes(raw);
            Ok(super::ClipboardFileDropEffect::from_win32_drop_effect_mask(
                mask,
            ))
        }
    }

    pub(super) fn write_files(
        hdrop_payload: &[u8],
        effect: super::ClipboardFileDropEffect,
    ) -> Result<(), super::ClipboardError> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if EmptyClipboard().is_err() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }

            let Ok(hdrop_mem) = GlobalAlloc(GMEM_MOVEABLE, hdrop_payload.len()) else {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            };
            let hdrop_dst = GlobalLock(hdrop_mem).cast::<u8>();
            if hdrop_dst.is_null() {
                let _ = GlobalFree(Some(hdrop_mem));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            core::ptr::copy_nonoverlapping(hdrop_payload.as_ptr(), hdrop_dst, hdrop_payload.len());
            let _ = GlobalUnlock(hdrop_mem);
            if SetClipboardData(CF_HDROP, Some(HANDLE(hdrop_mem.0))).is_err() {
                let _ = GlobalFree(Some(hdrop_mem));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }

            let preferred_fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if preferred_fmt == 0 {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let effect_bytes = effect.win32_drop_effect_mask().to_le_bytes();
            let Ok(effect_mem) = GlobalAlloc(GMEM_MOVEABLE, effect_bytes.len()) else {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            };
            let effect_dst = GlobalLock(effect_mem).cast::<u8>();
            if effect_dst.is_null() {
                let _ = GlobalFree(Some(effect_mem));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            core::ptr::copy_nonoverlapping(effect_bytes.as_ptr(), effect_dst, effect_bytes.len());
            let _ = GlobalUnlock(effect_mem);
            if SetClipboardData(preferred_fmt, Some(HANDLE(effect_mem.0))).is_err() {
                let _ = GlobalFree(Some(effect_mem));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }

            let _ = CloseClipboard();
            Ok(())
        }
    }

    pub(super) fn read_image() -> Result<Option<(u32, u32, Vec<u8>)>, super::ClipboardError> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if IsClipboardFormatAvailable(CF_DIB).is_err() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let Ok(handle) = GetClipboardData(CF_DIB) else {
                let _ = CloseClipboard();
                return Ok(None);
            };
            if handle.is_invalid() {
                let _ = CloseClipboard();
                return Ok(None);
            }
            let hglobal = HGLOBAL(handle.0);
            let pointer = GlobalLock(hglobal).cast::<u8>();
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
                let _ = GlobalUnlock(hglobal);
                let _ = CloseClipboard();
                return Ok(None);
            }

            let pixel_offset = header.biSize as usize;
            let pixel_bytes = pointer.add(pixel_offset);
            let row_stride = (((width * u32::from(bpp) + 31) & !31) / 8) as usize;
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

            let _ = GlobalUnlock(hglobal);
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
            if OpenClipboard(None).is_err() {
                return Err(super::ClipboardError::Unavailable);
            }
            if EmptyClipboard().is_err() {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }
            let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, total_size) else {
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            };
            let dest = GlobalLock(memory).cast::<u8>();
            if dest.is_null() {
                let _ = GlobalFree(Some(memory));
                let _ = CloseClipboard();
                return Err(super::ClipboardError::Os);
            }

            let header = BITMAPINFOHEADER {
                biSize: header_size as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: image_size as u32,
                biXPelsPerMeter: 3780, // ~96 DPI
                biYPelsPerMeter: 3780,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            core::ptr::copy_nonoverlapping((&raw const header).cast::<u8>(), dest, header_size);

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
            if SetClipboardData(CF_DIB, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(memory));
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
            let Some(text) = crate::wayland_logic::read_wayland_clipboard_text() else {
                return Ok(None);
            };
            if text.len() > self.max_bytes {
                return Err(ClipboardError::TooLarge {
                    bytes: text.len(),
                    limit: self.max_bytes,
                });
            }
            Ok(Some(text))
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        #[cfg(windows)]
        {
            win32::write(text, self.max_bytes)
        }
        #[cfg(not(windows))]
        {
            if text.len() > self.max_bytes {
                return Err(ClipboardError::TooLarge {
                    bytes: text.len(),
                    limit: self.max_bytes,
                });
            }
            if crate::wayland_logic::write_wayland_clipboard_text(text) {
                Ok(())
            } else {
                Err(ClipboardError::Unavailable)
            }
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
        clipboard.write_text("你好, unbound").unwrap(); // lang-ok: CJK clipboard sample
        assert_eq!(
            clipboard.read_text().unwrap().as_deref(),
            Some("你好, unbound") // lang-ok: CJK clipboard round-trip
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

    #[test]
    fn memory_clipboard_tracks_file_drop_effect() {
        let mut clipboard = MemoryClipboard::new();
        assert_eq!(
            clipboard.read_file_drop_effect(),
            ClipboardFileDropEffect::Copy
        );
        let files = vec![std::path::PathBuf::from("/tmp/example.txt")];
        clipboard.write_file_paths_with_effect(files.clone(), ClipboardFileDropEffect::Move);
        assert_eq!(clipboard.read_file_paths(), files);
        assert_eq!(
            clipboard.read_file_drop_effect(),
            ClipboardFileDropEffect::Move
        );
    }

    #[test]
    fn system_clipboard_headless_loopback_round_trips_file_paths() {
        static LOOPBACK_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOOPBACK_TEST_GUARD.lock().unwrap();
        set_headless_clipboard_loopback(true);
        assert!(is_headless_clipboard_loopback_enabled());

        let clipboard = SystemClipboard::new();
        assert_eq!(clipboard.read_file_paths(), Ok(Vec::new()));
        assert_eq!(clipboard.read_file_drop_effect(), Ok(None));

        let files = vec![
            std::path::PathBuf::from("/workspace/alpha.rs"),
            std::path::PathBuf::from("/workspace/beta file.txt"),
        ];
        clipboard
            .write_file_paths(&files, ClipboardFileDropEffect::Copy)
            .unwrap();
        assert_eq!(clipboard.read_file_paths(), Ok(files.clone()));
        assert_eq!(
            clipboard.read_file_drop_effect(),
            Ok(Some(ClipboardFileDropEffect::Copy))
        );

        clipboard
            .write_file_paths(&files, ClipboardFileDropEffect::Move)
            .unwrap();
        assert_eq!(clipboard.read_file_paths(), Ok(files));
        assert_eq!(
            clipboard.read_file_drop_effect(),
            Ok(Some(ClipboardFileDropEffect::Move))
        );

        // Empty file list is rejected before mutating clipboard state.
        assert_eq!(
            clipboard.write_file_paths(&[], ClipboardFileDropEffect::Copy),
            Err(ClipboardError::Os)
        );

        // Oversized payload is rejected before writing.
        let tiny = SystemClipboard::with_limit(16);
        let oversized = tiny
            .write_file_paths(
                &[std::path::PathBuf::from("/very/long/path/example.bin")],
                ClipboardFileDropEffect::Copy,
            )
            .unwrap_err();
        assert!(matches!(
            oversized,
            ClipboardError::TooLarge { limit: 16, .. }
        ));

        set_headless_clipboard_loopback(false);
        assert!(!is_headless_clipboard_loopback_enabled());
    }

    #[test]
    fn gnome_copied_files_codec_round_trips_copy_and_move() {
        let paths = vec![
            std::path::PathBuf::from("/home/user/notes.md"),
            std::path::PathBuf::from("/home/user/my file.txt"),
        ];
        let encoded_copy = encode_gnome_copied_files(&paths, ClipboardFileDropEffect::Copy);
        assert!(encoded_copy.starts_with("copy\nfile:///home/user/notes.md\n"));
        assert_eq!(
            decode_gnome_copied_files(&encoded_copy),
            Some((ClipboardFileDropEffect::Copy, paths.clone()))
        );

        let encoded_move = encode_gnome_copied_files(&paths, ClipboardFileDropEffect::Move);
        assert!(encoded_move.starts_with("cut\nfile:///home/user/notes.md\n"));
        assert_eq!(
            decode_gnome_copied_files(&encoded_move),
            Some((ClipboardFileDropEffect::Move, paths))
        );

        assert_eq!(decode_gnome_copied_files("invalid\nfile:///a"), None);
    }

    #[test]
    fn drop_effect_win32_mask_mapping_matches_shell_constants() {
        assert_eq!(ClipboardFileDropEffect::Copy.win32_drop_effect_mask(), 1);
        assert_eq!(ClipboardFileDropEffect::Move.win32_drop_effect_mask(), 2);
        assert_eq!(
            ClipboardFileDropEffect::from_win32_drop_effect_mask(1),
            Some(ClipboardFileDropEffect::Copy)
        );
        assert_eq!(
            ClipboardFileDropEffect::from_win32_drop_effect_mask(2),
            Some(ClipboardFileDropEffect::Move)
        );
        assert_eq!(
            ClipboardFileDropEffect::from_win32_drop_effect_mask(0),
            None
        );
    }
}

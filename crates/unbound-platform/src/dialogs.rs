//! 原生 Windows 10/11 文件对话框。
//!
//! 基于 Win32 `comdlg32` 提供同步与非阻塞后台文件选择对话框，
//! 无需引入沉重的外部依赖。

use std::path::PathBuf;

/// 文件打开/保存对话框的配置选项。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDialogConfig {
    /// 对话框显示的窗口标题。
    pub title: Option<String>,
    /// 打开时的初始目录。
    pub initial_dir: Option<PathBuf>,
    /// 保存对话框预填的默认文件名。
    pub default_name: Option<String>,
    /// `(显示名称, 通配模式)` 过滤器对，例如 `("PNG Image (*.png)", "*.png")`。
    pub filters: Vec<(String, String)>,
    /// 是否允许选择多个文件（仅适用于打开对话框）。
    pub multi_select: bool,
}

impl FileDialogConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn initial_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.initial_dir = Some(dir.into());
        self
    }

    #[must_use]
    pub fn default_name(mut self, name: impl Into<String>) -> Self {
        self.default_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn filter(mut self, name: impl Into<String>, pattern: impl Into<String>) -> Self {
        self.filters.push((name.into(), pattern.into()));
        self
    }

    #[must_use]
    pub fn multi_select(mut self, multi: bool) -> Self {
        self.multi_select = multi;
        self
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
    use super::*;
    use core::ffi::c_void;

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type DWORD = u32;
    type BOOL = i32;
    type LPCWSTR = *const u16;
    type LPWSTR = *mut u16;

    const OFN_EXPLORER: DWORD = 0x00080000;
    const OFN_FILEMUSTEXIST: DWORD = 0x00001000;
    const OFN_PATHMUSTEXIST: DWORD = 0x00000800;
    const OFN_ALLOWMULTISELECT: DWORD = 0x00000200;
    const OFN_OVERWRITEPROMPT: DWORD = 0x00000002;
    const OFN_NOCHANGEDIR: DWORD = 0x00000008;

    #[repr(C)]
    struct OPENFILENAMEW {
        lStructSize: DWORD,
        hwndOwner: HWND,
        hInstance: HINSTANCE,
        lpstrFilter: LPCWSTR,
        lpstrCustomFilter: LPWSTR,
        nMaxCustFilter: DWORD,
        nFilterIndex: DWORD,
        lpstrFile: LPWSTR,
        nMaxFile: DWORD,
        lpstrFileTitle: LPWSTR,
        nMaxFileTitle: DWORD,
        lpstrInitialDir: LPCWSTR,
        lpstrTitle: LPCWSTR,
        Flags: DWORD,
        nFileOffset: u16,
        nFileExtension: u16,
        lpstrDefExt: LPCWSTR,
        lCustData: usize,
        lpfnHook: *mut c_void,
        lpTemplateName: LPCWSTR,
        pvReserved: *mut c_void,
        dwReserved: DWORD,
        FlagsEx: DWORD,
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(lpofn: *mut OPENFILENAMEW) -> BOOL;
        fn GetSaveFileNameW(lpofn: *mut OPENFILENAMEW) -> BOOL;
    }

    fn build_filter_string(filters: &[(String, String)]) -> Vec<u16> {
        let mut out = Vec::new();
        if filters.is_empty() {
            out.extend("All Files (*.*)\0*.*\0".encode_utf16());
        } else {
            for (name, pat) in filters {
                out.extend(format!("{name}\0{pat}\0").encode_utf16());
            }
        }
        out.push(0);
        out
    }

    pub fn pick_open(config: &FileDialogConfig) -> Vec<PathBuf> {
        let mut buffer = vec![0u16; 65536];
        let filter_str = build_filter_string(&config.filters);
        let title_w: Option<Vec<u16>> = config
            .title
            .as_ref()
            .map(|t| t.encode_utf16().chain(core::iter::once(0)).collect());
        let init_dir_w: Option<Vec<u16>> = config
            .initial_dir
            .as_ref()
            .and_then(|p| p.to_str())
            .map(|s| s.encode_utf16().chain(core::iter::once(0)).collect());

        let mut flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if config.multi_select {
            flags |= OFN_ALLOWMULTISELECT;
        }

        let mut ofn = OPENFILENAMEW {
            lStructSize: core::mem::size_of::<OPENFILENAMEW>() as DWORD,
            hwndOwner: core::ptr::null_mut(),
            hInstance: core::ptr::null_mut(),
            lpstrFilter: filter_str.as_ptr(),
            lpstrCustomFilter: core::ptr::null_mut(),
            nMaxCustFilter: 0,
            nFilterIndex: 1,
            lpstrFile: buffer.as_mut_ptr(),
            nMaxFile: buffer.len() as DWORD,
            lpstrFileTitle: core::ptr::null_mut(),
            nMaxFileTitle: 0,
            lpstrInitialDir: init_dir_w
                .as_ref()
                .map_or(core::ptr::null(), |v| v.as_ptr()),
            lpstrTitle: title_w.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()),
            Flags: flags,
            nFileOffset: 0,
            nFileExtension: 0,
            lpstrDefExt: core::ptr::null(),
            lCustData: 0,
            lpfnHook: core::ptr::null_mut(),
            lpTemplateName: core::ptr::null(),
            pvReserved: core::ptr::null_mut(),
            dwReserved: 0,
            FlagsEx: 0,
        };

        let ok = unsafe { GetOpenFileNameW(&mut ofn) };
        if ok == 0 {
            return Vec::new();
        }

        // 从缓冲区解析以 NUL 分隔的字符串列表
        let mut results = Vec::new();
        let mut parts = Vec::new();
        let mut start = 0;
        for i in 0..buffer.len() {
            if buffer[i] == 0 {
                if i > start {
                    parts.push(String::from_utf16_lossy(&buffer[start..i]));
                }
                start = i + 1;
                if i + 1 < buffer.len() && buffer[i + 1] == 0 {
                    break;
                }
            }
        }

        if parts.len() == 1 {
            results.push(PathBuf::from(&parts[0]));
        } else if parts.len() > 1 {
            let base_dir = PathBuf::from(&parts[0]);
            for file_name in &parts[1..] {
                results.push(base_dir.join(file_name));
            }
        }

        results
    }

    pub fn pick_save(config: &FileDialogConfig) -> Option<PathBuf> {
        let mut buffer = vec![0u16; 4096];
        if let Some(default) = &config.default_name {
            for (idx, ch) in default.encode_utf16().enumerate().take(4000) {
                buffer[idx] = ch;
            }
        }
        let filter_str = build_filter_string(&config.filters);
        let title_w: Option<Vec<u16>> = config
            .title
            .as_ref()
            .map(|t| t.encode_utf16().chain(core::iter::once(0)).collect());
        let init_dir_w: Option<Vec<u16>> = config
            .initial_dir
            .as_ref()
            .and_then(|p| p.to_str())
            .map(|s| s.encode_utf16().chain(core::iter::once(0)).collect());

        let flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR;

        let mut ofn = OPENFILENAMEW {
            lStructSize: core::mem::size_of::<OPENFILENAMEW>() as DWORD,
            hwndOwner: core::ptr::null_mut(),
            hInstance: core::ptr::null_mut(),
            lpstrFilter: filter_str.as_ptr(),
            lpstrCustomFilter: core::ptr::null_mut(),
            nMaxCustFilter: 0,
            nFilterIndex: 1,
            lpstrFile: buffer.as_mut_ptr(),
            nMaxFile: buffer.len() as DWORD,
            lpstrFileTitle: core::ptr::null_mut(),
            nMaxFileTitle: 0,
            lpstrInitialDir: init_dir_w
                .as_ref()
                .map_or(core::ptr::null(), |v| v.as_ptr()),
            lpstrTitle: title_w.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()),
            Flags: flags,
            nFileOffset: 0,
            nFileExtension: 0,
            lpstrDefExt: core::ptr::null(),
            lCustData: 0,
            lpfnHook: core::ptr::null_mut(),
            lpTemplateName: core::ptr::null(),
            pvReserved: core::ptr::null_mut(),
            dwReserved: 0,
            FlagsEx: 0,
        };

        let ok = unsafe { GetSaveFileNameW(&mut ofn) };
        if ok == 0 {
            return None;
        }

        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        let path_str = String::from_utf16_lossy(&buffer[..len]);
        if path_str.is_empty() {
            None
        } else {
            Some(PathBuf::from(path_str))
        }
    }
}

/// 同步打开单文件选择对话框。
#[must_use]
pub fn open_file(config: FileDialogConfig) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        win32::pick_open(&config).into_iter().next()
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        None
    }
}

/// 同步打开多文件选择对话框。
#[must_use]
pub fn open_files(config: FileDialogConfig) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut cfg = config;
        cfg.multi_select = true;
        win32::pick_open(&cfg)
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        Vec::new()
    }
}

/// 同步打开保存文件对话框。
#[must_use]
pub fn save_file(config: FileDialogConfig) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        win32::pick_save(&config)
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        None
    }
}

/// 在工作线程上运行的非阻塞异步打开文件选择对话框。
pub fn open_file_async<F: FnOnce(Option<PathBuf>) + Send + 'static>(
    config: FileDialogConfig,
    on_complete: F,
) {
    std::thread::spawn(move || {
        let result = open_file(config);
        on_complete(result);
    });
}

/// 在工作线程上运行的非阻塞异步保存文件选择对话框。
pub fn save_file_async<F: FnOnce(Option<PathBuf>) + Send + 'static>(
    config: FileDialogConfig,
    on_complete: F,
) {
    std::thread::spawn(move || {
        let result = save_file(config);
        on_complete(result);
    });
}

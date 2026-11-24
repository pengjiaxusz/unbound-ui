//! 原生 Windows 10/11 文件对话框。
//!
//! 基于官方 `windows = "0.62.2"` crate 的 `Win32::UI::Controls::Dialogs` 提供同步与非阻塞
//! 后台文件选择对话框，无需手写 FFI 绑定。

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
mod win32 {
    use super::*;
    use ::windows::core::{PCWSTR, PWSTR};
    use ::windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
        OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

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
            lStructSize: core::mem::size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: PCWSTR(filter_str.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrInitialDir: init_dir_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            lpstrTitle: title_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            Flags: flags,
            ..Default::default()
        };

        let ok = unsafe { GetOpenFileNameW(&mut ofn) }.as_bool();
        if !ok {
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
            lStructSize: core::mem::size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: PCWSTR(filter_str.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrInitialDir: init_dir_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            lpstrTitle: title_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            Flags: flags,
            ..Default::default()
        };

        let ok = unsafe { GetSaveFileNameW(&mut ofn) }.as_bool();
        if !ok {
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
    if let Some(mock) = take_or_peek_mock_path() {
        return Some(mock);
    }
    #[cfg(windows)]
    {
        win32::pick_open(&config).into_iter().next()
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        crate::linux_dbus::portal_open_files(&config)
            .into_iter()
            .next()
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = config;
        None
    }
}

/// 同步打开多文件选择对话框。
#[must_use]
pub fn open_files(config: FileDialogConfig) -> Vec<PathBuf> {
    if let Some(mock) = take_or_peek_mock_path() {
        return vec![mock];
    }
    #[cfg(windows)]
    {
        let mut cfg = config;
        cfg.multi_select = true;
        win32::pick_open(&cfg)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        let mut cfg = config;
        cfg.multi_select = true;
        crate::linux_dbus::portal_open_files(&cfg)
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = config;
        Vec::new()
    }
}

/// 同步打开保存文件对话框。
#[must_use]
pub fn save_file(config: FileDialogConfig) -> Option<PathBuf> {
    if let Some(mock) = take_or_peek_mock_path() {
        return Some(mock);
    }
    #[cfg(windows)]
    {
        win32::pick_save(&config)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        crate::linux_dbus::portal_save_file(&config)
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
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

use std::sync::{Mutex, OnceLock};

fn mock_dialog_slot() -> &'static Mutex<Option<PathBuf>> {
    static MOCK_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    MOCK_PATH.get_or_init(|| Mutex::new(None))
}

/// 注入或清除供自动化测试 / E2E 验收使用的模拟对话框返回路径。
///
/// 当设置了路径或环境变量 `UNBOUND_E2E_DIALOG_PATH` 非空时，`open_file`、`save_file` 与
/// `pick_folder` 将立即返回该路径而不弹出模态系统窗口。
pub fn set_mock_dialog_path(path: Option<PathBuf>) {
    if let Ok(mut guard) = mock_dialog_slot().lock() {
        *guard = path;
    }
}

fn take_or_peek_mock_path() -> Option<PathBuf> {
    if let Ok(guard) = mock_dialog_slot().lock() {
        if let Some(p) = guard.clone() {
            return Some(p);
        }
    }
    if let Ok(env_path) = std::env::var("UNBOUND_E2E_DIALOG_PATH") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

/// 同步打开文件夹选择对话框（优先检查模拟路径，Windows 下回退至目录选择）。
#[must_use]
pub fn pick_folder(config: FileDialogConfig) -> Option<PathBuf> {
    if let Some(mock) = take_or_peek_mock_path() {
        return Some(mock);
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        crate::linux_dbus::portal_pick_folder(&config)
    }
    #[cfg(not(all(unix, not(target_os = "macos"), not(target_os = "android"))))]
    {
        if let Some(dir) = config.initial_dir.clone() {
            if dir.exists() {
                return Some(dir);
            }
        }
        open_file(config).map(|p| {
            if p.is_dir() {
                p
            } else {
                p.parent().map_or(p.clone(), PathBuf::from)
            }
        })
    }
}

/// 在工作线程上运行的非阻塞异步文件夹选择对话框。
pub fn pick_folder_async<F: FnOnce(Option<PathBuf>) + Send + 'static>(
    config: FileDialogConfig,
    on_complete: F,
) {
    std::thread::spawn(move || {
        let result = pick_folder(config);
        on_complete(result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_dialog_path_intercepts_open_files_as_well_as_open_file() {
        let sample = PathBuf::from("/tmp/mock_selection.png");
        set_mock_dialog_path(Some(sample.clone()));
        assert_eq!(open_file(FileDialogConfig::default()), Some(sample.clone()));
        assert_eq!(open_files(FileDialogConfig::default()), vec![sample]);
        set_mock_dialog_path(None);
    }
}

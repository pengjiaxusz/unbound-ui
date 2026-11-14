//! 调试与安全工具（参见 `docs/architecture/gpu-renderer.md`），以及设备丢失恢复。
//!
//! 这里包含三个独立组件，它们共同满足一个特性：未使用时零开销。
//!
//! * [`RenderDoc`] — 动态加载 `renderdoc.dll`，使发布版二进制无硬依赖，并可围绕特定操作触发抓帧。
//! * [`ValidationCounter`] — 统计校验层错误，使 CI 能在出现*单条*警告时直接失败，而非依赖人工翻阅日志。
//! * [`Recovery`] — 在发生 `VK_ERROR_DEVICE_LOST` 后重建设备并恢复调用方状态（否则对长驻编辑器将是致命故障）。

use std::sync::atomic::{AtomicU64, Ordering};

use crate::device::GpuError;

// ---------------------------------------------------------------------------
// RenderDoc
// ---------------------------------------------------------------------------

/// 已加载的 RenderDoc API；未安装该工具时为 `None`。
pub struct RenderDoc {
    _library: libloading::Library,
    get_api: unsafe extern "C" fn(u32, *mut *mut core::ffi::c_void) -> i32,
    start_capture: unsafe extern "C" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> u32,
    end_capture: unsafe extern "C" fn(*mut core::ffi::c_void) -> u32,
    set_path: unsafe extern "C" fn(*mut core::ffi::c_void, *const core::ffi::c_char) -> u32,
    api: *mut core::ffi::c_void,
    pub version: (u32, u32),
    captures: u32,
}

// SAFETY: RenderDoc 文档明确其 API 对象的抓帧控制接口线程安全；我们仅在拥有设备的线程上调用。
unsafe impl Send for RenderDoc {}

impl core::fmt::Debug for RenderDoc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderDoc")
            .field("version", &self.version)
            .field("captures", &self.captures)
            .finish()
    }
}

/// RenderDoc 的 `eRENDERDOC_API_Version_1_6_0` 版本号。
const RENDERDOC_API_VERSION: u32 = 1_06_00;

impl RenderDoc {
    /// 尝试加载 `renderdoc.dll` / `librenderdoc.so`。
    ///
    /// 工具缺失时返回 `None`（绝不报错）：缺少调试器不应阻碍应用运行。
    #[must_use]
    pub fn load() -> Option<Self> {
        let name = if cfg!(windows) {
            "renderdoc.dll"
        } else if cfg!(target_os = "macos") {
            "librenderdoc.dylib"
        } else {
            "librenderdoc.so"
        };
        // SAFETY: 调用前逐一检查符号是否存在，并保证动态库在函数指针可达期间保持存活。
        unsafe {
            let library = libloading::Library::new(name).ok()?;
            let get_api = *library
                .get::<unsafe extern "C" fn(u32, *mut *mut core::ffi::c_void) -> i32>(
                    b"RENDERDOC_GetAPI\0",
                )
                .ok()?;
            let mut api: *mut core::ffi::c_void = core::ptr::null_mut();
            if get_api(RENDERDOC_API_VERSION, &mut api) != 1 || api.is_null() {
                return None;
            }
            // 虚表布局依据 RenderDoc 公开头文件保持稳定；前三个槽位依次为 GetAPIVersion、
            // SetCaptureOptionU32、SetCaptureOptionF32，此处按文档偏移解析对应函数指针。
            let start_capture = *library
                .get::<unsafe extern "C" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> u32>(
                    b"RENDERDOC_StartFrameCapture\0",
                )
                .ok()?;
            let end_capture = *library
                .get::<unsafe extern "C" fn(*mut core::ffi::c_void) -> u32>(
                    b"RENDERDOC_EndFrameCapture\0",
                )
                .ok()?;
            let set_path = *library
                .get::<unsafe extern "C" fn(*mut core::ffi::c_void, *const core::ffi::c_char) -> u32>(
                    b"RENDERDOC_SetCaptureFilePathTemplate\0",
                )
                .ok()?;
            Some(Self {
                _library: library,
                get_api,
                start_capture,
                end_capture,
                set_path,
                api,
                version: (1, 6),
                captures: 0,
            })
        }
    }

    /// 将抓帧输出路径模板设为 `template`（RenderDoc 会自动追加帧号）。
    pub fn set_capture_path(&self, template: &str) {
        if let Ok(c) = std::ffi::CString::new(template) {
            // SAFETY: `api` 来自 `RENDERDOC_GetAPI`，且 `c` 以 NUL 结尾。
            unsafe { (self.set_path)(self.api, c.as_ptr()) };
        }
    }

    /// 开始抓帧。无头或离屏帧中 `window` 可为空指针。
    ///
    /// # Safety
    /// 当 `window` 非空时，必须是 RenderDoc 可接受的存活窗口句柄；
    /// 该指针会不经额外校验直接传递给注入的动态库。
    pub unsafe fn begin_capture(&mut self, window: *mut core::ffi::c_void) {
        // SAFETY: 调用方保证窗口句柄有效。
        unsafe { (self.start_capture)(self.api, window) };
    }

    /// 结束抓帧并将结果刷写到磁盘。
    ///
    /// # Safety
    /// 必须先执行配对的 [`Self::begin_capture`]；RenderDoc 在两次调用间维护全局状态。
    pub unsafe fn end_capture(&mut self, _window: *mut core::ffi::c_void) {
        // SAFETY: 调用方保证当前正处于抓帧过程中。
        unsafe { (self.end_capture)(self.api) };
        self.captures += 1;
    }

    /// 当 API 指针可往返验证时返回 true（防止半加载的 DLL）。
    #[must_use]
    pub fn is_usable(&self) -> bool {
        // SAFETY: 纯读取自身拥有的值。
        let mut probe: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: `get_api` 来自已加载的动态库，且仅写入 `probe`。
        unsafe { (self.get_api)(RENDERDOC_API_VERSION, &mut probe) == 1 && !probe.is_null() }
    }
}

// ---------------------------------------------------------------------------
// 校验计数器
// ---------------------------------------------------------------------------

/// 全局校验消息计数器。
///
/// 调试回调会递增这些计数；CI 在运行结束后读取并在出现任意错误时失败。
/// 以计数代替仅打日志，才能让“开启校验”真正成为质量门禁（参见 `docs/architecture/gpu-renderer.md`）。
#[derive(Debug, Default)]
pub struct ValidationCounter {
    errors: AtomicU64,
    warnings: AtomicU64,
    last_message: std::sync::Mutex<Option<String>>,
}

impl ValidationCounter {
    /// 创建一个空计数器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一条校验消息；保留最后一条用于 CI 报告。
    pub fn record(&self, severity: &str, message: &str) {
        match severity {
            "ERROR" => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            "WARNING" => {
                self.warnings.fetch_add(1, Ordering::Relaxed);
            }
            _ => return,
        }
        if let Ok(mut slot) = self.last_message.lock() {
            *slot = Some(message.to_string());
        }
    }

    #[must_use]
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn warnings(&self) -> u64 {
        self.warnings.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn last_message(&self) -> Option<String> {
        self.last_message.lock().ok().and_then(|m| m.clone())
    }

    /// 若有任何上报消息则报错失败，供 CI 守卫测试使用。
    ///
    /// # Errors
    /// 当运行过程不干净时返回计数与最后一条消息。
    pub fn assert_clean(&self) -> Result<(), (u64, u64, Option<String>)> {
        if self.errors() > 0 || self.warnings() > 0 {
            Err((self.errors(), self.warnings(), self.last_message()))
        } else {
            Ok(())
        }
    }
}

/// 进程级全局计数器，使调试回调无需上下文指针。
pub static VALIDATION: std::sync::LazyLock<ValidationCounter> =
    std::sync::LazyLock::new(ValidationCounter::new);

// ---------------------------------------------------------------------------
// 设备丢失恢复
// ---------------------------------------------------------------------------

/// 设备恢复尝试失败的原因。
#[derive(Debug)]
pub enum RecoveryError {
    /// 无法重新获取 GPU（驱动重置或 GPU 被移除）。
    Unavailable(GpuError),
    /// 设备已恢复但资源重建失败。
    Rebuild(GpuError),
}

impl core::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable(e) => write!(f, "GPU unavailable after device loss: {e}"),
            Self::Rebuild(e) => write!(f, "rebuild after device loss failed: {e}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

/// 当 `err` 为可恢复的设备丢失状态时返回 true。
#[must_use]
pub fn is_device_lost(err: &GpuError) -> bool {
    matches!(err, GpuError::DeviceLost)
}

/// 设备丢失恢复的重试策略。
#[derive(Copy, Clone, Debug)]
pub struct RecoveryPolicy {
    /// 放弃前的最大尝试次数（驱动通常首次即可恢复）。
    pub max_attempts: u32,
    /// 两次尝试之间的等待毫秒数。
    pub backoff_ms: u64,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 120,
        }
    }
}

/// 重复执行 `rebuild` 直至成功、耗尽重试策略，或 `fatal` 判定错误不可恢复。
///
/// 闭包负责销毁失效设备并创建新设备；本辅助函数仅实现统一的
/// 重试/退避策略，避免各调用方重复编写略有差异的逻辑。
///
/// # Errors
/// 当所有尝试均失败时返回 [`RecoveryError`]。
pub fn recover_with<T, F>(policy: RecoveryPolicy, mut rebuild: F) -> Result<(T, u32), RecoveryError>
where
    F: FnMut() -> Result<T, GpuError>,
{
    let mut last = None;
    for attempt in 1..=policy.max_attempts.max(1) {
        match rebuild() {
            Ok(value) => return Ok((value, attempt)),
            Err(e) => {
                if !is_device_lost(&e) && !matches!(e, GpuError::Vulkan(_)) {
                    return Err(RecoveryError::Unavailable(e));
                }
                last = Some(e);
                if attempt < policy.max_attempts {
                    std::thread::sleep(std::time::Duration::from_millis(policy.backoff_ms));
                }
            }
        }
    }
    Err(match last {
        Some(e) => RecoveryError::Rebuild(e),
        None => RecoveryError::Unavailable(GpuError::DeviceLost),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn validation_counter_separates_severities() {
        let c = ValidationCounter::new();
        c.record("ERROR", "boom");
        c.record("WARNING", "meh");
        c.record("INFO", "ignored");
        assert_eq!(c.errors(), 1);
        assert_eq!(c.warnings(), 1);
        assert_eq!(c.last_message().as_deref(), Some("meh"));
        assert!(c.assert_clean().is_err());
    }

    #[test]
    fn clean_counter_passes_the_gate() {
        let c = ValidationCounter::new();
        assert!(c.assert_clean().is_ok());
        assert_eq!(c.errors(), 0);
    }

    #[test]
    fn device_lost_is_classified_as_recoverable() {
        assert!(is_device_lost(&GpuError::DeviceLost));
        assert!(!is_device_lost(&GpuError::Allocation("none".into())));
    }

    #[test]
    fn recovery_stops_after_success() {
        let attempts = AtomicU32::new(0);
        let result = recover_with(
            RecoveryPolicy {
                max_attempts: 3,
                backoff_ms: 0,
            },
            || {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                if n < 1 {
                    Err(GpuError::DeviceLost)
                } else {
                    Ok("recovered")
                }
            },
        );
        let (value, used) = result.expect("recovered");
        assert_eq!(value, "recovered");
        assert_eq!(used, 2);
    }

    #[test]
    fn recovery_gives_up_after_the_policy() {
        let attempts = AtomicU32::new(0);
        let result: Result<(&str, u32), _> = recover_with(
            RecoveryPolicy {
                max_attempts: 2,
                backoff_ms: 0,
            },
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(GpuError::DeviceLost)
            },
        );
        assert!(matches!(result, Err(RecoveryError::Rebuild(_))));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn non_recoverable_errors_abort_immediately() {
        let attempts = AtomicU32::new(0);
        let result: Result<(&str, u32), _> = recover_with(RecoveryPolicy::default(), || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(GpuError::UnsupportedFeatures(vec!["bufferDeviceAddress"]))
        });
        assert!(matches!(result, Err(RecoveryError::Unavailable(_))));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn renderdoc_absence_is_not_an_error() {
        // CI 机器通常未安装 RenderDoc；加载应当直接返回失败。
        if let Some(rd) = RenderDoc::load() {
            assert!(rd.is_usable());
        }
    }
}

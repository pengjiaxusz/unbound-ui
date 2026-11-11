//! Debug and safety tooling (`docs` §8.3), plus device-lost recovery.
//!
//! Three independent pieces live here because they share one property: none of
//! them may cost anything when unused.
//!
//! * [`RenderDoc`] — dynamically loads `renderdoc.dll` so the release binary has
//!   no hard dependency on it, and triggers captures around a specific action.
//! * [`ValidationCounter`] — counts validation errors so CI can fail the build
//!   on a *single* warning instead of relying on someone reading logs.
//! * [`Recovery`] — rebuilds the device and re-inflates the caller's state after
//!   `VK_ERROR_DEVICE_LOST`, which is otherwise fatal for a long-running editor.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::device::GpuError;

// ---------------------------------------------------------------------------
// RenderDoc
// ---------------------------------------------------------------------------

/// A loaded RenderDoc API, or `None` when the tool is not installed.
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

// SAFETY: RenderDoc's API object is documented as thread-safe for capture
// control; we only ever call it from the thread that owns the device.
unsafe impl Send for RenderDoc {}

impl core::fmt::Debug for RenderDoc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderDoc")
            .field("version", &self.version)
            .field("captures", &self.captures)
            .finish()
    }
}

/// RenderDoc's `eRENDERDOC_API_Version_1_6_0`.
const RENDERDOC_API_VERSION: u32 = 1_06_00;

impl RenderDoc {
    /// Try to load `renderdoc.dll` / `librenderdoc.so`.
    ///
    /// Returns `None` (never an error) when the tool is absent: a missing
    /// debugger must not prevent the application from running.
    #[must_use]
    pub fn load() -> Option<Self> {
        let name = if cfg!(windows) {
            "renderdoc.dll"
        } else if cfg!(target_os = "macos") {
            "librenderdoc.dylib"
        } else {
            "librenderdoc.so"
        };
        // SAFETY: we check every symbol before calling it, and keep the library
        // alive for as long as the function pointers are reachable.
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
            // The vtable layout is stable per RenderDoc's public header; the
            // first three slots are GetAPIVersion, SetCaptureOptionU32,
            // SetCaptureOptionF32, so we read the documented offsets indirectly
            // by resolving the exported helpers instead.
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

    /// Point captures at `template` (RenderDoc appends the frame number).
    pub fn set_capture_path(&self, template: &str) {
        if let Ok(c) = std::ffi::CString::new(template) {
            // SAFETY: `api` came from `RENDERDOC_GetAPI` and `c` is NUL-terminated.
            unsafe { (self.set_path)(self.api, c.as_ptr()) };
        }
    }

    /// Begin a capture. `window` may be null for headless/offscreen frames.
    ///
    /// # Safety
    /// When `window` is not null it must be a live window handle that
    /// RenderDoc accepts; the pointer is handed to the injected library
    /// without further validation.
    pub unsafe fn begin_capture(&mut self, window: *mut core::ffi::c_void) {
        // SAFETY: the caller guarantees the window handle is valid.
        unsafe { (self.start_capture)(self.api, window) };
    }

    /// End the capture and flush it to disk.
    ///
    /// # Safety
    /// The matching [`Self::begin_capture`] must have run first; RenderDoc
    /// keeps global state between the two calls.
    pub unsafe fn end_capture(&mut self, _window: *mut core::ffi::c_void) {
        // SAFETY: the caller guarantees a capture is in flight.
        unsafe { (self.end_capture)(self.api) };
        self.captures += 1;
    }

    /// True when the API pointer round-trips (guards against a half-loaded DLL).
    #[must_use]
    pub fn is_usable(&self) -> bool {
        // SAFETY: a pure read of a value we own.
        let mut probe: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: `get_api` came from the loaded library and only writes to
        // `probe`.
        unsafe { (self.get_api)(RENDERDOC_API_VERSION, &mut probe) == 1 && !probe.is_null() }
    }
}

// ---------------------------------------------------------------------------
// Validation counters
// ---------------------------------------------------------------------------

/// Global validation message counters.
///
/// The debug callback bumps these; CI reads them after a run and fails on any
/// error. Counting instead of logging is what turns "validation is on" into a
/// real gate (`docs` §8.3).
#[derive(Debug, Default)]
pub struct ValidationCounter {
    errors: AtomicU64,
    warnings: AtomicU64,
    last_message: std::sync::Mutex<Option<String>>,
}

impl ValidationCounter {
    /// Create an empty counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a message; retains the last one for the CI report.
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

    /// Fail when anything was reported. Used by the CI guard test.
    ///
    /// # Errors
    /// Returns the count and last message when the run was not clean.
    pub fn assert_clean(&self) -> Result<(), (u64, u64, Option<String>)> {
        if self.errors() > 0 || self.warnings() > 0 {
            Err((self.errors(), self.warnings(), self.last_message()))
        } else {
            Ok(())
        }
    }
}

/// Process-wide counter, so the debug callback needs no context pointer.
pub static VALIDATION: std::sync::LazyLock<ValidationCounter> =
    std::sync::LazyLock::new(ValidationCounter::new);

// ---------------------------------------------------------------------------
// Device-lost recovery
// ---------------------------------------------------------------------------

/// Why a recovery attempt failed.
#[derive(Debug)]
pub enum RecoveryError {
    /// The GPU could not be re-acquired (driver reset, GPU removed).
    Unavailable(GpuError),
    /// The device came back but the rebuild failed.
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

/// True when `err` is the resumable device-lost condition.
#[must_use]
pub fn is_device_lost(err: &GpuError) -> bool {
    matches!(err, GpuError::DeviceLost)
}

/// Retry policy for device-lost recovery.
#[derive(Copy, Clone, Debug)]
pub struct RecoveryPolicy {
    /// Attempts before giving up (drivers usually recover on the first).
    pub max_attempts: u32,
    /// Milliseconds to wait between attempts.
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

/// Run `rebuild` until it succeeds, the policy is exhausted, or `fatal` says the
/// error is not recoverable.
///
/// The closure is responsible for dropping the dead device and creating a new
/// one; this helper only implements the retry/backoff policy so each caller does
/// not reinvent it slightly differently.
///
/// # Errors
/// Returns [`RecoveryError`] when every attempt failed.
pub fn recover_with<T, F>(
    policy: RecoveryPolicy,
    mut rebuild: F,
) -> Result<(T, u32), RecoveryError>
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
        // The CI machine usually has no RenderDoc; loading must simply fail.
        if let Some(rd) = RenderDoc::load() {
            assert!(rd.is_usable());
        }
    }
}

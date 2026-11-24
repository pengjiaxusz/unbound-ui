//! Win32 OLE 向外文件拖出通道与跨平台 `CF_HDROP` 载荷编码器。
//!
//! 本模块拆分为两部分：
//!
//! * **平台无关策略与编码层**：[`OutboundFileDragRequest`]、[`OutboundFileDragOutcome`]、
//!   [`build_cf_hdrop_payload`] 及 `DROPEFFECT_*` 位掩码映射，可在任意平台完成无头单元测试；
//! * **Windows 原生 OLE 实现**：基于 `windows = "0.62.2"` 的 COM `IDataObject`、`IDropSource`
//!   与 `DoDragDrop`，通过 `CF_HDROP`（宽字符双空终止 `DROPFILES`）将文件列表拖出至系统资源管理器或第三方桌面应用。

use std::path::PathBuf;

use crate::overlay_host::FileDragEffect;
use crate::window::PlatformError;

/// Win32 `DROPEFFECT_NONE` 位掩码（`0`）。
pub const DROPEFFECT_NONE_MASK: u32 = 0;
/// Win32 `DROPEFFECT_COPY` 位掩码（`1`）。
pub const DROPEFFECT_COPY_MASK: u32 = 1;
/// Win32 `DROPEFFECT_MOVE` 位掩码（`2`）。
pub const DROPEFFECT_MOVE_MASK: u32 = 2;
/// Win32 `DROPEFFECT_LINK` 位掩码（`4`）。
pub const DROPEFFECT_LINK_MASK: u32 = 4;

/// `DROPFILES` 头部固定字节长度（`pFiles: u32` + `pt: POINT(i32, i32)` + `fNC: u32` + `fWide: u32`）。
pub const DROPFILES_HEADER_BYTES: usize = 20;

/// 向外部桌面应用发起文件拖出操作的请求参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundFileDragRequest {
    /// 待向外拖出的文件路径列表。
    pub paths: Vec<PathBuf>,
    /// 是否允许目标应用执行复制（`DROPEFFECT_COPY`）。
    pub allow_copy: bool,
    /// 是否允许目标应用执行移动（`DROPEFFECT_MOVE`）。
    pub allow_move: bool,
    /// 是否允许目标应用创建快捷方式/链接（`DROPEFFECT_LINK`）。
    pub allow_link: bool,
}

impl OutboundFileDragRequest {
    /// 构造默认仅允许复制（`allow_copy = true`）的向外文件拖出请求。
    #[must_use]
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            allow_copy: true,
            allow_move: false,
            allow_link: false,
        }
    }

    /// 将允许的效果开关映射为 Win32 `DROPEFFECT_*` 位掩码。
    #[must_use]
    pub const fn allowed_effect_mask(&self) -> u32 {
        let mut mask = DROPEFFECT_NONE_MASK;
        if self.allow_copy {
            mask |= DROPEFFECT_COPY_MASK;
        }
        if self.allow_move {
            mask |= DROPEFFECT_MOVE_MASK;
        }
        if self.allow_link {
            mask |= DROPEFFECT_LINK_MASK;
        }
        mask
    }

    /// 校验请求合法性：路径列表不可为空、单条路径不可为空或含内嵌 NUL，且至少允许一种拖放效果。
    ///
    /// # Errors
    /// 当路径为空、包含非法 NUL 字符或未启用任何拖放效果时返回 [`PlatformError::Window`]。
    pub fn validate(&self) -> Result<(), PlatformError> {
        if self.paths.is_empty() {
            return Err(PlatformError::Window(
                "outbound file drag requires at least one path".to_string(),
            ));
        }
        if self.allowed_effect_mask() == DROPEFFECT_NONE_MASK {
            return Err(PlatformError::Window(
                "outbound file drag requires at least one allowed drop effect".to_string(),
            ));
        }
        for path in &self.paths {
            let units = encode_path_utf16(path);
            if units.is_empty() {
                return Err(PlatformError::Window(
                    "outbound file drag path must not be empty".to_string(),
                ));
            }
            if units.contains(&0) {
                return Err(PlatformError::Window(
                    "outbound file drag path must not contain interior NUL characters".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// 向外文件拖出操作的最终结果。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OutboundFileDragOutcome {
    /// 目标应用接受了拖放，附带协商后的最终效果。
    Completed(FileDragEffect),
    /// 用户取消了拖放（如按下 `Esc`、右键取消或释放在不接受拖放的位置）。
    Cancelled,
    /// 当前平台不支持原生 OLE 向外文件拖出。
    Unsupported,
}

/// 将 Win32 `DROPEFFECT` 掩码解析为 [`FileDragEffect`]。
#[must_use]
pub const fn effect_from_win32_mask(mask: u32) -> FileDragEffect {
    if (mask & DROPEFFECT_MOVE_MASK) != 0 {
        FileDragEffect::Move
    } else if (mask & DROPEFFECT_COPY_MASK) != 0 {
        FileDragEffect::Copy
    } else if (mask & DROPEFFECT_LINK_MASK) != 0 {
        FileDragEffect::Link
    } else {
        FileDragEffect::None
    }
}

/// 将单个路径编码为 UTF-16 码元序列（不含末尾 NUL）。
fn encode_path_utf16(path: &std::path::Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().encode_utf16().collect()
    }
}

/// 构造符合 Win32 `CF_HDROP` 规范的完整二进制载荷。
///
/// 布局严格遵循 Windows Shell `DROPFILES` 结构：
/// - 偏移 `0..4`：`pFiles = 20`（`u32` 小端序）
/// - 偏移 `4..8`：`pt.x = 0`（`i32` 小端序）
/// - 偏移 `8..12`：`pt.y = 0`（`i32` 小端序）
/// - 偏移 `12..16`：`fNC = 0`（`u32` 小端序）
/// - 偏移 `16..20`：`fWide = 1`（`u32` 小端序，`TRUE` 表示 UTF-16LE）
/// - 偏移 `20..`：逐条路径的 UTF-16LE 字节流，路径间以单 `0x0000` 分隔，末尾额外追加一个 `0x0000` 构成双空终止。
///
/// # Errors
/// 当 `paths` 为空、包含空路径或含内嵌 NUL 字符时返回 [`PlatformError::Window`]。
pub fn build_cf_hdrop_payload(paths: &[PathBuf]) -> Result<Vec<u8>, PlatformError> {
    if paths.is_empty() {
        return Err(PlatformError::Window(
            "CF_HDROP payload requires at least one path".to_string(),
        ));
    }

    let mut encoded_paths = Vec::with_capacity(paths.len());
    let mut total_units: usize = 1; // 末尾双空终止的第二个 NUL
    for path in paths {
        let units = encode_path_utf16(path);
        if units.is_empty() {
            return Err(PlatformError::Window(
                "CF_HDROP path must not be empty".to_string(),
            ));
        }
        if units.contains(&0) {
            return Err(PlatformError::Window(
                "CF_HDROP path must not contain interior NUL characters".to_string(),
            ));
        }
        total_units = total_units
            .checked_add(units.len() + 1)
            .ok_or_else(|| PlatformError::Window("CF_HDROP payload size overflow".to_string()))?;
        encoded_paths.push(units);
    }

    let total_bytes = DROPFILES_HEADER_BYTES
        .checked_add(total_units.saturating_mul(2))
        .ok_or_else(|| PlatformError::Window("CF_HDROP payload byte size overflow".to_string()))?;

    let mut bytes = Vec::with_capacity(total_bytes);
    // DROPFILES 头（20 字节，小端序）
    bytes.extend_from_slice(&(DROPFILES_HEADER_BYTES as u32).to_le_bytes()); // pFiles
    bytes.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    bytes.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    bytes.extend_from_slice(&0u32.to_le_bytes()); // fNC
    bytes.extend_from_slice(&1u32.to_le_bytes()); // fWide = TRUE

    for units in encoded_paths {
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    // 双空终止符
    bytes.extend_from_slice(&0u16.to_le_bytes());

    Ok(bytes)
}

/// 发起原生向外文件拖出操作。
///
/// - **Windows**：校验请求并编码 `CF_HDROP` 载荷后，在当前 STA 线程上通过 OLE
///   `IDataObject`、`IDropSource` 与 `DoDragDrop` 阻塞执行系统拖放协商。
/// - **非 Windows**：完成请求合法性校验后返回 `Ok(OutboundFileDragOutcome::Unsupported)`。
///
/// # Errors
/// 当请求非法或 Windows OLE/COM 调用失败时返回 [`PlatformError::Window`]。
pub fn begin_outbound_file_drag(
    request: &OutboundFileDragRequest,
) -> Result<OutboundFileDragOutcome, PlatformError> {
    request.validate()?;
    let payload = build_cf_hdrop_payload(&request.paths)?;
    #[cfg(windows)]
    {
        win32::run_ole_file_drag(payload, request.allowed_effect_mask())
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        let _ = payload;
        crate::linux_dnd::dispatch_linux_outbound_file_drag(request)
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = payload;
        Ok(OutboundFileDragOutcome::Unsupported)
    }
}

#[cfg(windows)]
mod win32 {
    //! 基于官方 `windows = "0.62.2"` crate 的 Win32 OLE 向外文件拖出实现。

    use core::mem::ManuallyDrop;

    use ::windows::core::{implement, Error, Ref, BOOL, HRESULT};
    use ::windows::Win32::Foundation::{
        GlobalFree, DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP,
        DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, E_INVALIDARG, E_NOTIMPL, E_OUTOFMEMORY,
        OLE_E_ADVISENOTSUPPORTED, RPC_E_CHANGED_MODE, S_OK,
    };
    use ::windows::Win32::System::Com::{
        IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DATADIR_GET,
        DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
    };
    use ::windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
    };
    use ::windows::Win32::System::Ole::{
        DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, DROPEFFECT,
    };
    use ::windows::Win32::System::SystemServices::{MK_LBUTTON, MK_RBUTTON, MODIFIERKEYS_FLAGS};
    use ::windows::Win32::UI::Shell::SHCreateStdEnumFmtEtc;

    use super::{effect_from_win32_mask, OutboundFileDragOutcome};
    use crate::overlay_host::FileDragEffect;
    use crate::window::PlatformError;

    /// Win32 标准剪贴板格式 `CF_HDROP`（值为 15）。
    const CF_HDROP_FORMAT: u16 = 15;

    /// 线程级 OLE 初始化守卫：确保 `OleInitialize` 与 `OleUninitialize` 严格配对。
    struct OleDragScope {
        initialized: bool,
    }

    impl OleDragScope {
        fn enter() -> Result<Self, PlatformError> {
            // SAFETY: 在当前 UI 线程按需初始化 OLE STA 套件，返回成功时必须配对反初始化。
            match unsafe { OleInitialize(None) } {
                Ok(()) => Ok(Self { initialized: true }),
                Err(err) if err.code() == RPC_E_CHANGED_MODE => Err(PlatformError::Window(
                    "OleInitialize failed: thread is initialized in MTA mode".to_string(),
                )),
                Err(err) => Err(PlatformError::Window(format!(
                    "OleInitialize failed: {err}"
                ))),
            }
        }
    }

    impl Drop for OleDragScope {
        fn drop(&mut self) {
            if self.initialized {
                // SAFETY: 仅在 `OleInitialize` 成功时配对调用 `OleUninitialize`。
                unsafe {
                    OleUninitialize();
                }
            }
        }
    }

    /// OLE 拖放源实现：响应 `Esc` / 右键取消与左键释放投递，光标交由系统默认反馈。
    #[implement(IDropSource)]
    struct FileDropSource;

    impl IDropSource_Impl for FileDropSource_Impl {
        fn QueryContinueDrag(
            &self,
            fescapepressed: BOOL,
            grfkeystate: MODIFIERKEYS_FLAGS,
        ) -> HRESULT {
            if fescapepressed.as_bool() || (grfkeystate.0 & MK_RBUTTON.0) != 0 {
                return DRAGDROP_S_CANCEL;
            }
            if (grfkeystate.0 & MK_LBUTTON.0) == 0 {
                return DRAGDROP_S_DROP;
            }
            S_OK
        }

        fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// 提供 `CF_HDROP` 数据的 COM `IDataObject` 实现。
    #[implement(IDataObject)]
    struct FileDataObject {
        hdrop_payload: Vec<u8>,
    }

    impl FileDataObject {
        const fn hdrop_format() -> FORMATETC {
            FORMATETC {
                cfFormat: CF_HDROP_FORMAT,
                ptd: core::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            }
        }

        fn matches_hdrop(fmt: &FORMATETC) -> bool {
            fmt.cfFormat == CF_HDROP_FORMAT
                && fmt.dwAspect == DVASPECT_CONTENT.0
                && (fmt.tymed & (TYMED_HGLOBAL.0 as u32)) != 0
        }
    }

    impl IDataObject_Impl for FileDataObject_Impl {
        fn GetData(&self, pformatetcin: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
            if pformatetcin.is_null() {
                return Err(Error::from(E_INVALIDARG));
            }
            // SAFETY: 调用方遵守 COM 契约传入有效只读 `FORMATETC` 指针。
            let fmt = unsafe { &*pformatetcin };
            if !FileDataObject::matches_hdrop(fmt) {
                return Err(Error::from(DV_E_FORMATETC));
            }

            // SAFETY: 分配可移动 `HGLOBAL` 并将 `CF_HDROP` 字节拷贝进去；返回 `STGMEDIUM`
            // 时将 `pUnkForRelease` 置为 `None`，由接收方通过 `ReleaseStgMedium` 负责释放。
            unsafe {
                let hglobal = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, self.hdrop_payload.len())
                    .map_err(|_| Error::from(E_OUTOFMEMORY))?;
                let dst = GlobalLock(hglobal).cast::<u8>();
                if dst.is_null() {
                    let _ = GlobalFree(Some(hglobal));
                    return Err(Error::from(E_OUTOFMEMORY));
                }
                core::ptr::copy_nonoverlapping(
                    self.hdrop_payload.as_ptr(),
                    dst,
                    self.hdrop_payload.len(),
                );
                let _ = GlobalUnlock(hglobal);

                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: hglobal },
                    pUnkForRelease: ManuallyDrop::new(None),
                })
            }
        }

        fn GetDataHere(
            &self,
            _pformatetc: *const FORMATETC,
            _pmedium: *mut STGMEDIUM,
        ) -> windows_core::Result<()> {
            Err(Error::from(E_NOTIMPL))
        }

        fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
            if pformatetc.is_null() {
                return E_INVALIDARG;
            }
            // SAFETY: 调用方传入只读 `FORMATETC` 指针。
            let fmt = unsafe { &*pformatetc };
            if FileDataObject::matches_hdrop(fmt) {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }

        fn GetCanonicalFormatEtc(
            &self,
            _pformatectin: *const FORMATETC,
            pformatetcout: *mut FORMATETC,
        ) -> HRESULT {
            if !pformatetcout.is_null() {
                // SAFETY: 按 COM 规范将输出目标设备置空并返回 `DATA_S_SAMEFORMATETC`。
                unsafe {
                    (*pformatetcout).ptd = core::ptr::null_mut();
                }
            }
            DATA_S_SAMEFORMATETC
        }

        fn SetData(
            &self,
            _pformatetc: *const FORMATETC,
            _pmedium: *const STGMEDIUM,
            _frelease: BOOL,
        ) -> windows_core::Result<()> {
            Err(Error::from(E_NOTIMPL))
        }

        fn EnumFormatEtc(&self, dwdirection: u32) -> windows_core::Result<IEnumFORMATETC> {
            if dwdirection == DATADIR_GET.0 as u32 {
                let formats = [FileDataObject::hdrop_format()];
                // SAFETY: `formats` 在本次调用期间有效，`SHCreateStdEnumFmtEtc` 内部会深拷贝数组。
                unsafe { SHCreateStdEnumFmtEtc(&formats) }
            } else {
                Err(Error::from(E_NOTIMPL))
            }
        }

        fn DAdvise(
            &self,
            _pformatetc: *const FORMATETC,
            _advf: u32,
            _padvsink: Ref<'_, IAdviseSink>,
        ) -> windows_core::Result<u32> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }

        fn DUnadvise(&self, _dwconnection: u32) -> windows_core::Result<()> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }

        fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }
    }

    pub(super) fn run_ole_file_drag(
        hdrop_payload: Vec<u8>,
        allowed_effects_mask: u32,
    ) -> Result<OutboundFileDragOutcome, PlatformError> {
        let _ole = OleDragScope::enter()?;

        let data_object: IDataObject = FileDataObject { hdrop_payload }.into();
        let drop_source: IDropSource = FileDropSource.into();
        let mut final_effect = DROPEFFECT(0);

        // SAFETY: `data_object` 与 `drop_source` 为合法 COM 对象，`final_effect` 为有效输出指针，
        // 且当前线程已由 `_ole` 确保处于 OLE STA 状态。
        let hr = unsafe {
            DoDragDrop(
                &data_object,
                &drop_source,
                DROPEFFECT(allowed_effects_mask),
                &mut final_effect,
            )
        };

        if hr == DRAGDROP_S_DROP {
            match effect_from_win32_mask(final_effect.0) {
                FileDragEffect::None => Ok(OutboundFileDragOutcome::Cancelled),
                effect => Ok(OutboundFileDragOutcome::Completed(effect)),
            }
        } else if hr == DRAGDROP_S_CANCEL {
            Ok(OutboundFileDragOutcome::Cancelled)
        } else {
            Err(PlatformError::Window(format!(
                "DoDragDrop failed with HRESULT 0x{:08X}",
                hr.0 as u32
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_cf_hdrop_payload_encodes_header_and_double_nul_utf16le() {
        let paths = vec![
            PathBuf::from("C:\\Projects\\My Assets\\sample file.png"),
            PathBuf::from("D:\\Archive\\sample image 01.webp"),
        ];
        let payload = build_cf_hdrop_payload(&paths).expect("valid payload");

        assert!(payload.len() > DROPFILES_HEADER_BYTES);
        // 核对 DROPFILES 20 字节头
        assert_eq!(
            u32::from_le_bytes(payload[0..4].try_into().unwrap()),
            DROPFILES_HEADER_BYTES as u32
        );
        assert_eq!(i32::from_le_bytes(payload[4..8].try_into().unwrap()), 0);
        assert_eq!(i32::from_le_bytes(payload[8..12].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(payload[12..16].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(payload[16..20].try_into().unwrap()), 1);

        // 解码后续 UTF-16LE 码元并验证单空分隔 + 双空终止
        let body = &payload[DROPFILES_HEADER_BYTES..];
        assert_eq!(body.len() % 2, 0);
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert!(units.len() >= 2);
        assert_eq!(units[units.len() - 1], 0, "must end with trailing NUL");
        assert_eq!(
            units[units.len() - 2],
            0,
            "must be double-NUL terminated after final path"
        );

        let decoded: Vec<String> = units[..units.len() - 1]
            .split(|&u| u == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect();
        assert_eq!(
            decoded,
            vec![
                "C:\\Projects\\My Assets\\sample file.png".to_string(),
                "D:\\Archive\\sample image 01.webp".to_string(),
            ]
        );
    }

    #[test]
    fn build_cf_hdrop_payload_rejects_empty_and_nul_paths() {
        assert!(build_cf_hdrop_payload(&[]).is_err());
        assert!(build_cf_hdrop_payload(&[PathBuf::from("")]).is_err());
        assert!(build_cf_hdrop_payload(&[PathBuf::from("bad\0path.txt")]).is_err());
    }

    #[test]
    fn request_maps_effect_masks_and_rejects_invalid_requests() {
        let mut req = OutboundFileDragRequest {
            paths: vec![PathBuf::from("C:\\tmp\\a.txt")],
            allow_copy: true,
            allow_move: true,
            allow_link: true,
        };
        assert_eq!(
            req.allowed_effect_mask(),
            DROPEFFECT_COPY_MASK | DROPEFFECT_MOVE_MASK | DROPEFFECT_LINK_MASK
        );
        assert!(req.validate().is_ok());

        req.allow_move = false;
        assert_eq!(
            req.allowed_effect_mask(),
            DROPEFFECT_COPY_MASK | DROPEFFECT_LINK_MASK
        );

        // 三种效果全关闭必须被拒绝
        let no_effects = OutboundFileDragRequest {
            paths: vec![PathBuf::from("C:\\tmp\\a.txt")],
            allow_copy: false,
            allow_move: false,
            allow_link: false,
        };
        assert_eq!(no_effects.allowed_effect_mask(), DROPEFFECT_NONE_MASK);
        assert!(no_effects.validate().is_err());
        assert!(begin_outbound_file_drag(&no_effects).is_err());

        // 空路径列表必须被拒绝
        let no_paths = OutboundFileDragRequest::new(Vec::new());
        assert!(no_paths.validate().is_err());
        assert!(begin_outbound_file_drag(&no_paths).is_err());
    }

    #[test]
    fn effect_mask_maps_back_to_file_drag_effect() {
        assert_eq!(
            effect_from_win32_mask(DROPEFFECT_NONE_MASK),
            FileDragEffect::None
        );
        assert_eq!(
            effect_from_win32_mask(DROPEFFECT_COPY_MASK),
            FileDragEffect::Copy
        );
        assert_eq!(
            effect_from_win32_mask(DROPEFFECT_MOVE_MASK),
            FileDragEffect::Move
        );
        assert_eq!(
            effect_from_win32_mask(DROPEFFECT_LINK_MASK),
            FileDragEffect::Link
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_outbound_drag_returns_unsupported() {
        let req = OutboundFileDragRequest::new(vec![PathBuf::from("/tmp/example.png")]);
        assert_eq!(
            begin_outbound_file_drag(&req).expect("non-windows fallback"),
            OutboundFileDragOutcome::Unsupported
        );
    }
}

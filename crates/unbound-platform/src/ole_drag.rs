//! Win32 OLE 向外文件拖出载荷构造与平台入口（ADR-0022 / `GAP-UI-003`）。
//!
//! 严格分离纯 CPU 协议编码与 Win32 COM 边界：
//! - [`encode_cf_hdrop_payload`] 在全平台均可直接单测，按 Win32 `DROPFILES` 20 字节头 +
//!   UTF-16LE 双空字符结尾编码本地路径列表；
//! - [`start_external_file_drag`] 提供统一调用契约，空路径列表直接早退返回
//!   [`FileDragEffect::None`]。

use std::path::PathBuf;

use crate::overlay_host::FileDragEffect;
use crate::window::PlatformError;

/// Win32 `DROPFILES` 结构头固定字节长度（`pFiles: u32, pt: (i32, i32), fNC: i32, fWide: i32`）。
pub const DROPFILES_HEADER_BYTES: usize = 20;

/// 将本地文件路径列表编码为符合 Win32 `CF_HDROP` 规范的 `DROPFILES` + UTF-16LE 双空字符字节块。
#[must_use]
pub fn encode_cf_hdrop_payload(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DROPFILES_HEADER_BYTES + paths.len() * 64 + 4);
    // pFiles = 20
    out.extend_from_slice(&(DROPFILES_HEADER_BYTES as u32).to_le_bytes());
    // pt.x = 0, pt.y = 0, fNC = 0
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    // fWide = 1 (UTF-16LE)
    out.extend_from_slice(&1i32.to_le_bytes());

    for path in paths {
        let text = path.to_string_lossy();
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    // 列表末尾额外追加一个 UTF-16 空终止符（形成双 NUL 结尾）
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// 发起向外文件拖出会话。
///
/// 当前已完成 `CF_HDROP` 载荷构造与参数校验；在 Win32 启用 `Win32_System_Ole` 后，
/// 可在下方平台分支中将编码后的 `HGLOBAL` 直接传入 `DoDragDrop`。
pub fn start_external_file_drag(
    _hwnd: isize,
    paths: &[PathBuf],
    allowed: FileDragEffect,
) -> Result<FileDragEffect, PlatformError> {
    if paths.is_empty() || allowed == FileDragEffect::None {
        return Ok(FileDragEffect::None);
    }
    let payload = encode_cf_hdrop_payload(paths);
    if payload.len() <= DROPFILES_HEADER_BYTES + 2 {
        return Ok(FileDragEffect::None);
    }
    // 占位返回允许效果；完整 COM IDropSource/IDataObject + DoDragDrop 挂接详见 HANDOFF.md。
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_hdrop_payload_writes_valid_dropfiles_header_and_utf16_double_nul() {
        let paths = vec![
            PathBuf::from("C:\\Assets\\photo_01.png"),
            PathBuf::from("D:\\Media\\clip_02.webp"),
        ];
        let bytes = encode_cf_hdrop_payload(&paths);
        assert!(bytes.len() > DROPFILES_HEADER_BYTES + 4);
        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            DROPFILES_HEADER_BYTES as u32
        );
        // fWide == 1
        assert_eq!(i32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        // 结尾必须是双 UTF-16 NUL（四个 0 字节）
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn empty_drag_returns_none_without_error() {
        let res = start_external_file_drag(0, &[], FileDragEffect::Copy).expect("empty drag ok");
        assert_eq!(res, FileDragEffect::None);
    }
}

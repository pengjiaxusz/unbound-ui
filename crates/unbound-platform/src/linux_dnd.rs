//! Linux 跨应用文件拖放（RFC 2483 `text/uri-list`）与 ADR 0023 跨窗标签撕裂纯逻辑层。
//!
//! 跨平台编译、零外部显示依赖，完整覆盖：
//! - RFC 2483 `text/uri-list` 往返编解码（注释过滤、百分号编解码、空格与 CJK UTF-8 路径）；
//! - ADR 0023 跨窗标签撕裂私有 MIME 载荷序列化与落点二元状态机（跨窗停靠 / 窗外独立成窗 / 取消还原）；
//! - Linux 向外文件拖出后端钩子注册（供 Wayland `wl_data_device` 与 X11 XDND 挂载）。

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::ole_drag::{OutboundFileDragOutcome, OutboundFileDragRequest};
use crate::portal_logic::decode_file_uri;
use crate::window::PlatformError;

/// 标准跨应用文件拖放 MIME 类型（RFC 2483）。
pub const URI_LIST_MIME: &str = "text/uri-list";

/// 框架私有跨窗标签撕裂 MIME 类型（ADR 0023，同进程跨窗识别）。
pub const UNBOUND_TAB_TEAR_MIME: &str = "application/x-unbound-tab-tearout";

/// 按 RFC 3986 对本地绝对路径执行百分号编码（保留 `/` 与安全路径字符）。
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 16);
    for &b in path.as_bytes() {
        let safe = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// 将本地文件路径列表编码为符合 RFC 2483 规范的 `text/uri-list` 载荷（CRLF 结尾）。
#[must_use]
pub fn encode_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        let s = path.to_string_lossy();
        if s.is_empty() {
            continue;
        }
        out.push_str("file://");
        if !s.starts_with('/') {
            out.push('/');
        }
        out.push_str(&percent_encode_path(&s));
        out.push_str("\r\n");
    }
    out
}

/// 解析 RFC 2483 `text/uri-list` 载荷为本地文件路径列表。
///
/// 自动忽略空行与 `#` 注释行，执行百分号解码并过滤非本地 `file://` URI。
#[must_use]
pub fn decode_uri_list(raw: &str) -> Vec<PathBuf> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(decode_file_uri)
        .collect()
}

/// ADR 0023 跨窗标签撕裂私有 MIME 载荷。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabTearPayload {
    /// 源窗口标识。
    pub source_window_id: u64,
    /// 被撕裂的标签页 ID。
    pub tab_id: u64,
    /// 标签标题（供拖拽预览或新窗标题使用）。
    pub title: String,
}

impl TabTearPayload {
    /// 编码为紧凑 UTF-8 载荷字符串。
    #[must_use]
    pub fn encode(&self) -> String {
        format!("{}\t{}\t{}", self.source_window_id, self.tab_id, self.title)
    }

    /// 从私有 MIME 载荷解码；格式非法时返回 `None`。
    #[must_use]
    pub fn decode(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, '\t');
        let source_window_id = parts.next()?.parse::<u64>().ok()?;
        let tab_id = parts.next()?.parse::<u64>().ok()?;
        let title = parts.next()?.to_owned();
        Some(Self {
            source_window_id,
            tab_id,
            title,
        })
    }
}

/// 跨窗标签撕裂拖拽结束后的最终语义结果（ADR 0023 落点二元语义 + 取消还原）。
#[derive(Clone, Debug, PartialEq)]
pub enum TabTearDropOutcome {
    /// 落入同进程另一窗口停靠区，由目标窗口合入该标签。
    DockIntoWindow {
        /// 目标窗口 ID。
        target_window_id: u64,
        /// 目标窗口内的局部逻辑 X 坐标。
        local_x: f64,
        /// 目标窗口内的局部逻辑 Y 坐标。
        local_y: f64,
        /// 标签载荷。
        payload: TabTearPayload,
    },
    /// 释放于本进程窗口之外且未取消，独立生成新顶层窗口。
    SpawnNewWindow {
        /// 可选的释放点屏幕物理坐标（X11 提供；Wayland 为 `None` 由合成器摆放）。
        screen_pos: Option<(i32, i32)>,
        /// 标签载荷。
        payload: TabTearPayload,
    },
    /// 用户按下 `Esc` 或右键取消，恢复源窗口原标签位。
    Cancelled,
}

/// ADR 0023 跨窗标签撕裂状态机（纯逻辑可单测）。
#[derive(Clone, Debug, Default)]
pub struct TabTearDragState {
    active: Option<TabTearPayload>,
    hovered_target: Option<(u64, f64, f64)>,
}

impl TabTearDragState {
    /// 当前是否处于活跃的标签撕裂拖拽中。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 超过撕裂阈值后启动一次原生跨窗标签拖拽。
    pub fn start(&mut self, payload: TabTearPayload) {
        self.active = Some(payload);
        self.hovered_target = None;
    }

    /// 指针拖入同进程某个目标窗口的客户区。
    pub fn enter_window(&mut self, target_window_id: u64, local_x: f64, local_y: f64) {
        if self.active.is_some() {
            self.hovered_target = Some((target_window_id, local_x, local_y));
        }
    }

    /// 指针在当前悬停目标窗口内移动。
    pub fn motion(&mut self, local_x: f64, local_y: f64) {
        if let Some((_, x, y)) = &mut self.hovered_target {
            *x = local_x;
            *y = local_y;
        }
    }

    /// 指针离开指定目标窗口。
    pub fn leave_window(&mut self, window_id: u64) {
        if self
            .hovered_target
            .is_some_and(|(id, _, _)| id == window_id)
        {
            self.hovered_target = None;
        }
    }

    /// 目标窗口接受了本次跨窗投递。
    pub fn finish_accepted(&mut self) -> Option<TabTearDropOutcome> {
        let payload = self.active.take()?;
        let (target_window_id, local_x, local_y) = self.hovered_target.take()?;
        Some(TabTearDropOutcome::DockIntoWindow {
            target_window_id,
            local_x,
            local_y,
            payload,
        })
    }

    /// 拖拽在未取消情况下结束但无同进程窗口接受，触发窗外独立成窗。
    pub fn finish_unaccepted(
        &mut self,
        screen_pos: Option<(i32, i32)>,
    ) -> Option<TabTearDropOutcome> {
        let payload = self.active.take()?;
        self.hovered_target = None;
        Some(TabTearDropOutcome::SpawnNewWindow {
            screen_pos,
            payload,
        })
    }

    /// 用户按下 `Esc` 或右键取消拖拽，还原源窗口原标签位。
    pub fn cancel(&mut self) -> Option<TabTearDropOutcome> {
        self.active.take()?;
        self.hovered_target = None;
        Some(TabTearDropOutcome::Cancelled)
    }
}

/// Linux 原生向外文件拖出后端钩子（由活跃的 Wayland 或 X11 会话注册）。
pub trait LinuxOutboundDragBackend: Send + Sync {
    /// 发起一次向外文件拖出并协商最终效果。
    ///
    /// # Errors
    /// 平台拖放会话初始化失败时返回 [`PlatformError`]。
    fn begin_file_drag(
        &self,
        request: &OutboundFileDragRequest,
    ) -> Result<OutboundFileDragOutcome, PlatformError>;
}

static LINUX_OUTBOUND_DRAG: OnceLock<Mutex<Option<Arc<dyn LinuxOutboundDragBackend>>>> =
    OnceLock::new();

/// 注册或注销当前活跃 Linux 显示会话的向外文件拖出处理器。
pub fn register_linux_outbound_drag(backend: Option<Arc<dyn LinuxOutboundDragBackend>>) {
    let lock = LINUX_OUTBOUND_DRAG.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = backend;
    }
}

/// 将向外文件拖出请求分派至当前活跃 Linux 显示后端；无活跃窗口会话时返回 `Unsupported`。
///
/// # Errors
/// 透传底层后端的拖放初始化错误。
pub fn dispatch_linux_outbound_file_drag(
    request: &OutboundFileDragRequest,
) -> Result<OutboundFileDragOutcome, PlatformError> {
    let lock = LINUX_OUTBOUND_DRAG.get_or_init(|| Mutex::new(None));
    let Some(backend) = lock.lock().ok().and_then(|g| g.clone()) else {
        return Ok(OutboundFileDragOutcome::Unsupported);
    };
    backend.begin_file_drag(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_round_trips_spaces_cjk_and_ignores_comments_and_remote_uris() {
        let paths = vec![
            PathBuf::from("/home/user/My Assets/图标 01.png"), // lang-ok: CJK UTF-8 test fixture
            PathBuf::from("/tmp/plain_file.txt"),
        ];
        let encoded = encode_uri_list(&paths);
        assert!(encoded.contains("file:///home/user/My%20Assets/"));
        assert!(encoded.ends_with("\r\n"));

        let with_noise =
            format!("# comment line\r\n{encoded}\r\nhttps://example.com/ignored.png\r\n   \r\n");
        let decoded = decode_uri_list(&with_noise);
        assert_eq!(decoded, paths);
    }

    #[test]
    fn tab_tear_payload_encodes_and_decodes_losslessly() {
        let payload = TabTearPayload {
            source_window_id: 1,
            tab_id: 42,
            title: "文档\t预览".to_string(), // lang-ok: CJK UTF-8 test fixture
        };
        let wire = payload.encode();
        assert_eq!(TabTearPayload::decode(&wire), Some(payload));
        assert_eq!(TabTearPayload::decode("invalid"), None);
    }

    #[test]
    fn tab_tear_state_machine_covers_dock_spawn_and_cancel() {
        let mut state = TabTearDragState::default();
        let payload = TabTearPayload {
            source_window_id: 1,
            tab_id: 7,
            title: "Showcase".to_string(),
        };

        // 1. 拖入另一窗口并接受 -> DockIntoWindow
        state.start(payload.clone());
        state.enter_window(2, 120.0, 24.0);
        state.motion(180.0, 28.0);
        assert_eq!(
            state.finish_accepted(),
            Some(TabTearDropOutcome::DockIntoWindow {
                target_window_id: 2,
                local_x: 180.0,
                local_y: 28.0,
                payload: payload.clone(),
            })
        );
        assert!(!state.is_active());

        // 2. 拖出窗外释放 -> SpawnNewWindow
        state.start(payload.clone());
        state.enter_window(2, 50.0, 10.0);
        state.leave_window(2);
        assert_eq!(
            state.finish_unaccepted(Some((640, 360))),
            Some(TabTearDropOutcome::SpawnNewWindow {
                screen_pos: Some((640, 360)),
                payload: payload.clone(),
            })
        );

        // 3. Esc/右键取消 -> Cancelled
        state.start(payload);
        assert_eq!(state.cancel(), Some(TabTearDropOutcome::Cancelled));
        assert!(!state.is_active());
    }
}

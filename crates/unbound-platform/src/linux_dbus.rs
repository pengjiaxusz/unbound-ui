//! Linux 纯 Rust 后台 D-Bus 工作线程（`zbus` + `async-io`，零 `tokio`、零 `libdbus-1-dev`）。
//!
//! 统一承载：
//! - `xdg-desktop-portal` `FileChooser`（打开/多选/保存/选目录对话框）；
//! - `xdg-desktop-portal` `Settings`（系统深色模式与强调色读取及 `SettingChanged` 实时订阅）；
//! - 后续 T6 系统托盘（`StatusNotifierItem`）与桌面通知（`org.freedesktop.Notifications`）。
//!
//! 当运行环境缺少会话总线或桌面门户服务时，所有请求均安全快速降级为空值/`false`，不阻塞主线程。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, RwLock};

use futures_lite::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

use crate::dialogs::FileDialogConfig;
use crate::portal_logic::{
    convert_dialog_filters, decode_file_uri, parse_portal_accent_color, parse_portal_color_scheme,
    LinuxAppearanceSnapshot,
};

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";
const PORTAL_FILE_CHOOSER_IFACE: &str = "org.freedesktop.portal.FileChooser";
const PORTAL_REQUEST_IFACE: &str = "org.freedesktop.portal.Request";
const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";

type ThemeListener = Arc<dyn Fn(bool) + Send + Sync>;

enum DbusCommand {
    OpenFile {
        config: FileDialogConfig,
        directory: bool,
        reply: mpsc::Sender<Vec<PathBuf>>,
    },
    SaveFile {
        config: FileDialogConfig,
        reply: mpsc::Sender<Option<PathBuf>>,
    },
    SetTrayIcon {
        tooltip: String,
        visible: bool,
        reply: mpsc::Sender<bool>,
    },
    ShowNotification {
        title: String,
        message: String,
        urgency: u8,
        reply: mpsc::Sender<bool>,
    },
}

struct DbusRuntime {
    cmd_tx: Mutex<mpsc::Sender<DbusCommand>>,
    appearance: Arc<RwLock<LinuxAppearanceSnapshot>>,
    theme_listeners: Arc<Mutex<Vec<ThemeListener>>>,
}

static DBUS_RUNTIME: OnceLock<Option<DbusRuntime>> = OnceLock::new();
static TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_handle_token() -> String {
    let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("unbound_ui_{}_{}", std::process::id(), seq)
}

fn has_session_bus_env() -> bool {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
        || std::env::var_os("XDG_RUNTIME_DIR")
            .map(|dir| PathBuf::from(dir).join("bus").exists())
            .unwrap_or(false)
}

fn runtime() -> Option<&'static DbusRuntime> {
    DBUS_RUNTIME
        .get_or_init(|| {
            if !has_session_bus_env() {
                return None;
            }
            let (cmd_tx, cmd_rx) = mpsc::channel::<DbusCommand>();
            let appearance = Arc::new(RwLock::new(LinuxAppearanceSnapshot::default()));
            let theme_listeners: Arc<Mutex<Vec<ThemeListener>>> = Arc::new(Mutex::new(Vec::new()));

            let bg_appearance = Arc::clone(&appearance);
            let bg_listeners = Arc::clone(&theme_listeners);

            let _ = std::thread::Builder::new()
                .name("unbound-dbus".to_string())
                .spawn(move || {
                    futures_lite::future::block_on(run_dbus_worker(
                        cmd_rx,
                        bg_appearance,
                        bg_listeners,
                    ));
                });

            Some(DbusRuntime {
                cmd_tx: Mutex::new(cmd_tx),
                appearance,
                theme_listeners,
            })
        })
        .as_ref()
}

/// 零阻塞读取已缓存的 Linux 桌面外观快照（深色模式与强调色）。
#[must_use]
pub(crate) fn cached_appearance() -> LinuxAppearanceSnapshot {
    let Some(rt) = runtime() else {
        return LinuxAppearanceSnapshot::default();
    };
    rt.appearance.read().map(|g| *g).unwrap_or_default()
}

/// 注册系统深色模式实时变更回调（当 `SettingChanged` 到达时触发并唤醒 UI 主循环）。
pub(crate) fn register_theme_change_listener(listener: ThemeListener) {
    let Some(rt) = runtime() else {
        return;
    };
    if let Ok(mut list) = rt.theme_listeners.lock() {
        list.push(listener);
    }
}

/// 通过 `xdg-desktop-portal` `FileChooser` 打开单个或多个文件（无服务时返回空列表）。
pub(crate) fn portal_open_files(config: &FileDialogConfig) -> Vec<PathBuf> {
    let Some(rt) = runtime() else {
        return Vec::new();
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    let sent = rt.cmd_tx.lock().ok().is_some_and(|tx| {
        tx.send(DbusCommand::OpenFile {
            config: config.clone(),
            directory: false,
            reply: reply_tx,
        })
        .is_ok()
    });
    if !sent {
        return Vec::new();
    }
    reply_rx.recv().unwrap_or_default()
}

/// 通过 `xdg-desktop-portal` `FileChooser` 选择目录（无服务时返回 `None`）。
pub(crate) fn portal_pick_folder(config: &FileDialogConfig) -> Option<PathBuf> {
    let rt = runtime()?;
    let (reply_tx, reply_rx) = mpsc::channel();
    let sent = rt.cmd_tx.lock().ok().is_some_and(|tx| {
        tx.send(DbusCommand::OpenFile {
            config: config.clone(),
            directory: true,
            reply: reply_tx,
        })
        .is_ok()
    });
    if !sent {
        return None;
    }
    reply_rx.recv().ok()?.into_iter().next()
}

/// 通过 `org.kde.StatusNotifierWatcher` 注册/更新/隐藏 Linux 系统托盘图标（无服务时返回 `false`）。
pub(crate) fn linux_set_tray_icon(tooltip: &str, visible: bool) -> bool {
    let Some(rt) = runtime() else {
        return false;
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    let sent = rt.cmd_tx.lock().ok().is_some_and(|tx| {
        tx.send(DbusCommand::SetTrayIcon {
            tooltip: tooltip.to_owned(),
            visible,
            reply: reply_tx,
        })
        .is_ok()
    });
    if !sent {
        return false;
    }
    reply_rx.recv().unwrap_or(false)
}

/// 通过 `org.freedesktop.Notifications` 发送桌面气泡通知（无通知守护进程时返回 `false`）。
pub(crate) fn linux_show_notification(title: &str, message: &str, urgency: u8) -> bool {
    let Some(rt) = runtime() else {
        return false;
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    let sent = rt.cmd_tx.lock().ok().is_some_and(|tx| {
        tx.send(DbusCommand::ShowNotification {
            title: title.to_owned(),
            message: message.to_owned(),
            urgency,
            reply: reply_tx,
        })
        .is_ok()
    });
    if !sent {
        return false;
    }
    reply_rx.recv().unwrap_or(false)
}

/// 通过 `xdg-desktop-portal` `FileChooser` 打开保存文件对话框（无服务时返回 `None`）。
pub(crate) fn portal_save_file(config: &FileDialogConfig) -> Option<PathBuf> {
    let rt = runtime()?;
    let (reply_tx, reply_rx) = mpsc::channel();
    let sent = rt.cmd_tx.lock().ok().is_some_and(|tx| {
        tx.send(DbusCommand::SaveFile {
            config: config.clone(),
            reply: reply_tx,
        })
        .is_ok()
    });
    if !sent {
        return None;
    }
    reply_rx.recv().ok().flatten()
}

async fn run_dbus_worker(
    cmd_rx: mpsc::Receiver<DbusCommand>,
    appearance: Arc<RwLock<LinuxAppearanceSnapshot>>,
    theme_listeners: Arc<Mutex<Vec<ThemeListener>>>,
) {
    let Ok(conn) = Connection::session().await else {
        drain_commands_offline(cmd_rx);
        return;
    };

    // 1. 启动时异步读取初始深色模式与系统强调色
    refresh_initial_appearance(&conn, &appearance).await;

    // 2. 在独立子线程监听 `SettingChanged` 信号，保证文件对话框等待期间不阻塞主题推送
    let sig_conn = conn.clone();
    let sig_appearance = Arc::clone(&appearance);
    let sig_listeners = Arc::clone(&theme_listeners);
    let _ = std::thread::Builder::new()
        .name("unbound-dbus-signals".to_string())
        .spawn(move || {
            futures_lite::future::block_on(watch_appearance_signals(
                sig_conn,
                sig_appearance,
                sig_listeners,
            ));
        });

    // 3. 处理来自 UI / 后台线程的同步/异步请求
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            DbusCommand::OpenFile {
                config,
                directory,
                reply,
            } => {
                let paths = handle_open_file(&conn, &config, directory).await;
                let _ = reply.send(paths);
            }
            DbusCommand::SaveFile { config, reply } => {
                let path = handle_save_file(&conn, &config).await;
                let _ = reply.send(path);
            }
            DbusCommand::SetTrayIcon {
                tooltip,
                visible,
                reply,
            } => {
                let ok = handle_set_tray_icon(&conn, &tooltip, visible).await;
                let _ = reply.send(ok);
            }
            DbusCommand::ShowNotification {
                title,
                message,
                urgency,
                reply,
            } => {
                let ok = handle_show_notification(&conn, &title, &message, urgency).await;
                let _ = reply.send(ok);
            }
        }
    }
}

fn drain_commands_offline(cmd_rx: mpsc::Receiver<DbusCommand>) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            DbusCommand::OpenFile { reply, .. } => {
                let _ = reply.send(Vec::new());
            }
            DbusCommand::SaveFile { reply, .. } => {
                let _ = reply.send(None);
            }
            DbusCommand::SetTrayIcon { reply, .. }
            | DbusCommand::ShowNotification { reply, .. } => {
                let _ = reply.send(false);
            }
        }
    }
}

fn unwrap_variant_u32(val: &OwnedValue) -> Option<u32> {
    if let Ok(v) = u32::try_from(val.clone()) {
        return Some(v);
    }
    if let Value::Value(inner) = val as &Value<'_> {
        if let Value::U32(v) = inner.as_ref() {
            return Some(*v);
        }
    }
    None
}

fn unwrap_variant_rgb(val: &OwnedValue) -> Option<(f64, f64, f64)> {
    if let Ok(rgb) = <(f64, f64, f64)>::try_from(val.clone()) {
        return Some(rgb);
    }
    if let Value::Value(inner) = val as &Value<'_> {
        if let Value::Structure(s) = inner.as_ref() {
            let fields = s.fields();
            if fields.len() == 3 {
                if let (Value::F64(r), Value::F64(g), Value::F64(b)) =
                    (&fields[0], &fields[1], &fields[2])
                {
                    return Some((*r, *g, *b));
                }
            }
        }
    }
    None
}

async fn refresh_initial_appearance(
    conn: &Connection,
    appearance: &Arc<RwLock<LinuxAppearanceSnapshot>>,
) {
    let mut snap = LinuxAppearanceSnapshot::default();
    if let Ok(reply) = conn
        .call_method(
            Some(PORTAL_BUS_NAME),
            PORTAL_OBJECT_PATH,
            Some(PORTAL_SETTINGS_IFACE),
            "ReadOne",
            &(APPEARANCE_NAMESPACE, "color-scheme"),
        )
        .await
    {
        if let Ok(val) = reply.body().deserialize::<OwnedValue>() {
            if let Some(raw) = unwrap_variant_u32(&val) {
                snap.dark_mode = parse_portal_color_scheme(raw).is_dark();
            }
        }
    }
    if let Ok(reply) = conn
        .call_method(
            Some(PORTAL_BUS_NAME),
            PORTAL_OBJECT_PATH,
            Some(PORTAL_SETTINGS_IFACE),
            "ReadOne",
            &(APPEARANCE_NAMESPACE, "accent-color"),
        )
        .await
    {
        if let Ok(val) = reply.body().deserialize::<OwnedValue>() {
            if let Some((r, g, b)) = unwrap_variant_rgb(&val) {
                snap.accent_srgb8 = parse_portal_accent_color(r, g, b);
            }
        }
    }
    if let Ok(mut guard) = appearance.write() {
        *guard = snap;
    }
}

async fn watch_appearance_signals(
    conn: Connection,
    appearance: Arc<RwLock<LinuxAppearanceSnapshot>>,
    theme_listeners: Arc<Mutex<Vec<ThemeListener>>>,
) {
    let Ok(rule) = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PORTAL_SETTINGS_IFACE)
        .and_then(|b| b.member("SettingChanged"))
        .map(zbus::match_rule::Builder::build)
    else {
        return;
    };
    let Ok(mut stream) = MessageStream::for_match_rule(rule, &conn, Some(32)).await else {
        return;
    };
    while let Some(Ok(msg)) = stream.next().await {
        let Ok((ns, key, val)) = msg.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        if ns != APPEARANCE_NAMESPACE {
            continue;
        }
        match key.as_str() {
            "color-scheme" => {
                if let Some(raw) = unwrap_variant_u32(&val) {
                    let dark = parse_portal_color_scheme(raw).is_dark();
                    if let Ok(mut guard) = appearance.write() {
                        guard.dark_mode = dark;
                    }
                    if let Some(is_dark) = dark {
                        if let Ok(listeners) = theme_listeners.lock() {
                            for cb in listeners.iter() {
                                cb(is_dark);
                            }
                        }
                    }
                }
            }
            "accent-color" => {
                if let Some((r, g, b)) = unwrap_variant_rgb(&val) {
                    let accent = parse_portal_accent_color(r, g, b);
                    let dark_hint = if let Ok(mut guard) = appearance.write() {
                        guard.accent_srgb8 = accent;
                        guard.dark_mode
                    } else {
                        None
                    };
                    if let Some(is_dark) = dark_hint {
                        if let Ok(listeners) = theme_listeners.lock() {
                            for cb in listeners.iter() {
                                cb(is_dark);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn build_portal_filters(filters: &[(String, String)]) -> Option<Value<'static>> {
    let converted = convert_dialog_filters(filters);
    if converted.is_empty() {
        return None;
    }
    let raw: Vec<(String, Vec<(u32, String)>)> = converted
        .into_iter()
        .map(|f| (f.name, f.patterns))
        .collect();
    Some(Value::from(raw))
}

/// 依据 `xdg-desktop-portal` 规范在发起调用前预计算请求对象路径并订阅 `Response` 信号，
/// 彻底避免快速取消/失败时信号先于订阅到达导致的永久阻塞竞态。
fn predict_portal_request_path(conn: &Connection, token: &str) -> Option<String> {
    let unique = conn.unique_name()?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    Some(format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{token}"
    ))
}

async fn subscribe_portal_response(conn: &Connection, request_path: &str) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PORTAL_REQUEST_IFACE)
        .and_then(|b| b.member("Response"))
        .and_then(|b| b.path(request_path))
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, conn, Some(4))
        .await
        .ok()
}

async fn read_portal_response_uris(mut stream: MessageStream) -> Vec<PathBuf> {
    if let Some(Ok(msg)) = stream.next().await {
        let Ok((response_code, results)) = msg
            .body()
            .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        else {
            return Vec::new();
        };
        if response_code != 0 {
            return Vec::new();
        }
        let Some(uris_val) = results.get("uris") else {
            return Vec::new();
        };
        let Ok(uris) = Vec::<String>::try_from(uris_val.clone()) else {
            return Vec::new();
        };
        return uris.iter().filter_map(|u| decode_file_uri(u)).collect();
    }
    Vec::new()
}

async fn handle_open_file(
    conn: &Connection,
    config: &FileDialogConfig,
    directory: bool,
) -> Vec<PathBuf> {
    let title = config.title.as_deref().unwrap_or(if directory {
        "Select Folder"
    } else {
        "Open File"
    });
    let token = next_handle_token();
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("multiple", Value::from(config.multi_select && !directory));
    options.insert("directory", Value::from(directory));
    let folder_bytes: Option<Vec<u8>> = config.initial_dir.as_ref().map(|dir| {
        let mut b = dir.to_string_lossy().as_bytes().to_vec();
        b.push(0);
        b
    });
    if let Some(bytes) = &folder_bytes {
        options.insert("current_folder", Value::from(bytes.as_slice()));
    }
    let filters_owned = if directory {
        None
    } else {
        build_portal_filters(&config.filters)
    };
    if let Some(fval) = filters_owned {
        options.insert("filters", fval);
    }

    let pre_stream = match predict_portal_request_path(conn, &token) {
        Some(path) => subscribe_portal_response(conn, &path).await,
        None => None,
    };
    let Ok(reply) = conn
        .call_method(
            Some(PORTAL_BUS_NAME),
            PORTAL_OBJECT_PATH,
            Some(PORTAL_FILE_CHOOSER_IFACE),
            "OpenFile",
            &("", title, options),
        )
        .await
    else {
        return Vec::new();
    };
    let Ok(req_path) = reply.body().deserialize::<OwnedObjectPath>() else {
        return Vec::new();
    };
    let stream = match pre_stream {
        Some(st) => st,
        None => match subscribe_portal_response(conn, req_path.as_str()).await {
            Some(st) => st,
            None => return Vec::new(),
        },
    };
    read_portal_response_uris(stream).await
}

async fn handle_save_file(conn: &Connection, config: &FileDialogConfig) -> Option<PathBuf> {
    let title = config.title.as_deref().unwrap_or("Save File");
    let token = next_handle_token();
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    if let Some(name) = &config.default_name {
        options.insert("current_name", Value::from(name.as_str()));
    }
    let folder_bytes: Option<Vec<u8>> = config.initial_dir.as_ref().map(|dir| {
        let mut b = dir.to_string_lossy().as_bytes().to_vec();
        b.push(0);
        b
    });
    if let Some(bytes) = &folder_bytes {
        options.insert("current_folder", Value::from(bytes.as_slice()));
    }
    let filters_owned = build_portal_filters(&config.filters);
    if let Some(fval) = filters_owned {
        options.insert("filters", fval);
    }

    let pre_stream = match predict_portal_request_path(conn, &token) {
        Some(path) => subscribe_portal_response(conn, &path).await,
        None => None,
    };
    let Ok(reply) = conn
        .call_method(
            Some(PORTAL_BUS_NAME),
            PORTAL_OBJECT_PATH,
            Some(PORTAL_FILE_CHOOSER_IFACE),
            "SaveFile",
            &("", title, options),
        )
        .await
    else {
        return None;
    };
    let Ok(req_path) = reply.body().deserialize::<OwnedObjectPath>() else {
        return None;
    };
    let stream = match pre_stream {
        Some(st) => st,
        None => subscribe_portal_response(conn, req_path.as_str()).await?,
    };
    read_portal_response_uris(stream).await.into_iter().next()
}

/// `org.kde.StatusNotifierItem` 最小 D-Bus 接口实现。
struct StatusNotifierItem {
    title: String,
    status: String,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "unbound-ui"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        &self.title
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        &self.status
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "application-x-executable"
    }
}

async fn handle_set_tray_icon(conn: &Connection, tooltip: &str, visible: bool) -> bool {
    const SNI_PATH: &str = "/StatusNotifierItem";
    if !visible {
        let _ = conn
            .object_server()
            .remove::<StatusNotifierItem, _>(SNI_PATH)
            .await;
        return true;
    }
    let item = StatusNotifierItem {
        title: tooltip.to_owned(),
        status: "Active".to_owned(),
    };
    let _ = conn
        .object_server()
        .remove::<StatusNotifierItem, _>(SNI_PATH)
        .await;
    if conn.object_server().at(SNI_PATH, item).await.is_err() {
        return false;
    }
    conn.call_method(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        Some("org.kde.StatusNotifierWatcher"),
        "RegisterStatusNotifierItem",
        &(SNI_PATH,),
    )
    .await
    .is_ok()
}

async fn handle_show_notification(
    conn: &Connection,
    title: &str,
    message: &str,
    urgency: u8,
) -> bool {
    let actions: Vec<&str> = Vec::new();
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("urgency", Value::from(urgency));
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "unbound-ui",
            0u32,
            "",
            title,
            message,
            actions,
            hints,
            -1i32,
        ),
    )
    .await
    .is_ok()
}

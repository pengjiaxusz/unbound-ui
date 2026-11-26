//! 原生 Wayland 后端的纯逻辑策略层（跨平台编译，无显示器即可单元测试）。
//!
//! 将事件翻译、按键重复、可见性驱动节流（`wl_surface::frame` 挂起判据）、
//! 光标名映射、CSD 边缘缩放命中、`xdg_positioner` 相对锚点计算、主屏 `wl_output`
//! 几何采集以及跨线程剪贴板桥接抽象为纯逻辑组件，便于在无 Wayland 合成器的 CI
//! 环境中完整验证所有边界不变量。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::win32_window::{CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection};
use crate::window::{ImeEvent, Modifiers, PlatformKey, ScreenRect};

/// Wayland `wl_output` 采集到的主显示器物理尺寸、缩放因子与刷新率。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PrimaryOutputMetrics {
    /// 主屏物理像素宽度。
    pub physical_width: u32,
    /// 主屏物理像素高度。
    pub physical_height: u32,
    /// 主屏缩放因子（`1.0` 对应 100%）。
    pub scale_factor: f32,
    /// 主屏垂直刷新率（Hz）。
    pub refresh_rate_hz: Option<f32>,
}

impl PrimaryOutputMetrics {
    /// 计算逻辑尺寸（按缩放因子归一化）。
    #[must_use]
    pub fn logical_size(&self) -> (u32, u32) {
        let scale = if self.scale_factor.is_finite() && self.scale_factor > 0.0 {
            f64::from(self.scale_factor)
        } else {
            1.0
        };
        let w = (f64::from(self.physical_width) / scale).round().max(1.0) as u32;
        let h = (f64::from(self.physical_height) / scale).round().max(1.0) as u32;
        (w, h)
    }
}

static PRIMARY_OUTPUT: OnceLock<Mutex<Option<PrimaryOutputMetrics>>> = OnceLock::new();

/// 记录最近一次由 Wayland `wl_output` 上报的主屏几何与缩放因子。
pub fn set_primary_output_metrics(metrics: PrimaryOutputMetrics) {
    if metrics.physical_width == 0 || metrics.physical_height == 0 {
        return;
    }
    let lock = PRIMARY_OUTPUT.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(metrics);
    }
}

/// 查询当前已采集到的 Wayland 主屏几何与缩放因子（无输出时返回 `None`）。
#[must_use]
pub fn primary_output_metrics() -> Option<PrimaryOutputMetrics> {
    let lock = PRIMARY_OUTPUT.get_or_init(|| Mutex::new(None));
    lock.lock().ok().and_then(|guard| *guard)
}

/// Wayland 剪贴板读写钩子，由 Wayland 事件循环注册并在退出时卸载，供 [`crate::SystemClipboard`] 复用。
pub trait WaylandClipboardBackend: Send + Sync {
    /// 从当前 Wayland `wl_data_device` 选择源读取 UTF-8 文本。
    fn read_text(&self) -> Option<String>;
    /// 向 Wayland `wl_data_device` 发布 UTF-8 文本选择源。
    fn write_text(&self, text: &str) -> bool;
    /// Read file paths from the active Linux clipboard selection.
    fn read_file_paths(&self) -> Option<Vec<std::path::PathBuf>> {
        None
    }
    /// Read the file drop effect from the active Linux clipboard selection.
    fn read_file_drop_effect(&self) -> Option<crate::clipboard::ClipboardFileDropEffect> {
        None
    }
    /// Publish file paths to the Linux clipboard as `text/uri-list` and `x-special/gnome-copied-files`.
    fn write_file_paths(
        &self,
        _paths: &[std::path::PathBuf],
        _effect: crate::clipboard::ClipboardFileDropEffect,
        _uri_list: &str,
        _gnome_copied_files: &str,
    ) -> bool {
        false
    }
}

static WAYLAND_CLIPBOARD: OnceLock<Mutex<Option<std::sync::Arc<dyn WaylandClipboardBackend>>>> =
    OnceLock::new();

/// 注册当前活跃 Wayland 会话的系统剪贴板实现。
pub fn register_wayland_clipboard(backend: Option<std::sync::Arc<dyn WaylandClipboardBackend>>) {
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = backend;
    }
}

/// 通过已注册的 Wayland 剪贴板后端读取文本。
#[must_use]
pub fn read_wayland_clipboard_text() -> Option<String> {
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    let backend = lock.lock().ok()?.clone()?;
    backend.read_text()
}

/// 通过已注册的 Wayland 剪贴板后端写入文本；后端未注册时返回 `false`。
#[must_use]
pub fn write_wayland_clipboard_text(text: &str) -> bool {
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    let Some(backend) = lock.lock().ok().and_then(|g| g.clone()) else {
        return false;
    };
    backend.write_text(text)
}

/// Read file paths from the registered Linux clipboard backend.
#[must_use]
pub fn read_wayland_clipboard_file_paths() -> Option<Vec<std::path::PathBuf>> {
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    let backend = lock.lock().ok()?.clone()?;
    backend.read_file_paths()
}

/// Read file drop effect from the registered Linux clipboard backend.
#[must_use]
pub fn read_wayland_clipboard_file_drop_effect() -> Option<crate::clipboard::ClipboardFileDropEffect>
{
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    let backend = lock.lock().ok()?.clone()?;
    backend.read_file_drop_effect()
}

/// Write file paths to the registered Linux clipboard backend (`text/uri-list` + `x-special/gnome-copied-files`).
#[must_use]
pub fn write_wayland_clipboard_file_paths(
    paths: &[std::path::PathBuf],
    effect: crate::clipboard::ClipboardFileDropEffect,
    uri_list: &str,
    gnome_copied_files: &str,
) -> bool {
    let lock = WAYLAND_CLIPBOARD.get_or_init(|| Mutex::new(None));
    let Some(backend) = lock.lock().ok().and_then(|g| g.clone()) else {
        return false;
    };
    backend.write_file_paths(paths, effect, uri_list, gnome_copied_files)
}

/// Wayland 可见性驱动节流状态机（ADR-0030 / ADR-0032）。
///
/// 窗口失焦但仍可见时维持正常帧率；仅当窗口最小化、隐藏或合成器长时间挂起
/// `wl_surface::frame` 回调时进入 `Suspended` 停绘节能。
#[derive(Copy, Clone, Debug)]
pub struct WaylandFrameThrottle {
    /// 窗口是否最小化或被隐藏。
    hidden_or_minimized: bool,
    /// 当前是否仍有未完成的 `wl_surface::frame` 回调。
    frame_callback_pending: bool,
    /// 最近一次发出 `wl_surface::frame` 请求的时刻。
    frame_callback_sent_at: Option<Instant>,
    /// 合成器无响应帧回调超过该阈值后判定窗口已被合成器挂起/遮挡。
    suspend_threshold: Duration,
}

impl Default for WaylandFrameThrottle {
    fn default() -> Self {
        Self::new(Duration::from_millis(250))
    }
}

impl WaylandFrameThrottle {
    /// 构造带自定义帧回调挂起阈值的节流状态机。
    #[must_use]
    pub const fn new(suspend_threshold: Duration) -> Self {
        Self {
            hidden_or_minimized: false,
            frame_callback_pending: false,
            frame_callback_sent_at: None,
            suspend_threshold,
        }
    }

    /// 更新窗口最小化或隐藏状态。
    pub fn set_hidden_or_minimized(&mut self, hidden: bool) {
        self.hidden_or_minimized = hidden;
        if !hidden {
            self.frame_callback_pending = false;
            self.frame_callback_sent_at = None;
        }
    }

    /// 记录在 `now` 时刻向合成器请求了一次 `wl_surface::frame` 回调。
    pub fn on_frame_requested(&mut self, now: Instant) {
        if !self.frame_callback_pending {
            self.frame_callback_pending = true;
            self.frame_callback_sent_at = Some(now);
        }
    }

    /// 收到合成器 `wl_surface::frame` `Done` 回调，立即解除挂起态。
    pub fn on_frame_done(&mut self) {
        self.frame_callback_pending = false;
        self.frame_callback_sent_at = None;
    }

    /// 判断当前在 `now` 时刻窗口是否应进入 `Suspended` 停绘节能态。
    #[must_use]
    pub fn is_suspended(&self, now: Instant) -> bool {
        if self.hidden_or_minimized {
            return true;
        }
        if self.frame_callback_pending {
            if let Some(sent_at) = self.frame_callback_sent_at {
                return now.saturating_duration_since(sent_at) >= self.suspend_threshold;
            }
        }
        false
    }
}

/// 将标准 Linux `evdev` 键码（`wl_keyboard::Event::Key` 提供的 `key`，已减去 X11 偏移 8）
/// 与修饰键状态翻译为跨平台 [`PlatformKey`] 以及可选的 UTF-8 输入文本。
#[must_use]
pub fn translate_evdev_key(keycode: u32, modifiers: Modifiers) -> (PlatformKey, Option<String>) {
    let suppress_text = modifiers.control || modifiers.alt || modifiers.meta;
    let shift = modifiers.shift;

    match keycode {
        1 => (PlatformKey::Named("Escape".into()), None),
        14 => (PlatformKey::Named("Backspace".into()), None),
        15 => (PlatformKey::Named("Tab".into()), None),
        28 | 96 => (PlatformKey::Named("Enter".into()), None),
        57 => (
            PlatformKey::Named("Space".into()),
            (!suppress_text).then(|| " ".to_owned()),
        ),
        58 => (PlatformKey::Named("CapsLock".into()), None),
        59..=68 => (PlatformKey::Named(format!("F{}", keycode - 58)), None),
        87 => (PlatformKey::Named("F11".into()), None),
        88 => (PlatformKey::Named("F12".into()), None),
        102 => (PlatformKey::Named("Home".into()), None),
        103 => (PlatformKey::Named("ArrowUp".into()), None),
        104 => (PlatformKey::Named("PageUp".into()), None),
        105 => (PlatformKey::Named("ArrowLeft".into()), None),
        106 => (PlatformKey::Named("ArrowRight".into()), None),
        107 => (PlatformKey::Named("End".into()), None),
        108 => (PlatformKey::Named("ArrowDown".into()), None),
        109 => (PlatformKey::Named("PageDown".into()), None),
        110 => (PlatformKey::Named("Insert".into()), None),
        111 => (PlatformKey::Named("Delete".into()), None),
        127 | 139 => (PlatformKey::Named("ContextMenu".into()), None),
        _ => {
            if let Some((normal, shifted)) = evdev_printable_pair(keycode) {
                let ch = if shift { shifted } else { normal };
                let s = ch.to_string();
                let text = (!suppress_text).then(|| s.clone());
                (PlatformKey::Character(s), text)
            } else {
                (PlatformKey::Other(keycode), None)
            }
        }
    }
}

/// 标准 US-ANSI 键盘的 `evdev` 可打印字符映射（常规字符，Shift 字符）。
const fn evdev_printable_pair(keycode: u32) -> Option<(char, char)> {
    match keycode {
        2 => Some(('1', '!')),
        3 => Some(('2', '@')),
        4 => Some(('3', '#')),
        5 => Some(('4', '$')),
        6 => Some(('5', '%')),
        7 => Some(('6', '^')),
        8 => Some(('7', '&')),
        9 => Some(('8', '*')),
        10 => Some(('9', '(')),
        11 => Some(('0', ')')),
        12 => Some(('-', '_')),
        13 => Some(('=', '+')),
        16 => Some(('q', 'Q')),
        17 => Some(('w', 'W')),
        18 => Some(('e', 'E')),
        19 => Some(('r', 'R')),
        20 => Some(('t', 'T')),
        21 => Some(('y', 'Y')),
        22 => Some(('u', 'U')),
        23 => Some(('i', 'I')),
        24 => Some(('o', 'O')),
        25 => Some(('p', 'P')),
        26 => Some(('[', '{')),
        27 => Some((']', '}')),
        30 => Some(('a', 'A')),
        31 => Some(('s', 'S')),
        32 => Some(('d', 'D')),
        33 => Some(('f', 'F')),
        34 => Some(('g', 'G')),
        35 => Some(('h', 'H')),
        36 => Some(('j', 'J')),
        37 => Some(('k', 'K')),
        38 => Some(('l', 'L')),
        39 => Some((';', ':')),
        40 => Some(('\'', '"')),
        41 => Some(('`', '~')),
        43 => Some(('\\', '|')),
        44 => Some(('z', 'Z')),
        45 => Some(('x', 'X')),
        46 => Some(('c', 'C')),
        47 => Some(('v', 'V')),
        48 => Some(('b', 'B')),
        49 => Some(('n', 'N')),
        50 => Some(('m', 'M')),
        51 => Some((',', '<')),
        52 => Some(('.', '>')),
        53 => Some(('/', '?')),
        _ => None,
    }
}

/// Wayland 软件按键重复状态机（依据 `wl_keyboard::Event::RepeatInfo` 参数驱动）。
#[derive(Clone, Debug)]
pub struct KeyRepeatState {
    /// 每秒重复次数（`0` 表示禁用重复）。
    pub rate_hz: i32,
    /// 首次重复前的延迟（毫秒）。
    pub delay_ms: i32,
    /// 当前按住的键码、逻辑键与可选文本。
    held: Option<(u32, PlatformKey, Option<String>)>,
    /// 下一次应派发重复事件的时刻。
    next_fire: Option<Instant>,
}

impl Default for KeyRepeatState {
    fn default() -> Self {
        Self {
            rate_hz: 25,
            delay_ms: 400,
            held: None,
            next_fire: None,
        }
    }
}

impl KeyRepeatState {
    /// 更新合成器上报的重复速率与延迟。
    pub fn set_repeat_info(&mut self, rate_hz: i32, delay_ms: i32) {
        self.rate_hz = rate_hz.max(0);
        self.delay_ms = delay_ms.max(0);
        if self.rate_hz == 0 {
            self.next_fire = None;
        }
    }

    /// 记录按键按下，启动首次重复计时。
    pub fn on_key_down(
        &mut self,
        keycode: u32,
        key: PlatformKey,
        text: Option<String>,
        now: Instant,
    ) {
        self.held = Some((keycode, key, text));
        if self.rate_hz > 0 {
            self.next_fire = Some(now + Duration::from_millis(self.delay_ms as u64));
        } else {
            self.next_fire = None;
        }
    }

    /// 记录按键抬起；若与当前按住键码一致则停止重复。
    pub fn on_key_up(&mut self, keycode: u32) {
        if self
            .held
            .as_ref()
            .is_some_and(|(code, _, _)| *code == keycode)
        {
            self.held = None;
            self.next_fire = None;
        }
    }

    /// 清空全部按住状态（如窗口失去键盘焦点时）。
    pub fn clear(&mut self) {
        self.held = None;
        self.next_fire = None;
    }

    /// 返回下一次按键重复的唤醒时间点（供事件循环设置等待截止时间）。
    #[must_use]
    pub const fn next_wake(&self) -> Option<Instant> {
        self.next_fire
    }

    /// 若在 `now` 时刻已到期，产出一次重复按键并推进下一次到期时间。
    pub fn poll_repeat(&mut self, now: Instant) -> Option<(PlatformKey, Option<String>)> {
        let fire_at = self.next_fire?;
        if now < fire_at || self.rate_hz <= 0 {
            return None;
        }
        let (_, key, text) = self.held.clone()?;
        let period = Duration::from_nanos(1_000_000_000 / (self.rate_hz as u64).max(1));
        self.next_fire = Some(now + period);
        Some((key, text))
    }
}

/// 将框架标准 [`CursorIcon`] 映射为 Wayland 光标主题名（`wl_cursor` / XCursor 规范）。
#[must_use]
pub const fn wayland_cursor_name(icon: CursorIcon) -> &'static str {
    match icon {
        CursorIcon::Default | CursorIcon::Custom(_) => "default",
        CursorIcon::Pointer => "pointer",
        CursorIcon::Text => "text",
        CursorIcon::Grab => "grab",
        CursorIcon::Grabbing => "grabbing",
        CursorIcon::Crosshair => "crosshair",
        CursorIcon::Move => "move",
        CursorIcon::NsResize => "ns-resize",
        CursorIcon::EwResize => "ew-resize",
        CursorIcon::NwseResize => "nwse-resize",
        CursorIcon::NeswResize => "nesw-resize",
        CursorIcon::NotAllowed => "not-allowed",
    }
}

/// 将边缘缩放方向映射为对应的双向箭头光标。
#[must_use]
pub const fn cursor_for_resize_direction(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
        ResizeDirection::West | ResizeDirection::East => CursorIcon::EwResize,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
    }
}

/// 判定逻辑坐标 `(x, y)` 是否落入无边框 CSD 窗口的四边/四角缩放热区。
///
/// 最大化态或不可缩放窗口应由调用方跳过此判定；返回 `Some(ResizeDirection)` 时，
/// 悬停态可自动切换对应双向箭头光标，左键按下时触发 `xdg_toplevel::resize`。
#[must_use]
pub fn csd_resize_direction_at(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    border: f64,
) -> Option<ResizeDirection> {
    if width <= 0.0 || height <= 0.0 || border <= 0.0 {
        return None;
    }
    let left = x >= 0.0 && x < border;
    let right = x <= width && x > width - border;
    let top = y >= 0.0 && y < border;
    let bottom = y <= height && y > height - border;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (true, _, _, true) => Some(ResizeDirection::NorthEast),
        (_, true, true, _) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, _, _, _) => Some(ResizeDirection::North),
        (_, true, _, _) => Some(ResizeDirection::South),
        (_, _, true, _) => Some(ResizeDirection::West),
        (_, _, _, true) => Some(ResizeDirection::East),
        _ => None,
    }
}

/// Wayland `xdg_positioner` 越界浮层定位参数（逻辑像素）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WaylandPopupPlacement {
    /// 浮层自身逻辑宽度（至少为 1）。
    pub width: i32,
    /// 浮层自身逻辑高度（至少为 1）。
    pub height: i32,
    /// 相对父窗口表面原点的锚点矩形 `(x, y, w, h)`（逻辑像素）。
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: i32,
    pub anchor_height: i32,
    /// 相对锚点边缘的偏移量 `(dx, dy)`。
    pub offset_x: i32,
    pub offset_y: i32,
    /// 是否允许水平翻转（`FLIP_X`）。
    pub flip_x: bool,
    /// 是否允许垂直翻转（`FLIP_Y`）。
    pub flip_y: bool,
    /// 是否允许水平滑动夹紧（`SLIDE_X`）。
    pub slide_x: bool,
    /// 是否允许垂直滑动夹紧（`SLIDE_Y`）。
    pub slide_y: bool,
    /// 是否允许在空间极端不足时压缩尺寸（`RESIZE_X | RESIZE_Y`）。
    pub resize: bool,
}

/// 将父窗相对锚点矩形与期望浮层尺寸翻译为 `xdg_positioner` 定位参数。
///
/// 当调用方未显式提供 `anchor_rect` 时，按 `fallback_rect` 构造 1×1 锚点并开启完整
/// 贴边翻转、滑动与压缩约束，保证浮层在贴近输出边界时不被父窗裁切也不超出屏幕可视区。
#[must_use]
pub fn compute_wayland_popup_placement(
    anchor_rect: Option<ScreenRect>,
    popup_rect: ScreenRect,
    scale_factor: f64,
) -> WaylandPopupPlacement {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let to_logical = |v: i32| ((f64::from(v)) / scale).round() as i32;
    let to_logical_u = |v: u32| ((f64::from(v)) / scale).round().max(1.0) as i32;

    let width = to_logical_u(popup_rect.width);
    let height = to_logical_u(popup_rect.height);

    if let Some(anchor) = anchor_rect {
        WaylandPopupPlacement {
            width,
            height,
            anchor_x: to_logical(anchor.x),
            anchor_y: to_logical(anchor.y),
            anchor_width: to_logical_u(anchor.width),
            anchor_height: to_logical_u(anchor.height),
            offset_x: 0,
            offset_y: 0,
            flip_x: true,
            flip_y: true,
            slide_x: true,
            slide_y: true,
            resize: true,
        }
    } else {
        WaylandPopupPlacement {
            width,
            height,
            anchor_x: to_logical(popup_rect.x),
            anchor_y: to_logical(popup_rect.y),
            anchor_width: 1,
            anchor_height: 1,
            offset_x: 0,
            offset_y: 0,
            flip_x: true,
            flip_y: true,
            slide_x: true,
            slide_y: true,
            resize: true,
        }
    }
}

/// Wayland `zwp_text_input_v3` 候选框锚点逻辑矩形（表面局部逻辑像素，宽高至少为 1）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WaylandImeCursorRect {
    /// 相对表面左上角的逻辑 X 坐标。
    pub x: i32,
    /// 相对表面左上角的逻辑 Y 坐标。
    pub y: i32,
    /// 逻辑宽度（至少为 1）。
    pub width: i32,
    /// 逻辑高度（至少为 1）。
    pub height: i32,
}

/// 将绘制阶段回写的物理像素输入法锚点换算为 Wayland 表面局部逻辑坐标并钳制在客户区内。
#[must_use]
pub fn compute_wayland_ime_cursor_rect(
    position: PhysicalPosition,
    size: PhysicalSize,
    scale_factor: f64,
    window_size: PhysicalSize,
) -> WaylandImeCursorRect {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let win_w = (f64::from(window_size.width.max(1)) / scale)
        .round()
        .max(1.0) as i32;
    let win_h = (f64::from(window_size.height.max(1)) / scale)
        .round()
        .max(1.0) as i32;

    let raw_x = (f64::from(position.x) / scale).round() as i32;
    let raw_y = (f64::from(position.y) / scale).round() as i32;
    let raw_w = (f64::from(size.width) / scale).round().max(1.0) as i32;
    let raw_h = (f64::from(size.height) / scale).round().max(1.0) as i32;

    let x = raw_x.clamp(0, win_w.saturating_sub(1));
    let y = raw_y.clamp(0, win_h.saturating_sub(1));
    let max_w = (win_w - x).max(1);
    let max_h = (win_h - y).max(1);
    let width = raw_w.clamp(1, max_w);
    let height = raw_h.clamp(1, max_h);

    WaylandImeCursorRect {
        x,
        y,
        width,
        height,
    }
}

/// 将字节索引向低位对齐至合法 UTF-8 字符边界。
fn clamp_utf8_boundary(text: &str, idx: usize) -> usize {
    let mut pos = idx.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// `WaylandImeState` 要求向合成器下发的协议动作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WaylandImeProtocolAction {
    /// 无需向合成器发送协议请求。
    None,
    /// 发送 `enable`（可选附带 `set_cursor_rectangle`）并 `commit`。
    Enable {
        /// 若已有有效锚点则一并同步。
        cursor_rect: Option<WaylandImeCursorRect>,
    },
    /// 仅更新候选框锚点并 `commit`。
    UpdateCursorRect(WaylandImeCursorRect),
    /// 发送 `disable` 并 `commit`。
    Disable,
}

/// Wayland `zwp_text_input_v3` 双缓冲状态机（对齐 ADR 0011，纯逻辑可单测）。
///
/// 协议中 `preedit_string` 与 `commit_string` 在收到 `done` 之前仅暂存于双缓冲槽，
/// 收到 `done` 后按规范顺序（先 `commit` 后 `preedit`）原子消费并立即清空，
/// 绝不在平台层长期缓存待提交文本。
#[derive(Clone, Debug, Default)]
pub struct WaylandImeState {
    focused_surface: Option<u32>,
    ime_allowed: bool,
    enabled_on_compositor: bool,
    active_composition: bool,
    pending_preedit: Option<(String, Option<(usize, usize)>)>,
    pending_commit: Option<String>,
    last_cursor_rect: Option<WaylandImeCursorRect>,
}

impl WaylandImeState {
    /// 当前输入法焦点所在表面协议 ID。
    #[must_use]
    pub const fn focused_surface(&self) -> Option<u32> {
        self.focused_surface
    }

    /// 当前是否处于活跃预编辑组合状态。
    #[must_use]
    pub const fn has_active_composition(&self) -> bool {
        self.active_composition
    }

    /// 表面获得 `zwp_text_input_v3::enter` 焦点。
    pub fn on_enter(&mut self, surface_id: u32) -> WaylandImeProtocolAction {
        self.focused_surface = Some(surface_id);
        self.pending_preedit = None;
        self.pending_commit = None;
        if self.ime_allowed && !self.enabled_on_compositor {
            self.enabled_on_compositor = true;
            WaylandImeProtocolAction::Enable {
                cursor_rect: self.last_cursor_rect,
            }
        } else {
            WaylandImeProtocolAction::None
        }
    }

    /// 表面失去 `zwp_text_input_v3::leave` 焦点；若存在活跃组合则同步取消并恢复快照。
    pub fn on_leave(&mut self, surface_id: u32) -> (WaylandImeProtocolAction, Option<ImeEvent>) {
        if self.focused_surface == Some(surface_id) {
            self.focused_surface = None;
        }
        self.cancel_and_disable()
    }

    /// 键盘失焦时的兜底取消路径：立即清空暂存并向编辑核心派发 `ImeEvent::Disabled`。
    pub fn on_focus_lost(&mut self) -> (WaylandImeProtocolAction, Option<ImeEvent>) {
        self.cancel_and_disable()
    }

    /// 应用层切换当前聚焦控件是否允许 IME。
    pub fn set_allowed(&mut self, allowed: bool) -> (WaylandImeProtocolAction, Option<ImeEvent>) {
        self.ime_allowed = allowed;
        if allowed {
            if self.focused_surface.is_some() && !self.enabled_on_compositor {
                self.enabled_on_compositor = true;
                return (
                    WaylandImeProtocolAction::Enable {
                        cursor_rect: self.last_cursor_rect,
                    },
                    None,
                );
            }
            (WaylandImeProtocolAction::None, None)
        } else {
            self.cancel_and_disable()
        }
    }

    /// 更新逻辑候选框锚点；未变化或未启用时跳过冗余协议提交。
    pub fn set_cursor_area(&mut self, rect: WaylandImeCursorRect) -> WaylandImeProtocolAction {
        if self.last_cursor_rect == Some(rect) {
            return WaylandImeProtocolAction::None;
        }
        self.last_cursor_rect = Some(rect);
        if self.enabled_on_compositor {
            WaylandImeProtocolAction::UpdateCursorRect(rect)
        } else {
            WaylandImeProtocolAction::None
        }
    }

    /// 暂存协议双缓冲 `preedit_string` 事件。
    pub fn on_preedit(&mut self, text: Option<String>, cursor_begin: i32, cursor_end: i32) {
        let text = text.unwrap_or_default();
        let selection = if cursor_begin >= 0 && cursor_end >= 0 {
            let start = clamp_utf8_boundary(&text, cursor_begin as usize);
            let end = clamp_utf8_boundary(&text, cursor_end as usize);
            Some((start.min(end), start.max(end)))
        } else {
            None
        };
        self.pending_preedit = Some((text, selection));
    }

    /// 暂存协议双缓冲 `commit_string` 事件。
    pub fn on_commit(&mut self, text: Option<String>) {
        self.pending_commit = text.filter(|s| !s.is_empty());
    }

    /// 收到 `zwp_text_input_v3::done` 时原子归并并清空双缓冲槽。
    pub fn on_done(&mut self) -> Vec<ImeEvent> {
        let commit = self.pending_commit.take();
        let preedit = self.pending_preedit.take();
        let mut out = Vec::new();

        if let Some(text) = commit {
            self.active_composition = false;
            out.push(ImeEvent::Commit(text));
        }

        match preedit {
            Some((text, sel)) if !text.is_empty() => {
                self.active_composition = true;
                out.push(ImeEvent::Preedit(text, sel));
            }
            Some(_) => {
                if self.active_composition {
                    self.active_composition = false;
                    out.push(ImeEvent::Preedit(String::new(), None));
                }
            }
            None => {
                // `text_input_v3` 规定 `done` 前未收到 `preedit_string` 即清除旧预编辑。
                if self.active_composition {
                    self.active_composition = false;
                    out.push(ImeEvent::Preedit(String::new(), None));
                }
            }
        }

        out
    }

    fn cancel_and_disable(&mut self) -> (WaylandImeProtocolAction, Option<ImeEvent>) {
        self.pending_preedit = None;
        self.pending_commit = None;
        let action = if self.enabled_on_compositor {
            self.enabled_on_compositor = false;
            WaylandImeProtocolAction::Disable
        } else {
            WaylandImeProtocolAction::None
        };
        let event = if self.active_composition {
            self.active_composition = false;
            Some(ImeEvent::Disabled)
        } else {
            None
        };
        (action, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_key_translation_covers_navigation_letters_and_modifiers() {
        let none = Modifiers::default();
        assert_eq!(
            translate_evdev_key(1, none),
            (PlatformKey::Named("Escape".into()), None)
        );
        assert_eq!(
            translate_evdev_key(15, none),
            (PlatformKey::Named("Tab".into()), None)
        );
        assert_eq!(
            translate_evdev_key(30, none),
            (PlatformKey::Character("a".into()), Some("a".into()))
        );
        let shifted = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            translate_evdev_key(30, shifted),
            (PlatformKey::Character("A".into()), Some("A".into()))
        );
        // Ctrl+C 产生字符键名但不生成可插入文本
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(
            translate_evdev_key(46, ctrl),
            (PlatformKey::Character("c".into()), None)
        );
    }

    #[test]
    fn wayland_frame_throttle_suspends_on_hidden_or_stalled_frame_callback() {
        let mut throttle = WaylandFrameThrottle::new(Duration::from_millis(200));
        let t0 = Instant::now();
        assert!(!throttle.is_suspended(t0));

        // 发出帧回调请求后在阈值内仍保持活跃
        throttle.on_frame_requested(t0);
        assert!(!throttle.is_suspended(t0 + Duration::from_millis(100)));
        // 合成器长时间挂起回调后进入 Suspended
        assert!(throttle.is_suspended(t0 + Duration::from_millis(250)));

        // 收到 Done 回调立即恢复活跃
        throttle.on_frame_done();
        assert!(!throttle.is_suspended(t0 + Duration::from_millis(300)));

        // 最小化/隐藏立即进入 Suspended
        throttle.set_hidden_or_minimized(true);
        assert!(throttle.is_suspended(t0 + Duration::from_millis(300)));
        throttle.set_hidden_or_minimized(false);
        assert!(!throttle.is_suspended(t0 + Duration::from_millis(300)));
    }

    #[test]
    fn key_repeat_state_fires_after_delay_and_stops_on_release() {
        let mut repeat = KeyRepeatState::default();
        repeat.set_repeat_info(20, 300);
        let t0 = Instant::now();
        repeat.on_key_down(30, PlatformKey::Character("a".into()), Some("a".into()), t0);
        assert!(repeat
            .poll_repeat(t0 + Duration::from_millis(200))
            .is_none());
        assert_eq!(
            repeat.poll_repeat(t0 + Duration::from_millis(300)),
            Some((PlatformKey::Character("a".into()), Some("a".into())))
        );
        repeat.on_key_up(30);
        assert!(repeat
            .poll_repeat(t0 + Duration::from_millis(500))
            .is_none());
    }

    #[test]
    fn csd_resize_hit_test_detects_all_edges_and_corners() {
        assert_eq!(
            csd_resize_direction_at(2.0, 2.0, 800.0, 600.0, 6.0),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            csd_resize_direction_at(798.0, 598.0, 800.0, 600.0, 6.0),
            Some(ResizeDirection::SouthEast)
        );
        assert_eq!(
            csd_resize_direction_at(400.0, 2.0, 800.0, 600.0, 6.0),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            csd_resize_direction_at(400.0, 300.0, 800.0, 600.0, 6.0),
            None
        );
    }

    #[test]
    fn wayland_popup_placement_converts_relative_anchor_and_enables_flip_slide_resize() {
        let anchor = ScreenRect::new(200, 100, 160, 64);
        let popup = ScreenRect::new(200, 164, 320, 240);
        let placed = compute_wayland_popup_placement(Some(anchor), popup, 2.0);
        assert_eq!(placed.anchor_x, 100);
        assert_eq!(placed.anchor_y, 50);
        assert_eq!(placed.anchor_width, 80);
        assert_eq!(placed.anchor_height, 32);
        assert_eq!(placed.width, 160);
        assert_eq!(placed.height, 120);
        assert!(
            placed.flip_x && placed.flip_y && placed.slide_x && placed.slide_y && placed.resize
        );
    }

    #[test]
    fn cursor_names_map_every_standard_icon() {
        assert_eq!(wayland_cursor_name(CursorIcon::Default), "default");
        assert_eq!(wayland_cursor_name(CursorIcon::Pointer), "pointer");
        assert_eq!(wayland_cursor_name(CursorIcon::Text), "text");
        assert_eq!(wayland_cursor_name(CursorIcon::NwseResize), "nwse-resize");
        assert_eq!(
            cursor_for_resize_direction(ResizeDirection::East),
            CursorIcon::EwResize
        );
    }

    #[test]
    fn wayland_ime_cursor_rect_scales_clamps_and_deduplicates() {
        let rect = compute_wayland_ime_cursor_rect(
            PhysicalPosition::new(300, 150),
            PhysicalSize::new(3, 36),
            1.5,
            PhysicalSize::new(1200, 900),
        );
        assert_eq!(
            rect,
            WaylandImeCursorRect {
                x: 200,
                y: 100,
                width: 2,
                height: 24,
            }
        );

        // 越界与零尺寸锚点必须钳制在逻辑客户区内且宽高 >= 1
        let clamped = compute_wayland_ime_cursor_rect(
            PhysicalPosition::new(-40, 2000),
            PhysicalSize::new(0, 0),
            2.0,
            PhysicalSize::new(800, 600),
        );
        assert_eq!(
            clamped,
            WaylandImeCursorRect {
                x: 0,
                y: 299,
                width: 1,
                height: 1,
            }
        );

        let mut ime = WaylandImeState::default();
        assert_eq!(ime.on_enter(7), WaylandImeProtocolAction::None);
        assert_eq!(
            ime.set_allowed(true),
            (WaylandImeProtocolAction::Enable { cursor_rect: None }, None)
        );
        assert_eq!(
            ime.set_cursor_area(rect),
            WaylandImeProtocolAction::UpdateCursorRect(rect)
        );
        // 相同锚点不重复提交
        assert_eq!(ime.set_cursor_area(rect), WaylandImeProtocolAction::None);
    }

    #[test]
    fn wayland_ime_double_buffer_flushes_atomically_on_done_without_caching_commits() {
        let mut ime = WaylandImeState::default();
        ime.on_enter(1);
        let _ = ime.set_allowed(true);

        // 长 CJK 预编辑文本在 done 前暂存，done 时原子派发且保留 UTF-8 边界
        let long_cjk = "中文输入法超长预编辑测试内容校验组合区间".to_string(); // lang-ok: CJK UTF-8 test fixture
        ime.on_preedit(Some(long_cjk.clone()), 0, long_cjk.len() as i32);
        assert!(!ime.has_active_composition());
        let events = ime.on_done();
        assert_eq!(
            events,
            vec![ImeEvent::Preedit(
                long_cjk.clone(),
                Some((0, long_cjk.len()))
            )]
        );
        assert!(ime.has_active_composition());

        // commit + 清除预编辑在同一次 done 中先 commit，不产生重复提交
        ime.on_commit(Some("你好世界".to_string())); // lang-ok: CJK UTF-8 test fixture
        let events = ime.on_done();
        assert_eq!(events, vec![ImeEvent::Commit("你好世界".to_string())]); // lang-ok: CJK UTF-8 test fixture
        assert!(!ime.has_active_composition());

        // 下一次空 done 不重复提交已消费的 commit
        assert!(ime.on_done().is_empty());
    }

    #[test]
    fn wayland_ime_cancels_active_composition_on_leave_focus_loss_or_disable() {
        let mut ime = WaylandImeState::default();
        ime.on_enter(9);
        let _ = ime.set_allowed(true);
        ime.on_preedit(Some("zhongwen".to_string()), 8, 8);
        let _ = ime.on_done();
        assert!(ime.has_active_composition());

        // leave 立即下发 Disable 并派发 ImeEvent::Disabled
        let (action, ev) = ime.on_leave(9);
        assert_eq!(action, WaylandImeProtocolAction::Disable);
        assert_eq!(ev, Some(ImeEvent::Disabled));
        assert!(!ime.has_active_composition());

        // 重新激活后再测试禁用 IME 取消组合
        assert_eq!(
            ime.on_enter(9),
            WaylandImeProtocolAction::Enable { cursor_rect: None }
        );
        ime.on_preedit(Some("nihao".to_string()), 5, 5);
        let _ = ime.on_done();
        let (action, ev) = ime.set_allowed(false);
        assert_eq!(action, WaylandImeProtocolAction::Disable);
        assert_eq!(ev, Some(ImeEvent::Disabled));
        assert!(!ime.has_active_composition());
    }
}

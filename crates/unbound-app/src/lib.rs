//! 应用运行时：面向所有应用的统一事件循环模型。
//!
//! 运行时统一接管了过去需要在每个应用外壳中手动复制的模块：
//!
//!
//! * [`Application`] 契约 —— `init`、`reduce`、`view`；
//! * 每帧重建并与上一帧 diff 的 [`SceneCollector`]，使渲染器仅接收
//!   上传计划而非整套场景；
//!
//! * 经由真实 [`Router`] 的指针路由，包括悬停、按下、点击、双击与拖拽转换；
//!
//! * 经由 [`FocusManager`] 的键盘焦点策略；
//! * 脏标志机制，使无变更帧可完全跳过重建。
//!
//! 运行时刻意保持无头（headless）：它产出图元、命中区域与语义树，
//! 从不直接创建窗口或交换链。窗口化外壳向其输入窗口事件并上传 [`FrameReport`]；
//! 测试则向其输入合成事件并对同一份报告进行断言。因此 showcase 外壳与
//! `runtime_headless.rs` 演练的是完全相同的代码路径。
//!

use std::any::Any;
use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use accesskit::Role;
use crossbeam_channel::{Receiver, Sender};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::{A11yAction, A11yNode, SemanticIntent};
use unbound_abi::{Dim, NodeFlags, RawNodeDesc, PORTAL_Z_BASE};
use unbound_gpu::atlas::AtlasUpload;
use unbound_gpu::data::{
    BlendMode, GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType,
    SubElement, CLIP_NONE,
};
use unbound_input::{
    CursorKind, FocusCause, FocusKey, FocusManager, KeyChord, KeyName, Keymap, PointerButton,
    PointerEvent, Router,
};
use unbound_layout::{LayoutEngine, LayoutStats};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine, TextStyle};
use unbound_theme::ThemeKind;
use unbound_widgets::alert::AlertDesc;
use unbound_widgets::badge::BadgeDesc;
use unbound_widgets::button::ButtonDesc;
use unbound_widgets::checkbox::CheckboxDesc;
use unbound_widgets::diff::{diff_primitives, SceneDiff};
use unbound_widgets::divider::DividerDesc;
use unbound_widgets::progress::{ProgressDesc, ProgressState};
use unbound_widgets::slider::SliderDesc;
use unbound_widgets::switch::SwitchDesc;
use unbound_widgets::{BackdropRequest, ControlState, SceneCollector, WidgetOutput};
use unbound_world::{NodeId, UiWorld};

pub mod assets;
pub mod guest_bridge;
pub mod i18n;
pub mod inspector;
pub mod overlay_manager;
pub mod prelude;
pub mod router;
pub mod scene_upload;
pub mod scheduler;
pub mod state_store;
pub mod window_runner;

pub use assets::{AssetHub, AssetResidency};
pub use guest_bridge::GuestAppAdapter;
pub use i18n::{global_locale, set_global_locale, I18nBundle, LayoutDirection, Locale};
pub use inspector::{EventTrace, Inspector, InspectorNode, InspectorScene};
pub use overlay_manager::OverlayManager;
pub use router::{RouteEntry, RouterStack, TransitionDirection};
pub use scene_upload::{collect_backdrops, scene_upload};
pub use scheduler::{AppWaker, TimerQueue, TimerRequest, TimerToken};
pub use state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use unbound_input::{FocusGroup, FocusGroupMode, RovingKey, RovingOrientation};
pub use unbound_widgets::ergonomics::UiWidget;
pub use window_runner::{
    run_windowed, run_windowed_with_frame_callback, run_windowed_with_hooks,
    run_windowed_with_hwnd, WindowedRunner,
};

/// [`ui_key!`] 使用的编译期 64 位 FNV-1a 哈希。
pub const fn const_fnv1a_hash(file: &str, line: u32, column: u32) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    let bytes = file.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash ^= line as u64;
    hash = hash.wrapping_mul(FNV_PRIME);
    hash ^= column as u64;
    hash = hash.wrapping_mul(FNV_PRIME);
    hash
}

/// 将基础键与运行时判别值或循环索引混合。
pub const fn mix_key(base: u64, discriminator: u64) -> u64 {
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = base ^ discriminator;
    hash = hash.wrapping_mul(FNV_PRIME);
    hash
}

/// 将各类判别类型转换为用于 [`ui_key!`] 的 u64 的 trait。
pub trait KeyDiscriminator {
    /// 将自身转换为 u64 哈希/值。
    fn into_u64(self) -> u64;
}

impl KeyDiscriminator for u64 {
    #[inline]
    fn into_u64(self) -> u64 {
        self
    }
}

impl KeyDiscriminator for usize {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for u32 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for i32 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for &str {
    #[inline]
    fn into_u64(self) -> u64 {
        const_fnv1a_hash(self, 0, 0)
    }
}

impl KeyDiscriminator for &String {
    #[inline]
    fn into_u64(self) -> u64 {
        const_fnv1a_hash(self.as_str(), 0, 0)
    }
}

/// 基于源码位置生成稳定编译期控件标识符的宏。
///
/// 若未传入区分符，则基于当前源码文件、行号与列号生成唯一的匿名容器键；
/// 若传入了区分符（如字符串字面量标识或基于索引的数值），则直接转换为确定性的 64 位键，
/// 确保声明处（view）与事件比对处（reduce）能够稳定匹配同一控件标识。
#[macro_export]
macro_rules! ui_key {
    () => {{
        const KEY: u64 = $crate::const_fnv1a_hash(file!(), line!(), column!());
        KEY
    }};
    ($discriminator:expr) => {{
        $crate::KeyDiscriminator::into_u64($discriminator)
    }};
}

/// [`unbound_widgets::UiWidget`] 的类型擦除 trait 对象适配器。
pub trait ErasedUiWidget: Send + Sync + core::fmt::Debug {
    /// 使用真实文本整形计算控件的固有尺寸。
    fn intrinsic_size_erased(&self, text_ctx: &mut unbound_widgets::TextCtx<'_>) -> Vec2;
    /// 将输入事件直接 reduce 到局部控件状态。
    fn reduce_erased(
        &self,
        key: u64,
        rect: Rect,
        event: &unbound_widgets::WidgetInputEvent,
        store: &mut WidgetStateStore,
    ) -> bool;
    /// 确保控件状态已在状态池中挂载/初始化。
    fn ensure_state_erased(&self, key: u64, store: &mut WidgetStateStore);
    /// 使用已解析的布局矩形与类型化状态发出图元。
    fn emit_laid_out_erased(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        store: &WidgetStateStore,
        text_ctx: &mut unbound_widgets::TextCtx<'_>,
    ) -> WidgetOutput;
}

impl<W: unbound_widgets::UiWidget> ErasedUiWidget for W {
    fn intrinsic_size_erased(&self, text_ctx: &mut unbound_widgets::TextCtx<'_>) -> Vec2 {
        self.intrinsic_size(text_ctx)
    }

    fn ensure_state_erased(&self, key: u64, store: &mut WidgetStateStore) {
        // 命中既有状态时也必须把本帧标记为「已访问」，否则状态池的帧末回收会把仍在渲染的
        // 控件误判为已卸载并丢弃其状态（表现为隔帧重置）。此处统一走 `use_state_or_init`。
        let state = store.use_state_or_init::<W::State>(key, || self.initial_state());
        self.sync_state(state);
    }

    fn reduce_erased(
        &self,
        key: u64,
        rect: Rect,
        event: &unbound_widgets::WidgetInputEvent,
        store: &mut WidgetStateStore,
    ) -> bool {
        let state = store.use_state_or_init::<W::State>(key, || self.initial_state());
        self.reduce(state, rect, event)
    }

    fn emit_laid_out_erased(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        store: &WidgetStateStore,
        text_ctx: &mut unbound_widgets::TextCtx<'_>,
    ) -> WidgetOutput {
        if let Some(state) = store.get::<W::State>(node) {
            self.emit_laid_out(node, rect, control, state, text_ctx)
        } else {
            let default_state = self.initial_state();
            self.emit_laid_out(node, rect, control, &default_state, text_ctx)
        }
    }
}

/// 语义根节点标识。
///
/// 这是**保留键**：控件键空间不得占用它。语义根本身占据语义树里的这个键，
/// 若某个控件恰好用同一个键，层级重建时语义根会被挂到它自己的子节点列表里，
/// 任何树遍历都会陷入死循环。运行时会把撞车的控件改派到派生键上。
pub const ROOT: u64 = 1;

/// 应用声明一次归约（reduction）所失效的范围。
///
/// 运行时据此跳过不必要的重建并告知窗口是否需要新的无障碍快照；
/// 渲染器本身始终只看到 [`SceneDiff`]。
///
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DirtyFlags {
    bits: u8,
}

impl DirtyFlags {
    /// 无任何变更。
    pub const NONE: Self = Self { bits: 0 };
    /// 仅像素内容变更。
    pub const PAINT: Self = Self { bits: 1 };
    /// 几何变更；必须重建。
    pub const LAYOUT: Self = Self { bits: 1 << 1 };
    /// 无障碍树变更。
    pub const A11Y: Self = Self { bits: 1 << 2 };
    /// 文本 / IME 布局变更。
    pub const TEXT: Self = Self { bits: 1 << 3 };
    /// 全部失效，包括新的场景根节点。
    pub const ALL: Self = Self { bits: 0b1111 };

    /// 原始位标志。
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    /// 从原始位标志构建，丢弃未知位。
    #[must_use]
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self {
            bits: bits & Self::ALL.bits,
        }
    }

    /// 是否已设置 `other` 的所有位。
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    /// 合并两个结果。
    #[must_use]
    pub const fn merge(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// 是否需要重建。
    #[must_use]
    pub const fn needs_rebuild(self) -> bool {
        self.contains(Self::LAYOUT) || self.contains(Self::ALL)
    }

    /// 是否必须重新发布无障碍树。
    #[must_use]
    pub const fn needs_a11y(self) -> bool {
        self.contains(Self::A11Y) || self.needs_rebuild()
    }
}

impl core::ops::BitOr for DirtyFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.merge(rhs)
    }
}

impl core::ops::BitOrAssign for DirtyFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.merge(rhs);
    }
}

/// 应用词汇表中的键盘按键。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AppKey {
    Tab,
    Escape,
    Enter,
    Space,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    ContextMenu,
    CapsLock,
    /// `F1`..`F24` 功能键。
    F(u8),
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    /// 其他按键，以其字符标识。
    Character(char),
}

impl AppKey {
    /// [`KeyChord`] 绑定使用的稳定名称。
    #[must_use]
    pub const fn chord_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Tab => "Tab",
            Self::Escape => "Escape",
            Self::Enter => "Enter",
            Self::Space => "Space",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::Insert => "Insert",
            Self::Left => "ArrowLeft",
            Self::Right => "ArrowRight",
            Self::Up => "ArrowUp",
            Self::Down => "ArrowDown",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::ContextMenu => "ContextMenu",
            Self::CapsLock => "CapsLock",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::F(_) | Self::Character(_) => return None,
        })
    }

    /// 该按键与 `modifiers` 构成的组合键。
    ///
    /// 字符会折叠为小写：按住 Shift 时键盘上报 `P`，
    /// 但写作 `Ctrl+Shift+P` 的绑定必须能匹配它。
    #[must_use]
    pub fn chord(self, modifiers: Modifiers) -> KeyChord {
        let key = match self {
            Self::Character(character) => KeyName::Char(character.to_ascii_lowercase()),
            Self::F(number) => KeyName::Function(number),
            other => KeyName::Named(other.chord_name().unwrap_or("Unidentified")),
        };
        KeyChord {
            key,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.meta,
        }
    }
}

/// 当前按下的修饰键。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// IME 组合输入更新。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeUpdate {
    /// 正在组合的文本；组合结束时为空。
    pub preedit: String,
    /// 预编辑文本内当前光标 / 选区字节范围。
    pub cursor_range: Option<(usize, usize)>,
    /// 若本帧完成组合输入，则为已提交的字符串。
    pub commit: Option<String>,
}

/// **已解析**的应用事件。
///
/// 指针目标已根据上一帧的命中区域拾取完毕；应用无需关心控件位于何处。
///
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// 指针移动；`hovered` 为其下方的节点。
    PointerMove { pos: Vec2, hovered: Option<u64> },
    /// 鼠标按键在 `target` 上按下。
    PointerDown {
        pos: Vec2,
        target: Option<u64>,
        button: PointerButton,
        click_count: u8,
    },
    /// 鼠标按键在 `target` 上释放。
    PointerUp {
        pos: Vec2,
        target: Option<u64>,
        button: PointerButton,
    },
    /// 完成一次单击。
    Click { node: u64, pos: Vec2, count: u8 },
    /// 完成一次双击。
    DoubleClick { node: u64, pos: Vec2 },
    /// 在 `node` 上开始拖拽。
    DragStart { node: u64, pos: Vec2 },
    /// 拖拽移动了 `delta`。
    Drag { node: u64, pos: Vec2, delta: Vec2 },
    /// 滚轮在 `target` 上滚动。
    Wheel {
        pos: Vec2,
        delta: Vec2,
        target: Option<u64>,
    },
    /// 在 `node` 上完成一次右键/辅助点击。
    ContextMenu { node: u64, pos: Vec2 },
    /// 键盘快捷键匹配了 [`Keymap`] 绑定。
    Shortcut { action: u64 },
    /// 按键被按下或释放。
    Key {
        key: AppKey,
        pressed: bool,
        modifiers: Modifiers,
    },
    /// 已键入文本（已经过 IME 提交）。
    Text { text: String },
    /// IME 组合输入状态变更。
    Ime { update: ImeUpdate },
    /// 窗口尺寸变更。
    Resize { size: Vec2 },
    /// 主题变更。
    Theme { kind: ThemeKind },
    /// 辅助技术请求执行动作。
    A11y {
        target: u64,
        action: A11yAction,
        value: Option<String>,
    },
    /// 帧时钟 tick。
    Tick { dt: f32 },
    /// 通过 [`AppContext::set_timeout`] 调度的定时器触发。
    Timer { token: TimerToken },
    /// 系统文件正在窗口上方悬停。
    FileHover(Vec<std::path::PathBuf>),
    /// 系统文件已拖放至窗口。
    FileDrop(Vec<std::path::PathBuf>),
    /// 系统文件拖拽离开窗口且未放下。
    FileHoverCancelled,
    /// 全局 UI 逻辑缩放比变更（对齐 `CANONICAL_SCALE_STEPS` 七档缩放）。
    UiScaleChanged { scale: f32 },
    /// 界面语言区域变更（默认 `"en-US"`，支持 `"zh-CN"` 等）。
    LocaleChanged { locale: String },
    /// 宿主后台线程或 Tokio 异步任务投递的自定义事件。
    Custom { id: u64, payload: String },
}

/// 跨线程事件投递句柄：供 Tokio 异步运行时、IPC 监听线程或后台作业随时向 UI 线程推送 [`AppEvent`] 并立即唤醒消息循环。
#[derive(Clone)]
pub struct AppEventSender {
    sender: Sender<AppEvent>,
    waker: Option<Arc<dyn AppWaker>>,
}

impl core::fmt::Debug for AppEventSender {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppEventSender").finish_non_exhaustive()
    }
}

impl AppEventSender {
    /// 发送任意 [`AppEvent`] 并唤醒窗口事件循环。
    pub fn send(&self, event: AppEvent) -> bool {
        let ok = self.sender.send(event).is_ok();
        if ok {
            if let Some(waker) = &self.waker {
                waker.wake();
            }
        }
        ok
    }

    /// 发送带数字标识与负载字符串的 [`AppEvent::Custom`] 事件。
    pub fn send_custom(&self, id: u64, payload: impl Into<String>) -> bool {
        self.send(AppEvent::Custom {
            id,
            payload: payload.into(),
        })
    }
}

/// 面向 `AppContext` 的平台无关剪贴板接口。
pub trait ClipboardProvider {
    /// 读取当前剪贴板文本（如有）。
    fn get_text(&self) -> Option<String>;
    /// 用 `text` 替换剪贴板内容。
    fn set_text(&mut self, text: &str);
}

/// 用于无头测试及回退场景的内存剪贴板。
#[derive(Clone, Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl MemoryClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardProvider for MemoryClipboard {
    fn get_text(&self) -> Option<String> {
        self.text.clone()
    }

    fn set_text(&mut self, text: &str) {
        self.text = Some(text.to_owned());
    }
}

/// 运行时在 `reduce` 期间提供的读写世界视图。
#[derive(Clone)]
pub struct AppContext {
    /// 当前视口尺寸。
    pub viewport: Vec2,
    /// 当前激活主题。
    pub theme: ThemeKind,
    /// 是否跟随操作系统明暗设置。
    ///
    /// `false` 时窗口运行器不再把 `ThemeChanged` 事件套用到应用上——应用自己管主题，
    /// 系统换肤不应该偷偷覆盖用户的选择（启动器的「主题跟随系统」开关就依赖这个位）。
    pub follow_system_theme: bool,
    /// 当前宿主产品主题风格预设（如 `ProductPreset::RenderDebuggerLauncher`）。
    pub preset: unbound_theme::ProductPreset,
    /// 当前 OKLCH 品牌强调色预设（共 10 套）。
    pub accent: unbound_theme::AccentBrand,
    /// 当前原生窗口是否处于最大化状态（由窗口运行器自动同步）。
    pub window_maximized: bool,
    /// 当前原生窗口是否处于激活聚焦状态（由窗口运行器自动同步）。
    pub window_focused: bool,
    /// 操作系统物理显示 DPI 缩放比（第 1 层缩放，默认 `1.0`）。
    pub os_scale: f32,
    /// 全局 7 档 UI 逻辑缩放因子（第 2 层缩放，默认 `1.0`，范围 `0.75..=2.0`）。
    pub ui_scale: f32,
    /// 当前渲染表面物理像素尺寸基准（用于缩放档位或 DPI 变更时同步推导逻辑 `viewport`）。
    physical_viewport: Vec2,
    /// 缩放 OSD 浮层剩余显示秒数（`> 0.0` 时在右上角渲染当前缩放百分比胶囊）。
    pub scale_osd_remaining: f32,
    /// 应用内建的国际化资源包与当前语言区域（默认 `en-US`）。
    pub i18n: Rc<RefCell<I18nBundle>>,
    /// 东亚文字显式绑定的系统字体族（`None` 表示按文本内容自动判定）。
    ///
    /// 界面语言切换时必须同步：简中 / 繁中 / 日文 / 韩文的汉字字形各不相同，
    /// 用错字体族会让繁体界面出现简体字、日文界面出现简体字形。
    pub cjk_family: Option<&'static str>,
    /// 当前聚焦节点，与焦点管理器保持同步。
    pub focused: Option<u64>,
    /// 是否应绘制焦点环。
    pub focus_visible: bool,
    /// 指针下方的节点。
    pub hovered: Option<u64>,
    /// 当前按下的节点。
    pub pressed: Option<u64>,
    /// 请求的光标样式。
    pub cursor: CursorKind,
    /// 上一次指针位置。
    pub pointer: Vec2,
    /// 自运行时启动以来的秒数。
    pub time: f64,
    /// 请求运行时在本次归约后聚焦某个节点。
    pub request_focus: Option<(u64, FocusCause)>,
    /// 请求运行时在本次归约后失焦。
    pub request_blur: bool,
    /// 局部状态池，与渲染它的视图共享。
    widget_state: Rc<RefCell<WidgetStateStore>>,
    /// 本次归约期间请求的定时器，由运行时排空。
    timer_requests: Vec<TimerRequest>,
    /// 本次归约期间取消的定时器，由运行时排空。
    timer_cancels: Vec<TimerToken>,
    /// 与运行时队列共享，确保两侧 token 唯一。
    timer_tokens: Rc<Cell<u64>>,
    /// 后台任务投递结果的目标通道。
    async_sender: Option<Sender<AppEvent>>,
    /// 后台任务投递结果时唤醒休眠的事件循环。
    waker: Option<Arc<dyn AppWaker>>,
    /// 上一个处理器消费了多少当前滚轮增量。
    scroll_consumed: Vec2,
    /// 冒泡派发过程中当前正在接收事件的逻辑节点键。
    event_target: Option<u64>,
    /// 共享剪贴板提供者（窗口模式为 SystemClipboard，无头模式为 MemoryClipboard）。
    clipboard: Rc<RefCell<Box<dyn ClipboardProvider>>>,
}

impl core::fmt::Debug for AppContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppContext")
            .field("viewport", &self.viewport)
            .field("theme", &self.theme)
            .field("focused", &self.focused)
            .field("focus_visible", &self.focus_visible)
            .field("hovered", &self.hovered)
            .field("pressed", &self.pressed)
            .field("cursor", &self.cursor)
            .field("pointer", &self.pointer)
            .field("time", &self.time)
            .field("widget_state", &self.widget_state)
            .finish_non_exhaustive()
    }
}

impl AppContext {
    fn new(viewport: Vec2) -> Self {
        Self {
            viewport,
            theme: ThemeKind::Dark,
            follow_system_theme: true,
            preset: unbound_theme::ProductPreset::Win11,
            accent: unbound_theme::AccentBrand::Blue,
            window_maximized: false,
            window_focused: true,
            os_scale: 1.0,
            ui_scale: 1.0,
            physical_viewport: viewport,
            scale_osd_remaining: 0.0,
            i18n: Rc::new(RefCell::new(I18nBundle::with_locale(Locale::EnUs))),
            cjk_family: None,
            focused: None,
            focus_visible: false,
            hovered: None,
            pressed: None,
            cursor: CursorKind::Pointer,
            pointer: Vec2::ZERO,
            time: 0.0,
            request_focus: None,
            request_blur: false,
            widget_state: Rc::new(RefCell::new(WidgetStateStore::new())),
            timer_requests: Vec::new(),
            timer_cancels: Vec::new(),
            timer_tokens: Rc::new(Cell::new(0)),
            async_sender: None,
            waker: None,
            scroll_consumed: Vec2::ZERO,
            event_target: None,
            clipboard: Rc::new(RefCell::new(Box::new(MemoryClipboard::new()))),
        }
    }

    /// 替换剪贴板提供者（例如替换为 `SystemClipboard`）。
    pub fn set_clipboard(&mut self, provider: Box<dyn ClipboardProvider>) {
        *self.clipboard.borrow_mut() = provider;
    }

    /// 从当前激活的剪贴板提供者读取文本。
    #[must_use]
    pub fn clipboard_get(&self) -> Option<String> {
        self.clipboard.borrow().get_text()
    }

    /// 向当前激活的剪贴板提供者写入文本。
    pub fn clipboard_set(&self, text: &str) {
        self.clipboard.borrow_mut().set_text(text);
    }

    /// 节点键对应的局部状态。
    ///
    /// 与视图层的 [`ViewBuilder::use_state`] 不同，此方法不会将键标记为已渲染：
    /// 事件处理器可读写控件状态而不必让已移除的控件继续存活。
    ///
    pub fn use_state<T: Default + Send + Sync + 'static>(&self, key: u64) -> RefMut<'_, T> {
        RefMut::map(self.widget_state.borrow_mut(), |store| store.entry(key))
    }

    /// 不可变借用控件状态存储。
    #[must_use]
    pub fn state_store(&self) -> core::cell::Ref<'_, WidgetStateStore> {
        self.widget_state.borrow()
    }

    /// 可变借用控件状态存储。
    pub fn state_store_mut(&self) -> RefMut<'_, WidgetStateStore> {
        self.widget_state.borrow_mut()
    }

    /// 获取不可变借用的国际化资源包。
    #[must_use]
    pub fn i18n(&self) -> core::cell::Ref<'_, I18nBundle> {
        self.i18n.borrow()
    }

    /// 获取可变借用的国际化资源包（用于注册多语言词条或自定义回退）。
    pub fn i18n_mut(&mut self) -> RefMut<'_, I18nBundle> {
        self.i18n.borrow_mut()
    }

    /// 获取当前激活的标准语言区域枚举。
    #[must_use]
    pub fn locale(&self) -> Locale {
        self.i18n.borrow().active_locale_enum()
    }

    /// 切换当前激活的语言区域（支持传入 `"en-US"` / `"zh-CN"` 等标签）。
    pub fn set_locale(&mut self, locale: impl Into<String>) {
        self.i18n.borrow_mut().set_locale(locale);
    }

    /// 指定东亚文字使用的系统字体族。
    ///
    /// 传 `None` 表示按文本内容自动判定（假名 → 日文字体、谚文 → 韩文字体、
    /// 其余 → 中文字体）。给定语言标签更稳妥：繁体中文与简体中文的汉字字形
    /// 并不相同，仅凭字符无法区分。
    pub fn set_cjk_family(&mut self, family: Option<&'static str>) {
        self.cjk_family = family;
    }

    /// 按界面语言标签自动选定东亚字体族。
    pub fn set_cjk_family_for_locale(&mut self, tag: &str) {
        self.cjk_family = unbound_text::cjk_family_for_locale(tag);
    }

    /// 通过强类型 [`Locale`] 枚举切换当前激活的语言区域。
    pub fn set_locale_enum(&mut self, locale: Locale) {
        self.i18n.borrow_mut().set_locale_enum(locale);
    }

    /// 按键翻译文本，若未命中则回退至 fallback 语言或原键名。
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        self.i18n.borrow().t(key).to_owned()
    }

    /// 按键与参数插值格式化本地化文本。
    #[must_use]
    pub fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n.borrow().format(key, args)
    }

    /// 设置产品主题风格与 10 套 OKLCH 品牌强调色。
    pub fn set_theme_preset(
        &mut self,
        preset: unbound_theme::ProductPreset,
        accent: unbound_theme::AccentBrand,
    ) {
        self.preset = preset;
        self.accent = accent;
    }

    /// 切换 OKLCH 品牌强调色。
    pub fn set_accent_brand(&mut self, accent: unbound_theme::AccentBrand) {
        self.accent = accent;
    }

    /// 读取指定 `TextInputDesc` 控件键当前的文本内容。
    #[must_use]
    pub fn text_input_value(&self, key: u64) -> Option<String> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::TextEditState>(key)
            .map(|st| st.text().to_owned())
    }

    /// 写入指定 `TextInputDesc` 控件键的文本内容。
    pub fn set_text_input_value(&self, key: u64, text: impl Into<String>) {
        self.widget_state
            .borrow_mut()
            .insert_state(key, unbound_widgets::TextEditState::with_text(text));
    }

    /// 读取指定 `SelectDesc` 控件键当前选中的索引。
    #[must_use]
    pub fn select_index(&self, key: u64) -> Option<usize> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::SelectState>(key)
            .and_then(|st| st.selected)
    }

    /// 设置指定 `SelectDesc` 控件键的选中索引。
    pub fn set_select_index(&self, key: u64, selected: Option<usize>) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::SelectState>(key);
        st.selected = selected;
        st.highlighted = selected;
    }

    /// 读取指定 `SwitchDesc` 控件键当前的布尔开关状态。
    #[must_use]
    pub fn switch_value(&self, key: u64) -> Option<bool> {
        self.widget_state.borrow().get::<bool>(key).copied()
    }

    /// 设置指定 `SwitchDesc` 控件键的布尔开关状态。
    pub fn set_switch_value(&self, key: u64, on: bool) {
        self.widget_state.borrow_mut().insert_state(key, on);
    }

    /// 读取指定 `SliderDesc` 控件键当前的浮点值。
    #[must_use]
    pub fn slider_value(&self, key: u64) -> Option<f32> {
        self.widget_state.borrow().get::<f32>(key).copied()
    }

    /// 设置指定 `SliderDesc` 控件键的浮点值。
    pub fn set_slider_value(&self, key: u64, value: f32) {
        self.widget_state.borrow_mut().insert_state(key, value);
    }

    /// 读取指定 `NumberInputDesc` 控件键当前的数值。
    #[must_use]
    pub fn number_input_value(&self, key: u64) -> Option<f64> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::NumberInputState>(key)
            .map(unbound_widgets::NumberInputState::value)
    }

    /// 设置指定 `NumberInputDesc` 控件键的数值。
    pub fn set_number_input_value(&self, key: u64, value: f64) {
        self.widget_state
            .borrow_mut()
            .insert_state(key, unbound_widgets::NumberInputState::new(value));
    }

    /// 读取指定 `TabsDesc` 控件键当前激活的标签索引。
    #[must_use]
    pub fn tab_active(&self, key: u64) -> Option<usize> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::TabsState>(key)
            .map(|st| st.active)
    }

    /// 设置指定 `TabsDesc` 控件键当前激活的标签索引。
    pub fn set_tab_active(&self, key: u64, active: usize) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::TabsState>(key);
        st.active = active;
    }

    /// 读取指定 `HotkeyInputDesc` 控件键已录制的快捷键字符串。
    #[must_use]
    pub fn hotkey_input_text(&self, key: u64) -> Option<String> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::hotkey_input::HotkeyInputState>(key)
            .and_then(unbound_widgets::hotkey_input::HotkeyInputState::formatted_chord)
    }

    /// 重置或清空指定 `HotkeyInputDesc` 控件键的录制状态。
    pub fn clear_hotkey_input(&self, key: u64, start_recording: bool) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::hotkey_input::HotkeyInputState>(key);
        st.clear();
        st.recording = start_recording;
    }

    /// 读取指定 `AutoCompleteDesc` 控件键当前的查询文本。
    #[must_use]
    pub fn autocomplete_value(&self, key: u64) -> Option<String> {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::autocomplete::AutoCompleteState>(key)
            .map(|st| st.query.clone())
    }

    /// 设置指定 `AutoCompleteDesc` 控件键的查询文本。
    pub fn set_autocomplete_value(&self, key: u64, value: impl Into<String>) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::autocomplete::AutoCompleteState>(key);
        st.query = value.into();
    }

    /// 在本次归约后调度一个经 `after` 触发的单次事件。
    ///
    /// 先分配 token 并传给 `make`，使事件能携带自身将被派发时的 token；
    /// 事件归约完成后，运行时将其正式入队。
    ///
    pub fn set_timeout(
        &mut self,
        after: Duration,
        make: impl FnOnce(TimerToken) -> AppEvent,
    ) -> TimerToken {
        let token = self.allocate_timer();
        self.timer_requests.push(TimerRequest {
            token,
            delay_ms: millis(after),
            period_ms: None,
            event: make(token),
        });
        token
    }

    /// 调度每隔 `period` 触发一次的重复事件。
    pub fn set_interval(
        &mut self,
        period: Duration,
        make: impl FnOnce(TimerToken) -> AppEvent,
    ) -> TimerToken {
        let token = self.allocate_timer();
        self.timer_requests.push(TimerRequest {
            token,
            delay_ms: millis(period),
            period_ms: Some(millis(period)),
            event: make(token),
        });
        token
    }

    /// 取消先前调度的定时器。
    pub fn cancel_timer(&mut self, token: TimerToken) {
        self.timer_cancels.push(token);
    }

    /// 报告当前处理器消费了多少滚轮增量。
    ///
    /// 运行时从总增量中减去该值，并将剩余量提供给指针下方的下一个区域，
    /// 嵌套滚动区域正是借此将多余滚轮事件传递给外层滚动容器。
    ///
    pub fn consume_scroll(&mut self, delta: Vec2) {
        self.scroll_consumed += delta;
    }

    /// 冒泡派发过程中当前正在处理的节点键（例如在 `AppEvent::Key` 冒泡时区分命中祖先）。
    #[must_use]
    pub const fn event_target(&self) -> Option<u64> {
        self.event_target
    }

    /// 在工作线程上运行 `task` 并归约其返回的事件。
    ///
    /// 结果将在下一次 [`AppRuntime::poll_async`]（每帧开头自动调用）中投递；
    /// 在此期间 [`AppWaker`] 会唤醒休眠的事件循环。
    ///
    /// 获取可跨线程克隆的事件发送器（供外部 Tokio 任务或 `LauncherEventHub` 桥接使用）。
    #[must_use]
    pub fn event_sender(&self) -> Option<AppEventSender> {
        self.async_sender.as_ref().map(|sender| AppEventSender {
            sender: sender.clone(),
            waker: self.waker.clone(),
        })
    }

    /// 当前生效的物理/逻辑总缩放比例（`os_scale * ui_scale`，钳制在 `[0.5, 4.0]`）。
    ///
    /// 对齐 `unbound_theme::ZoomLayers::ui_scale` 与 `Showcase::effective_scale`：
    /// 布局在逻辑单位（`physical_viewport / effective_scale`）下求解，
    /// 字形光栅化与 GPU 相机再按 `effective_scale` 映射回物理像素。
    #[must_use]
    pub fn effective_scale(&self) -> f32 {
        let os = if self.os_scale.is_finite() && self.os_scale > 0.0 {
            self.os_scale
        } else {
            1.0
        };
        let ui = if self.ui_scale.is_finite() && self.ui_scale > 0.0 {
            self.ui_scale
        } else {
            1.0
        };
        (os * ui).clamp(0.5, 4.0)
    }

    /// 当前渲染表面的物理像素尺寸基准。
    #[must_use]
    pub const fn physical_viewport(&self) -> Vec2 {
        self.physical_viewport
    }

    /// 根据当前物理像素视口与有效缩放比（`os_scale * ui_scale`）立即重算逻辑视口 `self.viewport`。
    pub fn sync_viewport_from_physical(&mut self) -> Vec2 {
        let scale = self.effective_scale();
        if self.physical_viewport.x > 0.0 && self.physical_viewport.y > 0.0 {
            self.viewport = Vec2::new(
                (self.physical_viewport.x / scale).max(1.0),
                (self.physical_viewport.y / scale).max(1.0),
            );
        }
        self.viewport
    }

    /// 更新操作系统物理显示 DPI 缩放比（`os_scale`）并同步重算逻辑视口。
    pub fn set_os_scale(&mut self, os_scale: f32) -> f32 {
        let valid = if os_scale.is_finite() && os_scale > 0.0 {
            os_scale.clamp(0.5, 4.0)
        } else {
            1.0
        };
        self.os_scale = valid;
        self.sync_viewport_from_physical();
        valid
    }

    /// 设置全局 UI 缩放比例（自动吸附到 `CANONICAL_SCALE_STEPS`、同步逻辑视口并激活 1.5 秒缩放 OSD 提示）。
    pub fn set_ui_scale(&mut self, scale: f32) -> f32 {
        let snapped = unbound_theme::presets::snap_ui_scale(scale);
        self.ui_scale = snapped;
        self.scale_osd_remaining = 1.5;
        self.sync_viewport_from_physical();
        snapped
    }

    /// 按档位步进全局 UI 缩放比例（`delta_steps = 1` 放大一档，`-1` 缩小一档，同步重算逻辑视口）。
    pub fn step_ui_scale(&mut self, delta_steps: i32) -> f32 {
        let next = unbound_theme::presets::step_ui_scale(self.ui_scale, delta_steps);
        self.ui_scale = next;
        self.scale_osd_remaining = 1.5;
        self.sync_viewport_from_physical();
        next
    }

    pub fn spawn(&self, task: impl FnOnce() -> AppEvent + Send + 'static) {
        let Some(sender) = self.async_sender.clone() else {
            return;
        };
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let event = task();
            if sender.send(event).is_ok() {
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
    }

    fn allocate_timer(&self) -> TimerToken {
        let id = self.timer_tokens.get().saturating_add(1);
        self.timer_tokens.set(id);
        TimerToken::new(id)
    }

    /// 为运行时排空定时器请求。
    fn take_timer_requests(&mut self) -> (Vec<TimerRequest>, Vec<TimerToken>) {
        (
            core::mem::take(&mut self.timer_requests),
            core::mem::take(&mut self.timer_cancels),
        )
    }

    /// 共享运行时的定时器 token 计数器。
    fn share_timer_tokens(&mut self, tokens: Rc<Cell<u64>>) {
        self.timer_tokens = tokens;
    }

    /// 与视图共享运行时的局部状态池。
    fn share_state(&mut self, state: Rc<RefCell<WidgetStateStore>>) {
        self.widget_state = state;
    }

    /// 安装 [`Self::spawn`] 使用的通道与唤醒器。
    fn install_async(&mut self, sender: Sender<AppEvent>, waker: Option<Arc<dyn AppWaker>>) {
        self.async_sender = Some(sender);
        self.waker = waker;
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `view` 构建场景期间可用的不可变上下文。
#[derive(Copy, Clone, Debug)]
pub struct ViewContext {
    /// 视口尺寸。
    pub viewport: Vec2,
    /// 当前激活主题。
    pub theme: ThemeKind,
    /// 指针下方的节点。
    pub hovered: Option<u64>,
    /// 当前按下的节点。
    pub pressed: Option<u64>,
    /// 当前聚焦的节点。
    pub focused: Option<u64>,
    /// 是否应绘制焦点环。
    pub focus_visible: bool,
    /// host 是否请求了完整重建。
    ///
    /// 由 [`AppRuntime::invalidate`] 设置：通常在无变更时跳过重建的应用
    /// 必须遵守此标志并执行重建。
    pub force: bool,
    /// 窗口当前是否处于最大化状态（决定标题栏最大化/还原按钮图标）。
    pub window_maximized: bool,
    /// 窗口当前是否处于激活聚焦状态（决定标题栏失焦降对比度视觉）。
    pub window_focused: bool,
    /// 当前激活的标准语言区域。
    pub locale: Locale,
    /// 当前激活区域的阅读布局方向（LTR / RTL）。
    pub direction: LayoutDirection,
    /// 当前操作系统 DPI 缩放比例。
    pub os_scale: f32,
    /// 当前全局 UI 缩放比例（`0.75..=2.0`）。
    pub ui_scale: f32,
    /// 当前生效的物理/逻辑总缩放比例（`os_scale * ui_scale`）。
    pub effective_scale: f32,
    /// 缩放 OSD 悬浮提示剩余可见时间（秒）。
    pub scale_osd_remaining: f32,
}

impl ViewContext {
    /// 基于悬停 / 按下 / 聚焦解析出的键控控件状态。
    #[must_use]
    pub fn control_state(&self, key: u64, disabled: bool, selected: bool) -> ControlState {
        let phase = if disabled {
            unbound_widgets::PointerPhase::Idle
        } else if self.pressed == Some(key) {
            unbound_widgets::PointerPhase::Pressed
        } else if self.hovered == Some(key) {
            unbound_widgets::PointerPhase::Hovered
        } else {
            unbound_widgets::PointerPhase::Idle
        };
        ControlState {
            phase,
            focused: self.focused == Some(key) && self.focus_visible,
            disabled,
            selected,
        }
    }
}

/// 声明式视图放置在布局树中的控件。
///
/// 控件在第二遍发射，待布局引擎解析出其矩形后再绘制；
/// 控件本身从不猜测自身几何。
#[derive(Clone, Debug)]
pub enum WidgetSpec {
    /// 实现自动固有尺寸测量与发射的声明式 [`UiWidget`]。
    UiWidget(Arc<dyn ErasedUiWidget>),
    /// 扁平按钮。其描述符矩形会被布局后的矩形替换。
    Button(ButtonDesc),
    /// 由文本引擎测量的自动换行段落。
    Text {
        /// 段落文本内容。
        text: String,
        /// 文本整形样式。
        style: TextStyle,
    },
    /// 单行自适应宽度的行内文本标签（带语义颜色令牌）。
    TextInline {
        /// 文本内容。
        text: String,
        /// 文本整形样式。
        style: TextStyle,
        /// 文本颜色令牌。
        color: unbound_theme::Token,
    },
    /// 容器或卡片表面背景、边框与阴影图元。
    BoxSurface {
        fill: u16,
        border: u16,
        border_width: f32,
        radii: [f32; 4],
        shadow_blur: f32,
        z: u16,
        hit_scrim: bool,
    },
    /// 开启滚动裁剪与平移作用域。
    ScrollScopeBegin {
        scroll_key: u64,
        content_node: NodeId,
        vertical: bool,
    },
    /// 关闭滚动裁剪与平移作用域。
    ScrollScopeEnd,
    /// 复选框控件。
    Checkbox(CheckboxDesc),
    /// 切换开关。
    Switch(SwitchDesc),
    /// 连续滑动条。
    Slider(SliderDesc),
    /// 进度条或环形进度指示器。
    Progress {
        /// 描述符（矩形由布局结果替换）。
        desc: ProgressDesc,
        /// 动画状态。
        state: ProgressState,
    },
    /// 胶囊徽章。
    Badge(BadgeDesc),
    /// 细分割线。
    Divider(DividerDesc),
    /// 状态提示横幅。
    Alert(AlertDesc),
}

/// 单个延迟控件及其由布局引擎求解的节点。
#[derive(Clone, Debug)]
pub struct PendingWidget {
    /// 用于输入与无障碍的控件标识。
    pub key: u64,
    /// 拥有该控件几何信息的布局节点。
    pub node: NodeId,
    /// 第二遍要发射的内容。
    pub spec: WidgetSpec,
    /// 浮层/模态框的基准绘制 Z 偏移（>= `PORTAL_Z_BASE` 时命中区域抬升至 Portal 层）。
    /// 绘制深度不决定事件边界，边界由 `NodeFlags::PORTAL_ROOT` 显式声明。
    pub z_base: u16,
}

/// 标题栏插槽的构建闭包：接收插槽容器并在其中声明自定义控件。
pub type TitleBarSlot<'a> = Box<dyn for<'b> FnOnce(&mut ViewBuilder<'b>) + 'a>;

/// 标题栏三段式自定义插槽（`left` / `center` / `actions`）。
#[derive(Default)]
pub struct TitleBarSlots<'a> {
    /// 标题栏左侧自定义控件区（如导航折叠按钮、应用图标、面包屑）。
    pub left: Option<TitleBarSlot<'a>>,
    /// 标题栏中间自定义控件区（如全局搜索框 / 命令面板入口）。
    pub center: Option<TitleBarSlot<'a>>,
    /// 标题栏右侧自定义操作区（位于窗口控制按钮左侧，如主题切换、缩放比、HUD 开关）。
    pub actions: Option<TitleBarSlot<'a>>,
}

impl<'a> TitleBarSlots<'a> {
    #[must_use]
    pub fn left(mut self, f: impl FnOnce(&mut ViewBuilder<'_>) + 'a) -> Self {
        self.left = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn center(mut self, f: impl FnOnce(&mut ViewBuilder<'_>) + 'a) -> Self {
        self.center = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn actions(mut self, f: impl FnOnce(&mut ViewBuilder<'_>) + 'a) -> Self {
        self.actions = Some(Box::new(f));
        self
    }
}

pub struct ViewBuilder<'a> {
    collector: &'a mut SceneCollector,
    /// 不可变帧上下文。
    pub ctx: ViewContext,
    world: &'a mut UiWorld,
    /// 运行时资产中心：应用可在此提交 PNG 字节并绘制已解码的位图。
    assets: &'a mut AssetHub,
    parent: NodeId,
    pending: &'a mut Vec<PendingWidget>,
    widget_state: Rc<RefCell<WidgetStateStore>>,
    i18n: Rc<RefCell<I18nBundle>>,
    node_keys: &'a mut FxHashMap<u64, NodeId>,
    seen_keys: &'a mut FxHashSet<u64>,
    ephemeral_nodes: &'a mut Vec<NodeId>,
    focus_groups: &'a mut Vec<FocusGroup>,
    title_bar_bindings: &'a mut FxHashMap<u64, TitleBarBinding>,
    /// 本帧声明的语义标注：控件键 → 标注意图。
    ///
    /// 意图在布局求解之后才被写成语义节点，因此控件的最终矩形已确定。
    semantic_intents: &'a mut FxHashMap<u64, SemanticIntent>,
    depth: u16,
    z_base: u16,
}

impl<'a> ViewBuilder<'a> {
    /// 获取当前激活的标准语言区域枚举。
    #[must_use]
    pub fn locale(&self) -> Locale {
        self.ctx.locale
    }

    /// 访问只读国际化资源包。
    #[must_use]
    pub fn i18n(&self) -> core::cell::Ref<'_, I18nBundle> {
        self.i18n.borrow()
    }

    /// 在声明式视图构建期按键翻译文本。
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        self.i18n.borrow().t(key).to_owned()
    }

    /// 在声明式视图构建期按键与命名参数插值翻译文本。
    #[must_use]
    pub fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n.borrow().format(key, args)
    }

    /// 在声明式视图构建期按数量单复数规则翻译文本。
    #[must_use]
    pub fn t_plural(&self, key: &str, count: usize) -> String {
        self.i18n.borrow().format_plural(key, count)
    }

    /// 按键返回控件在**上一帧布局**中的世界坐标矩形。
    ///
    /// 视图构建发生在布局求解之前，因此这里给出的是上一帧算出的矩形：
    /// 浮层（如下拉菜单弹层）用它做锚点时，弹层正好贴在用户看到的控件下方。
    /// 控件在当前帧首次挂载时还没有布局结果，返回 `None`。
    ///
    /// 与 [`AppRuntime::widget_rect`] 的区别：后者读的是本帧挂载表（含控件自身的
    /// 布局矩形），只能在帧外访问；本方法供视图构建期使用。
    #[must_use]
    pub fn widget_rect(&self, key: u64) -> Option<Rect> {
        self.node_keys
            .get(&key)
            .and_then(|node| self.world.get(*node))
            .map(|node| node.world_rect)
    }

    /// 为当前帧注册一个漫游 tabindex 焦点组。
    pub fn register_focus_group(&mut self, group: FocusGroup) {
        self.focus_groups.push(group);
    }

    /// 控件键对应的局部状态。
    ///
    /// 该键会被记录为已渲染，因此停止调用此方法的键将在帧末被卸载并回收其值。
    ///
    pub fn use_state<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> RefMut<'_, T> {
        RefMut::map(self.widget_state.borrow_mut(), |store| store.use_state(key))
    }

    /// 追加控件的输出。
    pub fn push(&mut self, key: u64, output: WidgetOutput) {
        self.collector.push(key, output);
    }

    /// 声明某个控件的语义。
    ///
    /// 这是**框架层通用**的标注通道：它只描述「这个控件在语义上是什么」，
    /// 不含任何业务知识——业务语义由应用通过本方法注入。
    ///
    /// 标注在布局求解**之后**生效，因此控件的最终矩形会一并写入语义节点。
    /// 自绘控件（不走原生控件 API）由此获得与原生控件同等的语义地位：
    /// 它们同样能被屏幕阅读器朗读、被 Agent 按角色与名称定位。
    ///
    /// 同一控件重复声明时，后一次覆盖前一次。
    pub fn declare_semantics(&mut self, key: u64, intent: SemanticIntent) {
        self.semantic_intents.insert(key, intent);
    }

    /// 声明某个控件的语义角色与名称。
    ///
    /// [`Self::declare_semantics`] 的简写形式。
    pub fn declare_semantic(&mut self, key: u64, role: Role, label: impl Into<String>) {
        self.declare_semantics(key, SemanticIntent::new(role, label));
    }

    /// 以固定图元预留空间追加控件输出，使动态字形序列不会移动后续控件的 span。
    ///
    pub fn push_with_capacity(&mut self, key: u64, output: WidgetOutput, reserved_slots: usize) {
        self.collector
            .push_with_capacity(key, output, reserved_slots);
    }

    /// 声明纵向堆叠容器。子节点按顺序布局。
    pub fn column(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        self.container(key, RawNodeDesc::container().column().gap(8.0), build)
    }

    /// 声明横向堆叠容器。
    pub fn row(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        self.container(key, RawNodeDesc::container().row().gap(8.0), build)
    }

    /// 声明无边框窗口的默认标题栏（包含标题文本、最小化、最大化/还原与关闭按钮）。
    pub fn title_bar(&mut self, title: impl Into<String>) -> NodeId {
        self.title_bar_with(unbound_widgets::TitleBarDesc::new(title))
    }

    /// 使用自定义配置声明无边框窗口标题栏。
    pub fn title_bar_with(&mut self, desc: unbound_widgets::TitleBarDesc) -> NodeId {
        self.title_bar_custom(desc, TitleBarSlots::default())
    }

    /// 声明支持三段式自定义插槽（`left` / `center` / `actions`）的无边框窗口标题栏。
    ///
    /// 右上角固定渲染与 Showcase 一致的贴边几何窗口控制按钮（最小化 / 最大化 / 关闭），
    /// 并自动同步 `AppContext::window_maximized` 状态；其余控件通过插槽注入，空白区域自动
    /// 回落为系统原生拖拽与双击最大化。
    pub fn title_bar_custom(
        &mut self,
        desc: unbound_widgets::TitleBarDesc,
        slots: TitleBarSlots<'_>,
    ) -> NodeId {
        let maximized = desc.maximized_override.unwrap_or(self.ctx.window_maximized);
        let focused = self.ctx.window_focused;
        let root_key = desc.id;
        self.title_bar_bindings.insert(
            root_key,
            TitleBarBinding {
                minimize_id: desc.minimize_id,
                maximize_id: desc.maximize_id,
                close_id: desc.close_id,
            },
        );
        let bar_desc = RawNodeDesc::container()
            .row()
            .width(Dim::percent(100.0))
            .height(Dim::px(desc.height))
            .align(unbound_abi::Align::Center)
            .gap(8.0);
        self.container(root_key, bar_desc, |bar| {
            // 背景绝对定位铺满整条标题栏，不随左右插槽的内边距偏移。
            let chrome_desc = RawNodeDesc::container()
                .absolute()
                .inset(unbound_abi::Edges::all(0.0))
                .width(Dim::percent(100.0))
                .height(Dim::px(desc.height));
            bar.add_with_layout(
                mix_key(root_key, 1),
                chrome_desc,
                unbound_widgets::TitleBarChrome::new(focused),
            );
            // 左区为可压缩的等权伸缩容器（`flex_grow = 1` + `min_width = 0`）：
            // 空间充足时它与右区等分留白，使 `center` 插槽保持视觉居中；
            // 空间不足时先压缩左侧留白，绝不挤走上方的窗口控制按钮。
            // 标题文本本身无法省略号截断（框架尚未提供 `TextOverflow`），
            // 因此打开裁剪，超长标题在左区边界被切断而不是压到搜索框与关闭按钮上。
            let mut left_desc = RawNodeDesc::container()
                .row()
                .align(unbound_abi::Align::Center)
                .padding(unbound_abi::Edges::trbl(12.0, 0.0, 0.0, 0.0))
                .gap(8.0)
                .flags(unbound_abi::NodeFlags::CLIP)
                .flex_grow(1.0);
            // 保留左侧最小安全宽度（容纳导航折叠按钮与品牌徽标），超出部分由 `CLIP` 裁剪。
            left_desc.min_width = Dim::px(68.0);
            bar.container(mix_key(root_key, 2), left_desc, |left| {
                if desc.show_title && !desc.title.is_empty() {
                    let mut style = TextStyle::sized(13.0);
                    style.weight = 600;
                    let tone = if focused {
                        unbound_theme::Token::TEXT_PRIMARY
                    } else {
                        unbound_theme::Token::TEXT_SECONDARY
                    };
                    left.text_inline(mix_key(root_key, 3), desc.title, style, tone);
                }
                if let Some(build_left) = slots.left {
                    build_left(left);
                }
            });
            if let Some(build_center) = slots.center {
                let mut center_desc = RawNodeDesc::container()
                    .row()
                    .align(unbound_abi::Align::Center)
                    .gap(8.0)
                    .flags(unbound_abi::NodeFlags::CLIP);
                center_desc.min_width = Dim::px(0.0);
                bar.container(mix_key(root_key, 4), center_desc, build_center);
            }
            let caption_count = u32::from(desc.show_minimize)
                + u32::from(desc.show_maximize)
                + u32::from(desc.show_close);
            let caption_min_w = caption_count as f32 * unbound_widgets::TITLE_BAR_CAPTION_WIDTH;
            let mut right_desc = RawNodeDesc::container()
                .row()
                .align(unbound_abi::Align::Center)
                .justify(unbound_abi::Justify::FlexEnd)
                .gap(8.0)
                .flex_grow(1.0);
            right_desc.min_width = Dim::px(caption_min_w);
            bar.container(mix_key(root_key, 5), right_desc, |right| {
                if let Some(build_actions) = slots.actions {
                    let mut actions_desc = RawNodeDesc::container()
                        .row()
                        .align(unbound_abi::Align::Center)
                        .gap(8.0)
                        .flags(unbound_abi::NodeFlags::CLIP);
                    actions_desc.min_width = Dim::px(0.0);
                    right.container(mix_key(root_key, 6), actions_desc, build_actions);
                }
                let mut caption_row_desc = RawNodeDesc::container()
                    .row()
                    .align(unbound_abi::Align::Center)
                    .gap(0.0);
                // 窗口控制按钮（最小化/最大化/关闭）严禁被 Flex 压缩或挤出视口右边界。
                caption_row_desc.flex_shrink = 0.0;
                right.container(mix_key(root_key, 7), caption_row_desc, |btns| {
                    if desc.show_minimize {
                        btns.add_with_layout(
                            desc.minimize_id,
                            RawNodeDesc::container()
                                .width(Dim::px(unbound_widgets::TITLE_BAR_CAPTION_WIDTH))
                                .height(Dim::px(desc.height)),
                            unbound_widgets::WindowCaptionButton::new(
                                unbound_widgets::WindowCaptionKind::Minimize,
                                desc.height,
                            )
                            .focused(focused),
                        );
                    }
                    if desc.show_maximize {
                        btns.add_with_layout(
                            desc.maximize_id,
                            RawNodeDesc::container()
                                .width(Dim::px(unbound_widgets::TITLE_BAR_CAPTION_WIDTH))
                                .height(Dim::px(desc.height)),
                            unbound_widgets::WindowCaptionButton::new(
                                unbound_widgets::WindowCaptionKind::Maximize { maximized },
                                desc.height,
                            )
                            .focused(focused),
                        );
                    }
                    if desc.show_close {
                        btns.add_with_layout(
                            desc.close_id,
                            RawNodeDesc::container()
                                .width(Dim::px(unbound_widgets::TITLE_BAR_CAPTION_WIDTH))
                                .height(Dim::px(desc.height)),
                            unbound_widgets::WindowCaptionButton::new(
                                unbound_widgets::WindowCaptionKind::Close,
                                desc.height,
                            )
                            .focused(focused),
                        );
                    }
                });
            });
        })
    }

    /// 声明最多包含 8 条显式列轨道的网格容器。
    pub fn grid(&mut self, key: u64, columns: &[Dim], build: impl FnOnce(&mut Self)) -> NodeId {
        let desc = RawNodeDesc::container()
            .display(unbound_abi::Display::Grid)
            .grid_columns(columns)
            .gap(8.0);
        self.container(key, desc, build)
    }

    /// 为撞上保留键的控件推导一个可用键。
    ///
    /// 结果保证非零、不等于语义根键，且本帧尚未被占用。
    /// 调用方负责把它登记进 `seen_keys`。
    fn derive_reserved_key(&self, base: u64) -> u64 {
        let mut salt = 1u64;
        loop {
            let candidate = mix_key(base, salt);
            if candidate != 0 && candidate != ROOT && !self.seen_keys.contains(&candidate) {
                return candidate;
            }
            salt = salt.wrapping_add(1);
        }
    }

    fn reconcile_node(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        loc: &'static core::panic::Location<'static>,
    ) -> (u64, NodeId) {
        let mut effective_key = key;
        if effective_key == ROOT {
            // 语义根键是保留键。控件占用它会让语义根成为自身的子节点，
            // 树遍历随即死循环；此处把控件改派到一个未被占用的派生键上。
            effective_key = self.derive_reserved_key(mix_key(key, 0x9E37_79B9_7F4A_7C15));
        }
        if effective_key != 0 && !self.seen_keys.insert(effective_key) {
            // 当调用方在循环体或复用辅助函数内使用无参 `ui_key!()` 时，同一帧会出现重复键；
            // 若直接覆盖 `node_keys` 并因父节点不同执行 `world.remove(existing)`，会把前序循环项
            // 的整棵子树误删。此处结合当前父节点键与递增序号派生确定性去重键，保证所有循环项完整保留。
            let parent_key = self
                .world
                .get(self.parent)
                .map_or(0, |parent_node| parent_node.desc.key);
            let mut salt = 1u64;
            loop {
                let candidate = mix_key(
                    effective_key ^ parent_key.rotate_left(13),
                    salt.wrapping_add(0x517C_C1B7_2722_0A95),
                );
                if candidate != 0 && self.seen_keys.insert(candidate) {
                    effective_key = candidate;
                    break;
                }
                salt = salt.wrapping_add(1);
            }
        }
        desc.key = effective_key;
        if effective_key != 0 {
            if let Some(&existing) = self.node_keys.get(&effective_key) {
                if let Some(node_data) = self.world.get(existing) {
                    if node_data.parent == self.parent {
                        self.world.set_desc(existing, desc);
                        return (effective_key, existing);
                    } else {
                        self.world.remove(existing);
                    }
                }
            }
        }
        let node = self.world.insert(self.parent, desc, loc);
        if effective_key != 0 {
            self.node_keys.insert(effective_key, node);
        } else {
            self.ephemeral_nodes.push(node);
        }
        (effective_key, node)
    }

    /// 使用显式描述符声明布局容器（若配置了背景填充、边框或阴影，将自动按树深度发射表面图元）。
    pub fn container(
        &mut self,
        key: u64,
        desc: RawNodeDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let has_surface = desc.fill != 0
            || (desc.border_width > 0.0 && desc.border_color != 0)
            || desc.shadow_blur > 0.0;
        let next_z_base = if desc.z_index >= PORTAL_Z_BASE {
            (desc.z_index as u16).max(self.z_base)
        } else {
            self.z_base
        };
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        if has_surface {
            let depth_z = (80u16.saturating_add(self.depth.saturating_mul(6))).min(480);
            let surface_z = next_z_base.saturating_add(depth_z);
            let hit_scrim = i32::from(next_z_base) >= PORTAL_Z_BASE
                && desc.fill == u32::from(unbound_theme::Token::SCRIM.raw());
            self.pending.push(PendingWidget {
                key: effective_key
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(1),
                node,
                spec: WidgetSpec::BoxSurface {
                    fill: u16::try_from(desc.fill).unwrap_or(u16::MAX),
                    border: u16::try_from(desc.border_color).unwrap_or(u16::MAX),
                    border_width: desc.border_width,
                    radii: desc.radius,
                    shadow_blur: desc.shadow_blur,
                    z: surface_z,
                    hit_scrim,
                },
                z_base: next_z_base,
            });
        }
        let parent = core::mem::replace(&mut self.parent, node);
        let prev_depth = self.depth;
        let prev_z_base = self.z_base;
        self.depth = self.depth.saturating_add(1);
        self.z_base = next_z_base;
        build(self);
        self.depth = prev_depth;
        self.z_base = prev_z_base;
        self.parent = parent;
        node
    }

    /// 在当前容器中放置控件；实际发射在布局完成后进行。
    pub fn widget(&mut self, key: u64, mut desc: RawNodeDesc, spec: WidgetSpec) -> NodeId {
        desc.key = key;
        let intrinsic = match &spec {
            WidgetSpec::UiWidget(_) => None,
            WidgetSpec::Button(button) => Some(Vec2::new(
                (button.label.chars().count() as f32 * 8.0 + 32.0).max(button.rect.width()),
                button.rect.height().max(32.0),
            )),
            WidgetSpec::Checkbox(cb) => Some(Vec2::new(
                cb.rect.width().max(120.0),
                cb.rect.height().max(24.0),
            )),
            WidgetSpec::Switch(sw) => Some(Vec2::new(
                sw.rect.width().max(48.0),
                sw.rect.height().max(26.0),
            )),
            WidgetSpec::Slider(sl) => Some(Vec2::new(
                sl.rect.width().max(160.0),
                sl.rect.height().max(28.0),
            )),
            WidgetSpec::Progress { desc: pr, .. } => Some(Vec2::new(
                pr.rect.width().max(160.0),
                pr.rect.height().max(12.0),
            )),
            WidgetSpec::Badge(badge) => Some(Vec2::new(
                badge.rect.width().max(22.0),
                badge.rect.height().max(20.0),
            )),
            WidgetSpec::Divider(div) => Some(Vec2::new(
                div.rect.width().max(1.0),
                div.rect.height().max(1.0),
            )),
            WidgetSpec::Alert(alert) => Some(Vec2::new(
                alert.rect.width().max(240.0),
                alert.rect.height().max(48.0),
            )),
            WidgetSpec::Text { .. }
            | WidgetSpec::TextInline { .. }
            | WidgetSpec::BoxSurface { .. }
            | WidgetSpec::ScrollScopeBegin { .. }
            | WidgetSpec::ScrollScopeEnd => None,
        };
        if let Some(size) = intrinsic {
            desc = desc.size(size.x, size.y);
        }
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        if let Some(size) = intrinsic {
            if let Some(data) = self.world.get_mut(node) {
                data.measure = Some(size);
            }
        }
        self.pending.push(PendingWidget {
            key: effective_key,
            node,
            spec,
            z_base: self.z_base,
        });
        node
    }

    /// 添加任意支持自动固有尺寸测量与布局的声明式 [`UiWidget`]。
    pub fn add<W: unbound_widgets::UiWidget + 'static>(&mut self, key: u64, widget: W) -> NodeId {
        let mut desc = RawNodeDesc::container();
        desc.flex_shrink = 0.0;
        self.add_with_layout(key, desc, widget)
    }

    /// 添加任意带自定义布局约束的声明式 [`UiWidget`]。
    pub fn add_with_layout<W: unbound_widgets::UiWidget + 'static>(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        widget: W,
    ) -> NodeId {
        desc.key = key;
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.pending.push(PendingWidget {
            key: effective_key,
            node,
            spec: WidgetSpec::UiWidget(Arc::new(widget)),
            z_base: self.z_base,
        });
        node
    }

    /// 声明式按钮快捷方式。
    pub fn button(&mut self, key: u64, desc: ButtonDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式复选框快捷方式。
    pub fn checkbox(&mut self, key: u64, desc: CheckboxDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式开关快捷方式。
    pub fn switch(&mut self, key: u64, desc: SwitchDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式滑动条快捷方式。
    pub fn slider(&mut self, key: u64, desc: SliderDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式文本输入框快捷方式。
    pub fn text_input(&mut self, key: u64, desc: unbound_widgets::TextInputDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式下拉选择框快捷方式。
    pub fn select(&mut self, key: u64, desc: unbound_widgets::SelectDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式徽章快捷方式。
    pub fn badge(&mut self, key: u64, desc: BadgeDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式分割线快捷方式。
    pub fn divider(&mut self, key: u64, desc: DividerDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式提示横幅快捷方式。
    pub fn alert(&mut self, key: u64, desc: AlertDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式进度指示器快捷方式。
    pub fn progress(&mut self, key: u64, desc: ProgressDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式工具提示（Tooltip）快捷方式（支持窗口内自动翻转与安全边距钳制）。
    pub fn tooltip(&mut self, key: u64, desc: unbound_widgets::TooltipDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式快捷键录制框快捷方式。
    pub fn hotkey_input(&mut self, key: u64, desc: unbound_widgets::HotkeyInputDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式颜色选择器快捷方式。
    pub fn color_picker(&mut self, key: u64, desc: unbound_widgets::ColorPickerDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式颜色色块快捷方式（Win11 设置应用「个性化 > 颜色」里的色块网格单元）。
    pub fn color_swatch(&mut self, key: u64, desc: unbound_widgets::ColorSwatchDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式语法高亮只读代码块快捷方式。
    pub fn code_block(&mut self, key: u64, desc: unbound_widgets::CodeBlockDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式代码编辑器快捷方式。
    pub fn code_editor(&mut self, key: u64, desc: unbound_widgets::CodeEditorDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式数字输入框快捷方式。
    pub fn number_input(&mut self, key: u64, desc: unbound_widgets::NumberInputDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式可搜索组合框快捷方式。
    pub fn combobox(&mut self, key: u64, desc: unbound_widgets::ComboboxDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式自动补全输入框快捷方式。
    pub fn autocomplete(&mut self, key: u64, desc: unbound_widgets::AutoCompleteDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式标签页栏快捷方式。
    pub fn tabs(&mut self, key: u64, desc: unbound_widgets::TabsDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式折线/柱状/面积图表快捷方式。
    pub fn chart(&mut self, key: u64, desc: unbound_widgets::ChartDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式泛型数据表快捷方式。
    pub fn generic_data_table(
        &mut self,
        key: u64,
        desc: unbound_widgets::GenericDataTableDesc,
    ) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式 Schema 驱动表单快捷方式。
    pub fn schema_form(&mut self, key: u64, desc: unbound_widgets::SchemaFormDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式终端日志视图快捷方式。
    pub fn terminal_view(&mut self, key: u64, desc: unbound_widgets::TerminalViewDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式任务进度悬浮坞快捷方式。
    pub fn task_hud(&mut self, key: u64, desc: unbound_widgets::TaskHudDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式流水线阶段条快捷方式。
    pub fn pipeline_view(&mut self, key: u64, desc: unbound_widgets::PipelineViewDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式几何矢量外壳图标快捷方式。
    pub fn shell_icon(&mut self, key: u64, desc: unbound_widgets::ShellIconDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式几何矢量图标按钮快捷方式。
    pub fn icon_button(&mut self, key: u64, desc: unbound_widgets::IconButtonDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式应用品牌徽标（圆角渐变色块 + 中心字母）快捷方式。
    pub fn brand_badge(&mut self, key: u64, desc: unbound_widgets::AppBrandBadgeDesc) -> NodeId {
        self.add(key, desc)
    }

    /// 声明式侧边栏分组标题快捷方式。
    pub fn sidebar_section(
        &mut self,
        key: u64,
        desc: unbound_widgets::SidebarSectionHeaderDesc,
    ) -> NodeId {
        let mut layout = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::px(desc.height));
        layout.flex_shrink = 0.0;
        self.add_with_layout(key, layout, desc)
    }

    /// 声明式侧边栏导航项（含矢量图标、选中态胶囊高亮、左侧 3px 强调色竖条与尾部徽标）快捷方式。
    pub fn sidebar_item(&mut self, key: u64, desc: unbound_widgets::SidebarItemDesc) -> NodeId {
        let mut layout = if desc.collapsed {
            RawNodeDesc::container()
                .width(Dim::px(40.0))
                .height(Dim::px(desc.height))
        } else {
            RawNodeDesc::container()
                .width(Dim::percent(100.0))
                .height(Dim::px(desc.height))
        };
        layout.flex_shrink = 0.0;
        self.add_with_layout(key, layout, desc)
    }

    /// 声明式 Showcase 风格特性入口卡片快捷方式。
    pub fn feature_card(&mut self, key: u64, desc: unbound_widgets::FeatureCardDesc) -> NodeId {
        let mut layout = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::px(desc.height));
        layout.flex_shrink = 0.0;
        self.add_with_layout(key, layout, desc)
    }

    /// 声明式 Showcase 风格内凹舞台容器（`radius = 10.0`, `SURFACE_SUNKEN`, `OUTLINE`）。
    pub fn stage_box(
        &mut self,
        key: u64,
        padding: f32,
        gap: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(padding))
            .gap(gap)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::SURFACE_SUNKEN.raw()))
            .border(u32::from(unbound_theme::Token::OUTLINE.raw()), 1.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明式 Showcase 风格章节卡片容器（`radius = 12.0`, 标题 + 右侧胶囊徽章组 + 副标题 + 内容区）。
    pub fn showcase_card(
        &mut self,
        key: u64,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        badges: &[(&str, unbound_widgets::BadgeVariant)],
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let title_str = title.into();
        let subtitle_str = subtitle.into();
        let mut card_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(20.0))
            .gap(14.0)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()))
            .border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        card_desc.flex_shrink = 0.0;
        self.container(key, card_desc, |card| {
            card.row_between(mix_key(key, 1), |hdr| {
                hdr.col_grow(mix_key(key, 2), 4.0, |titles| {
                    titles.text_primary(mix_key(key, 3), title_str, 16.0, 600);
                    if !subtitle_str.is_empty() {
                        titles.text_muted(mix_key(key, 4), subtitle_str, 12.5);
                    }
                });
                if !badges.is_empty() {
                    hdr.row_center(mix_key(key, 5), 6.0, |pills| {
                        for (idx, (label, variant)) in badges.iter().enumerate() {
                            pills.badge(
                                mix_key(key, 100 + idx as u64),
                                BadgeDesc::label(*label).variant(*variant),
                            );
                        }
                    });
                }
            });
            build(card);
        })
    }

    /// 声明式 Showcase 风格页面大标题头部（22px 粗体标题 + 副标题 + 顶部胶囊组 + 右侧动作栏）。
    pub fn page_header(
        &mut self,
        key: u64,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        pills: &[(&str, unbound_widgets::BadgeVariant)],
        actions: impl FnOnce(&mut Self),
    ) -> NodeId {
        let title_str = title.into();
        let subtitle_str = subtitle.into();
        self.col_stretch(key, 4.0, |hdr| {
            hdr.row_between(mix_key(key, 1), |top| {
                top.col_grow(mix_key(key, 2), 6.0, |left| {
                    // Win11 的内容页大标题：28px / 600，与副标题之间只留 2px 的呼吸，
                    // 让「标题 + 一句说明」读成一个整体，而不是两行各自为政。
                    left.text_primary(
                        mix_key(key, 3),
                        title_str,
                        unbound_theme::typography::font_size::PAGE_TITLE,
                        unbound_theme::typography::weight::SEMIBOLD,
                    );
                    if !subtitle_str.is_empty() {
                        left.text_muted(
                            mix_key(key, 4),
                            subtitle_str,
                            unbound_theme::typography::font_size::CAPTION,
                        );
                    }
                });
                top.row_center(mix_key(key, 5), 8.0, actions);
            });
            if !pills.is_empty() {
                hdr.row_wrap(mix_key(key, 6), 8.0, |row| {
                    for (idx, (label, variant)) in pills.iter().enumerate() {
                        row.badge(
                            mix_key(key, 200 + idx as u64),
                            BadgeDesc::label(*label).variant(*variant),
                        );
                    }
                });
            }
        })
    }

    /// 声明全屏遮罩居中模态弹窗容器（自动开启 `z_index = 1200` 浮层与 `SCRIM` 命中遮罩）。
    pub fn modal_scrim(
        &mut self,
        scrim_key: u64,
        dialog_key: u64,
        width: f32,
        height: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let vp = self.ctx.viewport;
        let scrim_desc = RawNodeDesc::container()
            .absolute()
            .size(vp.x, vp.y)
            .z_index(1200)
            .portal_root()
            .align(unbound_abi::Align::Center)
            .justify(unbound_abi::Justify::Center)
            .fill_token(u32::from(unbound_theme::Token::SCRIM.raw()));
        self.container(scrim_key, scrim_desc, |overlay| {
            let dlg_w = width.min((vp.x - 32.0).max(280.0));
            let dlg_h = height.min((vp.y - 32.0).max(200.0));
            let mut dlg_desc = RawNodeDesc::container()
                .column()
                .width(Dim::px(dlg_w))
                .height(Dim::px(dlg_h))
                .padding(unbound_abi::Edges::all(18.0))
                .gap(12.0)
                .radius(8.0)
                .fill_token(u32::from(unbound_theme::Token::SURFACE_RAISED.raw()))
                .border(u32::from(unbound_theme::Token::OUTLINE_STRONG.raw()), 1.0);
            // ABI 未提供 `shadow_blur` 链式构造器，直接写 POD 字段；
            // `container()` 依据它判断是否发射表面图元。
            dlg_desc.shadow_blur = 24.0;
            overlay.container(dialog_key, dlg_desc, build);
        })
    }

    /// 声明右侧滑出抽屉浮层容器（自动开启 `z_index = 1150` 浮层与左侧半透明遮罩）。
    pub fn drawer_right(
        &mut self,
        scrim_key: u64,
        drawer_key: u64,
        width: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let vp = self.ctx.viewport;
        let scrim_desc = RawNodeDesc::container()
            .row()
            .absolute()
            .size(vp.x, vp.y)
            .z_index(1150)
            .portal_root()
            .justify(unbound_abi::Justify::FlexEnd)
            .fill_token(u32::from(unbound_theme::Token::SCRIM.raw()));
        self.container(scrim_key, scrim_desc, |overlay| {
            let w = width.min((vp.x - 48.0).max(280.0));
            let mut drawer_desc = RawNodeDesc::container()
                .column()
                .width(Dim::px(w))
                .height(Dim::percent(100.0))
                .padding(unbound_abi::Edges::all(18.0))
                .gap(12.0)
                .fill_token(u32::from(unbound_theme::Token::SURFACE_RAISED.raw()))
                .border(u32::from(unbound_theme::Token::OUTLINE_STRONG.raw()), 1.0);
            // ABI 未提供 `shadow_blur` 链式构造器，直接写 POD 字段。
            drawer_desc.shadow_blur = 24.0;
            overlay.container(drawer_key, drawer_desc, build);
        })
    }

    /// 声明式卡片容器快捷方式（自动绘制 `CARD_BG` 背景、`CARD_BORDER` 边框与圆角）。
    pub fn card(
        &mut self,
        key: u64,
        desc: unbound_widgets::CardDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut node_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(16.0))
            .gap(10.0)
            .radius(
                desc.radius
                    .max(unbound_theme::typography::geometry::CARD_RADIUS),
            )
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()));
        node_desc.flex_shrink = 0.0;
        node_desc = node_desc.border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        self.container(key, node_desc, build)
    }

    /// 声明 Win11 风格面板卡片。
    ///
    /// Win11 的分组卡片是**近乎方角**的 4px 圆角 + 一层极细描边，
    /// 层次来自「同一张云母上叠一层薄纱」，而不是大圆角加投影——
    /// 后者是 Material / Fluent 旧版观感。这里刻意不发射阴影。
    pub fn panel_card(
        &mut self,
        key: u64,
        padding: f32,
        gap: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(padding))
            .gap(gap)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()))
            .border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明深色内凹舞台容器框（适用于代码预览、路径框、子列表）。
    pub fn sunken_box(
        &mut self,
        key: u64,
        padding: f32,
        gap: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(padding))
            .gap(gap)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::SURFACE_SUNKEN.raw()))
            .border(u32::from(unbound_theme::Token::OUTLINE.raw()), 1.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明 Win11 设置应用风格的分组卡片。
    ///
    /// 与 [`Self::panel_card`] 的关键差别：卡片**不承载内边距**，行与行之间
    /// 靠 1px 分隔线而不是留白隔开——这正是 Win11 设置页「一张卡片里排若干行」
    /// 的读法。卡片本身是 4px 圆角的薄纱加细描边，没有阴影。
    pub fn settings_card(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(0.0))
            .gap(0.0)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()))
            .border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明设置卡片内的一行（扁平排布：标题在上、说明在下，右侧可放行内控件）。
    ///
    /// 文字左缩进恒为 57px（`SETTINGS_ROW_TEXT_INSET`）——与 Win11 原生一致：
    /// 不论该行有没有前置图标，标题与副标题都落在同一条竖线上。
    pub fn settings_row(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::trbl(
                unbound_theme::typography::geometry::SETTINGS_ROW_TEXT_INSET,
                12.0,
                16.0,
                12.0,
            ))
            .gap(2.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明带前置图标槽的设置行（左侧留出图标位，文字缩进与无图标行对齐）。
    pub fn settings_row_with_icon(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::trbl(
                unbound_theme::typography::geometry::SETTINGS_ROW_ICON_INSET,
                12.0,
                16.0,
                12.0,
            ))
            .gap(2.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明设置页的分组小标题（`相关设置` / `相关支持` 那种 15px 半粗小节标题）。
    pub fn settings_group_header(&mut self, key: u64, text: impl Into<String>) -> NodeId {
        let text = text.into();
        self.text_primary(
            key,
            text,
            unbound_theme::typography::font_size::SECTION_TITLE,
            unbound_theme::typography::weight::SEMIBOLD,
        )
    }

    /// 声明两端对齐的水平行容器（`width: 100%`, `Justify::SpaceBetween`, `Align::Center`）。
    pub fn row_between(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .row()
            .width(Dim::percent(100.0))
            .align(unbound_abi::Align::Center)
            .justify(unbound_abi::Justify::SpaceBetween)
            .gap(8.0);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明左对齐的满宽水平行容器（`width: 100%`, `Justify::FlexStart`, `Align::Center`）。
    ///
    /// 与 [`Self::row_between`] 的差别：两端对齐会把「图标 + 文字」这一组整体
    /// 推到最左、把行内控件推到最右；左对齐版本让中间的 `col_grow` 自然吸收
    /// 剩余宽度，于是**文字紧跟在图标之后**——这正是 Win11 设置行的读法。
    pub fn row_start(&mut self, key: u64, gap: f32, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .row()
            .width(Dim::percent(100.0))
            .align(unbound_abi::Align::Center)
            .justify(unbound_abi::Justify::FlexStart)
            .gap(gap);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明垂直居中的水平行容器（`Align::Center`）。
    pub fn row_center(&mut self, key: u64, gap: f32, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .row()
            .align(unbound_abi::Align::Center)
            .gap(gap);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明自动换行的水平流式容器（适用于筛选项胶囊组、工具栏按钮组）。
    pub fn row_wrap(&mut self, key: u64, gap: f32, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .row()
            .width(Dim::percent(100.0))
            .align(unbound_abi::Align::Center)
            .gap(gap)
            .flags(NodeFlags::WRAP);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明占满宽度的纵向堆叠容器。
    pub fn col_stretch(&mut self, key: u64, gap: f32, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .gap(gap);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明弹性填充剩余宽度的纵向分栏（`flex_grow = 1.0`，最小宽度 `0`）。
    ///
    /// 与 [`Self::col_stretch`] 的区别：`col_stretch` 直接声明 `width: 100%`，
    /// 只适合独占整行的列。当左列要与右侧操作区共享同一个 `row` / `row_between` 时
    /// 必须改用本方法，否则左列会吃掉整行宽度，把右侧按钮挤出可视区。
    pub fn col_grow(&mut self, key: u64, gap: f32, build: impl FnOnce(&mut Self)) -> NodeId {
        let mut desc = RawNodeDesc::container().column().gap(gap);
        desc.min_width = Dim::px(0.0);
        desc.flex_grow = 1.0;
        desc.flex_basis = Dim::px(0.0);
        self.container(key, desc, build)
    }

    /// 弹性空白占位节点（`flex_grow = 1.0`）。
    pub fn spacer(&mut self, key: u64) -> NodeId {
        let mut desc = RawNodeDesc::container();
        desc.flex_grow = 1.0;
        self.reconcile_node(key, desc, core::panic::Location::caller())
            .1
    }

    /// 声明式滚动视图容器快捷方式（支持真实裁剪、平移与滚轮滚动）。
    pub fn scroll_view(
        &mut self,
        key: u64,
        vertical: bool,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut desc = RawNodeDesc::container()
            .width(Dim::fr(1.0))
            .height(Dim::fr(1.0));
        if vertical {
            desc = desc.column().flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        } else {
            desc = desc.row().flags(NodeFlags::CLIP | NodeFlags::SCROLL_X);
        }
        self.scroll_view_with(key, desc, vertical, build)
    }

    /// 使用自定义外部布局描述符声明滚动视图容器。
    pub fn scroll_view_with(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        vertical: bool,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        if vertical {
            desc = desc.column().flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        } else {
            desc = desc.row().flags(NodeFlags::CLIP | NodeFlags::SCROLL_X);
        }
        let (effective_key, scroll_node) =
            self.reconcile_node(key, desc, core::panic::Location::caller());
        let parent_saved = core::mem::replace(&mut self.parent, scroll_node);
        let mut inner_desc = if vertical {
            RawNodeDesc::container()
                .column()
                .width(Dim::percent(100.0))
                .gap(12.0)
        } else {
            RawNodeDesc::container()
                .row()
                .height(Dim::percent(100.0))
                .gap(12.0)
        };
        inner_desc.flex_shrink = 0.0;
        let content_key = effective_key
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(17);
        let (_, content_node) =
            self.reconcile_node(content_key, inner_desc, core::panic::Location::caller());
        self.pending.push(PendingWidget {
            key: effective_key,
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeBegin {
                scroll_key: effective_key,
                content_node,
                vertical,
            },
            z_base: self.z_base,
        });
        self.parent = content_node;
        let prev_depth = self.depth;
        self.depth = self.depth.saturating_add(1);
        build(self);
        self.depth = prev_depth;
        self.pending.push(PendingWidget {
            key: effective_key
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(18),
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeEnd,
            z_base: self.z_base,
        });
        self.parent = parent_saved;
        scroll_node
    }

    /// 声明式分栏容器快捷方式。
    pub fn split_pane(
        &mut self,
        key: u64,
        vertical: bool,
        first: impl FnOnce(&mut Self),
        second: impl FnOnce(&mut Self),
    ) -> NodeId {
        let desc = if vertical {
            RawNodeDesc::container().column()
        } else {
            RawNodeDesc::container().row()
        };
        self.container(key, desc, |ui| {
            first(ui);
            second(ui);
        })
    }

    /// 放置自动换行段落。其跨越容器宽度，测量高度来自 [`TextEngine::wrap`]。
    pub fn text(&mut self, key: u64, text: impl Into<String>, style: TextStyle) -> NodeId {
        let text = text.into();
        let is_long_paragraph = text.len() > 64 || text.contains('\n');
        let mut desc = RawNodeDesc::container().key(key);
        if is_long_paragraph {
            desc = desc.width(Dim::percent(100.0)).flags(NodeFlags::WRAP);
        } else {
            desc.flex_shrink = 0.0;
        }
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.world.set_text(node, text.clone());
        if is_long_paragraph {
            self.pending.push(PendingWidget {
                key: effective_key,
                node,
                spec: WidgetSpec::Text { text, style },
                z_base: self.z_base,
            });
        } else {
            self.pending.push(PendingWidget {
                key: effective_key,
                node,
                spec: WidgetSpec::TextInline {
                    text,
                    style,
                    color: unbound_theme::Token::TEXT_PRIMARY,
                },
                z_base: self.z_base,
            });
        }
        node
    }

    /// 放置带语义颜色令牌的单行自适应宽度文本标签。
    pub fn text_inline(
        &mut self,
        key: u64,
        text: impl Into<String>,
        style: TextStyle,
        color: unbound_theme::Token,
    ) -> NodeId {
        let text = text.into();
        let mut desc = RawNodeDesc::container().key(key);
        desc.flex_shrink = 0.0;
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.world.set_text(node, text.clone());
        self.pending.push(PendingWidget {
            key: effective_key,
            node,
            spec: WidgetSpec::TextInline { text, style, color },
            z_base: self.z_base,
        });
        node
    }

    /// `text_inline` 的语义别名。
    pub fn text_colored(
        &mut self,
        key: u64,
        text: impl Into<String>,
        style: TextStyle,
        color: unbound_theme::Token,
    ) -> NodeId {
        self.text_inline(key, text, style, color)
    }

    /// 放置主前景色（`TEXT_PRIMARY`）单行文本。
    pub fn text_primary(
        &mut self,
        key: u64,
        text: impl Into<String>,
        size: f32,
        weight: u16,
    ) -> NodeId {
        let mut style = TextStyle::sized(size);
        style.weight = weight;
        self.text_inline(key, text, style, unbound_theme::Token::TEXT_PRIMARY)
    }

    /// 放置次级柔和色（`TEXT_SECONDARY`）单行文本。
    pub fn text_muted(&mut self, key: u64, text: impl Into<String>, size: f32) -> NodeId {
        let style = TextStyle::sized(size);
        self.text_inline(key, text, style, unbound_theme::Token::TEXT_SECONDARY)
    }

    /// 放置强调色（`PRIMARY`）单行文本。
    pub fn text_accent(
        &mut self,
        key: u64,
        text: impl Into<String>,
        size: f32,
        weight: u16,
    ) -> NodeId {
        let mut style = TextStyle::sized(size);
        style.weight = weight;
        self.text_inline(key, text, style, unbound_theme::Token::PRIMARY)
    }

    /// 直接访问收集器，供需要追加变换或裁剪的控件使用。
    pub fn collector(&mut self) -> &mut SceneCollector {
        self.collector
    }

    /// 运行时资产中心（只读）：查询已解码位图、字体等资源。
    #[must_use]
    pub fn assets(&self) -> &AssetHub {
        self.assets
    }

    /// 提交一张 PNG 供运行时异步解码并打包进图集，返回资源句柄。
    ///
    /// 解码发生在工作线程，本帧不会阻塞；调用方应在后续帧用
    /// [`Self::backdrop_image`] 或 [`unbound_widgets::emit_image_with_assets`] 绘制它。
    pub fn request_png(&mut self, bytes: Vec<u8>) -> unbound_asset::AssetId {
        self.assets.request_png(bytes)
    }

    /// 在整窗最底层绘制一张位图（云母/壁纸背板）。
    ///
    /// 图元在本次视图构建的最开头压入收集器，因此位于所有容器与卡片之下；
    /// 资源仍在解码时返回 `false` 且什么都不画——占位符会把整窗涂成内凹灰块。
    pub fn backdrop_image(&mut self, key: u64, asset: unbound_asset::AssetId) -> bool {
        use unbound_widgets::{ImageAssetResolver, ImageDesc, ObjectFit};
        if self.assets.resolve(asset).is_none() {
            return false;
        }
        let viewport = self.ctx.viewport;
        let mut desc = ImageDesc::new(Rect::new(0.0, 0.0, viewport.x, viewport.y), viewport, 0)
            .with_asset(asset)
            .with_fit(ObjectFit::Fill);
        // z = 0：位列所有容器与卡片之下（图元按 z 排序，卡片背景在 1 以上）。
        desc.z = 0;
        let output = unbound_widgets::emit_image_with_assets(
            key,
            &desc,
            ControlState::default(),
            self.assets,
        );
        self.collector.push(key, output);
        true
    }

    /// 请求渲染器在 `rect` 后方执行背景模糊通道。
    ///
    /// 覆盖层发射器返回 `BackdropRequest`；将其转发至此会写入帧报告，
    /// 渲染器据此激活毛玻璃管线。列表为空时渲染图将完全跳过模糊通道。
    ///
    pub fn request_backdrop(&mut self, request: BackdropRequest) {
        self.collector.backdrops.push(request);
    }

    /// 以原点为基准的视口矩形。
    #[must_use]
    pub fn viewport_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.ctx.viewport.x, self.ctx.viewport.y)
    }
}

/// 应用契约。
///
/// 该 trait 刻意保持极简：输入状态、输入事件、输出图元。
/// 窗口化外壳、无头测试与 showcase 均驱动完全相同的三个方法。
///
pub trait Application {
    /// 应用状态类型。
    type State;

    /// 构建初始状态。
    fn init(&mut self, ctx: &mut AppContext) -> Self::State;

    /// 归约单个已解析事件。
    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome;

    /// 响应局部状态键的生命周期转换。
    ///
    /// 在视图构建之后运行，此时运行时已知晓哪些键出现或消失以及键盘焦点何时移动。
    /// 返回需要重建的标志将为下一帧安排重建。
    ///
    fn lifecycle(&mut self, _state: &mut Self::State, _event: LifecycleEvent) -> DirtyFlags {
        DirtyFlags::NONE
    }

    /// 为当前状态发射场景。
    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>);

    /// 快照应用状态。
    ///
    /// 默认返回 `None`——表示该应用不提供状态快照。框架不关心状态的形状：
    /// 应用想支持「回到探索之前的样子」，就把自己的状态序列化后放进这个
    /// 盒子；不想支持，就让默认实现留着。
    ///
    /// 盒子用 `Arc` 包裹，因此快照可以**反复恢复**——回放与最小复现剪枝
    /// 正是靠这一点：同一个基线要跑上很多遍。
    fn snapshot_state(&self, _state: &Self::State) -> Option<Arc<dyn Any>> {
        None
    }

    /// 恢复应用状态。
    ///
    /// 与 [`Self::snapshot_state`] 配对。默认什么都不做。
    fn restore_state(&mut self, _state: &mut Self::State, _snapshot: &dyn Any) {}
}

/// 标题栏通过冒泡兜底产出的窗口控制意图，由窗口运行器在输入处理后消费执行。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    /// 启动系统原生窗口拖拽。
    StartDrag,
    /// 最小化窗口。
    Minimize,
    /// 切换最大化与还原状态。
    ToggleMaximize,
    /// 请求关闭窗口。
    Close,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TitleBarBinding {
    minimize_id: u64,
    maximize_id: u64,
    close_id: u64,
}

/// 事件是否继续沿命中路径向上冒泡。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Propagation {
    /// 事件可继续提供给下一级祖先。
    #[default]
    Bubble,
    /// 处理器已消费事件；运行时停止沿路径向上遍历。
    Stop,
}

/// 归约单个事件的产出结果。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EventOutcome {
    /// 本次归约导致的失效范围。
    pub dirty: DirtyFlags,
    /// 事件是否仍应继续提供给祖先。
    pub propagation: Propagation,
}

impl EventOutcome {
    /// 产生 `dirty` 变更且继续冒泡的结果。
    #[must_use]
    pub const fn new(dirty: DirtyFlags) -> Self {
        Self {
            dirty,
            propagation: Propagation::Bubble,
        }
    }

    /// 停止事件传播的结果。
    #[must_use]
    pub const fn stopping(dirty: DirtyFlags) -> Self {
        Self {
            dirty,
            propagation: Propagation::Stop,
        }
    }

    /// 将同一结果标记为已消费。
    #[must_use]
    pub const fn stop(self) -> Self {
        Self {
            dirty: self.dirty,
            propagation: Propagation::Stop,
        }
    }
}

/// 冒泡父链的最大深度：异常树的兜底，正常界面远小于此值。
const MAX_BUBBLE_DEPTH: usize = 256;

/// 一次冒泡派发的轨迹采集器。
///
/// 离散指针事件、滚轮与键盘共用同一套「访问路径 / 终止点 / 检查器记录」语义，
/// 避免三处循环各自维护 `visited`、`stopped_at` 与 `record_event` 而逐渐漂移。
struct BubbleTrace {
    kind: &'static str,
    path: Vec<u64>,
    stopped_at: Option<u64>,
    record: bool,
}

impl BubbleTrace {
    fn new(kind: &'static str, record: bool) -> Self {
        Self {
            kind,
            path: Vec::new(),
            stopped_at: None,
            record,
        }
    }

    fn visit(&mut self, node: u64) {
        if self.record {
            self.path.push(node);
        }
    }

    fn stop(&mut self, node: u64) {
        self.stopped_at = Some(node);
    }

    fn finish(self, inspector: &mut Inspector) {
        if self.record {
            inspector.record_event(EventTrace {
                kind: self.kind,
                path: self.path,
                stopped_at: self.stopped_at,
            });
        }
    }
}

/// 重建单帧的结果。
#[derive(Clone, Debug)]
pub struct FrameReport {
    /// 渲染器必须上传的差异。
    pub diff: SceneDiff,
    /// 自上一帧以来累积的脏标志。
    pub dirty: DirtyFlags,
    /// 新帧中的图元数量。
    pub primitives: usize,
    /// 本次 diff 需上传的字节数。
    pub bytes: u32,
    /// 某个轻量辅助表（payloads、transforms、clips）是否发生变更，
    /// 需单独重传而无需重新发送整个图元池。
    ///
    pub aux_changed: bool,
    /// 本帧的背景模糊请求。
    pub backdrops: Vec<BackdropRequest>,
    /// 资产中心本帧解码完成的图集矩形。
    ///
    /// 外壳在绘制前通过 [`unbound_gpu::atlas::AtlasSet::upload_batch`] 上传它们；
    /// 运行时本身从不接触 GPU。
    pub atlas_uploads: Vec<AtlasUpload>,
}

/// 负责事件归约与场景 diff 的应用运行时。
pub struct AppRuntime<A: Application> {
    app: A,
    state: A::State,
    ctx: AppContext,
    router: Router,
    focus: FocusManager,
    collector: SceneCollector,
    previous_primitives: Vec<GpuPrimitiveInstance>,
    previous_payloads: Vec<GpuPayload>,
    previous_transforms: Vec<GpuTransform>,
    previous_clips: Vec<GpuClipNode>,
    diff: SceneDiff,
    dirty: DirtyFlags,
    backdrops: Vec<BackdropRequest>,
    time: f64,
    frame_count: u64,
    force_rebuild: bool,
    world: UiWorld,
    layout: LayoutEngine,
    text_engine: TextEngine,
    glyph_cache: GlyphCache,
    asset_hub: AssetHub,
    pending: Vec<PendingWidget>,
    mounted_widgets: FxHashMap<u64, (Rect, Arc<dyn ErasedUiWidget>)>,
    widget_state: Rc<RefCell<WidgetStateStore>>,
    timers: TimerQueue,
    async_rx: Receiver<AppEvent>,
    keymap: Keymap,
    key_context: Option<String>,
    last_focused: Option<u64>,
    inspector: Inspector,
    debug_sanitizers: bool,
    node_keys: FxHashMap<u64, NodeId>,
    /// 命中区域键 → 所属控件节点键，由 `SceneCollector` 每帧登记。
    hit_owners: FxHashMap<u64, u64>,
    seen_keys: FxHashSet<u64>,
    ephemeral_nodes: Vec<NodeId>,
    last_layout_stats: LayoutStats,
    layout_initialized: bool,
    overlays: OverlayManager,
    scroll_regions: Vec<(u64, Rect, Vec2, bool)>,
    last_pointer_down_target: Option<u64>,
    /// 按下那刻展开的冒泡路径，供释放时校验目标一致性（路径不再随帧间树变化漂移）。
    last_pointer_down_path: Vec<u64>,
    /// 本帧注册的标题栏根容器及其控制按钮映射。
    title_bar_bindings: FxHashMap<u64, TitleBarBinding>,
    /// 冒泡终点兜底产出的待执行窗口命令队列。
    window_commands: Vec<WindowCommand>,
    /// 上一帧生效的总缩放比（`os_scale * ui_scale`），用于检测 `reduce` 内缩放变更并同帧重排。
    last_effective_scale: f32,
    /// 已同步到文本引擎的东亚字体族（用于检测界面语言切换）。
    applied_cjk_family: Option<&'static str>,
    /// 本帧声明的语义标注：控件键 → 标注意图。
    ///
    /// 复用同一张表，避免每帧重新分配；语义标注的规模通常远小于控件数量。
    semantic_intents: FxHashMap<u64, SemanticIntent>,
}

impl<A: Application> AppRuntime<A> {
    /// 构建运行时并初始化应用状态。
    pub fn new(mut app: A, viewport: Vec2) -> Self {
        let mut ctx = AppContext::new(viewport);
        let widget_state = Rc::new(RefCell::new(WidgetStateStore::new()));
        let timers = TimerQueue::new();
        let (async_tx, async_rx) = crossbeam_channel::unbounded();
        ctx.share_state(Rc::clone(&widget_state));
        ctx.share_timer_tokens(timers.tokens());
        ctx.install_async(async_tx.clone(), None);
        let state = app.init(&mut ctx);
        let mut runtime = Self {
            app,
            state,
            ctx,
            router: Router::default(),
            focus: FocusManager::default(),
            collector: SceneCollector::new(ROOT),
            previous_primitives: Vec::new(),
            previous_payloads: Vec::new(),
            previous_transforms: Vec::new(),
            previous_clips: Vec::new(),
            diff: SceneDiff::Whole,
            dirty: DirtyFlags::ALL,
            backdrops: Vec::new(),
            time: 0.0,
            frame_count: 0,
            force_rebuild: true,
            world: UiWorld::new(),
            layout: LayoutEngine::new(),
            text_engine: TextEngine::new(),
            glyph_cache: GlyphCache::new(),
            asset_hub: AssetHub::new(),
            pending: Vec::new(),
            mounted_widgets: FxHashMap::default(),
            widget_state,
            timers,
            async_rx,
            keymap: Keymap::new(),
            key_context: None,
            last_focused: None,
            inspector: Inspector::new(),
            debug_sanitizers: false,
            node_keys: FxHashMap::default(),
            hit_owners: FxHashMap::default(),
            seen_keys: FxHashSet::default(),
            ephemeral_nodes: Vec::new(),
            last_layout_stats: LayoutStats::default(),
            layout_initialized: false,
            overlays: OverlayManager::new(),
            scroll_regions: Vec::new(),
            last_pointer_down_target: None,
            last_pointer_down_path: Vec::new(),
            title_bar_bindings: FxHashMap::default(),
            window_commands: Vec::new(),
            last_effective_scale: 1.0,
            applied_cjk_family: None,
            semantic_intents: FxHashMap::default(),
        };
        // `init` 中调度的定时器相对于零时钟可能已到期；
        // 在首帧推进越过它之前先将其入队。
        runtime.collect_timer_requests();
        runtime
    }

    /// 安装后台任务用于唤醒休眠事件循环的唤醒器。
    pub fn set_waker(&mut self, waker: Arc<dyn AppWaker>) {
        self.ctx.waker = Some(waker);
    }

    /// 可变快捷键映射表。
    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    /// 设置当前激活的快捷键作用域（`"TextInput"`、`"Modal"` 等）。
    pub fn set_key_context(&mut self, context: Option<&str>) {
        self.key_context = context.map(str::to_owned);
        self.keymap.clear_pending();
    }

    /// DevTools 检查器状态。
    #[must_use]
    pub const fn inspector(&self) -> &Inspector {
        &self.inspector
    }

    /// 可变访问检查器，供外壳侧接线使用（例如菜单项）。
    pub fn inspector_mut(&mut self) -> &mut Inspector {
        &mut self.inspector
    }

    /// 切换检查器覆盖层；返回新状态。
    pub fn toggle_inspector(&mut self) -> bool {
        let enabled = self.inspector.toggle();
        // 覆盖层属于场景的一部分，因此即使没有控件变脏，下一帧也必须重建。
        //
        self.force_rebuild = true;
        self.dirty |= DirtyFlags::PAINT;
        enabled
    }

    /// 开启或关闭每帧无头检查器通道。
    ///
    /// 开启时，每帧都会将 world 镜像为 `unbound_headless::Frame` 并运行五项检查器；
    /// 违规项会在检查器中显示为闪烁边框。这是调试开关，每帧需额外遍历整棵树。
    ///
    pub fn enable_debug_sanitizers(&mut self, enabled: bool) {
        self.debug_sanitizers = enabled;
        self.force_rebuild = true;
        self.dirty |= DirtyFlags::PAINT;
    }

    /// 检查器通道当前是否启用。
    #[must_use]
    pub const fn debug_sanitizers_enabled(&self) -> bool {
        self.debug_sanitizers
    }

    /// 强制完整重建场景并重新上传所有缓存资源。
    pub fn request_full_rebuild(&mut self) {
        self.force_rebuild = true;
        self.dirty |= DirtyFlags::ALL;
        self.glyph_cache.clear();
    }

    /// 距离下一个定时器触发的剩余时间，供 host 的 `ControlFlow` 使用。
    #[must_use]
    pub fn next_wake_in(&self) -> Option<Duration> {
        let deadline = self.timers.next_deadline_ms()?;
        Some(Duration::from_millis(
            deadline.saturating_sub(self.now_ms()),
        ))
    }

    /// 归约后台任务投递的所有结果。
    pub fn poll_async(&mut self) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        while let Ok(event) = self.async_rx.try_recv() {
            flags = flags.merge(self.dispatch(event));
        }
        flags
    }

    /// 归约当前时钟下到期的所有定时器。
    pub fn poll_timers(&mut self) -> DirtyFlags {
        self.collect_timer_requests();
        let due = self.timers.due(self.now_ms());
        let mut flags = DirtyFlags::NONE;
        for event in due {
            flags = flags.merge(self.dispatch(event));
        }
        flags
    }

    /// 按键访问单个局部状态值。
    pub fn with_state<T: Default + Send + Sync + 'static, R>(
        &mut self,
        key: u64,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        let mut store = self.widget_state.borrow_mut();
        f(store.entry(key))
    }

    /// 局部状态池，供测试或需要检查它的 host 使用。
    #[must_use]
    pub fn state_store(&self) -> core::cell::Ref<'_, WidgetStateStore> {
        self.widget_state.borrow()
    }

    /// 按键返回控件的世界坐标矩形。
    ///
    /// 已发射的 [`UiWidget`] 优先从挂载表读取；行内文本、容器表面等纯声明式节点
    /// 不会进入挂载表，此时回退到布局树按 key 查询，保证无头体检也能观测其矩形。
    #[must_use]
    pub fn widget_rect(&self, key: u64) -> Option<Rect> {
        self.mounted_widgets
            .get(&key)
            .map(|(rect, _)| *rect)
            .or_else(|| {
                self.node_keys
                    .get(&key)
                    .and_then(|node| self.world.get(*node))
                    .map(|node| node.world_rect)
            })
    }

    fn now_ms(&self) -> u64 {
        millis(Duration::from_secs_f64(self.time.max(0.0)))
    }

    fn collect_timer_requests(&mut self) {
        let (requests, cancels) = self.ctx.take_timer_requests();
        let now = self.now_ms();
        for request in requests {
            self.timers.schedule(
                request.token,
                now,
                request.delay_ms,
                request.period_ms,
                request.event,
            );
        }
        for token in cancels {
            self.timers.cancel(token);
        }
    }

    /// 获取应用状态。
    #[must_use]
    pub fn state(&self) -> &A::State {
        &self.state
    }

    /// 可变访问应用状态，供需要修改它的外壳使用。
    pub fn state_mut(&mut self) -> &mut A::State {
        &mut self.state
    }

    /// 获取应用实例本身。
    ///
    /// 窗口化外壳需要通过它访问运行时刻意不持有的部分：
    /// 字形缓存、相机以及平台适配器发布的无障碍树。
    ///
    #[must_use]
    pub fn application(&self) -> &A {
        &self.app
    }

    /// 可变访问应用实例。
    pub fn application_mut(&mut self) -> &mut A {
        &mut self.app
    }

    /// 同时可变借用应用实例与状态。
    ///
    /// 快照恢复需要成对地拿到两者：分两次借会与借用检查冲突——
    /// 第一个可变借用还活着，第二个就借不出来了。
    pub fn application_and_state_mut(&mut self) -> (&mut A, &mut A::State) {
        (&mut self.app, &mut self.state)
    }

    /// 推进时钟并归约一次 tick。
    ///
    /// 带帧时钟的外壳应调用此方法而非 [`Self::frame`] 的 `dt`，
    /// 使时间驱动型应用仅在真正需要时报告脏帧，而非每次 vsync 都重建。
    ///
    pub fn tick(&mut self, dt: f32) -> DirtyFlags {
        self.advance(dt);
        self.dispatch(AppEvent::Tick { dt })
    }

    /// 获取当前应用上下文。
    #[must_use]
    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    /// 可变访问应用上下文。
    ///
    /// 窗口 host 借此在每帧构建视图前同步窗口最大化状态等外部输入。
    pub fn context_mut(&mut self) -> &mut AppContext {
        &mut self.ctx
    }

    /// 获取最近一帧的场景。
    #[must_use]
    pub fn scene(&self) -> &SceneCollector {
        &self.collector
    }

    /// 可变访问最近一帧的场景。
    ///
    /// 窗口 host 借此将语义增量排入平台无障碍适配器。
    ///
    pub fn scene_mut(&mut self) -> &mut SceneCollector {
        &mut self.collector
    }

    /// 只读访问最近一帧的语义树。
    ///
    /// 这是 Agent 与屏幕阅读器**共享**的那棵树 —— 不存在第二套供 Agent 自学的
    /// 定制 AST。Agent 侧的一切查询（按角色找控件、按名称定位、读文本与数值）
    /// 都从这里出发。
    ///
    /// 注意：语义树在每帧的布局求解之后才完成层级重建，因此帧内调用应发生在
    /// [`Self::frame`] 返回之后。
    #[must_use]
    pub fn semantics(&self) -> &unbound_a11y::Semantics {
        &self.collector.semantics
    }

    /// 把本帧声明的语义标注写入语义树。
    ///
    /// 标注在布局求解之后应用，因此控件矩形已是最终值。已有语义节点的控件
    /// 只覆盖显式给出的字段；自绘控件（自身不产出语义节点）则据此新建节点，
    /// 从而与原生控件享有同等的语义地位。
    fn apply_semantic_intents(&mut self) {
        if self.semantic_intents.is_empty() {
            return;
        }
        for (&key, intent) in self.semantic_intents.iter() {
            let Some(node_id) = self.node_keys.get(&key).copied() else {
                continue;
            };
            let rect = self
                .world
                .get(node_id)
                .map(|node| node.world_rect)
                .unwrap_or(Rect::ZERO);
            let existing = self.collector.semantics.get(key).cloned();
            let mut node = existing.unwrap_or_default();
            intent.apply(&mut node);
            if node.role == Role::Unknown {
                // 未指定角色的标注退化为通用容器，而不是「未知」——
                // 后者在读取方眼里等同于没有这个节点。
                node.role = Role::GenericContainer;
            }
            node.bounds = rect;
            self.collector.upsert_semantic(key, node);
        }
    }

    /// 获取最近一帧的 diff。
    #[must_use]
    pub fn last_diff(&self) -> &SceneDiff {
        &self.diff
    }

    /// 获取自上一帧以来累积的脏标志。
    #[must_use]
    pub fn dirty(&self) -> DirtyFlags {
        self.dirty
    }

    /// 获取焦点管理器，供外壳接入平台焦点事件。
    #[must_use]
    pub fn focus(&self) -> &FocusManager {
        &self.focus
    }

    /// 不经过事件直接聚焦节点（或失焦）。
    ///
    /// host 将其用于窗口激活状态变更；测试用它复现运行时指针策略会解析出的焦点。
    ///
    pub fn focus_node(&mut self, node: Option<u64>) {
        let changed = match node {
            Some(node) => self.focus.focus(node, FocusCause::Programmatic),
            None => self.focus.blur(),
        };
        if changed {
            self.sync_focus_context();
            self.mark(DirtyFlags::LAYOUT);
        }
    }

    /// 将下一帧标记为完整重建。
    pub fn invalidate(&mut self) {
        self.dirty = self.dirty.merge(DirtyFlags::ALL);
        self.force_rebuild = true;
    }

    /// 移动指针，产生悬停 / 移动事件。
    pub fn pointer_move(&mut self, pos: Vec2) -> DirtyFlags {
        self.ctx.pointer = pos;
        let mut flags = DirtyFlags::PAINT;
        if let Some(event) = self.router.pointer_move(pos) {
            flags = flags.merge(self.reduce_pointer(event));
        }
        self.sync_router_context();
        self.mark(flags);
        flags
    }

    /// 借用运行时覆盖层与焦点陷阱管理器。
    #[must_use]
    pub fn overlays(&self) -> &OverlayManager {
        &self.overlays
    }

    /// 可变借用运行时覆盖层与焦点陷阱管理器。
    pub fn overlays_mut(&mut self) -> &mut OverlayManager {
        &mut self.overlays
    }

    /// 打开覆盖层，并自动管理焦点陷阱与焦点恢复。
    pub fn open_overlay(&mut self, entry: unbound_widgets::overlay::OverlayEntry) -> u64 {
        let key = self.overlays.open(entry, &mut self.focus);
        self.sync_focus_context();
        self.mark(DirtyFlags::PAINT);
        key
    }

    /// 按节点 ID 关闭覆盖层并恢复触发元素焦点。
    pub fn close_overlay(&mut self, node: u64) -> bool {
        let closed = self.overlays.close(node, &mut self.focus);
        if closed {
            self.sync_focus_context();
            self.mark(DirtyFlags::PAINT);
        }
        closed
    }

    /// 按下 `button`。
    pub fn pointer_down(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        if button == PointerButton::Left
            && self
                .overlays
                .handle_pointer_down(pos, &mut self.focus)
                .is_some()
        {
            self.sync_focus_context();
            flags = flags.merge(DirtyFlags::PAINT | DirtyFlags::LAYOUT);
        }
        let event = self.router.pointer_down_button(pos, button, self.time);
        self.ctx.pointer = pos;
        flags = flags.merge(event.map_or(DirtyFlags::NONE, |event| self.reduce_pointer(event)));
        self.sync_router_context();
        self.mark(flags.merge(DirtyFlags::PAINT));
        flags
    }

    /// 释放 `button`。
    pub fn pointer_up(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        let events = self.router.pointer_up_button(pos, button, self.time);
        self.ctx.pointer = pos;
        let mut flags = DirtyFlags::PAINT;
        for event in events {
            flags = flags.merge(self.reduce_pointer(event));
        }
        self.sync_router_context();
        self.mark(flags);
        flags
    }

    /// 取消指针捕获且不合成点击事件。
    ///
    /// 按下中途失焦的 host 必须丢弃捕获：释放事件永远不会到达，
    /// 下一次 `pointer_up` 绝不能把被放弃的按下变成一次激活。
    ///
    pub fn cancel_pointer(&mut self) -> DirtyFlags {
        let _ = self.router.cancel();
        self.sync_router_context();
        self.mark(DirtyFlags::LAYOUT);
        DirtyFlags::LAYOUT
    }

    /// 在 `pos` 处触发一次滚轮滚动。
    ///
    /// 增量按从顶层到底层的顺序遍历命中栈。每个处理器通过 [`AppContext::consume_scroll`]
    /// 报告其消费量；剩余部分继续提供给下层祖先，使滚动区域滚到底后将滚轮交给
    /// 后方页面。返回 [`Propagation::Stop`] 的处理器会立即终止链条。
    ///
    ///
    pub fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        let mut scroll_dirty = DirtyFlags::NONE;
        // 若当前存在位于 Portal / 模态层的滚动区或遮罩覆盖，背景层的滚动区不得越界抢夺滚轮。
        let has_portal_layer = self
            .router
            .regions()
            .iter()
            .any(|r| r.layer == unbound_input::PORTAL_LAYER && r.bounds.contains_point(pos));
        for &(scroll_key, viewport_rect, content_size, vertical) in self.scroll_regions.iter().rev()
        {
            let in_portal = self.node_keys.get(&scroll_key).copied().is_some_and(|id| {
                let mut cur = id;
                for _ in 0..MAX_BUBBLE_DEPTH {
                    let Some(d) = self.world.get(cur) else {
                        break;
                    };
                    if d.desc.has(NodeFlags::PORTAL_ROOT)
                        || d.desc.z_index >= PORTAL_Z_BASE
                        || self.overlays.stack().contains(d.desc.key)
                    {
                        return true;
                    }
                    if d.parent == cur {
                        break;
                    }
                    cur = d.parent;
                }
                false
            });
            if has_portal_layer && !in_portal {
                continue;
            }
            if viewport_rect.contains_point(pos) {
                let mut desc = unbound_widgets::ScrollViewDesc::new(viewport_rect, content_size);
                desc.axis = if vertical {
                    unbound_widgets::ScrollAxis::Vertical
                } else {
                    unbound_widgets::ScrollAxis::Horizontal
                };
                let step_px = Vec2::new(-delta.x * 44.0, -delta.y * 44.0);
                let mut store = self.widget_state.borrow_mut();
                let state = store.use_state_or_init::<unbound_widgets::ScrollState>(
                    scroll_key,
                    unbound_widgets::ScrollState::new,
                );
                let before = state.offset();
                state.jump_to(&desc, before + step_px);
                if (state.offset() - before).length() > 0.01 {
                    scroll_dirty = DirtyFlags::PAINT | DirtyFlags::LAYOUT;
                    drop(store);
                    self.force_rebuild = true;
                    self.mark(scroll_dirty);
                    break;
                }
            }
        }
        // 路由器的命中路径借用 `self.router`，先物化为节点键以结束可变借用，
        // 才能继续查询世界树并沿逻辑父链展开冒泡路径。
        let hit_nodes: Vec<u64> = self
            .router
            .wheel(delta, pos)
            .iter()
            .map(|region| region.node)
            .collect();
        let path: Vec<u64> = match hit_nodes.first().copied() {
            Some(top) if self.resolves_to_world_node(top) => {
                let mut path = self.ancestors_of(top);
                // 逻辑父链优先；命中栈里未挂进世界树的「裸区域」按空间栈回退补入，
                // 避免画布四叉树 / 外部注入的命中区域被整条丢弃。已能解析到世界树的
                // 节点不回退，否则浮层边界截断后背景控件又会渗透进来，破坏模态隔离。
                for node in hit_nodes.iter().skip(1) {
                    if !path.contains(node) && !self.resolves_to_world_node(*node) {
                        path.push(*node);
                    }
                }
                path
            }
            _ => hit_nodes,
        };
        if path.is_empty() {
            return self
                .dispatch(AppEvent::Wheel {
                    pos,
                    delta,
                    target: None,
                })
                .merge(scroll_dirty);
        }

        let mut remaining = delta;
        let mut flags = scroll_dirty;
        let mut trace = BubbleTrace::new("Wheel", self.inspector.enabled());
        for node in path {
            if remaining.length() <= 0.01 {
                break;
            }
            trace.visit(node);
            self.ctx.scroll_consumed = Vec2::ZERO;
            self.ctx.event_target = Some(node);
            let outcome = self.dispatch_outcome(AppEvent::Wheel {
                pos,
                delta: remaining,
                target: Some(node),
            });
            remaining -= self.ctx.scroll_consumed;
            flags = flags.merge(outcome.dirty);
            if outcome.propagation == Propagation::Stop {
                trace.stop(node);
                break;
            }
        }
        self.ctx.event_target = None;
        trace.finish(&mut self.inspector);
        flags
    }

    /// 处理原始指针事件，供自行持有路由器的外壳使用。
    pub fn pointer_event(&mut self, event: PointerEvent) -> DirtyFlags {
        self.reduce_pointer(event)
    }

    /// 处理按键按下或释放。
    pub fn key(&mut self, key: AppKey, pressed: bool, modifiers: Modifiers) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        if pressed && matches!(key, AppKey::F(12)) {
            // F12 为 DevTools 开关；绝不传递至应用。
            self.toggle_inspector();
            return DirtyFlags::PAINT;
        }

        // 先沿当前聚焦节点的逻辑父链冒泡；若某一级消费并返回 `Propagation::Stop`，
        // 则不再交由外层覆盖层 Escape、焦点导航或全局快捷键兜底截胡。
        let mut stopped = false;
        let mut dispatched_to_app = false;
        if let Some(focused_key) = self.ctx.focused {
            dispatched_to_app = true;
            let path = self.ancestors_of(focused_key);
            let mut trace = BubbleTrace::new("Key", self.inspector.enabled());
            for node in path {
                trace.visit(node);
                self.ctx.event_target = Some(node);
                let outcome = self.dispatch_outcome(AppEvent::Key {
                    key,
                    pressed,
                    modifiers,
                });
                flags = flags.merge(outcome.dirty);
                if outcome.propagation == Propagation::Stop {
                    stopped = true;
                    trace.stop(node);
                    break;
                }
            }
            self.ctx.event_target = None;
            trace.finish(&mut self.inspector);
        }

        if stopped {
            self.mark(flags);
            return flags;
        }

        if pressed {
            if key == AppKey::Escape && self.overlays.handle_escape(&mut self.focus).is_some() {
                flags = flags.merge(DirtyFlags::PAINT | DirtyFlags::LAYOUT);
                self.sync_focus_context();
            } else {
                let focus_key = match key {
                    AppKey::Tab if modifiers.shift => Some(FocusKey::ShiftTab),
                    AppKey::Tab => Some(FocusKey::Tab),
                    AppKey::Escape => Some(FocusKey::Escape),
                    _ => None,
                };
                if let Some(focus_key) = focus_key {
                    if self.focus.handle_key(focus_key).is_some() || focus_key == FocusKey::Escape {
                        flags = flags.merge(DirtyFlags::PAINT);
                    }
                    self.sync_focus_context();
                } else {
                    let roving_key = match key {
                        AppKey::Left => Some(RovingKey::Left),
                        AppKey::Right => Some(RovingKey::Right),
                        AppKey::Up => Some(RovingKey::Up),
                        AppKey::Down => Some(RovingKey::Down),
                        AppKey::Home => Some(RovingKey::Home),
                        AppKey::End => Some(RovingKey::End),
                        _ => None,
                    };
                    if let Some(rk) = roving_key {
                        if self.focus.handle_roving_key(rk).is_some() {
                            flags = flags.merge(DirtyFlags::PAINT);
                            self.sync_focus_context();
                        }
                    }
                }
            }

            // 已绑定的组合键是命令而非文本：归约它而非原始按键，
            // 可防止聚焦的文本字段同时插入该字符。
            let action =
                self.keymap
                    .feed(key.chord(modifiers), self.key_context.as_deref(), self.time);
            if let Some(action) = action {
                let flags = self.dispatch(AppEvent::Shortcut { action }).merge(flags);
                self.mark(flags);
                return flags;
            }
        }

        if !dispatched_to_app {
            flags = self
                .dispatch(AppEvent::Key {
                    key,
                    pressed,
                    modifiers,
                })
                .merge(flags);
        }
        self.mark(flags);
        flags
    }

    /// 处理文本输入。
    pub fn text(&mut self, text: impl Into<String>) -> DirtyFlags {
        self.dispatch(AppEvent::Text { text: text.into() })
    }

    /// 处理 IME 更新。
    pub fn ime(&mut self, update: ImeUpdate) -> DirtyFlags {
        self.dispatch(AppEvent::Ime { update })
    }

    /// 系统文件悬停在窗口上方。
    pub fn file_hover(&mut self, paths: Vec<std::path::PathBuf>) -> DirtyFlags {
        self.dispatch(AppEvent::FileHover(paths))
    }

    /// 系统文件拖放至窗口。
    pub fn file_drop(&mut self, paths: Vec<std::path::PathBuf>) -> DirtyFlags {
        self.dispatch(AppEvent::FileDrop(paths))
    }

    /// 系统文件悬停已取消。
    pub fn file_hover_cancelled(&mut self) -> DirtyFlags {
        self.dispatch(AppEvent::FileHoverCancelled)
    }

    /// 在运行时上下文中安装平台剪贴板提供者。
    pub fn set_clipboard(&mut self, provider: Box<dyn ClipboardProvider>) {
        self.ctx.set_clipboard(provider);
    }

    /// 处理逻辑视口尺寸调整（无头测试或直接指定逻辑尺寸时调用；同步更新物理视口基准）。
    pub fn resize(&mut self, size: Vec2) -> DirtyFlags {
        let size = Vec2::new(size.x.max(1.0), size.y.max(1.0));
        let scale = self.ctx.effective_scale();
        self.ctx.viewport = size;
        self.ctx.physical_viewport = Vec2::new(size.x * scale, size.y * scale);
        self.last_effective_scale = scale;
        self.force_rebuild = true;
        let flags = self
            .dispatch(AppEvent::Resize { size })
            .merge(DirtyFlags::LAYOUT);
        self.mark(flags);
        flags
    }

    /// 按物理像素视口尺寸与系统 DPI 缩放比同步更新运行时视口（窗口宿主专用）。
    ///
    /// 逻辑视口由 `physical_size / (os_scale * ui_scale)` 推导；当逻辑视口或有效缩放比
    /// 发生变化时立即触发 `AppEvent::Resize` 并标记完整重排，消除缩放与调整窗口大小时的帧间延迟。
    pub fn sync_physical_viewport(&mut self, physical_size: Vec2, os_scale: f32) -> DirtyFlags {
        let phys = Vec2::new(physical_size.x.max(1.0), physical_size.y.max(1.0));
        let valid_os = if os_scale.is_finite() && os_scale > 0.0 {
            os_scale.clamp(0.5, 4.0)
        } else {
            1.0
        };
        let prev_vp = self.ctx.viewport;
        let prev_scale = self.ctx.effective_scale();
        self.ctx.physical_viewport = phys;
        self.ctx.os_scale = valid_os;
        let next_vp = self.ctx.sync_viewport_from_physical();
        let next_scale = self.ctx.effective_scale();
        if (next_vp - prev_vp).length() > 0.01 || (next_scale - prev_scale).abs() > 1e-4 {
            self.last_effective_scale = next_scale;
            self.force_rebuild = true;
            let flags = self
                .dispatch(AppEvent::Resize { size: next_vp })
                .merge(DirtyFlags::ALL);
            self.mark(flags);
            flags
        } else {
            DirtyFlags::NONE
        }
    }

    /// 切换运行时语言区域并派发 [`AppEvent::LocaleChanged`]，触发完整布局与文本重绘。
    pub fn set_locale(&mut self, locale: impl Into<String>) -> DirtyFlags {
        let tag = locale.into();
        self.ctx.set_locale(tag.clone());
        self.force_rebuild = true;
        let flags = self
            .dispatch(AppEvent::LocaleChanged { locale: tag })
            .merge(DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::TEXT);
        self.mark(flags);
        flags
    }

    /// 应用是否声明跟随操作系统明暗设置。
    #[must_use]
    pub const fn follows_system_theme(&self) -> bool {
        self.ctx.follow_system_theme
    }

    /// 处理主题变更。
    pub fn set_theme(&mut self, kind: ThemeKind) -> DirtyFlags {
        self.ctx.theme = kind;
        let flags = self
            .dispatch(AppEvent::Theme { kind })
            .merge(DirtyFlags::PAINT);
        self.mark(flags);
        flags
    }

    /// 处理无障碍动作。
    pub fn a11y(&mut self, target: u64, action: A11yAction, value: Option<String>) -> DirtyFlags {
        self.dispatch(AppEvent::A11y {
            target,
            action,
            value,
        })
    }

    /// 推进时钟但不产生 `Tick` 事件。
    pub fn advance(&mut self, dt: f32) {
        let step = dt.max(0.0);
        self.time += f64::from(step);
        self.ctx.time = self.time;
        if self.ctx.scale_osd_remaining > 0.0 {
            self.ctx.scale_osd_remaining = (self.ctx.scale_osd_remaining - step).max(0.0);
            if self.ctx.scale_osd_remaining == 0.0 {
                self.force_rebuild = true;
                self.mark(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            }
        }
    }

    /// 从资产中心与字形缓存排空待上传任务。
    fn collect_atlas_uploads(&mut self) -> Vec<AtlasUpload> {
        let mut uploads = self.asset_hub.take_uploads();
        for update in self.glyph_cache.take_updates() {
            let kind = match update.target {
                unbound_text::AtlasTarget::Coverage => unbound_gpu::atlas::AtlasKind::R8,
                unbound_text::AtlasTarget::Color => unbound_gpu::atlas::AtlasKind::Rgba8,
            };
            uploads.push(AtlasUpload {
                kind,
                layer: update.layer,
                x: update.x,
                y: update.y,
                width: update.width,
                height: update.height,
                data: update.data,
            });
        }
        uploads
    }

    /// 重建场景、与上一帧做 diff，并更新路由器、焦点顺序与无障碍快照。
    ///
    ///
    /// 无脏标志且未强制重建的帧为空操作，报告 [`SceneDiff::Identical`]——零开销路径。
    ///
    pub fn frame(&mut self, dt: f32) -> FrameReport {
        self.frame_count = self.frame_count.wrapping_add(1);
        self.advance(dt);
        // 界面语言切换会改变东亚字体族（简中 / 繁中 / 日文 / 韩文的汉字字形不同）。
        // 这里在视图构建前同步给文本引擎，并强制整帧重排——否则旧字形会留在缓存里。
        if self.ctx.cjk_family != self.applied_cjk_family {
            self.applied_cjk_family = self.ctx.cjk_family;
            self.text_engine.set_cjk_family(self.applied_cjk_family);
            self.force_rebuild = true;
            self.dirty |= DirtyFlags::ALL;
        }
        // 先归约自上一帧以来到达的异步结果与定时器，
        // 使仅因它们而唤醒的帧仍会执行重建。
        self.poll_async();
        self.poll_timers();
        // 若事件归约、快捷键、设置页按钮或异步命令修改了 `ui_scale` / `os_scale`，
        // 在本帧视图构建与布局求解前立即同步逻辑视口并强制全量重排。
        let current_scale = self.ctx.effective_scale();
        if (current_scale - self.last_effective_scale).abs() > 1e-4 {
            self.last_effective_scale = current_scale;
            let resized_vp = self.ctx.sync_viewport_from_physical();
            let _ = self.dispatch(AppEvent::Resize { size: resized_vp });
            self.force_rebuild = true;
            self.dirty |= DirtyFlags::ALL;
        }
        // 已解码的资产以与归约相同的方式触发失效。
        let asset_dirty = self.asset_hub.poll(&mut self.text_engine);
        self.dirty = self.dirty.merge(asset_dirty);
        let dirty = core::mem::replace(&mut self.dirty, DirtyFlags::NONE);
        if dirty == DirtyFlags::NONE && !self.force_rebuild {
            return FrameReport {
                diff: SceneDiff::Identical,
                dirty,
                primitives: self.collector.primitives.len(),
                bytes: 0,
                aux_changed: false,
                backdrops: self.backdrops.clone(),
                atlas_uploads: self.collect_atlas_uploads(),
            };
        }

        // 焦点顺序在重建完成后（见下方 `set_order`）统一发布：此处不得提前用上一帧的
        // 顺序表裁剪焦点，否则同一帧内由事件请求聚焦、本帧才挂载的控件（如刚打开的命令
        // 面板搜索框）会因「不在上一帧顺序表里」被立刻清空焦点。`Tab` 事件在帧前归约时
        // 读取的仍是上一帧发布的顺序表，语义不受影响。
        self.sync_focus_context();

        self.collector.reset(ROOT);
        self.pending.clear();
        self.semantic_intents.clear();
        self.widget_state.borrow_mut().begin_frame();
        for old_ephemeral in self.ephemeral_nodes.drain(..) {
            if self.world.get(old_ephemeral).is_some() {
                self.world.remove(old_ephemeral);
            }
        }
        self.seen_keys.clear();
        let root = self.world.root();
        self.world.set_desc(
            root,
            RawNodeDesc::container().size(self.ctx.viewport.x, self.ctx.viewport.y),
        );

        self.title_bar_bindings.clear();
        let view_ctx = ViewContext {
            viewport: self.ctx.viewport,
            theme: self.ctx.theme,
            hovered: self.router.hover(),
            pressed: self.router.pointer().pressed(),
            focused: self.ctx.focused,
            focus_visible: self.ctx.focus_visible,
            force: self.force_rebuild,
            window_maximized: self.ctx.window_maximized,
            window_focused: self.ctx.window_focused,
            locale: self.ctx.i18n.borrow().active_locale_enum(),
            direction: self.ctx.i18n.borrow().direction(),
            os_scale: self.ctx.os_scale,
            ui_scale: self.ctx.ui_scale,
            effective_scale: self.ctx.effective_scale(),
            scale_osd_remaining: self.ctx.scale_osd_remaining,
        };
        let mut focus_groups = Vec::new();
        let mut view = ViewBuilder {
            collector: &mut self.collector,
            ctx: view_ctx,
            world: &mut self.world,
            assets: &mut self.asset_hub,
            parent: root,
            pending: &mut self.pending,
            widget_state: Rc::clone(&self.widget_state),
            i18n: Rc::clone(&self.ctx.i18n),
            node_keys: &mut self.node_keys,
            seen_keys: &mut self.seen_keys,
            ephemeral_nodes: &mut self.ephemeral_nodes,
            focus_groups: &mut focus_groups,
            title_bar_bindings: &mut self.title_bar_bindings,
            semantic_intents: &mut self.semantic_intents,
            depth: 0,
            z_base: 0,
        };
        self.app.view(&self.state, &mut view);
        let vanished: Vec<(u64, NodeId)> = self
            .node_keys
            .iter()
            .filter(|(k, _)| !self.seen_keys.contains(k))
            .map(|(k, v)| (*k, *v))
            .collect();
        for (k, node) in vanished {
            self.node_keys.remove(&k);
            if self.world.get(node).is_some() {
                self.world.remove(node);
            }
        }
        self.layout_pending();
        // 语义标注在布局求解之后应用：控件的最终矩形此时已确定。
        self.apply_semantic_intents();
        // 语义层级依据世界树重建，使 Agent 看到的嵌套与界面视觉一致。
        self.collector.rebuild_semantic_hierarchy(&self.world);
        self.dispatch_lifecycle();

        // 发布新的命中区域与焦点顺序。命中区域归属表与世界树同帧更新，
        // 使合成子命中键（sub_hits）能被映射回逻辑父链参与冒泡。
        let regions = core::mem::take(&mut self.collector.regions);
        self.hit_owners = core::mem::take(&mut self.collector.hit_owners)
            .into_iter()
            .collect();
        self.router.set_regions(regions);
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.focus.clear_groups();
        for group in focus_groups {
            self.focus.register_group(group);
        }
        self.sync_focus_context();

        // DevTools 覆盖层 + 调试检查器循环。两者均读取本帧刚产出的 world 与命中区域，
        // 且均追加到图元列表末尾，使调试通道最后绘制。
        //
        let inspector_active = self.inspector.enabled() || self.debug_sanitizers;
        if inspector_active {
            self.inspector.tick(dt);
            self.inspector
                .update(&self.world, self.router.regions(), self.ctx.pointer);
            if self.debug_sanitizers {
                let frame = inspector::capture_frame(&self.world, self.ctx.viewport);
                let report = unbound_headless::Harness::run(&frame);
                self.inspector.set_diagnostics(&report.diagnostics);
            } else {
                self.inspector.clear_diagnostics();
            }
            if self.inspector.enabled() {
                let scene = self.inspector.emit(self.ctx.viewport);
                self.collector.primitives.extend(scene.primitives);
                self.collector.payloads.extend(scene.payloads);
                let panel_w = inspector::PANEL_WIDTH.min(self.ctx.viewport.x * 0.45);
                let panel_x = self.ctx.viewport.x - panel_w + 12.0;
                let scale = self.ctx.effective_scale();
                let style = TextStyle::sized(11.0 * scale);
                for (index, line) in scene.lines.iter().enumerate().take(16) {
                    let shaped = self.text_engine.shape(line, &style);
                    let origin = Vec2::new(panel_x * scale, (16.0 + index as f32 * 18.0) * scale);
                    let mut quads = self.glyph_cache.quads_for(
                        &mut self.text_engine,
                        &shaped,
                        origin,
                        400,
                        false,
                    );
                    for quad in &mut quads {
                        if (scale - 1.0).abs() > 1e-6 {
                            quad.rect.origin.x /= scale;
                            quad.rect.origin.y /= scale;
                            quad.rect.size.x /= scale;
                            quad.rect.size.y /= scale;
                        }
                        let payload_idx = self.collector.payloads.len() as u32;
                        self.collector.payloads.push(GpuPayload {
                            p0: [quad.uv[0], quad.uv[1]],
                            p1: [quad.uv[2], quad.uv[3]],
                            p2: [0.0, 0.0],
                            line_width: 0.0,
                            extra: 0,
                            flags: 0,
                            _reserved: [0; 3],
                        });
                        let prim_type = if quad.color {
                            PrimType::GlyphColor
                        } else {
                            PrimType::Glyph
                        };
                        let glyph = GpuPrimitiveInstance {
                            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
                            size: [quad.rect.size.x, quad.rect.size.y],
                            clip_idx: CLIP_NONE,
                            border_token_or_tex: quad.layer,
                            extra_payload_idx: payload_idx,
                            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(u16::MAX, 1.0),
                            fill_token_or_uv: u32::from(unbound_theme::Token::TEXT_PRIMARY.raw()),
                            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                                prim_type,
                                BlendMode::Normal,
                                SubElement::Content as u16,
                            ) | if quad.msdf {
                                u32::from(PrimFlags::MSDF.bits()) << 16
                            } else {
                                0
                            },
                            ..Default::default()
                        };
                        self.collector.primitives.push(glyph);
                    }
                }
            }
        }

        // 仅图元*索引指向*的表决定切片修补（slice patch）能否修复图元池。
        // 负载变更（字形、渐变）通过 `aux_changed` 单独上传，因此在文本字段中
        // 打字可以持续修补图元池，而无需重发整个图元池。
        //
        let structure_changed = self.collector.transforms != self.previous_transforms
            || self.collector.clips != self.previous_clips;
        let aux_changed = self.collector.payloads != self.previous_payloads;
        let diff = diff_primitives(
            &self.previous_primitives,
            &self.collector.primitives,
            structure_changed || self.force_rebuild,
        );
        let bytes = diff.bytes(self.collector.primitives.len());

        self.previous_primitives.clear();
        self.previous_primitives
            .extend_from_slice(&self.collector.primitives);
        self.previous_payloads.clear();
        self.previous_payloads
            .extend_from_slice(&self.collector.payloads);
        self.previous_transforms.clear();
        self.previous_transforms
            .extend_from_slice(&self.collector.transforms);
        self.previous_clips.clear();
        self.previous_clips.extend_from_slice(&self.collector.clips);

        self.diff = diff.clone();
        self.backdrops.clear();
        self.backdrops.extend_from_slice(&self.collector.backdrops);
        // 检查器会闪烁并跟随指针，因此它的存在会强制下一帧重建，
        // 即便没有其他元素变脏。
        self.force_rebuild = inspector_active;

        FrameReport {
            diff,
            dirty,
            primitives: self.collector.primitives.len(),
            bytes,
            aux_changed,
            backdrops: self.backdrops.clone(),
            atlas_uploads: self.collect_atlas_uploads(),
        }
    }

    /// 资产中心，用于提交图像与字体。
    #[must_use]
    pub fn asset_hub(&self) -> &AssetHub {
        &self.asset_hub
    }

    /// 可变访问资产中心。
    pub fn asset_hub_mut(&mut self) -> &mut AssetHub {
        &mut self.asset_hub
    }

    /// 最近一帧的布局 world。
    #[must_use]
    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    /// 获取当前帧所有活跃滚动区域的 `(scroll_key, viewport_rect, content_size, vertical)` 清单。
    #[must_use]
    pub fn scroll_regions(&self) -> &[(u64, Rect, Vec2, bool)] {
        &self.scroll_regions
    }

    /// 借用路由状态，供测试与调试检查命中区域。
    #[must_use]
    pub fn router(&self) -> &Router {
        &self.router
    }

    /// 取出并清空本帧经事件冒泡产出的待执行窗口控制命令。
    pub fn drain_window_commands(&mut self) -> Vec<WindowCommand> {
        core::mem::take(&mut self.window_commands)
    }

    /// 同步原生窗口激活聚焦状态，并在变化时触发重绘以更新标题栏失焦视觉。
    pub fn set_window_focused(&mut self, focused: bool) {
        if self.ctx.window_focused != focused {
            self.ctx.window_focused = focused;
            self.force_rebuild = true;
            self.mark(DirtyFlags::PAINT);
        }
    }

    /// 返回当前激活标题栏的（最小化、最大化、关闭）按钮节点键，供平台同步原生热区。
    #[must_use]
    pub fn primary_title_bar_buttons(&self) -> Option<(u64, u64, u64)> {
        self.title_bar_bindings
            .values()
            .next()
            .map(|b| (b.minimize_id, b.maximize_id, b.close_id))
    }

    /// 应用实例本身，供测试与调试覆盖层检查。
    #[must_use]
    pub fn app(&self) -> &A {
        &self.app
    }

    /// 最近一次求解过程的布局统计信息。
    #[must_use]
    pub fn last_layout_stats(&self) -> LayoutStats {
        self.last_layout_stats
    }

    /// 求解声明式树并发射延迟控件。
    ///
    /// 第 1 遍协调 world 并预测量控件；第 2 遍求解整棵树（拓扑/视口变更时）
    /// 或通过 [`LayoutEngine::solve_boundaries`] 仅求解脏重布局边界，
    /// 随后在各自解析出的矩形位置发射每个控件。
    ///
    fn layout_pending(&mut self) {
        if self.pending.is_empty() && !self.world.topology_dirty() {
            let boundaries = self.world.dirty_boundaries();
            if boundaries.is_empty() {
                self.last_layout_stats = LayoutStats::default();
                return;
            }
            self.last_layout_stats =
                self.layout
                    .solve_boundaries(&mut self.world, &boundaries, self.ctx.viewport);
            self.world.clear_dirty();
            return;
        }

        let raster_scale = self.ctx.effective_scale();
        self.glyph_cache.begin_frame(self.frame_count);

        // 第 1 遍：使用 TextCtx 预测量控件
        {
            let mut text_ctx = unbound_widgets::TextCtx {
                engine: &mut self.text_engine,
                cache: &mut self.glyph_cache,
                atlas_clock: self.frame_count,
                raster_scale,
            };
            for entry in &self.pending {
                let intrinsic = match &entry.spec {
                    WidgetSpec::UiWidget(w) => Some(w.intrinsic_size_erased(&mut text_ctx)),
                    WidgetSpec::Button(b) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(b, &mut text_ctx))
                    }
                    WidgetSpec::Checkbox(cb) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(cb, &mut text_ctx))
                    }
                    WidgetSpec::Switch(sw) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(sw, &mut text_ctx))
                    }
                    WidgetSpec::Slider(sl) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(sl, &mut text_ctx))
                    }
                    WidgetSpec::Progress { desc, .. } => Some(
                        unbound_widgets::UiWidget::intrinsic_size(desc, &mut text_ctx),
                    ),
                    WidgetSpec::Badge(bg) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(bg, &mut text_ctx))
                    }
                    WidgetSpec::Divider(div) => Some(unbound_widgets::UiWidget::intrinsic_size(
                        div,
                        &mut text_ctx,
                    )),
                    WidgetSpec::Alert(al) => {
                        Some(unbound_widgets::UiWidget::intrinsic_size(al, &mut text_ctx))
                    }
                    WidgetSpec::TextInline { text, style, .. } => {
                        if text.is_empty() {
                            Some(Vec2::new(0.0, (style.font_size * 1.35).ceil()))
                        } else {
                            let measured =
                                text_ctx.measure_line(text, style.font_size, style.weight);
                            Some(Vec2::new(
                                measured.x.ceil() + 2.0,
                                measured.y.max(style.font_size * 1.35).ceil(),
                            ))
                        }
                    }
                    WidgetSpec::Text { .. }
                    | WidgetSpec::BoxSurface { .. }
                    | WidgetSpec::ScrollScopeBegin { .. }
                    | WidgetSpec::ScrollScopeEnd => None,
                };
                if let Some(size) = intrinsic {
                    if let Some(data) = self.world.get_mut(entry.node) {
                        data.measure = Some(size);
                    }
                }
            }
        }

        let viewport = self.ctx.viewport;
        let root = self.world.root();
        let texts: Vec<(NodeId, String, TextStyle)> = self
            .pending
            .iter()
            .filter_map(|entry| match &entry.spec {
                WidgetSpec::Text { text, style } => Some((entry.node, text.clone(), style.clone())),
                _ => None,
            })
            .collect();
        if !self.layout_initialized || self.world.topology_dirty() || self.force_rebuild {
            let world = &mut self.world;
            let text_engine = &mut self.text_engine;
            self.last_layout_stats = self.layout.solve_with_measurer(
                world,
                root,
                viewport,
                |id, known_width, available| {
                    let Some((_, text, style)) = texts.iter().find(|(node, _, _)| *node == id)
                    else {
                        return Vec2::ZERO;
                    };
                    let space = match available.width {
                        unbound_layout::AvailableSpace::Definite(width) => Some(width),
                        _ => None,
                    };
                    let width = known_width.or(space).unwrap_or(viewport.x).max(1.0);
                    let wrapped = text_engine.wrap(text, style, width);
                    Vec2::new(wrapped.width, wrapped.height)
                },
            );
            self.layout_initialized = true;
            self.world.clear_dirty();
        } else {
            let boundaries = self.world.dirty_boundaries();
            if !boundaries.is_empty() {
                self.last_layout_stats =
                    self.layout
                        .solve_boundaries(&mut self.world, &boundaries, viewport);
                self.world.clear_dirty();
            } else {
                self.last_layout_stats = LayoutStats::default();
            }
        }

        self.mounted_widgets.clear();
        self.scroll_regions.clear();
        let pending = core::mem::take(&mut self.pending);
        for entry in &pending {
            let rect = self
                .world
                .get(entry.node)
                .map(|node| node.world_rect)
                .unwrap_or(Rect::ZERO);
            let phase = if self.ctx.pressed == Some(entry.key) {
                unbound_widgets::PointerPhase::Pressed
            } else if self.ctx.hovered == Some(entry.key) {
                unbound_widgets::PointerPhase::Hovered
            } else {
                unbound_widgets::PointerPhase::Idle
            };
            let state = ControlState {
                phase,
                focused: self.ctx.focused == Some(entry.key) && self.ctx.focus_visible,
                disabled: false,
                selected: false,
            };
            let mut text_ctx = unbound_widgets::TextCtx {
                engine: &mut self.text_engine,
                cache: &mut self.glyph_cache,
                atlas_clock: self.frame_count,
                raster_scale,
            };
            let apply_z_base = |mut out: WidgetOutput, z_base: u16| -> WidgetOutput {
                if i32::from(z_base) >= PORTAL_Z_BASE {
                    for prim in &mut out.primitives {
                        let new_z = prim.depth_u16().saturating_add(z_base);
                        let op = prim.opacity();
                        prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(new_z, op);
                    }
                    if let Some(hit) = out.hit.as_mut() {
                        hit.layer = unbound_input::PORTAL_LAYER;
                        hit.z = hit.z.saturating_add(u32::from(z_base));
                    }
                    for sub_hit in &mut out.sub_hits {
                        sub_hit.layer = unbound_input::PORTAL_LAYER;
                        sub_hit.z = sub_hit.z.saturating_add(u32::from(z_base));
                    }
                }
                out
            };
            match &entry.spec {
                WidgetSpec::UiWidget(widget) => {
                    let scrolled_rect = rect.translate(self.collector.active_offset());
                    self.mounted_widgets
                        .insert(entry.key, (scrolled_rect, Arc::clone(widget)));
                    widget.ensure_state_erased(entry.key, &mut self.widget_state.borrow_mut());
                    let store = self.widget_state.borrow();
                    let output = apply_z_base(
                        widget.emit_laid_out_erased(entry.key, rect, state, &store, &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Button(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Checkbox(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &desc.state, &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Switch(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &desc.on, &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Slider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &desc.value, &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Progress {
                    desc,
                    state: pstate,
                } => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, pstate, &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Badge(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Divider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Alert(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = apply_z_base(
                        desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx),
                        entry.z_base,
                    );
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::BoxSurface {
                    fill,
                    border,
                    border_width,
                    radii,
                    shadow_blur,
                    z,
                    hit_scrim,
                } => {
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        let mut primitives = Vec::with_capacity(2);
                        let fill_tok = unbound_theme::Token::new(*fill);
                        let border_tok = unbound_theme::Token::new(*border);
                        if *shadow_blur > 0.0 {
                            primitives.push(unbound_widgets::primitive::instance(
                                rect,
                                *radii,
                                unbound_theme::Token::NONE,
                                unbound_theme::Token::NONE,
                                0.0,
                                *shadow_blur,
                                PrimType::Shadow,
                                SubElement::Shadow,
                                *z,
                            ));
                        }
                        primitives.push(unbound_widgets::primitive::instance(
                            rect,
                            *radii,
                            fill_tok,
                            border_tok,
                            *border_width,
                            0.0,
                            PrimType::RoundedRect,
                            SubElement::Background,
                            *z,
                        ));
                        let hit = if *hit_scrim {
                            Some(unbound_input::HitRegion::new(
                                entry.key,
                                2,
                                u32::from(*z),
                                rect,
                            ))
                        } else {
                            None
                        };
                        self.collector.push(
                            entry.key,
                            WidgetOutput {
                                primitives,
                                hit,
                                ..Default::default()
                            },
                        );
                    }
                }
                WidgetSpec::ScrollScopeBegin {
                    scroll_key,
                    content_node,
                    vertical,
                } => {
                    let content_size = self
                        .world
                        .get(*content_node)
                        .map(|n| n.world_rect.size)
                        .unwrap_or(rect.size);
                    let mut scroll_desc = unbound_widgets::ScrollViewDesc::new(rect, content_size);
                    scroll_desc.axis = if *vertical {
                        unbound_widgets::ScrollAxis::Vertical
                    } else {
                        unbound_widgets::ScrollAxis::Horizontal
                    };
                    scroll_desc.z = entry.z_base.saturating_add(740);
                    let scroll_state = {
                        let mut store = self.widget_state.borrow_mut();
                        let st = store.use_state_or_init::<unbound_widgets::ScrollState>(
                            *scroll_key,
                            unbound_widgets::ScrollState::new,
                        );
                        let clamped = scroll_desc.clamp_offset(st.offset());
                        if (clamped - st.offset()).length() > 0.01 {
                            st.jump_to(&scroll_desc, clamped);
                        }
                        st.reveal_scrollbar();
                        st.clone()
                    };
                    self.scroll_regions
                        .push((*scroll_key, rect, content_size, *vertical));
                    let mut sv_out = unbound_widgets::emit_scroll_view(
                        *scroll_key,
                        &scroll_desc,
                        &scroll_state,
                        state,
                    );
                    if i32::from(entry.z_base) >= PORTAL_Z_BASE {
                        if let Some(hit) = sv_out.output.hit.as_mut() {
                            hit.layer = unbound_input::PORTAL_LAYER;
                            hit.z = hit.z.saturating_add(u32::from(entry.z_base));
                        }
                        for hit in &mut sv_out.hits {
                            hit.layer = unbound_input::PORTAL_LAYER;
                            hit.z = hit.z.saturating_add(u32::from(entry.z_base));
                        }
                    }
                    let clip = sv_out.clip;
                    let transform = sv_out.transform;
                    sv_out.push_into(&mut self.collector, *scroll_key);
                    self.collector.push_clip(clip, rect);
                    self.collector.push_transform(transform);
                }
                WidgetSpec::ScrollScopeEnd => {
                    self.collector.pop_transform();
                    self.collector.pop_clip();
                }
                WidgetSpec::TextInline { text, style, color } => {
                    if !text.is_empty() && rect.width() > 0.0 && rect.height() > 0.0 {
                        let quads = text_ctx.shape_line(
                            text,
                            rect,
                            style.font_size,
                            style.weight,
                            unbound_widgets::AlignX::Left,
                            unbound_widgets::AlignY::Middle,
                        );
                        let mut primitives = Vec::with_capacity(quads.len());
                        let mut payloads = Vec::with_capacity(quads.len());
                        let z = entry.z_base.saturating_add(705);
                        unbound_widgets::primitive::glyphs(
                            &quads,
                            *color,
                            z,
                            &mut primitives,
                            &mut payloads,
                        );
                        self.collector.push(
                            entry.key,
                            WidgetOutput {
                                primitives,
                                payloads,
                                // 只读文本同样进入语义树：屏幕阅读器要靠它朗读内容，
                                // Agent 要靠它读到界面上的数值与状态。缺少文本节点的
                                // 界面在这两者眼里都是空的。
                                a11y: Some(A11yNode::static_text(text.clone(), rect)),
                                ..Default::default()
                            },
                        );
                    }
                }
                WidgetSpec::Text { text, style } => {
                    if !text.is_empty() && rect.width() > 0.0 && rect.height() > 0.0 {
                        let wrapped = text_ctx.engine.wrap(text, style, rect.width().max(1.0));
                        let line_h = wrapped.line_height.max(style.font_size * 1.3);
                        let mut primitives = Vec::new();
                        let mut payloads = Vec::new();
                        let z = entry.z_base.saturating_add(705);
                        for (idx, line) in wrapped.lines.iter().enumerate() {
                            let start = (line.start as usize).min(text.len());
                            let end = (line.end as usize).min(text.len()).max(start);
                            if let Some(slice) = text.get(start..end) {
                                let trimmed = slice.trim_end_matches(['\r', '\n']);
                                if trimmed.is_empty() {
                                    continue;
                                }
                                let line_r = Rect::new(
                                    rect.origin.x,
                                    rect.origin.y + idx as f32 * line_h,
                                    rect.width(),
                                    line_h,
                                );
                                let quads = text_ctx.shape_line(
                                    trimmed,
                                    line_r,
                                    style.font_size,
                                    style.weight,
                                    unbound_widgets::AlignX::Left,
                                    unbound_widgets::AlignY::Middle,
                                );
                                unbound_widgets::primitive::glyphs(
                                    &quads,
                                    unbound_theme::Token::TEXT_PRIMARY,
                                    z,
                                    &mut primitives,
                                    &mut payloads,
                                );
                            }
                        }
                        self.collector.push(
                            entry.key,
                            WidgetOutput {
                                primitives,
                                payloads,
                                // 多行文本同样进入语义树：整段内容作为一个节点，
                                // Agent 一次读取即可，不必逐行拼接。
                                a11y: Some(A11yNode::static_text(text.clone(), rect)),
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }
        self.pending = pending;
    }

    fn reduce_pointer(&mut self, event: PointerEvent) -> DirtyFlags {
        match event {
            PointerEvent::Down {
                target,
                position,
                button,
                click_count,
            } => {
                if button == PointerButton::Left {
                    for &(scroll_key, viewport_rect, content_size, vertical) in
                        self.scroll_regions.iter().rev()
                    {
                        let mut desc =
                            unbound_widgets::ScrollViewDesc::new(viewport_rect, content_size);
                        desc.axis = if vertical {
                            unbound_widgets::ScrollAxis::Vertical
                        } else {
                            unbound_widgets::ScrollAxis::Horizontal
                        };
                        let on_scrollbar = (vertical
                            && content_size.y > viewport_rect.height()
                            && desc.track_rect(false).contains_point(position))
                            || (!vertical
                                && content_size.x > viewport_rect.width()
                                && desc.track_rect(true).contains_point(position))
                            || target == Some(scroll_key ^ (1 << 63))
                            || target == Some(scroll_key ^ (1 << 62));
                        if on_scrollbar {
                            let handled = {
                                let mut store = self.widget_state.borrow_mut();
                                let st = store.use_state_or_init::<unbound_widgets::ScrollState>(
                                    scroll_key,
                                    unbound_widgets::ScrollState::new,
                                );
                                let changed = st.reduce_pointer(
                                    &desc,
                                    unbound_widgets::ScrollPointerEvent::Press(position),
                                );
                                if changed && st.dragging().is_none() {
                                    let tgt = st.target();
                                    st.jump_to(&desc, tgt);
                                }
                                changed
                            };
                            if handled {
                                self.last_pointer_down_target = None;
                                self.last_pointer_down_path.clear();
                                self.force_rebuild = true;
                                let dirty = DirtyFlags::PAINT | DirtyFlags::LAYOUT;
                                self.mark(dirty);
                                return dirty;
                            }
                        }
                    }
                }
                // 路径在按下那一刻固定：释放时无需重算父链，也不受帧间树变化影响。
                let down_path = match target {
                    Some(node) => self.ancestors_of(node),
                    None => Vec::new(),
                };
                self.last_pointer_down_target = target;
                self.last_pointer_down_path = down_path;
                match target {
                    Some(node) => self.dispatch_bubbling("PointerDown", node, |ancestor| {
                        AppEvent::PointerDown {
                            pos: position,
                            target: Some(ancestor),
                            button,
                            click_count,
                        }
                    }),
                    None => self.dispatch(AppEvent::PointerDown {
                        pos: position,
                        target: None,
                        button,
                        click_count,
                    }),
                }
            }
            // 高频连续事件与拖拽保持单目标直达，不沿祖先链冒泡。
            PointerEvent::Move {
                position,
                delta: _,
                target,
            } => self.dispatch(AppEvent::PointerMove {
                pos: position,
                hovered: target,
            }),
            PointerEvent::DragStart { target, position } => self.dispatch(AppEvent::DragStart {
                node: target,
                pos: position,
            }),
            PointerEvent::Drag {
                target,
                position,
                delta,
            } => {
                for &(scroll_key, viewport_rect, content_size, vertical) in
                    self.scroll_regions.iter().rev()
                {
                    let mut desc =
                        unbound_widgets::ScrollViewDesc::new(viewport_rect, content_size);
                    desc.axis = if vertical {
                        unbound_widgets::ScrollAxis::Vertical
                    } else {
                        unbound_widgets::ScrollAxis::Horizontal
                    };
                    let dragged = {
                        let mut store = self.widget_state.borrow_mut();
                        if let Some(st) = store.get_mut::<unbound_widgets::ScrollState>(scroll_key)
                        {
                            st.reduce_pointer(
                                &desc,
                                unbound_widgets::ScrollPointerEvent::Drag(position),
                            )
                        } else {
                            false
                        }
                    };
                    if dragged {
                        self.force_rebuild = true;
                        let dirty = DirtyFlags::PAINT | DirtyFlags::LAYOUT;
                        self.mark(dirty);
                        return dirty;
                    }
                }
                self.dispatch(AppEvent::Drag {
                    node: target,
                    pos: position,
                    delta,
                })
            }
            PointerEvent::Up {
                target,
                position,
                button,
            } => {
                let mut scroll_released = false;
                for &(scroll_key, viewport_rect, content_size, vertical) in
                    self.scroll_regions.iter().rev()
                {
                    let mut desc =
                        unbound_widgets::ScrollViewDesc::new(viewport_rect, content_size);
                    desc.axis = if vertical {
                        unbound_widgets::ScrollAxis::Vertical
                    } else {
                        unbound_widgets::ScrollAxis::Horizontal
                    };
                    let mut store = self.widget_state.borrow_mut();
                    if let Some(st) = store.get_mut::<unbound_widgets::ScrollState>(scroll_key) {
                        if st.reduce_pointer(&desc, unbound_widgets::ScrollPointerEvent::Release) {
                            scroll_released = true;
                        }
                    }
                }
                if scroll_released {
                    self.last_pointer_down_target = None;
                    self.last_pointer_down_path.clear();
                    let dirty = DirtyFlags::PAINT;
                    self.mark(dirty);
                    return dirty;
                }
                let mut flags = match target {
                    Some(node) => {
                        self.dispatch_bubbling("PointerUp", node, |ancestor| AppEvent::PointerUp {
                            pos: position,
                            target: Some(ancestor),
                            button,
                        })
                    }
                    None => self.dispatch(AppEvent::PointerUp {
                        pos: position,
                        target: None,
                        button,
                    }),
                };
                // 仅当释放位置仍在被按下的节点上时，辅助点击才打开菜单。
                //
                if button == PointerButton::Right {
                    if let Some(node) = target {
                        if self.router.hover() == Some(node) {
                            flags = flags.merge(self.dispatch_bubbling(
                                "ContextMenu",
                                node,
                                |ancestor| AppEvent::ContextMenu {
                                    node: ancestor,
                                    pos: position,
                                },
                            ));
                        }
                    }
                }
                flags
            }
            PointerEvent::Click {
                target,
                position,
                count,
            } => {
                let down_matches = self.last_pointer_down_target.is_none_or(|down| {
                    down == target || self.last_pointer_down_path.contains(&target)
                });
                self.last_pointer_down_target = None;
                self.last_pointer_down_path.clear();
                if !down_matches {
                    return DirtyFlags::NONE;
                }
                self.dispatch_bubbling("Click", target, |ancestor| AppEvent::Click {
                    node: ancestor,
                    pos: position,
                    count,
                })
            }
            PointerEvent::DoubleClick { target, position } => {
                self.dispatch_bubbling("DoubleClick", target, |ancestor| AppEvent::DoubleClick {
                    node: ancestor,
                    pos: position,
                })
            }
            PointerEvent::CaptureLost { .. } => {
                self.last_pointer_down_target = None;
                self.last_pointer_down_path.clear();
                DirtyFlags::NONE
            }
        }
    }

    /// 向应用报告挂载 / 卸载差集与焦点变更。
    fn dispatch_lifecycle(&mut self) {
        let changes = {
            // 钩子运行期间不得持有借用：钩子可能会再次请求同一键的状态。
            //
            self.widget_state.borrow_mut().end_frame()
        };
        let mut flags = DirtyFlags::NONE;
        for key in &changes.unmounted {
            flags = flags.merge(
                self.app
                    .lifecycle(&mut self.state, LifecycleEvent::Unmounted(*key)),
            );
        }
        for key in &changes.mounted {
            flags = flags.merge(
                self.app
                    .lifecycle(&mut self.state, LifecycleEvent::Mounted(*key)),
            );
        }

        let focused = self.ctx.focused;
        if focused != self.last_focused {
            if let Some(previous) = self.last_focused {
                flags = flags.merge(
                    self.app
                        .lifecycle(&mut self.state, LifecycleEvent::Blurred(previous)),
                );
            }
            if let Some(current) = focused {
                flags = flags.merge(
                    self.app
                        .lifecycle(&mut self.state, LifecycleEvent::Focused(current)),
                );
            }
            self.last_focused = focused;
        }
        self.mark(flags);
    }

    /// 在指针/键盘路径之外派发单个已解析事件。
    ///
    /// 合成事件的嵌入方（测试、远程控制、脚本桥接）使用此方法，
    /// 而无需深入路由器内部。返回的脏标志已标记在运行时上，因此下一帧 [`Self::frame`] 会重建。
    ///
    pub fn dispatch(&mut self, event: AppEvent) -> DirtyFlags {
        self.dispatch_outcome(event).dirty
    }

    /// 读取最近一次记录的事件冒泡轨迹（仅在检查器开启时采集）。
    #[must_use]
    pub fn last_event_trace(&self) -> Option<&EventTrace> {
        self.inspector.last_event()
    }

    /// 命中键能否解析到世界树中的逻辑节点（即能否展开逻辑父链）。
    ///
    /// 只有 `node_keys` / `UiWorld` 中的节点、或经归属表映射到这类节点的多部件子键
    /// 才算；仅通过 `view.push` / `set_regions` 注入的裸区域必须留给空间栈回退。
    fn resolves_to_world_node(&self, key: u64) -> bool {
        self.node_keys.contains_key(&key)
            || self.world.find_key(key).is_some()
            || self.hit_owners.get(&key).is_some_and(|owner| {
                self.node_keys.contains_key(owner) || self.world.find_key(*owner).is_some()
            })
    }

    /// 判断节点是否构成 Portal / 浮层边界，防止事件继续穿透至背景页面。
    ///
    /// 边界由 [`NodeFlags::PORTAL_ROOT`] 或浮层管理器显式声明；`z_index` 只管绘制深度，
    /// 不再参与判定——否则任何为了盖住兄弟节点而抬高 `z_index` 的普通容器都会提前截断冒泡。
    fn is_portal_boundary(&self, key: u64, id: NodeId) -> bool {
        if self.overlays.stack().contains(key) {
            return true;
        }
        self.world
            .get(id)
            .is_some_and(|data| data.desc.has(NodeFlags::PORTAL_ROOT))
    }

    /// 沿 `UiWorld` 逻辑父链由内向外收集 `[target, owner?, parent, ...]`，遇到浮层边界即截断。
    ///
    /// 多部件控件的 `sub_hits` 使用 `node ^ (1 << 63)` 一类合成键，本身不在世界树中；
    /// 先借助命中归属表把它接到所属控件节点，否则这类子部件永远收不到冒泡。
    fn ancestors_of(&self, start_key: u64) -> Vec<u64> {
        let mut path = vec![start_key];
        let owner_key = self.hit_owners.get(&start_key).copied();
        let Some(mut curr_id) = self
            .node_keys
            .get(&start_key)
            .copied()
            .or_else(|| owner_key.and_then(|k| self.node_keys.get(&k).copied()))
            .or_else(|| self.world.find_key(start_key))
            .or_else(|| owner_key.and_then(|k| self.world.find_key(k)))
        else {
            return path;
        };
        if let Some(owner) = owner_key.filter(|k| *k != start_key && !path.contains(k)) {
            path.push(owner);
        }
        if self.is_portal_boundary(start_key, curr_id) {
            return path;
        }
        let mut seen = FxHashSet::default();
        seen.insert(start_key);
        if let Some(owner) = owner_key {
            seen.insert(owner);
        }
        for _ in 0..MAX_BUBBLE_DEPTH {
            let Some(data) = self.world.get(curr_id) else {
                break;
            };
            let parent_id = data.parent;
            if parent_id == curr_id {
                break;
            }
            let Some(parent_data) = self.world.get(parent_id) else {
                break;
            };
            let parent_key = parent_data.desc.key;
            if parent_key != 0 && seen.insert(parent_key) {
                path.push(parent_key);
                if self.is_portal_boundary(parent_key, parent_id) {
                    break;
                }
            }
            curr_id = parent_id;
        }
        path
    }

    /// 沿逻辑父链派发离散事件，直到所有祖先处理完毕或某一级返回 [`Propagation::Stop`]。
    fn dispatch_bubbling(
        &mut self,
        kind: &'static str,
        target: u64,
        mut make_event: impl FnMut(u64) -> AppEvent,
    ) -> DirtyFlags {
        let path = self.ancestors_of(target);
        let mut trace = BubbleTrace::new(kind, self.inspector.enabled());
        let mut flags = DirtyFlags::NONE;
        let mut stopped = false;
        let mut hit_title_bar: Option<u64> = None;
        for node in path {
            trace.visit(node);
            if self.title_bar_bindings.contains_key(&node) {
                hit_title_bar = Some(node);
            }
            self.ctx.event_target = Some(node);
            let outcome = self.dispatch_outcome(make_event(node));
            flags = flags.merge(outcome.dirty);
            if outcome.propagation == Propagation::Stop {
                trace.stop(node);
                stopped = true;
                break;
            }
        }
        self.ctx.event_target = None;
        if !stopped {
            if let Some(tb_root) = hit_title_bar {
                if let Some(binding) = self.title_bar_bindings.get(&tb_root).copied() {
                    match make_event(tb_root) {
                        AppEvent::PointerDown {
                            button: PointerButton::Left,
                            click_count,
                            ..
                        } => {
                            if click_count >= 2 {
                                self.window_commands.push(WindowCommand::ToggleMaximize);
                            } else {
                                self.window_commands.push(WindowCommand::StartDrag);
                            }
                        }
                        AppEvent::Click { .. } => {
                            if target == binding.minimize_id {
                                self.window_commands.push(WindowCommand::Minimize);
                            } else if target == binding.maximize_id {
                                self.window_commands.push(WindowCommand::ToggleMaximize);
                            } else if target == binding.close_id {
                                self.window_commands.push(WindowCommand::Close);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        trace.finish(&mut self.inspector);
        flags
    }

    fn dispatch_outcome(&mut self, event: AppEvent) -> EventOutcome {
        let mut widget_dirty = DirtyFlags::NONE;
        let mut widget_consumed = false;
        match &event {
            AppEvent::PointerDown {
                pos,
                target: Some(target_key),
                click_count,
                ..
            } => {
                if self.focus.order().contains(target_key)
                    && self.focus.focus(*target_key, FocusCause::Pointer)
                {
                    self.sync_focus_context();
                    widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                }
                if let Some((rect, widget)) = self.mounted_widgets.get(target_key).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::PointerDown {
                        pos: *pos,
                        click_count: *click_count,
                    };
                    let mut store = self.widget_state.borrow_mut();
                    if widget.reduce_erased(*target_key, rect, &w_evt, &mut store) {
                        widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        widget_consumed = true;
                    }
                }
            }
            AppEvent::Drag { node, pos, .. } => {
                if let Some((rect, widget)) = self.mounted_widgets.get(node).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::PointerDrag { pos: *pos };
                    let mut store = self.widget_state.borrow_mut();
                    if widget.reduce_erased(*node, rect, &w_evt, &mut store) {
                        widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        widget_consumed = true;
                    }
                }
            }
            AppEvent::PointerUp {
                pos,
                target: Some(target_key),
                ..
            } => {
                if let Some((rect, widget)) = self.mounted_widgets.get(target_key).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::PointerUp { pos: *pos };
                    let mut store = self.widget_state.borrow_mut();
                    if widget.reduce_erased(*target_key, rect, &w_evt, &mut store) {
                        widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        widget_consumed = true;
                    }
                }
            }
            AppEvent::Wheel {
                delta,
                target: Some(target_key),
                ..
            } => {
                if let Some((rect, widget)) = self.mounted_widgets.get(target_key).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::Wheel { delta: *delta };
                    let mut store = self.widget_state.borrow_mut();
                    if widget.reduce_erased(*target_key, rect, &w_evt, &mut store) {
                        widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        widget_consumed = true;
                    }
                }
            }
            AppEvent::Text { text } => {
                if let Some(focused_key) = self.ctx.focused {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&focused_key).cloned() {
                        let w_evt = unbound_widgets::WidgetInputEvent::TextInput(text.clone());
                        let mut store = self.widget_state.borrow_mut();
                        if widget.reduce_erased(focused_key, rect, &w_evt, &mut store) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                            widget_consumed = true;
                        }
                    }
                }
            }
            AppEvent::Key {
                key,
                pressed: true,
                modifiers,
            } => {
                if let Some(target_key) = self.ctx.event_target.or(self.ctx.focused) {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&target_key).cloned() {
                        let chord = key.chord(*modifiers);
                        let w_evt = unbound_widgets::WidgetInputEvent::KeyDown(chord);
                        let mut store = self.widget_state.borrow_mut();
                        if widget.reduce_erased(target_key, rect, &w_evt, &mut store) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                            widget_consumed = true;
                        }
                    }
                }
            }
            _ => {}
        }

        let mut outcome = self.app.reduce(&mut self.state, event, &mut self.ctx);
        outcome.dirty = outcome.dirty.merge(widget_dirty);
        if widget_consumed {
            outcome.propagation = Propagation::Stop;
        }
        if let Some((key, cause)) = self.ctx.request_focus.take() {
            self.focus.focus(key, cause);
        }
        if core::mem::take(&mut self.ctx.request_blur) {
            self.focus.blur();
        }
        self.sync_focus_context();
        self.collect_timer_requests();
        self.mark(outcome.dirty);
        outcome
    }

    fn mark(&mut self, flags: DirtyFlags) {
        self.dirty = self.dirty.merge(flags);
        if flags.needs_rebuild() {
            self.force_rebuild = true;
        }
    }

    fn sync_router_context(&mut self) {
        self.ctx.hovered = self.router.hover();
        self.ctx.pressed = self.router.pointer().pressed();
        self.ctx.cursor = self.router.cursor();
    }

    fn sync_focus_context(&mut self) {
        self.ctx.focused = self.focus.focused();
        self.ctx.focus_visible = self.focus.focus_visible();
    }
}

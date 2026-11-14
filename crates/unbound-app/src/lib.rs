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

use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::A11yAction;
use unbound_abi::{Dim, NodeFlags, RawNodeDesc};
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
pub mod scheduler;
pub mod state_store;
pub mod window_runner;

pub use assets::{AssetHub, AssetResidency};
pub use guest_bridge::GuestAppAdapter;
pub use i18n::{I18nBundle, LayoutDirection};
pub use inspector::{Inspector, InspectorNode, InspectorScene};
pub use overlay_manager::OverlayManager;
pub use router::{RouteEntry, RouterStack, TransitionDirection};
pub use scheduler::{AppWaker, TimerQueue, TimerRequest, TimerToken};
pub use state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use unbound_input::{FocusGroup, FocusGroupMode, RovingKey, RovingOrientation};
pub use unbound_widgets::ergonomics::UiWidget;
pub use window_runner::{run_windowed, WindowedRunner};

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
#[macro_export]
macro_rules! ui_key {
    () => {{
        const KEY: u64 = $crate::const_fnv1a_hash(file!(), line!(), column!());
        KEY
    }};
    ($discriminator:expr) => {{
        const BASE: u64 = $crate::const_fnv1a_hash(file!(), line!(), column!());
        $crate::mix_key(BASE, $crate::KeyDiscriminator::into_u64($discriminator))
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
        if store.get::<W::State>(key).is_none() {
            let initial = self.initial_state();
            store.insert_state::<W::State>(key, initial);
        }
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

    /// 在工作线程上运行 `task` 并归约其返回的事件。
    ///
    /// 结果将在下一次 [`AppRuntime::poll_async`]（每帧开头自动调用）中投递；
    /// 在此期间 [`AppWaker`] 会唤醒休眠的事件循环。
    ///
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
}

/// 传递给 [`Application::view`] 的场景构建器。
pub struct ViewBuilder<'a> {
    collector: &'a mut SceneCollector,
    /// 不可变帧上下文。
    pub ctx: ViewContext,
    world: &'a mut UiWorld,
    parent: NodeId,
    pending: &'a mut Vec<PendingWidget>,
    widget_state: Rc<RefCell<WidgetStateStore>>,
    node_keys: &'a mut FxHashMap<u64, NodeId>,
    seen_keys: &'a mut FxHashSet<u64>,
    ephemeral_nodes: &'a mut Vec<NodeId>,
    focus_groups: &'a mut Vec<FocusGroup>,
}

impl<'a> ViewBuilder<'a> {
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

    /// 声明最多包含 8 条显式列轨道的网格容器。
    pub fn grid(&mut self, key: u64, columns: &[Dim], build: impl FnOnce(&mut Self)) -> NodeId {
        let desc = RawNodeDesc::container()
            .display(unbound_abi::Display::Grid)
            .grid_columns(columns)
            .gap(8.0);
        self.container(key, desc, build)
    }

    fn reconcile_node(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        loc: &'static core::panic::Location<'static>,
    ) -> NodeId {
        desc.key = key;
        if key != 0 {
            self.seen_keys.insert(key);
            if let Some(&existing) = self.node_keys.get(&key) {
                if let Some(node_data) = self.world.get(existing) {
                    if node_data.parent == self.parent {
                        self.world.set_desc(existing, desc);
                        return existing;
                    }
                }
            }
        }
        let node = self.world.insert(self.parent, desc, loc);
        if key != 0 {
            self.node_keys.insert(key, node);
        } else {
            self.ephemeral_nodes.push(node);
        }
        node
    }

    /// 使用显式描述符声明纯布局容器。
    pub fn container(
        &mut self,
        key: u64,
        desc: RawNodeDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        let parent = core::mem::replace(&mut self.parent, node);
        build(self);
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
                badge.rect.width().max(64.0),
                badge.rect.height().max(22.0),
            )),
            WidgetSpec::Divider(div) => Some(Vec2::new(
                div.rect.width().max(1.0),
                div.rect.height().max(1.0),
            )),
            WidgetSpec::Alert(alert) => Some(Vec2::new(
                alert.rect.width().max(240.0),
                alert.rect.height().max(48.0),
            )),
            WidgetSpec::Text { .. } => None,
        };
        if let Some(size) = intrinsic {
            desc = desc.size(size.x, size.y);
        }
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        if let Some(size) = intrinsic {
            if let Some(data) = self.world.get_mut(node) {
                data.measure = Some(size);
            }
        }
        self.pending.push(PendingWidget { key, node, spec });
        node
    }

    /// 添加任意支持自动固有尺寸测量与布局的声明式 [`UiWidget`]。
    pub fn add<W: unbound_widgets::UiWidget + 'static>(&mut self, key: u64, widget: W) -> NodeId {
        self.add_with_layout(key, RawNodeDesc::container(), widget)
    }

    /// 添加任意带自定义布局约束的声明式 [`UiWidget`]。
    pub fn add_with_layout<W: unbound_widgets::UiWidget + 'static>(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        widget: W,
    ) -> NodeId {
        desc.key = key;
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.pending.push(PendingWidget {
            key,
            node,
            spec: WidgetSpec::UiWidget(Arc::new(widget)),
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

    /// 声明式卡片容器快捷方式。
    pub fn card(
        &mut self,
        key: u64,
        _desc: unbound_widgets::CardDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        self.container(
            key,
            RawNodeDesc::container()
                .column()
                .padding(unbound_abi::Edges::all(16.0))
                .gap(8.0),
            build,
        )
    }

    /// 声明式滚动视图容器快捷方式。
    pub fn scroll_view(
        &mut self,
        key: u64,
        vertical: bool,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut desc = RawNodeDesc::container();
        if vertical {
            desc = desc.column().flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        } else {
            desc = desc.row().flags(NodeFlags::CLIP | NodeFlags::SCROLL_X);
        }
        self.container(key, desc, build)
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
    ///
    pub fn text(&mut self, key: u64, text: impl Into<String>, style: TextStyle) -> NodeId {
        let text = text.into();
        let desc = RawNodeDesc::container()
            .key(key)
            .width(Dim::percent(100.0))
            .flags(NodeFlags::WRAP);
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.world.set_text(node, text.clone());
        self.pending.push(PendingWidget {
            key,
            node,
            spec: WidgetSpec::Text { text, style },
        });
        node
    }

    /// 直接访问收集器，供需要追加变换或裁剪的控件使用。
    pub fn collector(&mut self) -> &mut SceneCollector {
        self.collector
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
    seen_keys: FxHashSet<u64>,
    ephemeral_nodes: Vec<NodeId>,
    last_layout_stats: LayoutStats,
    layout_initialized: bool,
    overlays: OverlayManager,
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
            seen_keys: FxHashSet::default(),
            ephemeral_nodes: Vec::new(),
            last_layout_stats: LayoutStats::default(),
            layout_initialized: false,
            overlays: OverlayManager::new(),
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

    /// 按键返回已挂载控件的世界坐标矩形。
    #[must_use]
    pub fn widget_rect(&self, key: u64) -> Option<Rect> {
        self.mounted_widgets.get(&key).map(|(rect, _)| *rect)
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
        let path: Vec<u64> = self
            .router
            .wheel(delta, pos)
            .iter()
            .map(|region| region.node)
            .collect();
        if path.is_empty() {
            return self.dispatch(AppEvent::Wheel {
                pos,
                delta,
                target: None,
            });
        }

        let mut remaining = delta;
        let mut flags = DirtyFlags::NONE;
        for node in path {
            if remaining.length() <= 0.01 {
                break;
            }
            self.ctx.scroll_consumed = Vec2::ZERO;
            let outcome = self.dispatch_outcome(AppEvent::Wheel {
                pos,
                delta: remaining,
                target: Some(node),
            });
            remaining -= self.ctx.scroll_consumed;
            flags = flags.merge(outcome.dirty);
            if outcome.propagation == Propagation::Stop {
                break;
            }
        }
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
        let flags = self
            .dispatch(AppEvent::Key {
                key,
                pressed,
                modifiers,
            })
            .merge(flags);
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

    /// 处理视口尺寸调整。
    pub fn resize(&mut self, size: Vec2) -> DirtyFlags {
        self.ctx.viewport = size;
        self.force_rebuild = true;
        let flags = self
            .dispatch(AppEvent::Resize { size })
            .merge(DirtyFlags::LAYOUT);
        self.mark(flags);
        flags
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
        self.time += f64::from(dt.max(0.0));
        self.ctx.time = self.time;
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
        // 先归约自上一帧以来到达的异步结果与定时器，
        // 使仅因它们而唤醒的帧仍会执行重建。
        self.poll_async();
        self.poll_timers();
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

        // 焦点顺序来自上一帧；在重建前采用它，使 `Tab` 可在帧间生效。
        //
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.sync_focus_context();

        self.collector.reset(ROOT);
        self.pending.clear();
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

        let view_ctx = ViewContext {
            viewport: self.ctx.viewport,
            theme: self.ctx.theme,
            hovered: self.router.hover(),
            pressed: self.router.pointer().pressed(),
            focused: self.ctx.focused,
            focus_visible: self.ctx.focus_visible,
            force: self.force_rebuild,
        };
        let mut focus_groups = Vec::new();
        let mut view = ViewBuilder {
            collector: &mut self.collector,
            ctx: view_ctx,
            world: &mut self.world,
            parent: root,
            pending: &mut self.pending,
            widget_state: Rc::clone(&self.widget_state),
            node_keys: &mut self.node_keys,
            seen_keys: &mut self.seen_keys,
            ephemeral_nodes: &mut self.ephemeral_nodes,
            focus_groups: &mut focus_groups,
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
        self.dispatch_lifecycle();

        // 发布新的命中区域与焦点顺序。
        let regions = core::mem::take(&mut self.collector.regions);
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
                let style = TextStyle::sized(11.0);
                for (index, line) in scene.lines.iter().enumerate().take(16) {
                    let shaped = self.text_engine.shape(line, &style);
                    let origin = Vec2::new(panel_x, 16.0 + index as f32 * 18.0);
                    let quads = self.glyph_cache.quads_for(
                        &mut self.text_engine,
                        &shaped,
                        origin,
                        400,
                        false,
                    );
                    for quad in quads {
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

        // 第 1 遍：使用 TextCtx 预测量控件
        {
            let mut text_ctx = unbound_widgets::TextCtx {
                engine: &mut self.text_engine,
                cache: &mut self.glyph_cache,
                atlas_clock: self.frame_count,
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
                    WidgetSpec::Text { .. } => None,
                };
                if let Some(size) = intrinsic {
                    if let Some(data) = self.world.get_mut(entry.node) {
                        if data.measure.is_none() {
                            data.measure = Some(size);
                        }
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
            };
            match &entry.spec {
                WidgetSpec::UiWidget(widget) => {
                    self.mounted_widgets
                        .insert(entry.key, (rect, Arc::clone(widget)));
                    widget.ensure_state_erased(entry.key, &mut self.widget_state.borrow_mut());
                    let store = self.widget_state.borrow();
                    let output =
                        widget.emit_laid_out_erased(entry.key, rect, state, &store, &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Button(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Checkbox(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output =
                        desc.emit_laid_out(entry.key, rect, state, &desc.state, &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Switch(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output =
                        desc.emit_laid_out(entry.key, rect, state, &desc.on, &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Slider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output =
                        desc.emit_laid_out(entry.key, rect, state, &desc.value, &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Progress {
                    desc,
                    state: pstate,
                } => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = desc.emit_laid_out(entry.key, rect, state, pstate, &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Badge(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Divider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Alert(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let output = desc.emit_laid_out(entry.key, rect, state, &(), &mut text_ctx);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Text { .. } => {}
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
            } => self.dispatch(AppEvent::PointerDown {
                pos: position,
                target,
                button,
                click_count,
            }),
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
            } => self.dispatch(AppEvent::Drag {
                node: target,
                pos: position,
                delta,
            }),
            PointerEvent::Up {
                target,
                position,
                button,
            } => {
                let mut flags = self.dispatch(AppEvent::PointerUp {
                    pos: position,
                    target,
                    button,
                });
                // 仅当释放位置仍在被按下的节点上时，辅助点击才打开菜单。
                //
                if button == PointerButton::Right {
                    if let Some(node) = target {
                        if self.router.hover() == Some(node) {
                            flags = flags.merge(self.dispatch(AppEvent::ContextMenu {
                                node,
                                pos: position,
                            }));
                        }
                    }
                }
                flags
            }
            PointerEvent::Click {
                target,
                position,
                count,
            } => self.dispatch(AppEvent::Click {
                node: target,
                pos: position,
                count,
            }),
            PointerEvent::DoubleClick { target, position } => {
                self.dispatch(AppEvent::DoubleClick {
                    node: target,
                    pos: position,
                })
            }
            PointerEvent::CaptureLost { .. } => DirtyFlags::NONE,
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

    fn dispatch_outcome(&mut self, event: AppEvent) -> EventOutcome {
        let mut widget_dirty = DirtyFlags::NONE;
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
                    }
                }
            }
            AppEvent::Drag { node, pos, .. } => {
                if let Some((rect, widget)) = self.mounted_widgets.get(node).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::PointerDrag { pos: *pos };
                    let mut store = self.widget_state.borrow_mut();
                    if widget.reduce_erased(*node, rect, &w_evt, &mut store) {
                        widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
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
                        }
                    }
                }
            }
            AppEvent::Key {
                key,
                pressed: true,
                modifiers,
            } => {
                if let Some(focused_key) = self.ctx.focused {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&focused_key).cloned() {
                        let chord = key.chord(*modifiers);
                        let w_evt = unbound_widgets::WidgetInputEvent::KeyDown(chord);
                        let mut store = self.widget_state.borrow_mut();
                        if widget.reduce_erased(focused_key, rect, &w_evt, &mut store) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        }
                    }
                }
            }
            _ => {}
        }

        let mut outcome = self.app.reduce(&mut self.state, event, &mut self.ctx);
        outcome.dirty = outcome.dirty.merge(widget_dirty);
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

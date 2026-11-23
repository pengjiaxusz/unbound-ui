//! lang-allow: 中文注释与文档
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

pub mod appearance;
pub mod assets;
pub mod backdrop;
pub mod form;
pub mod guest_bridge;
pub mod i18n;
pub mod inspector;
pub mod key_scope;
pub mod multi_window;
pub mod overlay_manager;
pub mod prelude;
pub mod router;
pub mod scene_upload;
pub mod scheduler;
pub mod settings;
pub mod state_store;
pub mod window_runner;

pub use appearance::{
    apply_appearance_diff_to_platform, backdrop_channel_request, dwm_material_to_platform,
    AppearanceCaps, AppearanceDiff, AppearanceIntent, BackdropChannel, WindowAppearance,
};
pub use assets::{
    AssetHub, AssetResidency, ExternalTextureHandle, ExternalTextureOp, ExternalTextureState,
};
pub use backdrop::{
    fallback_reason_zh, is_mica_ready_event, material_name_zh, plan_summary_zh, MicaBackdrop,
    WindowBackdropController, MICA_READY_EVENT_ID,
};
pub use form::{FormBindingStore, FormMutator, WidgetValue};
pub use guest_bridge::GuestAppAdapter;
pub use i18n::{global_locale, set_global_locale, I18nBundle, LayoutDirection, Locale};
pub use inspector::{EventTrace, Inspector, InspectorNode, InspectorScene};
pub use key_scope::KeyScope;
pub use multi_window::{
    run_multi_window, MultiWindowCoordinator, MultiWindowFactory, WindowContext,
};
pub use overlay_manager::OverlayManager;
pub use router::{RouteEntry, RouterStack, TransitionDirection};
pub use scene_upload::{collect_backdrops, scene_upload};
pub use scheduler::{AppWaker, TimerQueue, TimerRequest, TimerToken};
pub use settings::{
    overflow_menu_item_key, overflow_menu_popup_key, select_option_key, select_popup_key,
    SettingsCard,
};
pub use state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use unbound_input::{FocusGroup, FocusGroupMode, RovingKey, RovingOrientation};
pub use unbound_platform::WindowBackdropMaterial;
pub use unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH;
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

impl KeyDiscriminator for String {
    #[inline]
    fn into_u64(self) -> u64 {
        const_fnv1a_hash(self.as_str(), 0, 0)
    }
}

impl KeyDiscriminator for u16 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for u8 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for i64 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for i16 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for i8 {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for char {
    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl KeyDiscriminator for KeyScope {
    #[inline]
    fn into_u64(self) -> u64 {
        self.raw()
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
    /// 返回底层具体控件类型的动态引用（供运行时执行外部纹理句柄解析等桥接操作）。
    fn as_any(&self) -> &dyn core::any::Any;
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
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

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
    /// 当前输入法是否启用。
    pub enabled: bool,
    /// 正在组合的文本；组合结束时为空。
    pub preedit: String,
    /// 预编辑文本内当前光标 / 选区字节范围。
    pub cursor_range: Option<(usize, usize)>,
    /// 若本次完成组合输入，则为已提交的字符串。
    pub commit: Option<String>,
    /// 组合被取消（失焦或输入法关闭）。
    pub cancelled: bool,
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
    /// 原生 Win32 OLE 向外文件拖出操作结束回执。
    ExternalFileDragFinished(unbound_platform::OutboundFileDragOutcome),
    /// 越界原生弹出窗已被平台宿主关闭（如失焦、宿主最小化或级联回收）。
    NativePopupDismissed {
        /// 被关闭的原生弹出窗标识（对应浮层节点键）。
        id: u64,
    },
}

impl AppEvent {
    /// 返回产生本事件的目标控件键（若事件类型包含目标节点）。
    #[must_use]
    pub fn target_node(&self) -> Option<u64> {
        match self {
            AppEvent::Click { node, .. }
            | AppEvent::DoubleClick { node, .. }
            | AppEvent::ContextMenu { node, .. }
            | AppEvent::DragStart { node, .. }
            | AppEvent::Drag { node, .. } => Some(*node),
            AppEvent::PointerDown { target, .. }
            | AppEvent::PointerUp { target, .. }
            | AppEvent::Wheel { target, .. } => *target,
            AppEvent::A11y { target, .. } => Some(*target),
            _ => None,
        }
    }

    /// 若本事件包含交互目标节点，从上下文局部状态池中提取绑定的强类型 Action。
    #[must_use]
    pub fn action<A: Clone + Send + Sync + 'static>(&self, ctx: &AppContext) -> Option<A> {
        self.target_node().and_then(|node| ctx.action::<A>(node))
    }
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

/// 标签页命中目标（同时携带逻辑索引与稳定 `TabItem::id`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabHitTarget {
    /// 标签页在 `TabsDesc::items` 中的逻辑索引。
    pub index: usize,
    /// 标签页的稳定标识（对应 `TabItem::id`）。
    pub id: u64,
}

/// 标签栏子热区命中解析结果（运行时内部路由使用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabSubHit {
    /// 标签页主体（附带该标签当前是否允许关闭）。
    Item {
        target: TabHitTarget,
        closable: bool,
    },
    /// 标签页关闭按钮 `×`。
    Close(TabHitTarget),
    /// 标签栏末尾 `+` 新建按钮。
    Add,
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
    /// 当前宿主产品主题风格预设（如 `ProductPreset::RenderWorkbench`）。
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
    /// 运行期请求应用的文本渲染设置变更（若为 Some 则在每帧开始前生效）。
    pub text_settings: Option<unbound_text::TextRenderSettings>,
    /// 应用在 `reduce` 中排入的待执行窗口控制命令队列。
    pending_window_commands: Vec<WindowCommand>,
    /// 各标签栏待消费的关闭请求（左键点击 `×` 或中键点击可关闭标签页）。
    tab_close_requests: Rc<RefCell<FxHashMap<u64, TabHitTarget>>>,
    /// 各标签栏待消费的新建按钮（`+`）点击请求。
    tab_add_requests: Rc<RefCell<FxHashSet<u64>>>,
    /// 各标签栏最近一次右键命中的具体标签目标（索引与 ID）。
    tab_context_targets: Rc<RefCell<FxHashMap<u64, TabHitTarget>>>,
    /// 各上下文菜单待消费的最近一次左键点击激活项索引（按 `ViewBuilder::context_menu` 的 `key` 索引）。
    context_menu_activated: Rc<RefCell<FxHashMap<u64, usize>>>,
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
            text_settings: None,
            pending_window_commands: Vec::new(),
            tab_close_requests: Rc::new(RefCell::new(FxHashMap::default())),
            tab_add_requests: Rc::new(RefCell::new(FxHashSet::default())),
            tab_context_targets: Rc::new(RefCell::new(FxHashMap::default())),
            context_menu_activated: Rc::new(RefCell::new(FxHashMap::default())),
        }
    }

    /// 请求在下一帧应用新的文本渲染设置。
    pub fn set_text_settings(&mut self, settings: unbound_text::TextRenderSettings) {
        self.text_settings = Some(settings);
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

    /// 若指定输入框当前持有键盘焦点且文本发生变化，将局部状态同步回字段。
    ///
    /// 返回 `true` 表示字段被修改。
    pub fn sync_text(&self, key: u64, field: &mut String) -> bool {
        if self.focused == Some(key) {
            if let Some(txt) = self.text_input_value(key) {
                if *field != txt {
                    *field = txt;
                    return true;
                }
            }
        }
        false
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

    /// 查询指定 `SelectDesc` 控件键当前是否处于展开态。
    #[must_use]
    pub fn select_is_open(&self, key: u64) -> bool {
        self.widget_state
            .borrow()
            .get::<unbound_widgets::SelectState>(key)
            .is_some_and(|st| st.open)
    }

    /// 设置指定 `SelectDesc` 控件键的展开态。
    pub fn set_select_open(&self, key: u64, open: bool) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::SelectState>(key);
        st.open = open;
        if !open {
            st.highlighted = None;
        }
    }

    /// 切换指定 `SelectDesc` 控件键的展开态。
    pub fn toggle_select_open(&self, key: u64) {
        let mut store = self.widget_state.borrow_mut();
        let st = store.entry::<unbound_widgets::SelectState>(key);
        st.toggle();
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

    /// 若指定滑块当前被按压/拖拽/聚焦且数值发生变化，将局部状态同步回字段。
    ///
    /// 返回 `true` 表示字段被修改。
    pub fn sync_slider(&self, key: u64, field: &mut f32) -> bool {
        if self.pressed == Some(key) || self.focused == Some(key) {
            if let Some(val) = self.slider_value(key) {
                if (*field - val).abs() > f32::EPSILON {
                    *field = val;
                    return true;
                }
            }
        }
        false
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

    /// 取出并消费指定标签栏最近一次关闭请求（含左键点击 `×` 与鼠标中键点击标签页）。
    pub fn take_tab_close_request(&self, key: u64) -> Option<TabHitTarget> {
        self.tab_close_requests.borrow_mut().remove(&key)
    }

    /// 取出并消费指定标签栏最近一次 `+` 新建按钮点击请求。
    pub fn take_tab_add_request(&self, key: u64) -> bool {
        self.tab_add_requests.borrow_mut().remove(&key)
    }

    /// 查询指定标签栏最近一次右键上下文菜单命中的具体标签目标（索引与 ID）。
    #[must_use]
    pub fn tab_context_target(&self, key: u64) -> Option<TabHitTarget> {
        self.tab_context_targets.borrow().get(&key).copied()
    }

    /// 取出并消费指定标签栏最近一次右键命中的标签目标。
    pub fn take_tab_context_target(&self, key: u64) -> Option<TabHitTarget> {
        self.tab_context_targets.borrow_mut().remove(&key)
    }

    pub(crate) fn record_tab_close_request(&self, key: u64, target: TabHitTarget) {
        self.tab_close_requests.borrow_mut().insert(key, target);
    }

    pub(crate) fn record_tab_add_request(&self, key: u64) {
        self.tab_add_requests.borrow_mut().insert(key);
    }

    pub(crate) fn record_tab_context_target(&self, key: u64, target: TabHitTarget) {
        self.tab_context_targets.borrow_mut().insert(key, target);
    }

    /// 取出并消费指定上下文菜单（按传给 `ViewBuilder::context_menu` 的 `key` 索引）
    /// 最近一次被用户左键点击激活的菜单项索引。
    pub fn take_context_menu_activated(&self, key: u64) -> Option<usize> {
        self.context_menu_activated.borrow_mut().remove(&key)
    }

    pub(crate) fn record_context_menu_activated(&self, key: u64, index: usize) {
        self.context_menu_activated.borrow_mut().insert(key, index);
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

    /// 查询绑定在指定控件键上的强类型 Action。
    #[must_use]
    pub fn action<A: Clone + Send + Sync + 'static>(&self, key: u64) -> Option<A> {
        self.widget_state.borrow().get::<A>(key).cloned()
    }

    /// 从已解析的交互事件中直接提取绑定的强类型 Action（若事件具有目标节点且匹配类型）。
    #[must_use]
    pub fn action_from_event<A: Clone + Send + Sync + 'static>(
        &self,
        event: &AppEvent,
    ) -> Option<A> {
        event.action::<A>(self)
    }

    /// 提取并移除绑定在指定控件键上的强类型 Action（单次消费模式）。
    pub fn take_action<A: Send + Sync + 'static>(&self, key: u64) -> Option<A> {
        self.widget_state.borrow_mut().remove_state::<A>(key)
    }

    /// 排入一条待窗口运行器执行的窗口控制命令。
    pub fn push_window_command(&mut self, command: WindowCommand) {
        self.pending_window_commands.push(command);
    }

    /// 请求平台启动原生 OLE 向外文件拖出操作。
    pub fn request_external_file_drag(
        &mut self,
        request: unbound_platform::OutboundFileDragRequest,
    ) {
        self.pending_window_commands
            .push(WindowCommand::BeginExternalFileDrag(request));
    }

    /// 动态更新窗口运行器的自适应帧节流策略。
    pub fn set_throttle_policy(&mut self, policy: unbound_platform::RenderThrottlePolicy) {
        self.pending_window_commands
            .push(WindowCommand::SetThrottlePolicy(policy));
    }

    /// 请求多窗口协调器打开一个辅助窗口。
    pub fn open_window(&mut self, config: unbound_platform::WindowConfig) {
        self.pending_window_commands
            .push(WindowCommand::OpenWindow(config));
    }

    /// 请求多窗口协调器关闭指定标识的窗口。
    pub fn close_window(&mut self, id: unbound_platform::WindowId) {
        self.pending_window_commands
            .push(WindowCommand::CloseWindow(id));
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

/// 虚拟树形视图单行渲染快照。
#[derive(Clone, Debug)]
pub struct TreeViewRowSnapshot {
    /// 扁平化树行元数据。
    row: unbound_world::TreeRow,
    /// 该行在内容空间中的顺序索引。
    item: usize,
    /// 是否处于选中态。
    selected: bool,
    /// 行前置图标槽配置。
    icon: unbound_widgets::TreeRowIcon,
    /// 可选节点显示标签（未设置时发射阶段回退 `Node <id>`）。
    label: Option<String>,
    /// 是否在该行顶部上方绘制水平分区分隔线。
    divider_before: bool,
}

/// 树形视图滚轮同步水位记录。
///
/// 跟踪上次与运行时 [`unbound_widgets::ScrollState`] 对齐的纵向偏移，
/// 仅在滚轮/滚动条相对上次同步发生增量变化时才同步到 [`unbound_widgets::TreeViewState`]，
/// 防止每帧用旧滚轮值覆盖调用方通过 `scroll_by` / `scroll_to_row` 设置的程序化滚动。
#[derive(Clone, Copy, Debug, Default)]
struct TreeViewScrollSync {
    /// 上次同步完成时 `ScrollState.offset().y` 的快照值。
    last_wheel_y: f32,
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
    /// 可交互容器的纯命中区域注册（不发射绘制图元，直接绑定容器逻辑键）。
    InteractiveHit { z: u16, cursor: CursorKind },
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
    /// 图像控件（支持图集层与异步资产解析）。
    Image(unbound_widgets::ImageDesc),
    /// 外部托管 RGBA8 纹理图像控件（未就绪时绘制等尺寸占位框）。
    ExternalImage {
        /// 外部纹理逻辑句柄。
        handle: ExternalTextureHandle,
        /// 布局占位与绘制目标尺寸。
        size: Vec2,
        /// 图像适应方式。
        fit: unbound_widgets::ObjectFit,
    },
    /// 虚拟树形视图。
    TreeView {
        /// 树形视图描述符。
        desc: unbound_widgets::TreeViewDesc,
        /// 本帧滚动偏移（逻辑像素）。
        scroll_y: f32,
        /// 扁平化总内容高度（逻辑像素）。
        content_height: f32,
        /// 当前指针悬停的单行出现键。
        hovered_row: Option<u64>,
        /// 本帧可见行快照列表（发射阶段将基于最终布局矩形重算每行世界坐标）。
        rows: Vec<TreeViewRowSnapshot>,
    },
    /// 双模路径地址栏。
    AddressBar {
        /// 地址栏描述符。
        desc: unbound_widgets::AddressBarDesc,
        /// 地址栏交互状态快照。
        /// 地址栏交互状态快照（装箱：状态含两套文本编辑核心，直接内联会让整个枚举膨胀近 600 字节）。
        state: Box<unbound_widgets::AddressBarState>,
    },
    /// 停靠面板树。
    Dock {
        /// 停靠区状态快照。
        state: unbound_widgets::DockState,
    },
    /// 缩略导航小地图。
    Minimap {
        /// 小地图描述符。
        desc: unbound_widgets::MinimapDesc,
        /// 小地图色块列表。
        items: Vec<unbound_widgets::MinimapItem>,
    },
    /// 锚定气泡浮层外壳。
    Popover {
        /// 锚点节点键。
        anchor_key: u64,
        /// 气泡框描述符。
        desc: unbound_widgets::PopoverDesc,
    },
    /// 右键上下文菜单。
    ContextMenu {
        /// 调用 `ViewBuilder::context_menu` 时传入的有效菜单键（用于回传点击激活项索引）。
        owner_key: u64,
        /// 触发菜单的锚点节点键。
        anchor_key: u64,
        /// 菜单描述符。
        desc: unbound_widgets::MenuDesc,
        /// 菜单内部高亮与滚动状态。
        menu_state: unbound_widgets::MenuState,
    },
    /// 全局命令面板。
    CommandPalette {
        /// 命令面板描述符。
        desc: unbound_widgets::CommandPaletteDesc,
        /// 命令面板查询与选中状态。
        state: unbound_widgets::CommandPaletteState,
    },
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

/// 活跃展开的下拉选择框弹出浮层元数据。
#[derive(Clone, Debug)]
pub struct ActiveSelectPopup {
    pub select_key: u64,
    pub bounds: Rect,
    pub option_keys: Vec<u64>,
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
    /// 跨线程事件投递句柄：后台任务（如图片生成）完成后唤醒 UI 线程。
    event_sender: Option<AppEventSender>,
    depth: u16,
    z_base: u16,
    /// 当前视图所在的分层键作用域（根级为 `KeyScope::root()`）。
    key_scope: KeyScope,
    form_bindings: &'a mut FormBindingStore,
    registered_selects: &'a mut Vec<(u64, unbound_widgets::SelectDesc)>,
    active_select_popups: &'a mut Vec<ActiveSelectPopup>,
    overlays: &'a mut OverlayManager,
    focus: &'a mut FocusManager,
    declared_overlays: &'a mut FxHashSet<u64>,
    overlay_pending_ranges: &'a mut Vec<(u64, NodeId, usize, usize)>,
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
        let effective_key = if !self.key_scope.is_root() && key != 0 {
            self.key_scope.key(key)
        } else {
            key
        };
        self.node_keys
            .get(&effective_key)
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
        let effective_key = if !self.key_scope.is_root() && key != 0 {
            self.key_scope.key(key)
        } else {
            key
        };
        RefMut::map(self.widget_state.borrow_mut(), |store| {
            store.use_state(effective_key)
        })
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
        let effective_key = if !self.key_scope.is_root() && key != 0 {
            self.key_scope.key(key)
        } else {
            key
        };
        self.semantic_intents.insert(effective_key, intent);
    }

    /// 声明某个控件的语义角色与名称。
    ///
    /// [`Self::declare_semantics`] 的简写形式。
    pub fn declare_semantic(&mut self, key: u64, role: Role, label: impl Into<String>) {
        self.declare_semantics(key, SemanticIntent::new(role, label));
    }

    /// 在指定判别值的作用域命名空间下声明子视图。
    ///
    /// 作用域内部的所有容器、控件与文本节点将自动基于父作用域哈希
    /// 派生出确定性、互不碰撞的命名空间键，彻底消除调用方手动计算
    /// 基址与步长（如 `base + idx * 30 + offset`）的算术负担。
    pub fn scope<R>(
        &mut self,
        discriminator: impl KeyDiscriminator,
        build: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let prev_scope = self.key_scope;
        self.key_scope = self.key_scope.scope(discriminator);
        let result = build(self);
        self.key_scope = prev_scope;
        result
    }

    /// 临时脱离当前层级作用域，在无作用域（根级）命名空间下声明子视图。
    pub fn unscoped<R>(&mut self, build: impl FnOnce(&mut Self) -> R) -> R {
        let prev_scope = self.key_scope;
        self.key_scope = KeyScope::root();
        let result = build(self);
        self.key_scope = prev_scope;
        result
    }

    /// 获取当前生效的作用域对象。
    #[must_use]
    pub fn key_scope(&self) -> KeyScope {
        self.key_scope
    }

    /// 获取当前生效的作用域 64 位哈希（若位于根作用域则返回 0）。
    #[must_use]
    pub fn current_scope(&self) -> u64 {
        self.key_scope.raw()
    }

    /// 在当前作用域下派生叶子节点的全局有效键（不触碰世界树节点）。
    #[must_use]
    pub fn scoped_key(&self, discriminator: impl KeyDiscriminator) -> u64 {
        self.key_scope.key(discriminator)
    }

    /// 为指定控件键绑定强类型 Action。
    ///
    /// Action 被存入运行时局部状态池，在事件归约（`reduce`）阶段可通过
    /// [`AppContext::action`] 或 [`AppEvent::action`] 直接提取强类型意图，
    /// 无需手动遍历匹配裸 `u64` 节点键。
    pub fn bind_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        action: A,
    ) {
        let raw_key = key.into_u64();
        let effective_key = if !self.key_scope.is_root() && raw_key != 0 {
            self.key_scope.key(raw_key)
        } else {
            raw_key
        };
        self.widget_state
            .borrow_mut()
            .insert_state(effective_key, action);
    }

    /// [`Self::bind_action`] 的语义别名，明确表达点击交互意图。
    pub fn on_click<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        action: A,
    ) {
        self.bind_action(key, action);
    }

    /// 声明绑定了强类型 Action 的按钮快捷方式。
    pub fn button_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: ButtonDesc,
        action: A,
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.bind_action(raw_key, action);
        self.button(raw_key, desc)
    }

    /// 声明绑定了强类型 Action 的图标按钮快捷方式。
    pub fn icon_button_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: unbound_widgets::IconButtonDesc,
        action: A,
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.bind_action(raw_key, action);
        self.icon_button(raw_key, desc)
    }

    /// 声明绑定了强类型 Action 的颜色色块快捷方式。
    pub fn color_swatch_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: unbound_widgets::ColorSwatchDesc,
        action: A,
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.bind_action(raw_key, action);
        self.color_swatch(raw_key, desc)
    }

    /// 声明绑定了强类型 Action 的侧边栏导航条目快捷方式。
    pub fn sidebar_item_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: unbound_widgets::SidebarItemDesc,
        action: A,
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.bind_action(raw_key, action);
        self.sidebar_item(raw_key, desc)
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
        let focused = self.ctx.window_focused;
        let caption = desc.caption_buttons(focused);
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
                unbound_widgets::TitleBarChrome::from_desc(&desc, focused),
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
            let caption_min_w = caption.total_width();
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
                right.window_caption_buttons(mix_key(root_key, 7), caption);
            });
        })
    }

    /// 挂载独立的窗口控制按钮组（[`unbound_widgets::WindowCaptionButtons`]，ADR-0029）。
    ///
    /// 供标准标题栏与自定义顶栏共同复用：按 Win11 规范无间隔横向排布可见按钮、禁止 Flex 压缩，
    /// 并自动跟随 `AppContext::window_maximized` 同步最大化/还原图标。
    pub fn window_caption_buttons(
        &mut self,
        key: u64,
        desc: unbound_widgets::WindowCaptionButtons,
    ) -> NodeId {
        let window_maximized = self.ctx.window_maximized;
        let mut caption_row_desc = RawNodeDesc::container()
            .row()
            .align(unbound_abi::Align::Center)
            .gap(0.0);
        // 窗口控制按钮（最小化/最大化/关闭）严禁被 Flex 压缩或挤出视口右边界。
        caption_row_desc.flex_shrink = 0.0;
        self.container(key, caption_row_desc, |btns| {
            for (button_id, widget) in desc.buttons(window_maximized) {
                btns.add_with_layout(
                    button_id,
                    RawNodeDesc::container()
                        .width(Dim::px(desc.button_width))
                        .height(Dim::px(desc.height)),
                    widget,
                );
            }
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
        let mut effective_key = if !self.key_scope.is_root() && key != 0 {
            self.key_scope.key(key)
        } else {
            key
        };
        if effective_key == ROOT {
            // 语义根键是保留键。控件占用它会让语义根成为自身的子节点，
            // 树遍历随即死循环；此处把控件改派到一个未被占用的派生键上。
            effective_key = self.derive_reserved_key(mix_key(effective_key, 0x9E37_79B9_7F4A_7C15));
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
        let is_interactive = desc.has(NodeFlags::INTERACTIVE);
        let next_z_base = if desc.z_index >= PORTAL_Z_BASE {
            (desc.z_index as u16).max(self.z_base)
        } else {
            self.z_base
        };
        let (effective_key, node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        if is_interactive && effective_key != 0 {
            let depth_z = (80u16.saturating_add(self.depth.saturating_mul(6))).min(480);
            let hit_z = next_z_base.saturating_add(depth_z);
            self.pending.push(PendingWidget {
                key: effective_key,
                node,
                spec: WidgetSpec::InteractiveHit {
                    z: hit_z,
                    cursor: CursorKind::Pointer,
                },
                z_base: next_z_base,
            });
        }
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
            WidgetSpec::Image(img) => (img.rect.width() > 0.0 || img.rect.height() > 0.0)
                .then_some(Vec2::new(img.rect.width(), img.rect.height())),
            WidgetSpec::ExternalImage { size, .. } => Some(*size),
            WidgetSpec::TreeView { desc, .. } => (desc.rect.width() > 0.0
                && desc.rect.height() > 0.0)
                .then_some(Vec2::new(desc.rect.width(), desc.rect.height())),
            WidgetSpec::AddressBar { desc, .. } => (desc.rect.width() > 0.0).then_some(Vec2::new(
                desc.rect.width(),
                if desc.rect.height() > 0.0 {
                    desc.rect.height()
                } else {
                    unbound_widgets::ADDRESS_BAR_HEIGHT
                },
            )),
            WidgetSpec::Minimap { desc, .. } => (desc.rect.width() > 0.0
                || desc.rect.height() > 0.0)
                .then_some(Vec2::new(desc.rect.width(), desc.rect.height())),
            WidgetSpec::Popover { desc, .. } => Some(desc.size),
            WidgetSpec::ContextMenu { desc, .. } => {
                Some(Vec2::new(desc.rect.width(), desc.rect.height()))
            }
            WidgetSpec::CommandPalette { desc, .. } => {
                Some(Vec2::new(desc.rect.width(), desc.rect.height()))
            }
            WidgetSpec::Dock { .. }
            | WidgetSpec::Text { .. }
            | WidgetSpec::TextInline { .. }
            | WidgetSpec::BoxSurface { .. }
            | WidgetSpec::InteractiveHit { .. }
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
        if let Some(select_desc) =
            (&widget as &dyn Any).downcast_ref::<unbound_widgets::SelectDesc>()
        {
            self.registered_selects
                .push((effective_key, select_desc.clone()));
        }
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

    /// 注册文本框变更的双向状态绑定。
    pub fn bind_text<S: 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        setter: impl FnMut(&mut S, &str) + 'static,
    ) {
        let raw_key = key.into_u64();
        let effective_key = if !self.key_scope.is_root() && raw_key != 0 {
            self.key_scope.key(raw_key)
        } else {
            raw_key
        };
        self.form_bindings.insert_text::<S>(effective_key, setter);
    }

    /// 注册滑块变更的双向状态绑定。
    pub fn bind_slider<S: 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        setter: impl FnMut(&mut S, f32) + 'static,
    ) {
        let raw_key = key.into_u64();
        let effective_key = if !self.key_scope.is_root() && raw_key != 0 {
            self.key_scope.key(raw_key)
        } else {
            raw_key
        };
        self.form_bindings.insert_slider::<S>(effective_key, setter);
    }

    /// 注册开关变更的双向状态绑定。
    pub fn bind_switch<S: 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        setter: impl FnMut(&mut S, bool) + 'static,
    ) {
        let raw_key = key.into_u64();
        let effective_key = if !self.key_scope.is_root() && raw_key != 0 {
            self.key_scope.key(raw_key)
        } else {
            raw_key
        };
        self.form_bindings.insert_switch::<S>(effective_key, setter);
    }

    /// 声明式双向绑定文本输入框快捷方式。
    ///
    /// 渲染文本框的同时建立与应用状态的双向写回绑定。当文本框编辑内容发生变化时，
    /// 运行时将在 `reduce()` 调用前自动触发 `setter` 将新字符串写回 `&mut State`。
    pub fn text_input_bound<S: 'static>(
        &mut self,
        key: u64,
        desc: unbound_widgets::TextInputDesc,
        setter: impl FnMut(&mut S, &str) + 'static,
    ) -> NodeId {
        let node = self.text_input(key, desc);
        let effective_key = self.world.get(node).map_or(key, |n| n.desc.key);
        self.form_bindings.insert_text::<S>(effective_key, setter);
        node
    }

    /// 声明式双向绑定滑块快捷方式。
    ///
    /// 渲染滑块的同时建立与应用状态的双向写回绑定。当滑块拖拽或点击改变数值时，
    /// 运行时将在 `reduce()` 调用前自动触发 `setter` 将新浮点值写回 `&mut State`。
    pub fn slider_bound<S: 'static>(
        &mut self,
        key: u64,
        desc: SliderDesc,
        setter: impl FnMut(&mut S, f32) + 'static,
    ) -> NodeId {
        let node = self.slider(key, desc);
        let effective_key = self.world.get(node).map_or(key, |n| n.desc.key);
        self.form_bindings.insert_slider::<S>(effective_key, setter);
        node
    }

    /// 声明式双向绑定开关快捷方式。
    ///
    /// 渲染开关的同时建立与应用状态的双向写回绑定。当开关点击改变状态时，
    /// 运行时将在 `reduce()` 调用前自动触发 `setter` 将新布尔值写回 `&mut State`。
    pub fn switch_bound<S: 'static>(
        &mut self,
        key: u64,
        desc: SwitchDesc,
        setter: impl FnMut(&mut S, bool) + 'static,
    ) -> NodeId {
        let node = self.switch(key, desc);
        let effective_key = self.world.get(node).map_or(key, |n| n.desc.key);
        self.form_bindings.insert_switch::<S>(effective_key, setter);
        node
    }

    /// 声明式下拉选择框快捷方式。
    pub fn select(&mut self, key: u64, desc: unbound_widgets::SelectDesc) -> NodeId {
        let node = self.add(key, desc.clone());
        let effective_key = self.world.get(node).map_or(key, |n| n.desc.key);
        self.registered_selects.push((effective_key, desc));
        node
    }

    /// 声明式双向绑定下拉选择框快捷方式。
    ///
    /// 渲染选择框的同时建立与应用状态的双向写回绑定。当用户展开浮层并选择新选项时，
    /// 运行时将在 `reduce()` 调用前自动触发 `setter` 将新选中索引写回 `&mut State`，
    /// 并自动轻关闭选项浮层。
    pub fn select_bound<S: 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: unbound_widgets::SelectDesc,
        setter: impl FnMut(&mut S, usize) + 'static,
    ) -> NodeId {
        let raw_key = key.into_u64();
        let node = self.select(raw_key, desc);
        let effective_key = self.world.get(node).map_or(raw_key, |n| n.desc.key);
        self.form_bindings.insert_select::<S>(effective_key, setter);
        node
    }

    /// 声明式双向绑定下拉选择框快捷方式（带自定义节点布局）。
    pub fn select_bound_with_layout<S: 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        layout: RawNodeDesc,
        desc: unbound_widgets::SelectDesc,
        setter: impl FnMut(&mut S, usize) + 'static,
    ) -> NodeId {
        let raw_key = key.into_u64();
        let node = self.add_with_layout(raw_key, layout, desc.clone());
        let effective_key = self.world.get(node).map_or(raw_key, |n| n.desc.key);
        self.registered_selects.push((effective_key, desc));
        self.form_bindings.insert_select::<S>(effective_key, setter);
        node
    }

    /// 声明式 Action 绑定下拉选择框快捷方式。
    ///
    /// 为下拉框的各个选项预绑定派生 Action。
    pub fn select_action<A: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl KeyDiscriminator,
        desc: unbound_widgets::SelectDesc,
        action_fn: impl Fn(usize) -> A + 'static,
    ) -> NodeId {
        let raw_key = key.into_u64();
        let option_count = desc.options.len();
        let node = self.select(raw_key, desc);
        let effective_key = self.world.get(node).map_or(raw_key, |n| n.desc.key);
        for idx in 0..option_count {
            let opt_key = select_option_key(effective_key, idx);
            self.bind_action(opt_key, action_fn(idx));
        }
        node
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

    /// 声明式图像控件（支持图集层与异步资产中心解析）。
    pub fn image(&mut self, key: u64, desc: unbound_widgets::ImageDesc) -> NodeId {
        let mut node_desc = RawNodeDesc::container();
        if desc.rect.width() > 0.0 {
            node_desc = node_desc.width(Dim::px(desc.rect.width()));
        }
        if desc.rect.height() > 0.0 {
            node_desc = node_desc.height(Dim::px(desc.rect.height()));
        }
        self.widget(key, node_desc, WidgetSpec::Image(desc))
    }

    /// 声明式外部纹理图像控件：外部纹理未就绪时自动绘制同尺寸占位框，就绪后绑定真实外部槽位与 UV。
    pub fn image_external(
        &mut self,
        key: u64,
        handle: ExternalTextureHandle,
        size: Vec2,
        fit: unbound_widgets::ObjectFit,
    ) -> NodeId {
        let node_desc = RawNodeDesc::container().size(size.x.max(0.0), size.y.max(0.0));
        self.widget(
            key,
            node_desc,
            WidgetSpec::ExternalImage { handle, size, fit },
        )
    }

    /// 声明式虚拟树形视图控件。
    pub fn tree_view(
        &mut self,
        key: u64,
        desc: unbound_widgets::TreeViewDesc,
        state: &mut unbound_widgets::TreeViewState,
    ) -> NodeId {
        let effective_key = self.scoped_key(key);
        let prev_rect = self.widget_rect(effective_key);
        let has_explicit_size = desc.rect.width() > 0.0 && desc.rect.height() > 0.0;
        // 使用上一帧已知布局尺寸（或首帧视口尺寸）作为虚拟窗口采样矩形，但不写回 `desc.rect`，
        // 避免零尺寸弹性树视图被锁死为固定像素尺寸；最终绘制坐标在布局求解后按真实 `rect` 重算。
        let mut query_desc = desc.clone();
        if !has_explicit_size {
            query_desc.rect = prev_rect
                .filter(|r| r.width() > 0.0 && r.height() > 0.0)
                .unwrap_or_else(|| self.viewport_rect());
        } else if let Some(prev) = prev_rect {
            if prev.width() > 0.0 && prev.height() > 0.0 {
                query_desc.rect = Rect::new(
                    prev.origin.x,
                    prev.origin.y,
                    prev.width().max(desc.rect.width()),
                    prev.height().max(desc.rect.height()),
                );
            }
        }
        // 仅当运行时 `ScrollState.offset().y` 相对上次同步水位发生变化时，
        // 才将滚轮/滚动条增量同步到 `TreeViewState`，避免覆盖调用方通过 `scroll_by` / `scroll_to_row` 设置的程序化滚动。
        {
            let mut store = self.widget_state.borrow_mut();
            let wheel_y = store
                .get::<unbound_widgets::ScrollState>(effective_key)
                .map(|s| s.offset().y);
            let last_wheel_y = store
                .use_state::<TreeViewScrollSync>(effective_key)
                .last_wheel_y;
            if let Some(wy) = wheel_y {
                let delta_y = wy - last_wheel_y;
                if delta_y.abs() > 0.01 {
                    let s_desc = query_desc.scroll_desc(state.row_count());
                    let target_y = state.scroll_offset() + delta_y;
                    state.scroll.jump_to(&s_desc, Vec2::new(0.0, target_y));
                    let synced_y = state.scroll_offset();
                    if let Some(scroll_state) =
                        store.get_mut::<unbound_widgets::ScrollState>(effective_key)
                    {
                        scroll_state.jump_to(&s_desc, Vec2::new(0.0, synced_y));
                    }
                    store
                        .use_state::<TreeViewScrollSync>(effective_key)
                        .last_wheel_y = synced_y;
                }
            }
        }
        let scroll_y = state.scroll_offset();
        let content_height = state.row_count() as f32 * desc.row_height;
        let hovered_row = state.hovered_row();
        let rows = state
            .rows(&query_desc)
            .into_iter()
            .map(|row| {
                let id = row.row.id;
                TreeViewRowSnapshot {
                    row: row.row,
                    item: row.item,
                    selected: state.selected.contains(&id),
                    icon: state.row_icon(id),
                    label: state.row_label(id).map(str::to_owned),
                    divider_before: state.has_row_divider_before(id),
                }
            })
            .collect();
        let mut node_desc = RawNodeDesc::container();
        if has_explicit_size {
            node_desc = node_desc.size(desc.rect.width(), desc.rect.height());
        } else {
            node_desc = node_desc
                .width(Dim::percent(100.0))
                .height(Dim::percent(100.0))
                .flex_grow(1.0);
            node_desc.min_width = Dim::px(0.0);
            node_desc.min_height = Dim::px(0.0);
        }
        self.widget(
            key,
            node_desc,
            WidgetSpec::TreeView {
                desc,
                scroll_y,
                content_height,
                hovered_row,
                rows,
            },
        )
    }

    /// 声明式双模路径地址栏控件。
    pub fn address_bar(
        &mut self,
        key: u64,
        desc: unbound_widgets::AddressBarDesc,
        state: &unbound_widgets::AddressBarState,
    ) -> NodeId {
        let mut node_desc = RawNodeDesc::container();
        if desc.rect.width() > 0.0 {
            node_desc = node_desc.width(Dim::px(desc.rect.width()));
        } else {
            node_desc = node_desc.width(Dim::percent(100.0)).flex_grow(1.0);
            node_desc.min_width = Dim::px(0.0);
        }
        let height = if desc.rect.height() > 0.0 {
            desc.rect.height()
        } else {
            unbound_widgets::ADDRESS_BAR_HEIGHT
        };
        node_desc = node_desc.height(Dim::px(height));
        self.widget(
            key,
            node_desc,
            WidgetSpec::AddressBar {
                desc,
                state: Box::new(state.clone()),
            },
        )
    }

    /// 声明式停靠面板树控件。
    pub fn dock(&mut self, key: u64, state: &unbound_widgets::DockState) -> NodeId {
        let node_desc = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::percent(100.0));
        self.widget(
            key,
            node_desc,
            WidgetSpec::Dock {
                state: state.clone(),
            },
        )
    }

    /// 声明式画布缩略导航小地图控件。
    pub fn minimap(
        &mut self,
        key: u64,
        desc: unbound_widgets::MinimapDesc,
        items: &[unbound_widgets::MinimapItem],
    ) -> NodeId {
        let mut node_desc = RawNodeDesc::container();
        if desc.rect.width() > 0.0 && desc.rect.height() > 0.0 {
            node_desc = node_desc.size(desc.rect.width(), desc.rect.height());
        }
        self.widget(
            key,
            node_desc,
            WidgetSpec::Minimap {
                desc,
                items: items.to_vec(),
            },
        )
    }

    /// 声明式统一浮层容器：接入 [`OverlayManager`] Z 序、焦点陷阱、外部点击与 `Esc` 关闭以及锚点裁剪。
    pub fn overlay_layer(
        &mut self,
        key: u64,
        mut entry: unbound_widgets::overlay::OverlayEntry,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let effective_key = self.scoped_key(key);
        entry.node = effective_key;
        self.declared_overlays.insert(effective_key);
        if self.overlays.is_dismissed(effective_key) {
            return self.world.root();
        }
        let _ = self.overlays.open(entry.clone(), self.focus);
        let stack_idx = self
            .overlays
            .stack()
            .entries()
            .iter()
            .position(|e| e.node == effective_key)
            .unwrap_or(0);
        let overlay_z = (PORTAL_Z_BASE as u16)
            .saturating_add(100)
            .saturating_add((stack_idx as u16).saturating_mul(32));

        let parent_saved = core::mem::replace(&mut self.parent, self.world.root());
        let z_saved = core::mem::replace(&mut self.z_base, overlay_z);
        let depth_saved = self.depth;
        self.depth = self.depth.saturating_add(1);

        let container_desc = RawNodeDesc::container()
            .column()
            .absolute()
            .bounds(
                entry.bounds.origin.x,
                entry.bounds.origin.y,
                entry.bounds.width().max(1.0),
                entry.bounds.height().max(1.0),
            )
            .z_index(i32::from(overlay_z))
            .portal_root();
        let (_, overlay_node) =
            self.reconcile_node(key, container_desc, core::panic::Location::caller());
        let pending_start = self.pending.len();
        self.parent = overlay_node;
        build(self);
        let pending_end = self.pending.len();

        self.depth = depth_saved;
        self.z_base = z_saved;
        self.parent = parent_saved;

        self.overlay_pending_ranges
            .push((effective_key, overlay_node, pending_start, pending_end));
        overlay_node
    }

    /// 声明式锚定气泡框浮层：自动交由 [`OverlayManager`] 管理关闭与锚点失效裁剪。
    pub fn popover(
        &mut self,
        key: u64,
        anchor_key: u64,
        mut desc: unbound_widgets::PopoverDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let effective_key = self.scoped_key(key);
        let effective_anchor = self.scoped_key(anchor_key);
        if desc.anchor.width() <= 0.0 && desc.anchor.height() <= 0.0 {
            if let Some(r) = self.widget_rect(effective_anchor) {
                desc.anchor = r;
            }
        }
        if desc.viewport.width() <= 0.0 && desc.viewport.height() <= 0.0 {
            desc.viewport = self.viewport_rect();
        }
        let bounds = desc.rect();
        let entry = unbound_widgets::overlay::OverlayEntry::popover(
            effective_key,
            effective_anchor,
            bounds,
        );
        self.overlay_layer(key, entry, |ov| {
            let bg_key = mix_key(effective_key, 0x504F_5042);
            let bg_desc = RawNodeDesc::container().absolute().bounds(
                0.0,
                0.0,
                bounds.width().max(1.0),
                bounds.height().max(1.0),
            );
            ov.widget(
                bg_key,
                bg_desc,
                WidgetSpec::Popover {
                    anchor_key: effective_anchor,
                    desc,
                },
            );
            build(ov);
        })
    }

    /// 声明式右键上下文菜单浮层：打开时自动定位并交由 [`OverlayManager`] 管理。
    pub fn context_menu(
        &mut self,
        key: u64,
        desc: unbound_widgets::MenuDesc,
        state: &unbound_widgets::ContextMenuState,
    ) -> NodeId {
        if !state.is_open() {
            return self.world.root();
        }
        let effective_key = self.scoped_key(key);
        let placed_desc = state.desc_at(desc, self.viewport_rect());
        let anchor = state.target.map(|t| self.scoped_key(t));
        let mut entry = unbound_widgets::overlay::OverlayEntry::menu(
            effective_key,
            anchor.unwrap_or(0),
            placed_desc.rect,
        );
        entry.anchor = anchor;
        let menu_state = state.menu.clone();
        let anchor_key = anchor.unwrap_or(0);
        self.overlay_layer(key, entry, |ov| {
            let menu_key = mix_key(effective_key, 0x4354_584D);
            let menu_node_desc = RawNodeDesc::container().absolute().bounds(
                0.0,
                0.0,
                placed_desc.rect.width().max(1.0),
                placed_desc.rect.height().max(1.0),
            );
            ov.widget(
                menu_key,
                menu_node_desc,
                WidgetSpec::ContextMenu {
                    owner_key: effective_key,
                    anchor_key,
                    desc: placed_desc,
                    menu_state,
                },
            );
        })
    }

    /// 声明式命令面板模态浮层：打开时自动接入 [`OverlayManager`] 焦点陷阱与 `Esc` 关闭。
    pub fn command_palette(
        &mut self,
        key: u64,
        mut desc: unbound_widgets::CommandPaletteDesc,
        state: &unbound_widgets::CommandPaletteState,
    ) -> NodeId {
        if !state.open {
            return self.world.root();
        }
        let effective_key = self.scoped_key(key);
        if desc.rect.width() <= 0.0 || desc.rect.height() <= 0.0 {
            let vp = self.viewport_rect();
            let w = 560.0f32.min((vp.width() - 32.0).max(240.0));
            let h = 360.0f32.min((vp.height() - 64.0).max(180.0));
            desc.rect = Rect::new(
                vp.origin.x + (vp.width() - w) * 0.5,
                vp.origin.y + (vp.height() - h) * 0.25,
                w,
                h,
            );
        }
        let entry = unbound_widgets::overlay::OverlayEntry::modal(
            effective_key,
            desc.rect,
            vec![effective_key],
        );
        let state_snapshot = state.clone();
        self.overlay_layer(key, entry, |ov| {
            let cp_key = mix_key(effective_key, 0x434D_4450);
            let cp_node_desc = RawNodeDesc::container().absolute().bounds(
                0.0,
                0.0,
                desc.rect.width().max(1.0),
                desc.rect.height().max(1.0),
            );
            ov.widget(
                cp_key,
                cp_node_desc,
                WidgetSpec::CommandPalette {
                    desc,
                    state: state_snapshot,
                },
            );
        })
    }

    /// 自定义子项构建闭包的可变行高虚拟列表容器。
    ///
    /// 仅对与当前视口（含上下 `overscan` 行余量）相交的索引区间调用 `build_item`，
    /// 前后不可见区域不构造任何子节点。
    pub fn virtual_list(
        &mut self,
        key: u64,
        list: &unbound_widgets::VariableVirtualList,
        overscan: usize,
        mut build_item: impl FnMut(&mut Self, usize, Rect),
    ) -> NodeId {
        let effective_key = self.scoped_key(key);
        let viewport_rect = self
            .widget_rect(effective_key)
            .unwrap_or_else(|| self.viewport_rect());
        let scroll_y = self
            .widget_state
            .borrow()
            .get::<unbound_widgets::ScrollState>(effective_key)
            .map_or(0.0, |st| st.offset().y)
            .max(0.0);
        let window = list.window(scroll_y, viewport_rect.height(), overscan);

        let mut desc = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::percent(100.0))
            .column()
            .flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        // 满高容器在纵向父容器里必须禁止收缩，否则同层还有其它满高兄弟时会被压扁。
        desc.flex_shrink = 0.0;
        let (_, scroll_node) = self.reconcile_node(key, desc, core::panic::Location::caller());
        self.declare_semantic(key, Role::List, "VirtualList");

        let parent_saved = core::mem::replace(&mut self.parent, scroll_node);
        let total_height = list.total_height().max(viewport_rect.height());
        let mut inner_desc = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::px(total_height));
        inner_desc.flex_shrink = 0.0;
        let content_key = mix_key(effective_key, 0x564C_4354);
        let (_, content_node) =
            self.reconcile_node(content_key, inner_desc, core::panic::Location::caller());

        self.pending.push(PendingWidget {
            key: effective_key,
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeBegin {
                scroll_key: effective_key,
                content_node,
                vertical: true,
            },
            z_base: self.z_base,
        });

        self.parent = content_node;
        let prev_depth = self.depth;
        self.depth = self.depth.saturating_add(1);
        for idx in window.first..window.last {
            let y = list.offset_of(idx);
            let h = list.height(idx);
            let item_rect = Rect::new(0.0, y, viewport_rect.width(), h);
            let item_key = mix_key(effective_key, idx as u64);
            let item_desc = RawNodeDesc::container().column().absolute().bounds(
                item_rect.origin.x,
                item_rect.origin.y,
                item_rect.width(),
                item_rect.height(),
            );
            self.container(item_key, item_desc, |row_view| {
                build_item(row_view, idx, item_rect);
            });
        }
        self.depth = prev_depth;

        self.pending.push(PendingWidget {
            key: mix_key(effective_key, 0x564C_454E),
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeEnd,
            z_base: self.z_base,
        });
        self.parent = parent_saved;
        scroll_node
    }

    /// 自定义子项构建闭包的多列虚拟瀑布流容器。
    ///
    /// 按最短列优先排布全部条目矩形，但仅对与当前视口（含上下 `overscan` 逻辑像素余量）
    /// 相交的条目调用 `build_item`。
    pub fn virtual_masonry(
        &mut self,
        key: u64,
        desc: unbound_widgets::VirtualMasonryDesc,
        items: &[unbound_widgets::virtual_masonry::MasonryItem],
        overscan: f32,
        mut build_item: impl FnMut(&mut Self, usize, Rect),
    ) -> NodeId {
        let effective_key = self.scoped_key(key);
        let viewport_rect = if desc.rect.width() > 0.0 && desc.rect.height() > 0.0 {
            desc.rect
        } else {
            self.widget_rect(effective_key)
                .unwrap_or_else(|| self.viewport_rect())
        };
        let scroll_y = self
            .widget_state
            .borrow()
            .get::<unbound_widgets::ScrollState>(effective_key)
            .map_or(0.0, |st| st.offset().y)
            .max(0.0);

        let cols = desc.columns.max(1);
        let gap = desc.gap.max(0.0);
        let col_w = ((viewport_rect.width() - gap * (cols.saturating_sub(1) as f32)) / cols as f32)
            .max(1.0);
        let mut col_heights = vec![0.0f32; cols];
        let overscan = overscan.max(0.0);
        let view_top = (scroll_y - overscan).max(0.0);
        let view_bottom = scroll_y + viewport_rect.height() + overscan;
        let mut visible = Vec::new();

        for (idx, item) in items.iter().enumerate() {
            let mut best_col = 0usize;
            let mut best_y = col_heights[0];
            for (c, &h) in col_heights.iter().enumerate().skip(1) {
                if h < best_y {
                    best_y = h;
                    best_col = c;
                }
            }
            let aspect = if item.aspect_ratio.is_finite() && item.aspect_ratio > 0.05 {
                item.aspect_ratio
            } else {
                1.0
            };
            let item_h = (col_w / aspect).max(1.0);
            let x = best_col as f32 * (col_w + gap);
            let y = best_y;
            col_heights[best_col] = y + item_h + gap;
            if y + item_h >= view_top && y <= view_bottom {
                visible.push((idx, Rect::new(x, y, col_w, item_h)));
            }
        }
        let total_height = col_heights
            .into_iter()
            .fold(0.0f32, f32::max)
            .max(viewport_rect.height());

        let mut container_desc = RawNodeDesc::container()
            .column()
            .flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        if desc.rect.width() > 0.0 && desc.rect.height() > 0.0 {
            container_desc = container_desc.size(desc.rect.width(), desc.rect.height());
        } else {
            container_desc = container_desc
                .width(Dim::percent(100.0))
                .height(Dim::percent(100.0));
        }
        let (_, scroll_node) =
            self.reconcile_node(key, container_desc, core::panic::Location::caller());
        self.declare_semantic(key, Role::Grid, "VirtualMasonry");

        let parent_saved = core::mem::replace(&mut self.parent, scroll_node);
        let mut inner_desc = RawNodeDesc::container()
            .width(Dim::percent(100.0))
            .height(Dim::px(total_height));
        inner_desc.flex_shrink = 0.0;
        let content_key = mix_key(effective_key, 0x564D_4354);
        let (_, content_node) =
            self.reconcile_node(content_key, inner_desc, core::panic::Location::caller());

        self.pending.push(PendingWidget {
            key: effective_key,
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeBegin {
                scroll_key: effective_key,
                content_node,
                vertical: true,
            },
            z_base: self.z_base,
        });

        self.parent = content_node;
        let prev_depth = self.depth;
        self.depth = self.depth.saturating_add(1);
        for (idx, item_rect) in visible {
            let item_key = mix_key(effective_key, items[idx].id ^ (idx as u64));
            let item_desc = RawNodeDesc::container().column().absolute().bounds(
                item_rect.origin.x,
                item_rect.origin.y,
                item_rect.width(),
                item_rect.height(),
            );
            self.container(item_key, item_desc, |cell_view| {
                build_item(cell_view, idx, item_rect);
            });
        }
        self.depth = prev_depth;

        self.pending.push(PendingWidget {
            key: mix_key(effective_key, 0x564D_454E),
            node: scroll_node,
            spec: WidgetSpec::ScrollScopeEnd,
            z_base: self.z_base,
        });
        self.parent = parent_saved;
        scroll_node
    }

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
    /// 内置 [`SettingsCard`] 包装器，自动管理设置行排版与连续多行间的 1px 分隔线。
    pub fn settings_card(
        &mut self,
        key: impl KeyDiscriminator,
        build: impl FnOnce(&mut SettingsCard<'_, '_>),
    ) -> NodeId {
        let raw_key = key.into_u64();
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(0.0))
            .gap(0.0)
            .radius(unbound_theme::typography::geometry::CARD_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()))
            .border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        desc.flex_shrink = 0.0;
        self.container(raw_key, desc, |ui| {
            let mut card = SettingsCard::new(ui, raw_key);
            build(&mut card);
        })
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
            .width(Dim::percent(100.0))
            .height(Dim::percent(100.0));
        // 满高容器在纵向父容器里必须禁止收缩，避免被同层的其它满高兄弟压扁。
        desc.flex_shrink = 0.0;
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

    /// 跨线程事件投递句柄：后台任务完成后可用它唤醒 UI 线程。
    ///
    /// 未安装异步通道时返回 `None`；调用方应把“无句柄”视为“本帧不需要唤醒”。
    #[must_use]
    pub fn event_sender(&self) -> Option<AppEventSender> {
        self.event_sender.clone()
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
        self.backdrop_image_sampled(key, asset, None)
    }

    /// 在整窗最底层绘制一张位图，并只采样其归一化子矩形。
    ///
    /// `sample_uv` 为 `None` 时与 [`Self::backdrop_image`] 完全一致；`Some` 表示纹理覆盖
    /// 的范围大于窗口（例如一整张主屏壁纸纹理），窗口只显示自己的那一块。
    pub fn backdrop_image_sampled(
        &mut self,
        key: u64,
        asset: unbound_asset::AssetId,
        sample_uv: Option<Rect>,
    ) -> bool {
        use unbound_widgets::{ImageAssetResolver, ImageDesc, ObjectFit};
        if self.assets.resolve(asset).is_none() {
            return false;
        }
        let viewport = self.ctx.viewport;
        let mut desc = ImageDesc::new(Rect::new(0.0, 0.0, viewport.x, viewport.y), viewport, 0)
            .with_asset(asset)
            .with_fit(ObjectFit::Fill);
        if let Some(uv) = sample_uv {
            desc = desc.with_sample_uv(uv);
        }
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

    /// 为本帧挂载且处于展开态（`open == true`）的下拉选择框自动在顶层渲染选项列表浮层。
    pub fn render_open_select_popups(&mut self) {
        let registered = core::mem::take(self.registered_selects);
        for (select_key, desc) in registered {
            let is_open = self
                .widget_state
                .borrow()
                .get::<unbound_widgets::SelectState>(select_key)
                .is_some_and(|st| st.open);
            if !is_open {
                continue;
            }
            let current_selected = self
                .widget_state
                .borrow()
                .get::<unbound_widgets::SelectState>(select_key)
                .and_then(|st| st.selected);
            let anchor = self
                .widget_rect(select_key)
                .unwrap_or(Rect::new(0.0, 0.0, 160.0, 32.0));
            let width = anchor.width().max(120.0);
            let height = desc.options.len() as f32 * unbound_widgets::select::SELECT_ROW_HEIGHT;
            let placed = unbound_widgets::place_combo_popup(
                anchor,
                Vec2::new(width, height),
                unbound_widgets::select::SELECT_ROW_HEIGHT,
                0.0,
                current_selected,
                Rect::new(0.0, 0.0, self.ctx.viewport.x, self.ctx.viewport.y),
                unbound_widgets::select::SELECT_POPUP_MARGIN,
            );
            let popup_key = select_popup_key(select_key);
            let mut popup_desc = RawNodeDesc::container()
                .column()
                .absolute()
                .bounds(
                    placed.origin.x,
                    placed.origin.y,
                    placed.width(),
                    placed.height(),
                )
                .z_index(1250)
                .portal_root()
                .padding(unbound_abi::Edges::all(0.0))
                .gap(0.0)
                .radius(8.0)
                .fill_token(u32::from(unbound_theme::Token::SURFACE_RAISED.raw()))
                .border(u32::from(unbound_theme::Token::OUTLINE.raw()), 1.0);
            popup_desc.shadow_blur = 24.0;

            let mut option_keys = Vec::with_capacity(desc.options.len());
            let old_scope = self.key_scope;
            self.key_scope = KeyScope::root();
            self.container(popup_key, popup_desc, |popup| {
                for (idx, label) in desc.options.iter().enumerate() {
                    let is_selected = current_selected == Some(idx);
                    let is_disabled = desc.option_disabled(idx);
                    let option_key = select_option_key(select_key, idx);
                    option_keys.push(option_key);

                    let tokens = if is_selected {
                        unbound_widgets::ButtonTokens {
                            fill: unbound_theme::Token::HOVER,
                            fill_hover: unbound_theme::Token::HOVER,
                            fill_pressed: unbound_theme::Token::PRESSED,
                            fill_disabled: unbound_theme::Token::NONE,
                            border: unbound_theme::Token::NONE,
                            label: unbound_theme::Token::TEXT_PRIMARY,
                            label_disabled: unbound_theme::Token::ON_DISABLED,
                            focus_ring: unbound_theme::Token::NONE,
                        }
                    } else {
                        unbound_widgets::ButtonTokens {
                            fill: unbound_theme::Token::NONE,
                            fill_hover: unbound_theme::Token::CONTROL_HOVER,
                            fill_pressed: unbound_theme::Token::SURFACE_SUNKEN,
                            fill_disabled: unbound_theme::Token::NONE,
                            border: unbound_theme::Token::NONE,
                            label: if is_disabled {
                                unbound_theme::Token::TEXT_DISABLED
                            } else {
                                unbound_theme::Token::TEXT_PRIMARY
                            },
                            label_disabled: unbound_theme::Token::ON_DISABLED,
                            focus_ring: unbound_theme::Token::NONE,
                        }
                    };

                    let mut btn = ButtonDesc::label(label)
                        .variant(unbound_widgets::ButtonVariant::Subtle)
                        .left_align()
                        .selected(is_selected)
                        .with_font_size(14.0)
                        .with_label_inset(unbound_widgets::select::SELECT_TEXT_INSET)
                        .with_tokens(tokens);
                    btn.radius = 4.0;
                    btn.shadow = 0.0;

                    popup.declare_semantics(
                        option_key,
                        SemanticIntent::new(Role::ListBoxOption, label.clone())
                            .with_selected(is_selected),
                    );

                    let mut row_desc = RawNodeDesc::container().column();
                    row_desc.width = Dim::percent(100.0);
                    row_desc.height = Dim::px(unbound_widgets::select::SELECT_ROW_HEIGHT);
                    row_desc.padding = unbound_abi::Edges::ltrb(
                        unbound_widgets::select::SELECT_HIGHLIGHT_INSET_X,
                        unbound_widgets::select::SELECT_HIGHLIGHT_INSET_Y,
                        unbound_widgets::select::SELECT_HIGHLIGHT_INSET_X,
                        unbound_widgets::select::SELECT_HIGHLIGHT_INSET_Y,
                    );
                    row_desc.flex_shrink = 0.0;
                    let item_box_key = mix_key(popup_key, (idx as u64) + 1);
                    popup.container(item_box_key, row_desc, |row| {
                        let mut item_layout = RawNodeDesc::container();
                        item_layout.width = Dim::percent(100.0);
                        item_layout.height = Dim::px(
                            unbound_widgets::select::SELECT_ROW_HEIGHT
                                - 2.0 * unbound_widgets::select::SELECT_HIGHLIGHT_INSET_Y,
                        );
                        row.add_with_layout(option_key, item_layout, btn);
                    });
                }
            });
            self.key_scope = old_scope;
            self.active_select_popups.push(ActiveSelectPopup {
                select_key,
                bounds: placed,
                option_keys,
            });
        }
    }
}

/// 应用契约。
///
/// 该 trait 刻意保持极简：输入状态、输入事件、输出图元。
/// 窗口化外壳、无头测试与 showcase 均驱动完全相同的三个方法。
///
pub trait Application {
    /// 应用状态类型。
    type State: 'static;

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
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    /// 启动系统原生窗口拖拽。
    StartDrag,
    /// 最小化窗口。
    Minimize,
    /// 切换最大化与还原状态。
    ToggleMaximize,
    /// 请求关闭窗口。
    Close,
    /// 请求平台启动 Win32 OLE 向外文件拖出操作。
    BeginExternalFileDrag(unbound_platform::OutboundFileDragRequest),
    /// 同步（创建或重定位）越界无焦点原生弹出窗。
    SyncNativePopup {
        /// 弹出窗逻辑标识（对应浮层节点键）。
        id: u64,
        /// 目标屏幕物理矩形与宿主关联配置。
        config: unbound_platform::PopupWindowConfig,
    },
    /// 显式关闭指定的越界原生弹出窗。
    CloseNativePopup {
        /// 待关闭的弹出窗逻辑标识。
        id: u64,
    },
    /// 更新运行器帧节流与遮挡/最小化暂停策略。
    SetThrottlePolicy(unbound_platform::RenderThrottlePolicy),
    /// 请求多窗口协调器按给定配置打开新窗口。
    OpenWindow(unbound_platform::WindowConfig),
    /// 请求多窗口协调器关闭指定标识的窗口。
    CloseWindow(unbound_platform::WindowId),
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
    /// 本帧待窗口运行器执行的外部纹理槽上传、更新与释放操作队列。
    pub external_texture_ops: Vec<ExternalTextureOp>,
}

/// 单帧已挂载上下文菜单的点击反解条目。
struct ActiveContextMenu {
    /// 调用 `ViewBuilder::context_menu` 时传入的有效键（经 `scoped_key` 作用域混合）。
    owner_key: u64,
    /// 实际发射菜单控件的节点键（`entry.key`）。
    widget_key: u64,
    /// 每项是否可选（非分隔线且非禁用）。
    selectable: Vec<bool>,
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
    /// 根背景清屏 alpha：0 让 DWM 后景材质透出，1 输出主题表面色。
    ///
    /// 宿主在每帧回调里按后景计划写入，窗口运行器在渲染前应用。它只影响清屏色，
    /// 不改变交换链配置，因此可以在运行期自由切换。
    clear_alpha: f32,
    /// 窗口外观意图（默认关闭）。
    backdrop_intent: AppearanceIntent,
    /// 当前交换链是否真的按预乘 alpha 创建（原生后景的前提）。
    ///
    /// 宿主据此决定后景来源：驱动或表面不支持时交换链会回退为不透明，此时
    /// 请求原生材质只会得到「内容盖住材质」的结果，必须继续走自绘。
    premultiplied_surface: bool,
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
    /// 表单控件双向绑定集合。
    form_bindings: FormBindingStore,
    registered_selects: Vec<(u64, unbound_widgets::SelectDesc)>,
    active_select_popups: Vec<ActiveSelectPopup>,
    /// 本帧已挂载的上下文菜单项可选表，用于将 `menu_item_node` 点击回传至宿主菜单键。
    active_context_menus: Vec<ActiveContextMenu>,
    /// 主窗口客户区左上角在屏幕物理坐标系中的位置，用于换算越界原生弹出窗矩形。
    window_screen_origin: (i32, i32),
    /// 归属的平台窗口标识（用于构造 `PopupWindowConfig::owner`）。
    window_owner_id: unbound_platform::WindowId,
    /// 当前已同步至原生弹出窗宿主的越界浮层集合：`overlay_key -> config`。
    active_native_popups: FxHashMap<u64, unbound_platform::PopupWindowConfig>,
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
            clear_alpha: 1.0,
            backdrop_intent: AppearanceIntent::Material(None),
            premultiplied_surface: false,
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
            form_bindings: FormBindingStore::new(),
            registered_selects: Vec::new(),
            active_select_popups: Vec::new(),
            active_context_menus: Vec::new(),
            window_screen_origin: (0, 0),
            window_owner_id: unbound_platform::WindowId(0),
            active_native_popups: FxHashMap::default(),
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

    /// 设置根背景清屏 alpha（仅供兼容与自绘路径专用）。
    ///
    /// `0.0` 让窗口内容之外的区域完全透明；`1.0` 输出主题表面色。
    ///
    /// ### 语义优先级说明
    /// 该接口专供无原生后景意图时的传统兼容路径或自绘后景使用。
    /// 当窗口后景意图开启时（例如通过 [`Self::set_backdrop`] 设置了系统云母或亚克力等原生材质），
    /// 外观意图账本会根据后景计划自动派生对应的清屏 alpha（原生材质通常派生为 `0.0`），
    /// 此时本方法显式指定的值会被外观账本的派生值覆盖；仅在后景意图关闭时，本方法设置的
    /// 清屏 alpha 才会生效并传递至渲染器。
    pub fn set_clear_alpha(&mut self, alpha: f32) {
        self.clear_alpha = alpha.clamp(0.0, 1.0);
    }

    /// 当前根背景清屏 alpha。
    #[must_use]
    pub const fn clear_alpha(&self) -> f32 {
        self.clear_alpha
    }

    /// 当前交换链是否按预乘 alpha 创建。
    ///
    /// 只有它为 `true` 时，把根背景清屏 alpha 降到 0 才能让 DWM 材质透出；
    /// 为 `false` 时应继续走自绘后景，否则窗口会变成不透光的纯色壳。
    #[must_use]
    pub const fn premultiplied_surface(&self) -> bool {
        self.premultiplied_surface
    }

    /// 记录交换链的实际合成方式（由窗口运行器在创建交换链后写入）。
    pub(crate) fn set_premultiplied_surface(&mut self, supported: bool) {
        self.premultiplied_surface = supported;
    }

    /// 设置窗口后景材质意图。
    ///
    /// 传入 `Some(...)` 开启对应材质（云母、亚克力等）；传入 `None` 或 `Some(WindowBackdropMaterial::None)`
    /// 关闭后景恢复不透明。窗口运行器在每帧渲染前读取该值，通过外观意图账本按需向系统下发变更。
    pub fn set_backdrop(&mut self, backdrop: Option<WindowBackdropMaterial>) {
        self.backdrop_intent = AppearanceIntent::Material(backdrop);
    }

    /// 当前窗口后景材质意图（若当前为亚克力自定义色调等扩展意图则返回 `None`）。
    #[must_use]
    pub const fn backdrop(&self) -> Option<WindowBackdropMaterial> {
        match self.backdrop_intent {
            AppearanceIntent::Material(mat) => mat,
            AppearanceIntent::AccentTint(_) | AppearanceIntent::Selection { .. } => None,
        }
    }

    /// 设置亚克力自定义色调后景（走 accent policy 通道，ABGR 格式）。
    #[cfg(test)]
    pub(crate) fn set_accent_tint(&mut self, argb: u32) {
        self.backdrop_intent = AppearanceIntent::AccentTint(argb);
    }

    /// 设置完整后景选择意图。
    #[cfg(test)]
    pub(crate) fn set_backdrop_selection(
        &mut self,
        selection: unbound_backdrop::BackdropSelection,
    ) {
        self.backdrop_intent = AppearanceIntent::Selection {
            selection,
            accent_tint: None,
        };
    }

    /// 设置底层外观意图。
    #[cfg(test)]
    pub(crate) fn set_backdrop_intent(&mut self, intent: AppearanceIntent) {
        self.backdrop_intent = intent;
    }

    /// 当前底层外观意图。
    #[must_use]
    pub(crate) const fn backdrop_intent(&self) -> AppearanceIntent {
        self.backdrop_intent
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

    /// 设置指定下拉选择框的展开/收起状态。
    pub fn set_select_open(&self, key: u64, open: bool) {
        self.ctx.set_select_open(key, open);
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
        if button == PointerButton::Left && !self.active_select_popups.is_empty() {
            let in_any = self.active_select_popups.iter().any(|p| {
                p.bounds.contains_point(pos)
                    || self
                        .widget_rect(p.select_key)
                        .is_some_and(|r| r.contains_point(pos))
            });
            if !in_any {
                let mut store = self.widget_state.borrow_mut();
                for popup in &self.active_select_popups {
                    let st = store.entry::<unbound_widgets::SelectState>(popup.select_key);
                    st.close();
                }
                drop(store);
                self.active_select_popups.clear();
                flags = flags.merge(DirtyFlags::ALL);
            }
        }
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

    /// 获取当前文本渲染生效报告只读快照。
    #[must_use]
    pub fn text_render_report(&self) -> unbound_text::TextRenderReport {
        self.glyph_cache.report()
    }

    /// 获取当前生效的文本渲染设置快照。
    #[must_use]
    pub fn text_settings(&self) -> &unbound_text::TextRenderSettings {
        self.glyph_cache.settings()
    }

    /// 运行期应用文本渲染设置，按三档失效策略清空对应缓存并按需重建光栅器。
    pub fn apply_text_settings(
        &mut self,
        settings: &unbound_text::TextRenderSettings,
    ) -> unbound_text::InvalidationTier {
        let tier = self
            .glyph_cache
            .apply_settings(settings, Some(&mut self.text_engine));
        if tier != unbound_text::InvalidationTier::None {
            self.force_rebuild = true;
            self.dirty |= DirtyFlags::ALL;
        }
        tier
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
        if step > 0.0 && !self.scroll_regions.is_empty() {
            let scrollbar_changed = {
                let mut store = self.widget_state.borrow_mut();
                let mut changed = false;
                for &(scroll_key, viewport_rect, _, _) in &self.scroll_regions {
                    if let Some(st) = store.get_mut::<unbound_widgets::ScrollState>(scroll_key) {
                        let hovering = (self.ctx.hovered == Some(scroll_key)
                            || self.ctx.hovered == Some(scroll_key ^ (1 << 63))
                            || self.ctx.hovered == Some(scroll_key ^ (1 << 62))
                            || (self.ctx.hovered.is_some()
                                && viewport_rect.contains_point(self.ctx.pointer)))
                            && viewport_rect.contains_point(self.ctx.pointer);
                        if st.tick_scrollbar(step, hovering) {
                            changed = true;
                        }
                    }
                }
                changed
            };
            if scrollbar_changed {
                self.force_rebuild = true;
                self.mark(DirtyFlags::PAINT);
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
        // 运行期文本渲染设置变更在视图构建前生效，按三档失效粒度处理字形与排版缓存。
        if let Some(settings) = self.ctx.text_settings.take() {
            self.apply_text_settings(&settings);
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
                external_texture_ops: self.asset_hub.take_external_texture_ops(),
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
        self.form_bindings.clear();
        self.registered_selects.clear();
        self.active_select_popups.clear();
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
        let mut declared_overlays = FxHashSet::default();
        let mut overlay_pending_ranges = Vec::new();
        let event_sender = self.ctx.event_sender();
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
            event_sender,
            depth: 0,
            z_base: 0,
            key_scope: KeyScope::root(),
            form_bindings: &mut self.form_bindings,
            registered_selects: &mut self.registered_selects,
            active_select_popups: &mut self.active_select_popups,
            overlays: &mut self.overlays,
            focus: &mut self.focus,
            declared_overlays: &mut declared_overlays,
            overlay_pending_ranges: &mut overlay_pending_ranges,
        };
        self.app.view(&self.state, &mut view);
        view.render_open_select_popups();
        drop(view);
        self.finalize_declared_overlays(&declared_overlays, &overlay_pending_ranges);
        self.sync_native_popups();
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
            external_texture_ops: self.asset_hub.take_external_texture_ops(),
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

    /// 取出并清空本帧经事件冒泡或应用显式请求产出的待执行窗口控制命令。
    pub fn drain_window_commands(&mut self) -> Vec<WindowCommand> {
        let mut cmds = core::mem::take(&mut self.window_commands);
        cmds.append(&mut self.ctx.pending_window_commands);
        cmds
    }

    /// 请求平台启动 Win32 OLE 向外文件拖出操作。
    pub fn request_external_file_drag(
        &mut self,
        request: unbound_platform::OutboundFileDragRequest,
    ) {
        self.window_commands
            .push(WindowCommand::BeginExternalFileDrag(request));
    }

    /// 动态更新窗口运行器的自适应帧节流策略。
    pub fn set_throttle_policy(&mut self, policy: unbound_platform::RenderThrottlePolicy) {
        self.window_commands
            .push(WindowCommand::SetThrottlePolicy(policy));
    }

    /// 请求多窗口协调器打开一个辅助窗口。
    pub fn open_window(&mut self, config: unbound_platform::WindowConfig) {
        self.window_commands.push(WindowCommand::OpenWindow(config));
    }

    /// 请求多窗口协调器关闭指定标识的窗口。
    pub fn close_window(&mut self, id: unbound_platform::WindowId) {
        self.window_commands.push(WindowCommand::CloseWindow(id));
    }

    /// 同步宿主窗口客户区在屏幕物理坐标系下的原点与窗口标识，用于换算越界原生弹出窗位置。
    pub fn set_window_screen_origin(
        &mut self,
        origin: (i32, i32),
        owner_id: unbound_platform::WindowId,
    ) {
        self.window_screen_origin = origin;
        self.window_owner_id = owner_id;
    }

    /// 将原生 OLE 向外文件拖出操作的结果回灌给应用运行时。
    pub fn notify_external_file_drag_finished(
        &mut self,
        outcome: unbound_platform::OutboundFileDragOutcome,
    ) -> DirtyFlags {
        let flags = self.dispatch(AppEvent::ExternalFileDragFinished(outcome));
        self.mark(flags);
        flags
    }

    /// 将平台侧越界原生弹出窗关闭信号回流给应用运行时并同步关闭对应浮层。
    pub fn notify_native_popup_dismissed(&mut self, id: u64) -> DirtyFlags {
        self.active_native_popups.remove(&id);
        let _ = self.overlays.close(id, &mut self.focus);
        self.sync_focus_context();
        let flags = self
            .dispatch(AppEvent::NativePopupDismissed { id })
            .merge(DirtyFlags::PAINT | DirtyFlags::LAYOUT);
        self.mark(flags);
        flags
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
    /// 在视图构建完成后同步 [`OverlayManager`] 锚点裁剪并按浮层 Z 序重排延迟控件。
    fn finalize_declared_overlays(
        &mut self,
        declared_overlays: &FxHashSet<u64>,
        overlay_pending_ranges: &[(u64, NodeId, usize, usize)],
    ) {
        let _ = self.overlays.sync_declared_overlays(
            declared_overlays,
            &self.seen_keys,
            &mut self.focus,
        );
        if overlay_pending_ranges.is_empty() {
            return;
        }
        let mut is_overlay_idx = vec![false; self.pending.len()];
        for &(_, _, start, end) in overlay_pending_ranges {
            let end = end.min(self.pending.len());
            for flag in is_overlay_idx.iter_mut().take(end).skip(start) {
                *flag = true;
            }
        }
        let old_pending = core::mem::take(&mut self.pending);
        let mut base_pending = Vec::with_capacity(old_pending.len());
        let mut slots: Vec<Option<PendingWidget>> = old_pending.into_iter().map(Some).collect();
        for (idx, flag) in is_overlay_idx.iter().enumerate() {
            if !*flag {
                if let Some(item) = slots[idx].take() {
                    base_pending.push(item);
                }
            }
        }
        let mut surviving: Vec<(usize, usize, usize)> = Vec::new();
        for &(overlay_key, overlay_node, start, end) in overlay_pending_ranges {
            if let Some(order) = self
                .overlays
                .stack()
                .entries()
                .iter()
                .position(|e| e.node == overlay_key)
            {
                surviving.push((order, start, end.min(slots.len())));
            } else {
                self.seen_keys.remove(&overlay_key);
                self.node_keys.remove(&overlay_key);
                if self.world.get(overlay_node).is_some() {
                    self.world.remove(overlay_node);
                }
            }
        }
        surviving.sort_by_key(|&(order, _, _)| order);
        for (_, start, end) in surviving {
            for slot in slots.iter_mut().take(end).skip(start) {
                if let Some(item) = slot.take() {
                    base_pending.push(item);
                }
            }
        }
        self.pending = base_pending;
    }

    /// 检查当前活跃浮层是否越出主窗口客户区，并生成 `SyncNativePopup` / `CloseNativePopup` 命令。
    fn sync_native_popups(&mut self) {
        let vp_w = self.ctx.viewport.x;
        let vp_h = self.ctx.viewport.y;
        let scale = self.ctx.effective_scale();
        let mut desired: FxHashMap<u64, unbound_platform::PopupWindowConfig> = FxHashMap::default();
        for entry in self.overlays.stack().visible_entries() {
            let b = entry.bounds;
            if b.width() <= 0.0 || b.height() <= 0.0 {
                continue;
            }
            let overflows =
                b.origin.x < 0.0 || b.origin.y < 0.0 || b.max().x > vp_w || b.max().y > vp_h;
            if !overflows && !entry.prefer_native_popup {
                continue;
            }
            let screen_rect = unbound_platform::ScreenRect::new(
                self.window_screen_origin.0 + (b.origin.x * scale).round() as i32,
                self.window_screen_origin.1 + (b.origin.y * scale).round() as i32,
                (b.width() * scale).round().max(1.0) as u32,
                (b.height() * scale).round().max(1.0) as u32,
            );
            let config =
                unbound_platform::PopupWindowConfig::new(self.window_owner_id, screen_rect);
            desired.insert(entry.node, config);
        }

        for (&id, &config) in &desired {
            if self.active_native_popups.get(&id) != Some(&config) {
                self.active_native_popups.insert(id, config);
                self.window_commands
                    .push(WindowCommand::SyncNativePopup { id, config });
            }
        }
        let stale: Vec<u64> = self
            .active_native_popups
            .keys()
            .copied()
            .filter(|id| !desired.contains_key(id))
            .collect();
        for id in stale {
            self.active_native_popups.remove(&id);
            self.window_commands
                .push(WindowCommand::CloseNativePopup { id });
        }
    }

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
                    WidgetSpec::Image(img) => (img.rect.width() > 0.0 || img.rect.height() > 0.0)
                        .then_some(Vec2::new(img.rect.width(), img.rect.height())),
                    WidgetSpec::ExternalImage { size, .. } => Some(*size),
                    WidgetSpec::TreeView { desc, .. } => (desc.rect.width() > 0.0
                        && desc.rect.height() > 0.0)
                        .then_some(Vec2::new(desc.rect.width(), desc.rect.height())),
                    WidgetSpec::AddressBar { desc, .. } => {
                        (desc.rect.width() > 0.0).then_some(Vec2::new(
                            desc.rect.width(),
                            if desc.rect.height() > 0.0 {
                                desc.rect.height()
                            } else {
                                unbound_widgets::ADDRESS_BAR_HEIGHT
                            },
                        ))
                    }
                    WidgetSpec::Minimap { desc, .. } => (desc.rect.width() > 0.0
                        || desc.rect.height() > 0.0)
                        .then_some(Vec2::new(desc.rect.width(), desc.rect.height())),
                    WidgetSpec::Popover { desc, .. } => Some(desc.size),
                    WidgetSpec::ContextMenu { desc, .. } => {
                        Some(Vec2::new(desc.rect.width(), desc.rect.height()))
                    }
                    WidgetSpec::CommandPalette { desc, .. } => {
                        Some(Vec2::new(desc.rect.width(), desc.rect.height()))
                    }
                    WidgetSpec::Dock { .. }
                    | WidgetSpec::Text { .. }
                    | WidgetSpec::BoxSurface { .. }
                    | WidgetSpec::InteractiveHit { .. }
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
        self.active_context_menus.clear();
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
                    let raw_output = if let Some(tabs_desc) =
                        widget.as_any().downcast_ref::<unbound_widgets::TabsDesc>()
                    {
                        let mut resolved = tabs_desc.clone();
                        resolved.rect = rect;
                        for it in &mut resolved.items {
                            it.widget_icon = resolve_widget_icon(&self.asset_hub, it.widget_icon);
                        }
                        let tabs_state = store
                            .get::<unbound_widgets::TabsState>(entry.key)
                            .cloned()
                            .unwrap_or_default();
                        let focused_key = self.ctx.focused.filter(|_| self.ctx.focus_visible);
                        let hovered_key = self.ctx.hovered;
                        let pressed_key = self.ctx.pressed;
                        let resolve_sub = |sub_key: u64| -> ControlState {
                            let sub_phase = if pressed_key == Some(sub_key) {
                                unbound_widgets::PointerPhase::Pressed
                            } else if hovered_key == Some(sub_key) {
                                unbound_widgets::PointerPhase::Hovered
                            } else {
                                unbound_widgets::PointerPhase::Idle
                            };
                            ControlState {
                                phase: sub_phase,
                                focused: focused_key == Some(sub_key),
                                disabled: state.disabled,
                                selected: false,
                            }
                        };
                        unbound_widgets::emit_tabs_with_control_states(
                            entry.key,
                            &resolved,
                            &tabs_state,
                            &mut text_ctx,
                            |idx| {
                                let item_key = unbound_widgets::tab_item_node(entry.key, idx);
                                let close_key = unbound_widgets::tab_close_node(entry.key, idx);
                                let mut ctrl = resolve_sub(item_key);
                                // 指针悬停或按下所属关闭按钮时，当前标签卡片维持悬停底板，
                                // 绝不影响其他标签页或关闭按钮的独立状态。
                                if ctrl.phase == unbound_widgets::PointerPhase::Idle
                                    && (hovered_key == Some(close_key)
                                        || pressed_key == Some(close_key))
                                {
                                    ctrl.phase = unbound_widgets::PointerPhase::Hovered;
                                }
                                ctrl
                            },
                            |idx| resolve_sub(unbound_widgets::tab_close_node(entry.key, idx)),
                            resolve_sub(unbound_widgets::tab_add_node(entry.key)),
                        )
                    } else if let Some(addr_desc) = widget
                        .as_any()
                        .downcast_ref::<unbound_widgets::AddressBarDesc>()
                    {
                        if addr_desc.segments.iter().any(|seg| {
                            matches!(
                                seg.widget_icon,
                                unbound_widgets::WidgetIcon::ExternalTexture(_)
                            )
                        }) {
                            let mut resolved = addr_desc.clone();
                            for seg in &mut resolved.segments {
                                seg.widget_icon =
                                    resolve_widget_icon(&self.asset_hub, seg.widget_icon);
                            }
                            resolved.emit_laid_out_erased(
                                entry.key,
                                rect,
                                state,
                                &store,
                                &mut text_ctx,
                            )
                        } else {
                            widget.emit_laid_out_erased(
                                entry.key,
                                rect,
                                state,
                                &store,
                                &mut text_ctx,
                            )
                        }
                    } else if let Some(btn_desc) = widget
                        .as_any()
                        .downcast_ref::<unbound_widgets::ButtonDesc>()
                    {
                        if matches!(
                            btn_desc.widget_icon,
                            unbound_widgets::WidgetIcon::ExternalTexture(_)
                        ) {
                            let mut resolved = btn_desc.clone();
                            resolved.widget_icon =
                                resolve_widget_icon(&self.asset_hub, resolved.widget_icon);
                            resolved.emit_laid_out_erased(
                                entry.key,
                                rect,
                                state,
                                &store,
                                &mut text_ctx,
                            )
                        } else {
                            widget.emit_laid_out_erased(
                                entry.key,
                                rect,
                                state,
                                &store,
                                &mut text_ctx,
                            )
                        }
                    } else {
                        widget.emit_laid_out_erased(entry.key, rect, state, &store, &mut text_ctx)
                    };
                    let output = apply_z_base(raw_output, entry.z_base);
                    self.collector.push(entry.key, output);
                }
                WidgetSpec::Button(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    desc.widget_icon = resolve_widget_icon(&self.asset_hub, desc.widget_icon);
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
                WidgetSpec::Image(desc) => {
                    let mut desc = desc.clone();
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        desc.rect = rect;
                    }
                    desc.z = entry.z_base.saturating_add(desc.z);
                    let out = unbound_widgets::emit_image_with_assets(
                        entry.key,
                        &desc,
                        state,
                        &self.asset_hub,
                    );
                    self.collector.push(entry.key, out);
                }
                WidgetSpec::ExternalImage { handle, size, fit } => {
                    let draw_rect = if rect.width() > 0.0 && rect.height() > 0.0 {
                        rect
                    } else {
                        Rect::new(rect.origin.x, rect.origin.y, size.x, size.y)
                    };
                    let mut desc =
                        if let Some((slot, w, h)) = self.asset_hub.external_texture_slot(*handle) {
                            unbound_widgets::ImageDesc::new(
                                draw_rect,
                                Vec2::new(w as f32, h as f32),
                                slot,
                            )
                        } else {
                            unbound_widgets::ImageDesc::new(draw_rect, *size, 0)
                                .with_asset(unbound_asset::AssetId(0))
                        };
                    desc.fit = *fit;
                    desc.z = entry.z_base.saturating_add(desc.z);
                    let out = unbound_widgets::emit_image_with_assets(
                        entry.key,
                        &desc,
                        state,
                        &self.asset_hub,
                    );
                    self.collector.push(entry.key, out);
                }
                WidgetSpec::TreeView {
                    desc,
                    scroll_y,
                    content_height,
                    hovered_row,
                    rows,
                } => {
                    let mut desc = desc.clone();
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        desc.rect = rect;
                    }
                    desc.z = entry.z_base.saturating_add(desc.z);
                    let tokens = if desc.navigation_pane {
                        unbound_widgets::TreeViewTokens::navigation_pane()
                    } else {
                        unbound_widgets::TreeViewTokens::standard()
                    };
                    let mut bg = unbound_widgets::emit_tree_background(&desc, tokens);
                    if bg.a11y.is_none() {
                        bg.a11y = Some(A11yNode::new(Role::Tree, "TreeView", desc.rect));
                    }
                    self.collector.push(entry.key, bg);

                    // 基于最终布局视口钳制有效滚动偏移，并反向对齐运行时 `ScrollState` 与同步水位，
                    // 确保程序化滚动与后续滚轮增量从同一基线衔接，同时建立严格对齐视口边界的 GPU 裁剪作用域。
                    let scroll_desc = unbound_widgets::ScrollViewDesc::new(
                        desc.rect,
                        Vec2::new(desc.rect.width(), *content_height),
                    );
                    let effective_scroll_y = scroll_desc.clamp_offset(Vec2::new(0.0, *scroll_y)).y;
                    {
                        let mut store = self.widget_state.borrow_mut();
                        let scroll_state = store.use_state_or_init::<unbound_widgets::ScrollState>(
                            entry.key,
                            unbound_widgets::ScrollState::new,
                        );
                        if (scroll_state.offset().y - effective_scroll_y).abs() > 0.01 {
                            scroll_state.jump_to(&scroll_desc, Vec2::new(0.0, effective_scroll_y));
                        }
                        store
                            .use_state::<TreeViewScrollSync>(entry.key)
                            .last_wheel_y = effective_scroll_y;
                    }
                    let clip_node = GpuClipNode::from_aabb([
                        desc.rect.min().x,
                        desc.rect.min().y,
                        desc.rect.max().x,
                        desc.rect.max().y,
                    ]);
                    self.collector.push_clip(clip_node, desc.rect);

                    let font_size = if desc.density == unbound_widgets::TreeViewDensity::Compact {
                        12.5
                    } else {
                        13.0
                    };
                    let mut any_row_hovered = false;
                    for snap in rows {
                        // 基于布局求解后的最终 `desc.rect` 重算每行世界坐标，杜绝 `(0, 0)` 原点烘焙溢出。
                        let row_y = desc.rect.origin.y + snap.item as f32 * desc.row_height
                            - effective_scroll_y;
                        let row_rect =
                            Rect::new(desc.rect.origin.x, row_y, desc.rect.size.x, desc.row_height);
                        if row_rect.max().y < desc.rect.origin.y
                            || row_rect.origin.y > desc.rect.max().y
                        {
                            continue;
                        }
                        let row = unbound_widgets::TreeViewRow {
                            row: snap.row,
                            item: snap.item,
                            rect: row_rect,
                        };
                        if snap.divider_before {
                            let divider_out =
                                unbound_widgets::emit_tree_row_divider(&desc, &row, tokens);
                            self.collector
                                .push(row.row.id ^ 0x5452_4545_0002, divider_out);
                        }
                        // 将 `WidgetIcon::ExternalTexture(handle_raw)` 统一桥接至 `AssetHub` 已上传的外部纹理 GPU 槽位；
                        // 若句柄尚未驻留则退化为 `WidgetIcon::None`，避免绑定无效纹理槽。
                        let resolved_icon = resolve_widget_icon(&self.asset_hub, snap.icon);
                        let has_icon = !matches!(resolved_icon, unbound_widgets::TreeRowIcon::None);
                        let raw_label = snap
                            .label
                            .clone()
                            .unwrap_or_else(|| format!("Node {}", row.row.id));
                        let label_x =
                            unbound_widgets::tree_row_label_x(&desc, row.row.depth, has_icon);
                        let avail_width = (row.rect.max().x - label_x - 8.0).max(0.0);
                        let elided = unbound_widgets::elide_text_with_measure(
                            &raw_label,
                            avail_width,
                            unbound_widgets::ElideMode::End,
                            |candidate| text_ctx.measure_line(candidate, font_size, 400).x,
                        );
                        let label_rect =
                            Rect::new(label_x, row.rect.origin.y, avail_width, row.rect.height());
                        let label = if avail_width > 0.0 && !elided.display.is_empty() {
                            text_ctx.shape_line(
                                &elided.display,
                                label_rect,
                                font_size,
                                400,
                                unbound_widgets::AlignX::Left,
                                unbound_widgets::AlignY::Middle,
                            )
                        } else {
                            Vec::new()
                        };
                        let toggle_key = unbound_widgets::tree_toggle_node(row.row.id);
                        // 单行悬停与按压态：将展开箭头子热区 `tree_toggle_node(row.row.id)` 归属到所属行，
                        // 避免鼠标移入展开箭头时整行高亮闪烁消失，同时兼容业务层手动设置的 `hovered_row`。
                        let is_row_hovered = self.ctx.hovered == Some(row.row.id)
                            || self.ctx.hovered == Some(toggle_key)
                            || *hovered_row == Some(row.row.id);
                        if is_row_hovered {
                            any_row_hovered = true;
                        }
                        let is_row_pressed = self.ctx.pressed == Some(row.row.id)
                            || self.ctx.pressed == Some(toggle_key);
                        let row_phase = if is_row_pressed {
                            unbound_widgets::PointerPhase::Pressed
                        } else if is_row_hovered {
                            unbound_widgets::PointerPhase::Hovered
                        } else {
                            unbound_widgets::PointerPhase::Idle
                        };
                        let row_state = ControlState {
                            phase: row_phase,
                            focused: self.ctx.focused == Some(row.row.id) && self.ctx.focus_visible,
                            disabled: state.disabled,
                            selected: snap.selected,
                        };
                        let mut row_out = unbound_widgets::emit_tree_row_with_icon(
                            &desc,
                            &row,
                            resolved_icon,
                            snap.selected,
                            row_state,
                            &label,
                            tokens,
                        );
                        if let Some(a11y) = row_out.a11y.as_mut() {
                            a11y.label = raw_label;
                        }
                        self.collector.push(row.row.id, row_out);
                    }
                    self.collector.pop_clip();

                    // 内容高度超过视口时注册滚轮区域并复用持久化 `ScrollState` 按需显隐右侧细滚动条。
                    if *content_height > desc.rect.height() && desc.rect.height() > 0.0 {
                        self.scroll_regions.push((
                            entry.key,
                            desc.rect,
                            Vec2::new(desc.rect.width(), *content_height),
                            true,
                        ));
                        let mut sb_desc = scroll_desc;
                        sb_desc.axis = unbound_widgets::ScrollAxis::Vertical;
                        sb_desc.z = desc.z.saturating_add(5);
                        let thumb_key = entry.key ^ (1 << 63);
                        let tree_hovered = any_row_hovered
                            || self.ctx.hovered == Some(entry.key)
                            || self.ctx.hovered == Some(thumb_key)
                            || self.ctx.pressed == Some(thumb_key);
                        let sb_state = {
                            let mut store = self.widget_state.borrow_mut();
                            let st = store.use_state_or_init::<unbound_widgets::ScrollState>(
                                entry.key,
                                unbound_widgets::ScrollState::new,
                            );
                            if tree_hovered || st.is_animating() || st.dragging().is_some() {
                                st.reveal_scrollbar();
                            }
                            st.clone()
                        };
                        let sb_control = ControlState {
                            phase: if self.ctx.pressed == Some(thumb_key) {
                                unbound_widgets::PointerPhase::Pressed
                            } else if tree_hovered {
                                unbound_widgets::PointerPhase::Hovered
                            } else {
                                unbound_widgets::PointerPhase::Idle
                            },
                            focused: state.focused,
                            disabled: state.disabled,
                            selected: state.selected,
                        };
                        let sb_out = unbound_widgets::emit_scroll_view(
                            entry.key, &sb_desc, &sb_state, sb_control,
                        );
                        sb_out.push_into(&mut self.collector, entry.key ^ 0x5452_4545_0003);
                    }
                }
                WidgetSpec::AddressBar {
                    desc,
                    state: bar_state,
                } => {
                    let mut desc = desc.clone();
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        desc.rect = rect;
                    }
                    desc.z = entry.z_base.saturating_add(desc.z);
                    for seg in &mut desc.segments {
                        seg.widget_icon = resolve_widget_icon(&self.asset_hub, seg.widget_icon);
                    }
                    let layout = unbound_widgets::compute_layout(&desc);
                    // 按 `address_bar_sub_node(entry.key, kind)` 自动反解悬停与按压子热区，
                    // 兼顾业务层手动覆写的 `hovered_hit` / `pressed_hit`。
                    let resolve_sub_hit =
                        |target: Option<u64>| -> Option<unbound_widgets::AddressBarHitKind> {
                            let key = target?;
                            use unbound_widgets::{address_bar_sub_node, AddressBarHitKind};
                            for kind in [
                                AddressBarHitKind::NavBack,
                                AddressBarHitKind::NavForward,
                                AddressBarHitKind::NavUp,
                                AddressBarHitKind::Refresh,
                                AddressBarHitKind::RootChevron,
                                AddressBarHitKind::OverflowButton,
                                AddressBarHitKind::EditTrigger,
                                AddressBarHitKind::SearchBox,
                                AddressBarHitKind::SearchClear,
                            ] {
                                if address_bar_sub_node(entry.key, kind) == key {
                                    return Some(kind);
                                }
                            }
                            for seg in &layout.segments {
                                let seg_kind = AddressBarHitKind::Segment(seg.index);
                                if address_bar_sub_node(entry.key, seg_kind) == key {
                                    return Some(seg_kind);
                                }
                                if seg.chevron_rect.is_some() {
                                    let chev_kind = AddressBarHitKind::SegmentChevron(seg.index);
                                    if address_bar_sub_node(entry.key, chev_kind) == key {
                                        return Some(chev_kind);
                                    }
                                }
                            }
                            None
                        };
                    let mut effective_state = (**bar_state).clone();
                    if effective_state.hovered_hit.is_none() {
                        effective_state.hovered_hit = resolve_sub_hit(self.ctx.hovered);
                    }
                    if effective_state.pressed_hit.is_none() {
                        effective_state.pressed_hit = resolve_sub_hit(self.ctx.pressed);
                    }

                    // 先发射地址栏外框、导航按钮、图标、子热区与无障碍节点（文本与光标/选区/IME 层在独立裁剪作用域内绘制，避免重复占位图元）
                    let chrome_out = unbound_widgets::emit_address_bar_chrome(
                        entry.key,
                        &desc,
                        &effective_state,
                        state,
                    );
                    self.collector.push(entry.key, chrome_out);

                    // 路径区：施加独立 GPU 裁剪作用域；面包屑执行尾部省略，活动编辑态保留未省略原文并按 `scroll_x` 平移
                    let mut addr_glyphs = Vec::new();
                    let mut edit_visual =
                        unbound_widgets::AddressBarTextVisual::default().edit_visual;
                    if bar_state.editing {
                        let edit_text = if bar_state.edit_core.text().is_empty()
                            && !bar_state.edit_buffer.is_empty()
                        {
                            bar_state.edit_buffer.as_str()
                        } else {
                            bar_state.edit_core.text()
                        };
                        let text_x = layout.root_icon_rect.max().x + 6.0;
                        let edit_w = (layout.address_rect.max().x - text_x - 8.0).max(0.0);
                        let edit_rect = Rect::new(
                            text_x,
                            layout.address_rect.origin.y,
                            edit_w,
                            layout.address_rect.size.y.max(0.0),
                        );
                        let scroll_x = bar_state.edit_core.scroll_x();
                        let scrolled_edit_rect = Rect::new(
                            edit_rect.origin.x - scroll_x,
                            edit_rect.origin.y,
                            edit_rect.size.x + scroll_x.max(0.0),
                            edit_rect.size.y,
                        );
                        if edit_w > 0.0 && !edit_text.is_empty() {
                            addr_glyphs.extend(text_ctx.shape_line(
                                edit_text,
                                scrolled_edit_rect,
                                13.0,
                                400,
                                unbound_widgets::AlignX::Left,
                                unbound_widgets::AlignY::Middle,
                            ));
                        }
                        edit_visual = unbound_widgets::compute_text_edit_visual(
                            &bar_state.edit_core,
                            edit_rect,
                            !state.disabled,
                            |candidate| text_ctx.measure_line(candidate, 13.0, 400).x,
                        );
                    } else {
                        for seg_layout in &layout.segments {
                            if let Some(seg) = desc.segments.get(seg_layout.index) {
                                let avail_w = (seg_layout.label_rect.width() - 8.0).max(0.0);
                                if avail_w > 0.0 && !seg.label.is_empty() {
                                    let elided = unbound_widgets::elide_text_with_measure(
                                        &seg.label,
                                        avail_w,
                                        unbound_widgets::ElideMode::End,
                                        |candidate| text_ctx.measure_line(candidate, 12.5, 400).x,
                                    );
                                    if !elided.display.is_empty() {
                                        addr_glyphs.extend(text_ctx.shape_line(
                                            &elided.display,
                                            seg_layout.label_rect,
                                            12.5,
                                            400,
                                            unbound_widgets::AlignX::Center,
                                            unbound_widgets::AlignY::Middle,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    let addr_clip = GpuClipNode::from_aabb([
                        layout.address_rect.min().x,
                        layout.address_rect.min().y,
                        layout.address_rect.max().x,
                        layout.address_rect.max().y,
                    ]);
                    self.collector.push_clip(addr_clip, layout.address_rect);
                    let addr_text_out = unbound_widgets::emit_address_bar_text_layer(
                        &addr_glyphs,
                        &[],
                        &edit_visual,
                        bar_state.editing,
                        bar_state.edit_core.preedit().is_some(),
                        state.disabled,
                        desc.z,
                    );
                    self.collector
                        .push(entry.key ^ 0x4142_5458_0001, addr_text_out);
                    self.collector.pop_clip();

                    // 搜索区：施加独立 GPU 裁剪作用域，区分主文本与次级占位色字形，并绘制搜索态光标/选区/IME
                    if let Some(sr) = layout.search_rect {
                        if !layout.search_collapsed {
                            let mut search_primary_glyphs = Vec::new();
                            let mut search_placeholder_glyphs = Vec::new();
                            let mut search_visual =
                                unbound_widgets::AddressBarTextVisual::default().search_visual;
                            let search_text = bar_state.search_core.text();
                            let is_placeholder = search_text.is_empty();
                            let display = if is_placeholder {
                                desc.search_placeholder.as_str()
                            } else {
                                search_text
                            };
                            let left_x = layout
                                .search_icon_rect
                                .map_or(sr.origin.x + 8.0, |ir| ir.max().x + 6.0);
                            let show_clear = !state.disabled && !is_placeholder;
                            let right_x = if show_clear {
                                layout
                                    .search_clear_rect
                                    .map_or(sr.max().x - 6.0, |cr| cr.origin.x - 6.0)
                            } else {
                                sr.max().x - 6.0
                            };
                            let search_w = (right_x - left_x).max(0.0);
                            let search_text_rect =
                                Rect::new(left_x, sr.origin.y, search_w, sr.size.y.max(0.0));
                            if search_w > 0.0 && !display.is_empty() {
                                if bar_state.search_focused && !is_placeholder {
                                    // 搜索聚焦输入态保留未省略原文并按 `scroll_x` 平移，避免与光标/选区错位。
                                    let scroll_x = bar_state.search_core.scroll_x();
                                    let scrolled_search_rect = Rect::new(
                                        search_text_rect.origin.x - scroll_x,
                                        search_text_rect.origin.y,
                                        search_text_rect.size.x + scroll_x.max(0.0),
                                        search_text_rect.size.y,
                                    );
                                    search_primary_glyphs.extend(text_ctx.shape_line(
                                        search_text,
                                        scrolled_search_rect,
                                        12.5,
                                        400,
                                        unbound_widgets::AlignX::Left,
                                        unbound_widgets::AlignY::Middle,
                                    ));
                                } else {
                                    let elided = unbound_widgets::elide_text_with_measure(
                                        display,
                                        search_w,
                                        unbound_widgets::ElideMode::End,
                                        |candidate| text_ctx.measure_line(candidate, 12.5, 400).x,
                                    );
                                    if !elided.display.is_empty() {
                                        let shaped = text_ctx.shape_line(
                                            &elided.display,
                                            search_text_rect,
                                            12.5,
                                            400,
                                            unbound_widgets::AlignX::Left,
                                            unbound_widgets::AlignY::Middle,
                                        );
                                        if is_placeholder {
                                            search_placeholder_glyphs.extend(shaped);
                                        } else {
                                            search_primary_glyphs.extend(shaped);
                                        }
                                    }
                                }
                            }
                            if bar_state.search_focused {
                                search_visual = unbound_widgets::compute_text_edit_visual(
                                    &bar_state.search_core,
                                    search_text_rect,
                                    !state.disabled,
                                    |candidate| text_ctx.measure_line(candidate, 12.5, 400).x,
                                );
                            }
                            let search_clip = GpuClipNode::from_aabb([
                                sr.min().x,
                                sr.min().y,
                                sr.max().x,
                                sr.max().y,
                            ]);
                            self.collector.push_clip(search_clip, sr);
                            let search_text_out = unbound_widgets::emit_address_bar_text_layer(
                                &search_primary_glyphs,
                                &search_placeholder_glyphs,
                                &search_visual,
                                bar_state.search_focused,
                                bar_state.search_core.preedit().is_some(),
                                state.disabled,
                                desc.z,
                            );
                            self.collector
                                .push(entry.key ^ 0x4142_5458_0002, search_text_out);
                            self.collector.pop_clip();
                        }
                    }
                }
                WidgetSpec::Dock { state: dock_state } => {
                    let dock_rect = if rect.width() > 0.0 && rect.height() > 0.0 {
                        rect
                    } else {
                        Rect::new(0.0, 0.0, self.ctx.viewport.x, self.ctx.viewport.y)
                    };
                    let tokens = unbound_widgets::DockTokens::standard();
                    let z = entry.z_base.saturating_add(300);
                    let panes = dock_state.tree.layout(dock_rect);
                    let mut root_a11y_pushed = false;
                    for pane in &panes {
                        let mut pane_out = unbound_widgets::emit_dock_pane(pane, tokens, z);
                        if !root_a11y_pushed {
                            pane_out.a11y = Some(A11yNode::new(Role::Region, "Dock", dock_rect));
                            root_a11y_pushed = true;
                        }
                        self.collector.push(entry.key, pane_out);
                        let tab_count = pane.tabs.len().max(1);
                        let tab_w = (pane.tab_bar().width() / tab_count as f32).clamp(64.0, 180.0);
                        for (idx, tab) in pane.tabs.iter().enumerate() {
                            let tab_rect = pane.tab_rect(idx, tab_w);
                            let quads = text_ctx.shape_line(
                                &tab.title,
                                tab_rect,
                                12.0,
                                500,
                                unbound_widgets::AlignX::Left,
                                unbound_widgets::AlignY::Middle,
                            );
                            let tab_out = unbound_widgets::emit_dock_tab(
                                pane,
                                idx,
                                tab_w,
                                state,
                                &quads,
                                tokens,
                                z.saturating_add(2),
                            );
                            self.collector.push(tab.id, tab_out);
                        }
                    }
                    if !root_a11y_pushed {
                        let out = WidgetOutput {
                            a11y: Some(A11yNode::new(Role::Region, "Dock", dock_rect)),
                            ..WidgetOutput::default()
                        };
                        self.collector.push(entry.key, out);
                    }
                    if let Some(hover) = dock_state.hover {
                        let preview =
                            unbound_widgets::emit_dock_preview(hover, tokens, z.saturating_add(20));
                        self.collector
                            .push(mix_key(entry.key, 0x444F_434B), preview);
                    }
                }
                WidgetSpec::Minimap { desc, items } => {
                    let mut desc = *desc;
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        desc.rect = rect;
                    }
                    desc.z = entry.z_base.saturating_add(desc.z);
                    let mm_state = self
                        .widget_state
                        .borrow()
                        .get::<unbound_widgets::MinimapState>(entry.key)
                        .copied()
                        .unwrap_or_else(unbound_widgets::MinimapState::new);
                    let (mut out, hit) =
                        unbound_widgets::emit_minimap(entry.key, &desc, items, &mm_state, state);
                    if out.hit.is_none() {
                        out.hit = Some(hit);
                    }
                    if out.a11y.is_none() {
                        out.a11y = Some(A11yNode::new(Role::Region, "Minimap", desc.rect));
                    }
                    self.collector.push(entry.key, out);
                }
                WidgetSpec::Popover { anchor_key, desc } => {
                    let mut desc = desc.clone();
                    desc.z = entry.z_base.saturating_add(desc.z.min(100));
                    let mut pop =
                        unbound_widgets::emit_popover(entry.key, *anchor_key, &desc, state);
                    if pop.output.a11y.is_none() {
                        pop.output.a11y = Some(A11yNode::new(Role::Dialog, "Popover", desc.rect()));
                    }
                    if let Some(hit) = pop.output.hit.as_mut() {
                        hit.layer = unbound_input::PORTAL_LAYER;
                        hit.z = hit.z.saturating_add(u32::from(entry.z_base));
                    }
                    if let Some(backdrop) = pop.backdrop {
                        self.collector.backdrops.push(backdrop);
                    }
                    self.collector.push(entry.key, pop.output);
                }
                WidgetSpec::ContextMenu {
                    owner_key,
                    anchor_key,
                    desc,
                    menu_state,
                } => {
                    let mut desc = desc.clone();
                    desc.z = entry.z_base.saturating_add(desc.z.min(100));
                    resolve_menu_item_icons(&self.asset_hub, &mut desc.items);
                    self.active_context_menus.push(ActiveContextMenu {
                        owner_key: *owner_key,
                        widget_key: entry.key,
                        selectable: desc
                            .items
                            .iter()
                            .map(|item| !state.disabled && item.selectable())
                            .collect(),
                    });
                    let has_icon_col = desc.has_icon_column();
                    let scroll_offset = menu_state.scroll_offset();
                    let shaped_rows: Vec<(
                        Vec<unbound_text::GlyphQuad>,
                        Vec<unbound_text::GlyphQuad>,
                        unbound_widgets::PointerPhase,
                    )> = desc
                        .items
                        .iter()
                        .enumerate()
                        .map(|(idx, item)| {
                            if item.separator {
                                return (
                                    Vec::new(),
                                    Vec::new(),
                                    unbound_widgets::PointerPhase::Idle,
                                );
                            }
                            let row_rect = desc
                                .item_rect(idx)
                                .translate(Vec2::new(0.0, -scroll_offset.y));
                            let pill_left = row_rect.origin.x + unbound_widgets::MENU_PANEL_PADDING;
                            let pill_right = row_rect.max().x - unbound_widgets::MENU_PANEL_PADDING;
                            let label_left = if has_icon_col {
                                pill_left + 10.0 + unbound_widgets::MENU_ICON_SIZE + 8.0
                            } else {
                                pill_left + 10.0
                            };
                            let (shortcut_glyphs, right_reserved) = if item.has_submenu() {
                                (Vec::new(), 30.0)
                            } else if !item.shortcut.is_empty() {
                                let sc_size = 11.5;
                                let sc_w = text_ctx.measure_line(&item.shortcut, sc_size, 400).x;
                                let max_sc_w = (pill_right - 10.0 - label_left).max(0.0);
                                let sc_box_w = (sc_w + 4.0).min(max_sc_w);
                                let sc_rect = Rect::new(
                                    pill_right - 10.0 - sc_box_w,
                                    row_rect.origin.y,
                                    sc_box_w,
                                    row_rect.size.y,
                                );
                                let quads = if sc_box_w > 0.0 {
                                    text_ctx.shape_line(
                                        &item.shortcut,
                                        sc_rect,
                                        sc_size,
                                        400,
                                        unbound_widgets::AlignX::Right,
                                        unbound_widgets::AlignY::Middle,
                                    )
                                } else {
                                    Vec::new()
                                };
                                (quads, 10.0 + sc_box_w + 10.0)
                            } else {
                                (Vec::new(), 10.0)
                            };
                            let label_avail_w = (pill_right - right_reserved - label_left).max(0.0);
                            let label_glyphs = if label_avail_w > 0.0 && !item.label.is_empty() {
                                let elided = unbound_widgets::elide_text_with_measure(
                                    &item.label,
                                    label_avail_w,
                                    unbound_widgets::ElideMode::End,
                                    |cand| text_ctx.measure_line(cand, 13.0, 400).x,
                                );
                                if !elided.display.is_empty() {
                                    let label_rect = Rect::new(
                                        label_left,
                                        row_rect.origin.y,
                                        label_avail_w,
                                        row_rect.size.y,
                                    );
                                    text_ctx.shape_line(
                                        &elided.display,
                                        label_rect,
                                        13.0,
                                        400,
                                        unbound_widgets::AlignX::Left,
                                        unbound_widgets::AlignY::Middle,
                                    )
                                } else {
                                    Vec::new()
                                }
                            } else {
                                Vec::new()
                            };
                            let item_key = unbound_widgets::menu_item_node(entry.key, idx);
                            let item_phase = if state.disabled || item.disabled {
                                unbound_widgets::PointerPhase::Idle
                            } else if self.ctx.pressed == Some(item_key) {
                                unbound_widgets::PointerPhase::Pressed
                            } else if self.ctx.hovered == Some(item_key) {
                                unbound_widgets::PointerPhase::Hovered
                            } else {
                                unbound_widgets::PointerPhase::Idle
                            };
                            (label_glyphs, shortcut_glyphs, item_phase)
                        })
                        .collect();
                    let render_rows: Vec<unbound_widgets::MenuItemRenderRow<'_>> = shaped_rows
                        .iter()
                        .map(
                            |(label, shortcut, phase)| unbound_widgets::MenuItemRenderRow {
                                label_glyphs: label.as_slice(),
                                shortcut_glyphs: shortcut.as_slice(),
                                phase: *phase,
                            },
                        )
                        .collect();
                    let (mut out, _) = unbound_widgets::emit_menu_with_rows(
                        entry.key,
                        *anchor_key,
                        &desc,
                        menu_state,
                        state,
                        &render_rows,
                    );
                    if out.a11y.is_none() {
                        out.a11y = Some(A11yNode::new(Role::Menu, "ContextMenu", desc.rect));
                    }
                    if let Some(hit) = out.hit.as_mut() {
                        hit.layer = unbound_input::PORTAL_LAYER;
                        hit.z = hit.z.saturating_add(u32::from(entry.z_base));
                    }
                    for sub_hit in &mut out.sub_hits {
                        sub_hit.layer = unbound_input::PORTAL_LAYER;
                        sub_hit.z = sub_hit.z.saturating_add(u32::from(entry.z_base));
                    }
                    self.collector.push(entry.key, out);
                }
                WidgetSpec::CommandPalette {
                    desc,
                    state: cp_state,
                } => {
                    let mut desc = desc.clone();
                    desc.z = entry.z_base.saturating_add(desc.z.min(100));
                    let tokens = unbound_widgets::CommandPaletteTokens::standard();
                    let mut out = unbound_widgets::emit_command_palette(&desc, cp_state, tokens);
                    if out.a11y.is_none() {
                        out.a11y = Some(A11yNode::new(Role::Dialog, "CommandPalette", desc.rect));
                    }
                    if let Some(hit) = out.hit.as_mut() {
                        hit.layer = unbound_input::PORTAL_LAYER;
                        hit.z = hit.z.saturating_add(u32::from(entry.z_base));
                    }
                    self.collector.push(entry.key, out);
                    let visible = cp_state.visible_results(&desc);
                    for (vis_idx, cmd_idx) in visible.into_iter().enumerate() {
                        if let Some(cmd) = cp_state.commands.get(cmd_idx) {
                            let row_rect = desc.row_rect(vis_idx);
                            let label_quads = text_ctx.shape_line(
                                &cmd.title,
                                row_rect,
                                13.0,
                                400,
                                unbound_widgets::AlignX::Left,
                                unbound_widgets::AlignY::Middle,
                            );
                            let shortcut_quads = match cmd.shortcut.as_deref() {
                                Some(shortcut) if !shortcut.is_empty() => text_ctx.shape_line(
                                    shortcut,
                                    row_rect,
                                    11.0,
                                    400,
                                    unbound_widgets::AlignX::Right,
                                    unbound_widgets::AlignY::Middle,
                                ),
                                _ => Vec::new(),
                            };
                            let row_out = unbound_widgets::emit_command_row(
                                &desc,
                                cp_state,
                                vis_idx,
                                state,
                                &label_quads,
                                &shortcut_quads,
                                tokens,
                            );
                            self.collector.push(mix_key(entry.key, cmd.id), row_out);
                        }
                    }
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
                WidgetSpec::InteractiveHit { z, cursor } => {
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        let hit = Some(
                            unbound_input::HitRegion::ui(entry.key, u32::from(*z), rect)
                                .with_cursor(*cursor),
                        );
                        self.collector.push(
                            entry.key,
                            WidgetOutput {
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
                if button == PointerButton::Middle {
                    if let (Some(node), Some(hovered)) = (target, self.router.hover()) {
                        if let (Some((down_tabs, down_target)), Some((up_tabs, up_target))) = (
                            self.resolve_tab_middle_close_target(node),
                            self.resolve_tab_middle_close_target(hovered),
                        ) {
                            if down_tabs == up_tabs && down_target == up_target {
                                self.ctx.record_tab_close_request(down_tabs, down_target);
                            }
                        }
                    }
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
                            if let Some((tabs_key, hit_target)) =
                                self.resolve_tab_context_target(node)
                            {
                                self.ctx.record_tab_context_target(tabs_key, hit_target);
                            }
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
                if let Some((tabs_key, sub_hit)) = self.resolve_tab_sub_hit(target) {
                    match sub_hit {
                        TabSubHit::Close(hit_target) => {
                            self.ctx.record_tab_close_request(tabs_key, hit_target);
                        }
                        TabSubHit::Add => {
                            self.ctx.record_tab_add_request(tabs_key);
                        }
                        TabSubHit::Item { .. } => {}
                    }
                }
                if let Some((menu_owner_key, item_index)) = self.resolve_context_menu_item(target) {
                    self.ctx
                        .record_context_menu_activated(menu_owner_key, item_index);
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

    fn sync_form_binding(&mut self, key: u64) -> bool {
        if !self.form_bindings.contains(key) {
            return false;
        }
        let maybe_val = {
            let store = self.widget_state.borrow();
            if let Some(st) = store.get::<unbound_widgets::TextEditState>(key) {
                Some(WidgetValue::Text(st.text().to_owned()))
            } else if let Some(&val) = store.get::<f32>(key) {
                Some(WidgetValue::Float(val))
            } else if let Some(&b) = store.get::<bool>(key) {
                Some(WidgetValue::Bool(b))
            } else if let Some(st) = store.get::<unbound_widgets::SelectState>(key) {
                st.selected.map(WidgetValue::Index)
            } else {
                None
            }
        };
        if let Some(val) = maybe_val {
            self.form_bindings
                .apply(key, &mut self.state as &mut dyn Any, &val)
        } else {
            false
        }
    }

    /// 将命中键反解为所属标签栏键与子热区类型（标签主体、关闭按钮或新建按钮）。
    fn resolve_tab_sub_hit(&self, node: u64) -> Option<(u64, TabSubHit)> {
        let owner = *self.hit_owners.get(&node)?;
        let (_, widget) = self.mounted_widgets.get(&owner)?;
        let tabs_desc = widget
            .as_any()
            .downcast_ref::<unbound_widgets::TabsDesc>()?;
        if node == unbound_widgets::tab_add_node(owner) {
            return Some((owner, TabSubHit::Add));
        }
        for (idx, item) in tabs_desc.items.iter().enumerate() {
            if node == unbound_widgets::tab_close_node(owner, idx) {
                return Some((
                    owner,
                    TabSubHit::Close(TabHitTarget {
                        index: idx,
                        id: item.id,
                    }),
                ));
            }
            if node == unbound_widgets::tab_item_node(owner, idx) {
                return Some((
                    owner,
                    TabSubHit::Item {
                        target: TabHitTarget {
                            index: idx,
                            id: item.id,
                        },
                        closable: tabs_desc.closable && item.closable && !item.pinned,
                    },
                ));
            }
        }
        None
    }

    /// 反解鼠标中键点击命中的可关闭标签目标。
    fn resolve_tab_middle_close_target(&self, node: u64) -> Option<(u64, TabHitTarget)> {
        match self.resolve_tab_sub_hit(node)? {
            (
                tabs_key,
                TabSubHit::Item {
                    target,
                    closable: true,
                },
            )
            | (tabs_key, TabSubHit::Close(target)) => Some((tabs_key, target)),
            _ => None,
        }
    }

    /// 反解右键上下文菜单命中的标签目标。
    fn resolve_tab_context_target(&self, node: u64) -> Option<(u64, TabHitTarget)> {
        match self.resolve_tab_sub_hit(node)? {
            (tabs_key, TabSubHit::Item { target, .. }) | (tabs_key, TabSubHit::Close(target)) => {
                Some((tabs_key, target))
            }
            _ => None,
        }
    }

    /// 将命中的子热区键反解为所属上下文菜单的用户键与可选菜单项索引。
    fn resolve_context_menu_item(&self, target: u64) -> Option<(u64, usize)> {
        for menu in self.active_context_menus.iter().rev() {
            for (idx, &selectable) in menu.selectable.iter().enumerate() {
                if selectable && target == unbound_widgets::menu_item_node(menu.widget_key, idx) {
                    return Some((menu.owner_key, idx));
                }
            }
        }
        None
    }

    fn handle_select_option_click(&mut self, target_key: u64) -> bool {
        let mut hit = None;
        for popup in &self.active_select_popups {
            if let Some(pos) = popup.option_keys.iter().position(|k| *k == target_key) {
                hit = Some((popup.select_key, pos));
                break;
            }
        }
        if let Some((select_key, idx)) = hit {
            {
                let mut store = self.widget_state.borrow_mut();
                let st = store.entry::<unbound_widgets::SelectState>(select_key);
                st.selected = Some(idx);
                st.highlighted = Some(idx);
                st.close();
            }
            self.sync_form_binding(select_key);
            self.active_select_popups
                .retain(|p| p.select_key != select_key);
            self.mark(DirtyFlags::ALL);
            return true;
        }
        false
    }

    fn dispatch_outcome(&mut self, event: AppEvent) -> EventOutcome {
        let mut widget_dirty = DirtyFlags::NONE;
        let mut widget_consumed = false;
        let mut ime_absorbed = false;
        match &event {
            AppEvent::Click { node, .. } => {
                if self.handle_select_option_click(*node) {
                    widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                    widget_consumed = true;
                }
            }
            AppEvent::PointerDown {
                pos,
                target: Some(target_key),
                button,
                click_count,
            } => {
                if self.handle_select_option_click(*target_key) {
                    widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                    widget_consumed = true;
                }
                if self.focus.order().contains(target_key)
                    && self.focus.focus(*target_key, FocusCause::Pointer)
                {
                    self.sync_focus_context();
                    widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                }
                if let Some((rect, widget)) = self.mounted_widgets.get(target_key).cloned() {
                    let skip_non_left_tabs = *button != PointerButton::Left
                        && widget.as_any().is::<unbound_widgets::TabsDesc>();
                    if !skip_non_left_tabs {
                        let w_evt = unbound_widgets::WidgetInputEvent::PointerDown {
                            pos: *pos,
                            click_count: *click_count,
                        };
                        let mut store = self.widget_state.borrow_mut();
                        let changed = widget.reduce_erased(*target_key, rect, &w_evt, &mut store);
                        drop(store);
                        if changed {
                            if self.sync_form_binding(*target_key) {
                                widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                            } else {
                                widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                            }
                            widget_consumed = true;
                        }
                    }
                }
            }
            AppEvent::Drag { node, pos, .. } => {
                if let Some((rect, widget)) = self.mounted_widgets.get(node).cloned() {
                    let w_evt = unbound_widgets::WidgetInputEvent::PointerDrag { pos: *pos };
                    let mut store = self.widget_state.borrow_mut();
                    let changed = widget.reduce_erased(*node, rect, &w_evt, &mut store);
                    drop(store);
                    if changed {
                        if self.sync_form_binding(*node) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                        } else {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        }
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
                    let changed = widget.reduce_erased(*target_key, rect, &w_evt, &mut store);
                    drop(store);
                    if changed {
                        if self.sync_form_binding(*target_key) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                        } else {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        }
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
                    let changed = widget.reduce_erased(*target_key, rect, &w_evt, &mut store);
                    drop(store);
                    if changed {
                        if self.sync_form_binding(*target_key) {
                            widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                        } else {
                            widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                        }
                        widget_consumed = true;
                    }
                }
            }
            AppEvent::Text { text } => {
                if let Some(focused_key) = self.ctx.focused {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&focused_key).cloned() {
                        let w_evt = unbound_widgets::WidgetInputEvent::TextInput(text.clone());
                        let mut store = self.widget_state.borrow_mut();
                        let changed = widget.reduce_erased(focused_key, rect, &w_evt, &mut store);
                        drop(store);
                        if changed {
                            if self.sync_form_binding(focused_key) {
                                widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                            } else {
                                widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                            }
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
                if *key == AppKey::Escape && !self.active_select_popups.is_empty() {
                    let mut store = self.widget_state.borrow_mut();
                    for popup in &self.active_select_popups {
                        let st = store.entry::<unbound_widgets::SelectState>(popup.select_key);
                        st.close();
                    }
                    drop(store);
                    self.active_select_popups.clear();
                    widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                    widget_consumed = true;
                }
                if let Some(target_key) = self.ctx.event_target.or(self.ctx.focused) {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&target_key).cloned() {
                        let chord = key.chord(*modifiers);
                        let w_evt = unbound_widgets::WidgetInputEvent::KeyDown(chord);
                        let mut store = self.widget_state.borrow_mut();
                        let changed = widget.reduce_erased(target_key, rect, &w_evt, &mut store);
                        drop(store);
                        if changed {
                            if self.sync_form_binding(target_key) {
                                widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                            } else {
                                widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                            }
                            widget_consumed = true;
                        }
                    }
                }
            }
            AppEvent::Ime { update } => {
                // IME 更新优先交给聚焦的已挂载文本控件；控件消费后不再转交应用层，
                // 保证同一次提交在整条链路上只有一个消费者。
                if let Some(focused_key) = self.ctx.focused {
                    if let Some((rect, widget)) = self.mounted_widgets.get(&focused_key).cloned() {
                        let w_evt = if update.cancelled {
                            Some(unbound_widgets::WidgetInputEvent::ImeCancel)
                        } else if let Some(commit) = &update.commit {
                            Some(unbound_widgets::WidgetInputEvent::ImeCommit(commit.clone()))
                        } else if update.enabled || !update.preedit.is_empty() {
                            Some(unbound_widgets::WidgetInputEvent::ImePreedit {
                                text: update.preedit.clone(),
                                cursor: update.cursor_range.map(|(start, _)| start),
                            })
                        } else {
                            None
                        };
                        if let Some(w_evt) = w_evt {
                            let mut store = self.widget_state.borrow_mut();
                            let changed =
                                widget.reduce_erased(focused_key, rect, &w_evt, &mut store);
                            drop(store);
                            if changed {
                                if self.sync_form_binding(focused_key) {
                                    widget_dirty = widget_dirty.merge(DirtyFlags::ALL);
                                } else {
                                    widget_dirty = widget_dirty.merge(DirtyFlags::PAINT);
                                }
                                widget_consumed = true;
                                ime_absorbed = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        let mut outcome = if ime_absorbed {
            EventOutcome::new(widget_dirty)
        } else {
            self.app.reduce(&mut self.state, event, &mut self.ctx)
        };
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

/// 将 [`unbound_widgets::WidgetIcon::ExternalTexture`] 中的逻辑句柄解析为
/// [`crate::assets::AssetHub`] 已上传的 GPU Bindless 纹理槽位；
/// 若句柄尚未驻留或已释放，则安全降级为 [`unbound_widgets::WidgetIcon::None`]，避免绑定无效纹理槽。
#[must_use]
fn resolve_widget_icon(
    asset_hub: &crate::assets::AssetHub,
    icon: unbound_widgets::WidgetIcon,
) -> unbound_widgets::WidgetIcon {
    match icon {
        unbound_widgets::WidgetIcon::ExternalTexture(handle_raw) => asset_hub
            .external_texture_slot(ExternalTextureHandle(handle_raw))
            .map(|(slot, _, _)| unbound_widgets::WidgetIcon::ExternalTexture(u64::from(slot)))
            .unwrap_or(unbound_widgets::WidgetIcon::None),
        other => other,
    }
}

/// 递归解析菜单项及其子菜单中的 [`unbound_widgets::WidgetIcon::ExternalTexture`] 句柄。
fn resolve_menu_item_icons(
    asset_hub: &crate::assets::AssetHub,
    items: &mut [unbound_widgets::MenuItem],
) {
    for item in items {
        item.widget_icon = resolve_widget_icon(asset_hub, item.widget_icon);
        if let Some(sub) = item.submenu.as_mut() {
            resolve_menu_item_icons(asset_hub, sub);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_math::Vec2;

    struct DummyApp;
    impl Application for DummyApp {
        type State = ();
        fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}
        fn reduce(
            &mut self,
            _state: &mut Self::State,
            _event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }
        fn view(&mut self, _state: &Self::State, _view: &mut crate::ViewBuilder<'_>) {}
    }

    #[test]
    fn runtime_internal_backdrop_intent_roundtrip() {
        let mut runtime = AppRuntime::new(DummyApp, Vec2::new(800.0, 600.0));
        assert_eq!(runtime.backdrop_intent(), AppearanceIntent::Material(None));

        runtime.set_backdrop(Some(WindowBackdropMaterial::Mica));
        assert_eq!(
            runtime.backdrop_intent(),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::Mica))
        );

        let tint = 0xCCD07800;
        runtime.set_accent_tint(tint);
        assert_eq!(runtime.backdrop(), None);
        assert_eq!(
            runtime.backdrop_intent(),
            AppearanceIntent::AccentTint(tint)
        );

        let sel = unbound_backdrop::BackdropSelection::new(
            unbound_backdrop::MaterialKind::MicaAlt,
            unbound_backdrop::BackdropSource::Native,
        );
        runtime.set_backdrop_selection(sel);
        assert_eq!(runtime.backdrop(), None);
        assert_eq!(
            runtime.backdrop_intent(),
            AppearanceIntent::Selection {
                selection: sel,
                accent_tint: None,
            }
        );

        runtime.set_backdrop_intent(AppearanceIntent::Material(Some(
            WindowBackdropMaterial::Acrylic,
        )));
        assert_eq!(
            runtime.backdrop_intent(),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::Acrylic))
        );
        assert_eq!(runtime.backdrop(), Some(WindowBackdropMaterial::Acrylic));
    }

    #[test]
    fn frame_report_drains_external_texture_ops_in_order() {
        struct EmptyApp;
        impl Application for EmptyApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), _: &mut ViewBuilder<'_>) {}
        }

        let mut rt = AppRuntime::new(EmptyApp, Vec2::new(640.0, 480.0));
        let _ = rt.frame(0.0);

        let handle = rt
            .asset_hub_mut()
            .upload_external_rgba8(2, 2, true, vec![200u8; 16]);
        let report1 = rt.frame(0.0);
        assert_eq!(report1.external_texture_ops.len(), 1);
        assert!(matches!(
            report1.external_texture_ops[0],
            ExternalTextureOp::Upload { handle: h, width: 2, height: 2, srgb: true, .. } if h == handle
        ));

        rt.asset_hub_mut()
            .commit_external_texture_slot(handle, Ok(50));
        assert_eq!(
            rt.asset_hub().external_texture_slot(handle),
            Some((50, 2, 2))
        );

        assert!(rt
            .asset_hub_mut()
            .update_external_rgba8(handle, 2, 2, vec![100u8; 16]));
        assert!(rt.asset_hub_mut().release_external_texture(handle));
        let report2 = rt.frame(0.0);
        assert_eq!(
            report2.external_texture_ops,
            vec![ExternalTextureOp::Release { handle }]
        );

        let cancelled = rt
            .asset_hub_mut()
            .upload_external_rgba8(2, 2, false, vec![9u8; 16]);
        assert!(rt.asset_hub_mut().release_external_texture(cancelled));
        let report3 = rt.frame(0.0);
        assert!(report3.external_texture_ops.is_empty());
    }

    #[test]
    fn desktop_widgets_emit_primitives_and_semantics() {
        use unbound_widgets::{
            AddressBarDesc, AddressBarState, Command, CommandPaletteDesc, CommandPaletteState,
            ContextMenuState, DockState, DockTab, DockTree, ImageDesc, MenuDesc, MenuItem,
            MinimapDesc, MinimapItem, ObjectFit, PopoverDesc, TreeViewDesc, TreeViewState,
        };
        use unbound_world::TreeRow;

        struct DesktopWidgetsApp {
            ext_handle: Option<ExternalTextureHandle>,
            tree_state: RefCell<TreeViewState>,
            address_state: AddressBarState,
            dock_state: DockState,
            menu_state: ContextMenuState,
            palette_state: CommandPaletteState,
        }
        impl Application for DesktopWidgetsApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.column(1, |col| {
                    col.image(
                        10,
                        ImageDesc::new(Rect::new(0.0, 0.0, 64.0, 64.0), Vec2::new(64.0, 64.0), 0),
                    );
                    if let Some(h) = self.ext_handle {
                        col.image_external(11, h, Vec2::new(80.0, 60.0), ObjectFit::Contain);
                    }
                    col.tree_view(
                        12,
                        TreeViewDesc::new(Rect::new(0.0, 0.0, 220.0, 120.0)),
                        &mut self.tree_state.borrow_mut(),
                    );
                    col.address_bar(
                        13,
                        AddressBarDesc::new(
                            Rect::new(0.0, 0.0, 300.0, 32.0),
                            vec!["root".into(), "docs".into()],
                        ),
                        &self.address_state,
                    );
                    col.dock(14, &self.dock_state);
                    col.minimap(
                        15,
                        MinimapDesc::new(
                            Rect::new(0.0, 0.0, 140.0, 100.0),
                            Rect::new(0.0, 0.0, 1000.0, 1000.0),
                            Rect::new(100.0, 100.0, 200.0, 200.0),
                        ),
                        &[MinimapItem {
                            bounds: Rect::new(50.0, 50.0, 100.0, 80.0),
                            token: unbound_theme::Token::PRIMARY,
                        }],
                    );
                    col.button(
                        16,
                        ButtonDesc::new(Rect::new(10.0, 10.0, 80.0, 32.0), "Anchor"),
                    );
                    col.popover(
                        17,
                        16,
                        PopoverDesc::new(
                            Rect::new(10.0, 10.0, 80.0, 32.0),
                            Rect::new(0.0, 0.0, 1200.0, 800.0),
                            Vec2::new(160.0, 90.0),
                        ),
                        |p| {
                            p.text_primary(18, "Popover Body", 13.0, 400);
                        },
                    );
                    col.context_menu(
                        19,
                        MenuDesc::new(
                            Rect::new(0.0, 0.0, 160.0, 80.0),
                            vec![MenuItem::action("Copy"), MenuItem::action("Paste")],
                        ),
                        &self.menu_state,
                    );
                    col.command_palette(
                        20,
                        CommandPaletteDesc::new(Rect::new(200.0, 80.0, 480.0, 300.0)),
                        &self.palette_state,
                    );
                });
            }
        }

        let mut tree = TreeViewState::new(28.0);
        let roots = vec![TreeRow::new(1001, 0, false)];
        tree.rebuild(&roots, |_| Vec::new());

        let dock = DockState::new(DockTree::new(vec![DockTab::new(2001, "Main")]));
        let mut menu_state = ContextMenuState::new();
        menu_state.open_at(Vec2::new(120.0, 120.0), 16, None);
        let mut palette_state =
            CommandPaletteState::new(vec![Command::new(3001, "Open File", "File")]);
        palette_state.open();

        let app = DesktopWidgetsApp {
            ext_handle: None,
            tree_state: RefCell::new(tree),
            address_state: AddressBarState::new(),
            dock_state: dock,
            menu_state,
            palette_state,
        };
        let mut rt = AppRuntime::new(app, Vec2::new(1200.0, 800.0));
        let ext_h =
            rt.asset_hub_mut()
                .upload_external_rgba8(16, 16, true, vec![255u8; 16 * 16 * 4]);
        rt.application_mut().ext_handle = Some(ext_h);
        rt.invalidate();

        let report_pending = rt.frame(0.0);
        assert!(report_pending.primitives > 0);
        assert!(!rt.semantics().is_empty());
        let pending_rect = rt.widget_rect(11).expect("external image laid out");
        assert_eq!(pending_rect.size, Vec2::new(80.0, 60.0));
        assert!(!rt
            .scene()
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex == 55));

        rt.asset_hub_mut()
            .commit_external_texture_slot(ext_h, Ok(55));
        let _ = rt.frame(0.0);
        let ready_rect = rt.widget_rect(11).expect("external image still laid out");
        assert_eq!(ready_rect.size, pending_rect.size);
        assert!(rt
            .scene()
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex == 55));
    }

    #[test]
    fn virtual_list_and_virtual_masonry_only_build_visible_plus_overscan() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use unbound_widgets::{
            virtual_masonry::MasonryItem, VariableVirtualList, VirtualMasonryDesc,
        };

        struct VirtApp {
            list: VariableVirtualList,
            masonry_items: Vec<MasonryItem>,
            list_calls: Arc<AtomicUsize>,
            masonry_calls: Arc<AtomicUsize>,
            last_list_first: Arc<AtomicUsize>,
        }
        impl Application for VirtApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                self.list_calls.store(0, Ordering::SeqCst);
                self.masonry_calls.store(0, Ordering::SeqCst);
                self.last_list_first.store(usize::MAX, Ordering::SeqCst);
                v.virtual_list(100, &self.list, 2, |row, idx, _| {
                    let prev = self.last_list_first.load(Ordering::SeqCst);
                    if idx < prev {
                        self.last_list_first.store(idx, Ordering::SeqCst);
                    }
                    self.list_calls.fetch_add(1, Ordering::SeqCst);
                    row.spacer(mix_key(1000, idx as u64));
                });
                v.virtual_masonry(
                    200,
                    VirtualMasonryDesc::new(Rect::new(0.0, 0.0, 400.0, 400.0), 4).gap(0.0),
                    &self.masonry_items,
                    100.0,
                    |cell, idx, _| {
                        self.masonry_calls.fetch_add(1, Ordering::SeqCst);
                        cell.spacer(mix_key(2000, idx as u64));
                    },
                );
            }
        }

        let list_calls = Arc::new(AtomicUsize::new(0));
        let masonry_calls = Arc::new(AtomicUsize::new(0));
        let last_list_first = Arc::new(AtomicUsize::new(usize::MAX));
        let masonry_items: Vec<MasonryItem> = (0..10_000)
            .map(|i| MasonryItem {
                id: i as u64 + 1,
                aspect_ratio: 1.0,
            })
            .collect();
        let app = VirtApp {
            list: VariableVirtualList::new(10_000, 40.0),
            masonry_items,
            list_calls: Arc::clone(&list_calls),
            masonry_calls: Arc::clone(&masonry_calls),
            last_list_first: Arc::clone(&last_list_first),
        };
        let mut rt = AppRuntime::new(app, Vec2::new(400.0, 400.0));
        let _ = rt.frame(0.0);
        assert_eq!(list_calls.load(Ordering::SeqCst), 12);
        assert_eq!(last_list_first.load(Ordering::SeqCst), 0);
        let m_count = masonry_calls.load(Ordering::SeqCst);
        assert!(
            (20..=24).contains(&m_count),
            "unexpected masonry calls: {m_count}"
        );

        rt.with_state::<unbound_widgets::ScrollState, _>(100, |st| {
            let desc = unbound_widgets::ScrollViewDesc::new(
                Rect::new(0.0, 0.0, 400.0, 400.0),
                Vec2::new(400.0, 400_000.0),
            );
            st.jump_to(&desc, Vec2::new(0.0, 2000.0));
        });
        rt.invalidate();
        let _ = rt.frame(0.0);
        assert_eq!(last_list_first.load(Ordering::SeqCst), 48);
        assert_eq!(list_calls.load(Ordering::SeqCst), 14);
    }

    #[test]
    fn overlay_layer_and_popover_dismiss_on_outside_click_escape_and_anchor_removal() {
        use unbound_widgets::overlay::OverlayEntry;
        use unbound_widgets::PopoverDesc;

        struct OverlayApp {
            show_anchor: bool,
            show_second: bool,
        }
        impl Application for OverlayApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                if self.show_anchor {
                    v.button(
                        50,
                        ButtonDesc::new(Rect::new(20.0, 20.0, 80.0, 30.0), "Anchor"),
                    );
                }
                v.popover(
                    51,
                    50,
                    PopoverDesc::new(
                        Rect::new(20.0, 20.0, 80.0, 30.0),
                        Rect::new(0.0, 0.0, 800.0, 600.0),
                        Vec2::new(150.0, 80.0),
                    ),
                    |p| {
                        p.spacer(52);
                    },
                );
                if self.show_second {
                    v.overlay_layer(
                        60,
                        OverlayEntry::popover(60, 50, Rect::new(300.0, 100.0, 120.0, 60.0)),
                        |o| {
                            o.spacer(61);
                        },
                    );
                }
            }
        }

        let mut rt = AppRuntime::new(
            OverlayApp {
                show_anchor: true,
                show_second: true,
            },
            Vec2::new(800.0, 600.0),
        );
        let _ = rt.frame(0.0);
        assert!(rt.overlays().stack().contains(51));
        assert!(rt.overlays().stack().contains(60));

        rt.key(AppKey::Escape, true, Modifiers::default());
        let _ = rt.frame(0.0);
        assert!(!rt.overlays().stack().contains(60));
        assert!(rt.overlays().stack().contains(51));

        rt.pointer_down(Vec2::new(750.0, 550.0), PointerButton::Left);
        let _ = rt.frame(0.0);
        assert!(!rt.overlays().stack().contains(51));

        rt.overlays_mut().clear_dismissed(51);
        rt.invalidate();
        let _ = rt.frame(0.0);
        assert!(rt.overlays().stack().contains(51));

        rt.application_mut().show_anchor = false;
        rt.invalidate();
        let _ = rt.frame(0.0);
        assert!(!rt.overlays().stack().contains(51));
    }

    #[test]
    fn outbound_file_drag_command_cancels_pointer_and_delivers_outcome() {
        use std::path::PathBuf;
        use unbound_platform::{OutboundFileDragOutcome, OutboundFileDragRequest};

        struct DragApp {
            outcomes: Vec<OutboundFileDragOutcome>,
        }
        impl Application for DragApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), event: AppEvent, _: &mut AppContext) -> EventOutcome {
                if let AppEvent::ExternalFileDragFinished(outcome) = event {
                    self.outcomes.push(outcome);
                    return EventOutcome::new(DirtyFlags::PAINT);
                }
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.button(1, ButtonDesc::new(Rect::new(0.0, 0.0, 120.0, 40.0), "Drag"));
            }
        }

        let mut rt = AppRuntime::new(
            DragApp {
                outcomes: Vec::new(),
            },
            Vec2::new(400.0, 300.0),
        );
        let _ = rt.frame(0.0);
        rt.pointer_down(Vec2::new(20.0, 20.0), PointerButton::Left);
        assert!(rt.router().pointer().pressed().is_some());

        let req = OutboundFileDragRequest::new(vec![PathBuf::from("C:/tmp/sample.txt")]);
        rt.request_external_file_drag(req.clone());
        let cmds = rt.drain_window_commands();
        assert_eq!(cmds, vec![WindowCommand::BeginExternalFileDrag(req)]);

        let _ = rt.cancel_pointer();
        assert!(rt.router().pointer().pressed().is_none());
        let _ = rt.notify_external_file_drag_finished(OutboundFileDragOutcome::Cancelled);
        assert_eq!(rt.app().outcomes, vec![OutboundFileDragOutcome::Cancelled]);
    }

    #[test]
    fn native_popup_syncs_only_when_overlay_overflows_client_bounds() {
        use unbound_platform::{ScreenRect, WindowId};
        use unbound_widgets::overlay::OverlayEntry;

        struct PopupApp {
            bounds: Option<Rect>,
        }
        impl Application for PopupApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.button(
                    10,
                    ButtonDesc::new(Rect::new(10.0, 10.0, 80.0, 30.0), "Anchor"),
                );
                if let Some(bounds) = self.bounds {
                    v.overlay_layer(20, OverlayEntry::popover(20, 10, bounds), |o| {
                        o.spacer(21);
                    });
                }
            }
        }

        let mut rt = AppRuntime::new(
            PopupApp {
                bounds: Some(Rect::new(100.0, 100.0, 150.0, 80.0)),
            },
            Vec2::new(800.0, 600.0),
        );
        rt.set_window_screen_origin((50, 40), WindowId(7));

        let _ = rt.frame(0.0);
        assert!(rt.drain_window_commands().is_empty());

        rt.application_mut().bounds = Some(Rect::new(720.0, 100.0, 160.0, 90.0));
        rt.invalidate();
        let _ = rt.frame(0.0);
        let cmds = rt.drain_window_commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WindowCommand::SyncNativePopup { id, config } => {
                assert_eq!(*id, 20);
                assert_eq!(config.owner, WindowId(7));
                assert_eq!(config.screen_rect, ScreenRect::new(770, 140, 160, 90));
            }
            other => panic!("expected SyncNativePopup, got {other:?}"),
        }

        let _ = rt.close_overlay(20);
        rt.application_mut().bounds = None;
        rt.invalidate();
        let _ = rt.frame(0.0);
        assert_eq!(
            rt.drain_window_commands(),
            vec![WindowCommand::CloseNativePopup { id: 20 }]
        );
    }

    #[test]
    fn prefer_native_popup_triggers_sync_even_when_overlay_is_within_viewport() {
        use unbound_platform::{ScreenRect, WindowId};
        use unbound_widgets::overlay::OverlayEntry;

        struct PreferNativeApp;
        impl Application for PreferNativeApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.button(
                    10,
                    ButtonDesc::new(Rect::new(10.0, 10.0, 80.0, 30.0), "Anchor"),
                );
                let entry = OverlayEntry::popover(20, 10, Rect::new(100.0, 100.0, 150.0, 80.0))
                    .with_prefer_native_popup(true);
                v.overlay_layer(20, entry, |o| {
                    o.spacer(21);
                });
            }
        }

        let mut rt = AppRuntime::new(PreferNativeApp, Vec2::new(800.0, 600.0));
        rt.set_window_screen_origin((50, 40), WindowId(9));
        let _ = rt.frame(0.0);
        let cmds = rt.drain_window_commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WindowCommand::SyncNativePopup { id, config } => {
                assert_eq!(*id, 20);
                assert_eq!(config.owner, WindowId(9));
                assert_eq!(config.screen_rect, ScreenRect::new(150, 140, 150, 80));
            }
            other => panic!("expected SyncNativePopup, got {other:?}"),
        }
    }

    #[test]
    fn tree_view_recomputes_row_rects_under_clip_and_registers_scrollbar() {
        use unbound_theme::Token;
        use unbound_widgets::{TreeViewDensity, TreeViewDesc, TreeViewState};
        use unbound_world::TreeRow;

        struct NavPaneApp {
            tree_state: RefCell<TreeViewState>,
        }
        impl Application for NavPaneApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.column(1, |col| {
                    // 顶部 80px 占位栏，模拟标题栏与工具栏
                    col.button(
                        2,
                        ButtonDesc::new(Rect::new(0.0, 0.0, 240.0, 80.0), "TopBar"),
                    );
                    // 刻意传入 (0, 0) 原点描述符，验证发射阶段按布局后的真实原点重算行矩形
                    col.tree_view(
                        12,
                        TreeViewDesc::new(Rect::new(0.0, 0.0, 220.0, 140.0))
                            .with_density(TreeViewDensity::Compact)
                            .with_navigation_pane(true),
                        &mut self.tree_state.borrow_mut(),
                    );
                });
            }
        }

        let mut tree = TreeViewState::new_with_density(TreeViewDensity::Compact);
        let roots: Vec<TreeRow> = (1001..=1010).map(|id| TreeRow::new(id, 0, false)).collect();
        tree.rebuild(&roots, |_| Vec::new());
        tree.set_row_label(1001, "Quick Access");
        tree.set_row_label(
            1002,
            "VeryLongNavigationPaneDirectoryNameThatMustElideBeforeSidebarRightEdge",
        );
        tree.set_row_divider_before(1002, true);
        tree.select(1001, false);
        tree.hovered = Some(1002);

        let mut rt = AppRuntime::new(
            NavPaneApp {
                tree_state: RefCell::new(tree),
            },
            Vec2::new(800.0, 600.0),
        );
        let _ = rt.frame(0.0);

        let tree_rect = rt.widget_rect(12).expect("tree view laid out");
        assert!(
            tree_rect.origin.y >= 80.0,
            "tree view must be placed below top bar, got y={}",
            tree_rect.origin.y
        );

        let scene = rt.scene();
        // 验证存在对齐 tree_rect 的 GPU 裁剪节点
        assert!(
            scene.clips.iter().any(|c| {
                (c.aabb_clip[0] - tree_rect.min().x).abs() < 0.5
                    && (c.aabb_clip[1] - tree_rect.min().y).abs() < 0.5
                    && (c.aabb_clip[2] - tree_rect.max().x).abs() < 0.5
                    && (c.aabb_clip[3] - tree_rect.max().y).abs() < 0.5
            }),
            "tree view must push a clip node matching its laid out bounds"
        );

        // 验证第一行与第二行均严格落在 tree_rect 内部且受 clip 约束，未溢出到顶部 y = 0
        for row_id in [1001u64, 1002u64] {
            let (start, end) = scene
                .node_spans
                .get(&row_id)
                .copied()
                .expect("tree row primitives emitted");
            assert!(end > start);
            for prim in &scene.primitives[start as usize..end as usize] {
                assert_ne!(prim.clip_idx, CLIP_NONE, "tree row must be clipped");
                assert!(
                    prim.local_pos[1] >= tree_rect.origin.y - 2.0,
                    "row {row_id} primitive overflowed above tree rect: y={} < {}",
                    prim.local_pos[1],
                    tree_rect.origin.y
                );
                assert!(
                    prim.local_pos[0] + prim.size[0] <= tree_rect.max().x + 2.0,
                    "row {row_id} primitive overflowed right edge"
                );
            }
        }

        // 验证第 1002 行的前置分隔线与悬停填充
        let (div_start, div_end) = scene
            .node_spans
            .get(&(1002u64 ^ 0x5452_4545_0002))
            .copied()
            .expect("row divider emitted");
        assert_eq!(div_end - div_start, 1);
        assert_eq!(
            scene.primitives[div_start as usize].fill_token_or_uv,
            Token::DIVIDER.raw() as u32
        );

        let (r1_start, _) = scene.node_spans[&1001];
        let (r2_start, _) = scene.node_spans[&1002];
        let (r3_start, _) = scene.node_spans[&1003];
        assert_eq!(
            scene.primitives[r1_start as usize].fill_token_or_uv,
            Token::NAV_SELECTED.raw() as u32
        );
        assert_eq!(
            scene.primitives[r2_start as usize].fill_token_or_uv,
            Token::NAV_HOVER.raw() as u32
        );
        assert_eq!(
            scene.primitives[r3_start as usize].fill_token_or_uv,
            Token::NONE.raw() as u32
        );

        // 验证 10 行 × 28px = 280px > 140px 时注册滚轮区域并发射滚动条
        assert!(
            rt.scroll_regions()
                .iter()
                .any(|&(k, r, content, vertical)| {
                    k == 12
                        && vertical
                        && (r.origin.y - tree_rect.origin.y).abs() < 0.5
                        && content.y > r.height()
                }),
            "overflowing tree view must register a vertical scroll region"
        );
        assert!(
            scene.node_spans.contains_key(&(12u64 ^ 0x5452_4545_0003)),
            "overflowing tree view must emit scrollbar thumb primitives"
        );
    }

    #[test]
    fn address_bar_defaults_height_clips_path_and_search_and_keeps_search_text_inside() {
        use unbound_theme::Token;
        use unbound_widgets::{AddressBarDesc, AddressBarState, PathSegment, ADDRESS_BAR_HEIGHT};

        struct AddressBarTestApp {
            bar_state: AddressBarState,
        }
        impl Application for AddressBarTestApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                // 刻意传入 height = 0.0，验证 ViewBuilder::address_bar 默认回退至 32px
                let desc = AddressBarDesc::with_path_segments(
                    Rect::new(0.0, 0.0, 760.0, 0.0),
                    "C:/Workspace/Assets/Shaders",
                    vec![
                        PathSegment::new("C:", "C:/"),
                        PathSegment::new("Workspace", "C:/Workspace"),
                        PathSegment::new("Assets", "C:/Workspace/Assets"),
                        PathSegment::new("Shaders", "C:/Workspace/Assets/Shaders")
                            .with_subfolders(false),
                    ],
                )
                .with_search("Search Shaders");
                v.address_bar(30, desc, &self.bar_state);
            }
        }

        let mut rt = AppRuntime::new(
            AddressBarTestApp {
                bar_state: AddressBarState::new(),
            },
            Vec2::new(800.0, 600.0),
        );
        let _ = rt.frame(0.0);

        let bar_rect = rt.widget_rect(30).expect("address bar laid out");
        assert!(
            (bar_rect.height() - ADDRESS_BAR_HEIGHT).abs() < 0.5,
            "address bar must fall back to ADDRESS_BAR_HEIGHT (32.0), got {}",
            bar_rect.height()
        );

        let layout = unbound_widgets::compute_layout(
            &AddressBarDesc::new(bar_rect, vec!["C:".to_owned()]).with_search("Search Shaders"),
        );
        let search_rect = layout.search_rect.expect("search rect");
        let scene = rt.scene();

        // 验证路径区与搜索区均注册了对应的 GPU 裁剪节点
        assert!(
            scene.clips.iter().any(|c| {
                (c.aabb_clip[0] - layout.address_rect.min().x).abs() < 0.5
                    && (c.aabb_clip[2] - layout.address_rect.max().x).abs() < 0.5
            }),
            "address bar must push a clip node matching address_rect"
        );
        assert!(
            scene.clips.iter().any(|c| {
                (c.aabb_clip[0] - search_rect.min().x).abs() < 0.5
                    && (c.aabb_clip[2] - search_rect.max().x).abs() < 0.5
            }),
            "address bar must push a clip node matching search_rect"
        );

        // 验证路径区文本层受 clip 约束
        let (addr_start, addr_end) = scene
            .node_spans
            .get(&(30u64 ^ 0x4142_5458_0001))
            .copied()
            .expect("address text layer emitted");
        assert!(addr_end > addr_start);
        for prim in &scene.primitives[addr_start as usize..addr_end as usize] {
            assert_ne!(prim.clip_idx, CLIP_NONE, "address text must be clipped");
        }

        // 验证搜索区占位文本层受 clip 约束、使用次级文本色且不越出 search_rect 右边界
        let (search_start, search_end) = scene
            .node_spans
            .get(&(30u64 ^ 0x4142_5458_0002))
            .copied()
            .expect("search text layer emitted");
        assert!(search_end > search_start);
        let search_prims = &scene.primitives[search_start as usize..search_end as usize];
        let mut saw_placeholder_glyph = false;
        for prim in search_prims {
            assert_ne!(prim.clip_idx, CLIP_NONE, "search text must be clipped");
            if prim.fill_token_or_uv == Token::TEXT_SECONDARY.raw() as u32 {
                saw_placeholder_glyph = true;
                assert!(
                    prim.local_pos[0] >= search_rect.origin.x - 1.0,
                    "search placeholder glyph escaped left of search_rect"
                );
                assert!(
                    prim.local_pos[0] + prim.size[0] <= search_rect.max().x + 1.0,
                    "search placeholder glyph overflowed right edge of search_rect"
                );
            }
        }
        assert!(
            saw_placeholder_glyph,
            "empty search box must render placeholder with Token::TEXT_SECONDARY"
        );
    }

    #[test]
    fn zero_sized_address_bar_and_tree_view_fill_flex_parent_and_shrink_on_resize() {
        use unbound_widgets::{
            AddressBarDesc, AddressBarState, TreeViewDensity, TreeViewDesc, TreeViewState,
            ADDRESS_BAR_HEIGHT,
        };
        use unbound_world::TreeRow;

        struct FlexLayoutApp {
            tree_state: RefCell<TreeViewState>,
            bar_state: AddressBarState,
        }
        impl Application for FlexLayoutApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.container(
                    1,
                    RawNodeDesc::container()
                        .width(Dim::percent(100.0))
                        .height(Dim::percent(100.0)),
                    |root| {
                        // 横向 flex 容器中的零宽地址栏应撑满剩余宽度，不得塌陷为 0px
                        root.container(
                            2,
                            RawNodeDesc::container()
                                .row()
                                .width(Dim::percent(100.0))
                                .height(Dim::px(40.0)),
                            |bar_row| {
                                bar_row.button(
                                    3,
                                    ButtonDesc::new(Rect::new(0.0, 0.0, 100.0, 32.0), "Side"),
                                );
                                bar_row.address_bar(
                                    4,
                                    AddressBarDesc::new(Rect::ZERO, vec!["C:".to_owned()]),
                                    &self.bar_state,
                                );
                            },
                        );
                        // 弹性主区内的零尺寸树视图应严格匹配父容器尺寸并随窗口缩放收缩
                        root.container(
                            5,
                            RawNodeDesc::container()
                                .width(Dim::percent(100.0))
                                .flex_grow(1.0),
                            |body| {
                                body.tree_view(
                                    6,
                                    TreeViewDesc::new(Rect::ZERO)
                                        .with_density(TreeViewDensity::Compact),
                                    &mut self.tree_state.borrow_mut(),
                                );
                            },
                        );
                    },
                );
            }
        }

        let mut tree = TreeViewState::new_with_density(TreeViewDensity::Compact);
        let roots: Vec<TreeRow> = (1..=6).map(|id| TreeRow::new(id, 0, false)).collect();
        tree.rebuild(&roots, |_| Vec::new());

        let mut rt = AppRuntime::new(
            FlexLayoutApp {
                tree_state: RefCell::new(tree),
                bar_state: AddressBarState::new(),
            },
            Vec2::new(800.0, 600.0),
        );
        let _ = rt.frame(0.0);

        let bar_rect = rt.widget_rect(4).expect("address bar laid out");
        assert!(
            bar_rect.width() >= 680.0,
            "zero-width address bar in flex row must expand to remaining width, got {}",
            bar_rect.width()
        );
        assert!((bar_rect.height() - ADDRESS_BAR_HEIGHT).abs() < 0.5);

        let tree_rect_large = rt.widget_rect(6).expect("tree view laid out");
        assert!((tree_rect_large.width() - 800.0).abs() < 1.0);
        assert!(
            (tree_rect_large.height() - 560.0).abs() < 2.0,
            "zero-size tree view must fill remaining height, got {}",
            tree_rect_large.height()
        );

        // 缩小窗口后，弹性树视图与地址栏均必须随之收缩，不能被上一帧尺寸锁死
        rt.resize(Vec2::new(500.0, 320.0));
        rt.invalidate();
        let _ = rt.frame(0.0);

        let bar_rect_small = rt.widget_rect(4).expect("address bar resized");
        assert!(
            bar_rect_small.width() <= 405.0 && bar_rect_small.width() >= 380.0,
            "address bar must shrink with resized row, got {}",
            bar_rect_small.width()
        );
        let tree_rect_small = rt.widget_rect(6).expect("tree view resized");
        assert!(
            tree_rect_small.width() <= 501.0 && tree_rect_small.height() <= 282.0,
            "zero-size tree view must shrink on resize, got {:?}",
            tree_rect_small
        );
    }

    #[test]
    fn tree_view_wheel_sync_preserves_programmatic_scroll_and_applies_wheel_delta() {
        use unbound_widgets::{TreeViewDensity, TreeViewDesc, TreeViewState};
        use unbound_world::TreeRow;

        struct ScrollSyncApp {
            tree_state: RefCell<TreeViewState>,
        }
        impl Application for ScrollSyncApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.tree_view(
                    20,
                    TreeViewDesc::new(Rect::new(0.0, 0.0, 220.0, 140.0))
                        .with_density(TreeViewDensity::Compact),
                    &mut self.tree_state.borrow_mut(),
                );
            }
        }

        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 220.0, 140.0))
            .with_density(TreeViewDensity::Compact);
        let mut tree = TreeViewState::new_with_density(TreeViewDensity::Compact);
        let roots: Vec<TreeRow> = (1..=20).map(|id| TreeRow::new(id, 0, false)).collect();
        tree.rebuild(&roots, |_| Vec::new());

        let mut rt = AppRuntime::new(
            ScrollSyncApp {
                tree_state: RefCell::new(tree),
            },
            Vec2::new(400.0, 300.0),
        );
        let _ = rt.frame(0.0);

        // 先触发一次滚轮滚动，建立运行时 ScrollState
        rt.wheel(Vec2::new(60.0, 60.0), Vec2::new(0.0, -1.0));
        let _ = rt.frame(0.0);
        let after_wheel = rt.app().tree_state.borrow().scroll_offset();
        assert!(
            after_wheel > 30.0,
            "wheel event must scroll tree view, got {after_wheel}"
        );

        // 随后调用方通过程序化 scroll_by 滚动到更深位置，下一帧重绘不得被旧滚轮值覆盖
        let _ = rt.app().tree_state.borrow_mut().scroll_by(&desc, 120.0);
        let expected_prog = rt.app().tree_state.borrow().scroll_offset();
        rt.invalidate();
        let _ = rt.frame(0.0);
        let after_prog_frame = rt.app().tree_state.borrow().scroll_offset();
        assert!(
            (after_prog_frame - expected_prog).abs() < 0.5,
            "programmatic scroll must not be overwritten by stale wheel offset: expected {expected_prog}, got {after_prog_frame}"
        );

        // 再次触发滚轮时应从程序化偏移基线继续累加
        rt.wheel(Vec2::new(60.0, 60.0), Vec2::new(0.0, -1.0));
        let _ = rt.frame(0.0);
        let after_second_wheel = rt.app().tree_state.borrow().scroll_offset();
        assert!(
            after_second_wheel > after_prog_frame + 30.0,
            "subsequent wheel delta must accumulate on top of programmatic offset: {after_second_wheel} <= {after_prog_frame}"
        );
    }

    #[test]
    fn address_bar_active_edit_does_not_elide_and_shifts_by_scroll_x() {
        use unbound_widgets::{AddressBarDesc, AddressBarState, ADDRESS_BAR_HEIGHT};

        struct EditBarApp {
            bar_state: AddressBarState,
        }
        impl Application for EditBarApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                let desc = AddressBarDesc::new(
                    Rect::new(0.0, 0.0, 320.0, ADDRESS_BAR_HEIGHT),
                    vec!["C:".to_owned(), "Workspace".to_owned()],
                );
                v.address_bar(40, desc, &self.bar_state);
            }
        }

        let long_path = "C:/VeryLongWorkspaceDirectory/SubFolderAlpha/SubFolderBeta/TargetFile.rs";
        let mut bar_state = AddressBarState::new();
        bar_state.begin_edit(long_path);
        bar_state.edit_core.set_scroll_x(40.0);

        let mut rt = AppRuntime::new(EditBarApp { bar_state }, Vec2::new(400.0, 120.0));
        let _ = rt.frame(0.0);

        let scene = rt.scene();
        let (addr_start, addr_end) = scene
            .node_spans
            .get(&(40u64 ^ 0x4142_5458_0001))
            .copied()
            .expect("address edit text layer emitted");
        let prims = &scene.primitives[addr_start as usize..addr_end as usize];
        // 活动编辑态不执行省略，应发射完整原文对应字形并按 scroll_x 左移起点（受 clip 裁剪）
        let bar_rect = rt.widget_rect(40).expect("address bar laid out");
        let layout = unbound_widgets::compute_layout(&AddressBarDesc::new(
            bar_rect,
            vec!["C:".to_owned(), "Workspace".to_owned()],
        ));
        let unshifted_text_x = layout.root_icon_rect.max().x + 6.0;
        let glyph_count = prims
            .iter()
            .filter(|p| p.size[1] > 4.0 && p.size[0] > 2.0)
            .count();
        assert!(
            glyph_count >= 25,
            "active edit text must not be elided down to narrow width, got {glyph_count} glyph prims"
        );
        assert!(
            prims
                .iter()
                .any(|p| p.local_pos[0] < unshifted_text_x - 20.0 && p.clip_idx != CLIP_NONE),
            "active edit glyphs must shift left by scroll_x under clip"
        );
    }

    #[test]
    fn widget_icon_external_textures_resolve_across_widgets_and_fallback_when_uncommitted() {
        use unbound_gpu::data::PrimType;
        use unbound_widgets::{
            AddressBarDesc, AddressBarState, ContextMenuState, MenuDesc, MenuItem, PathSegment,
            ShellIcon, TabItem, TabsDesc, TreeViewDesc, TreeViewState, WidgetIcon,
        };
        use unbound_world::TreeRow;

        struct IconBridgeApp {
            tree_state: RefCell<TreeViewState>,
            address_state: AddressBarState,
            menu_state: ContextMenuState,
            ready_handle: Option<ExternalTextureHandle>,
            pending_handle: Option<ExternalTextureHandle>,
        }

        impl Application for IconBridgeApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                let ready_icon = self
                    .ready_handle
                    .map_or(WidgetIcon::None, |h| WidgetIcon::ExternalTexture(h.0));
                let pending_icon = self
                    .pending_handle
                    .map_or(WidgetIcon::None, |h| WidgetIcon::ExternalTexture(h.0));

                v.column(1, |col| {
                    col.tree_view(
                        10,
                        TreeViewDesc::new(Rect::new(0.0, 0.0, 240.0, 120.0)),
                        &mut self.tree_state.borrow_mut(),
                    );
                    col.address_bar(
                        11,
                        AddressBarDesc::with_path_segments(
                            Rect::new(0.0, 0.0, 420.0, 32.0),
                            "C:\\Work",
                            vec![
                                PathSegment::new("C:", "C:\\").with_widget_icon(ready_icon),
                                PathSegment::new("Work", "C:\\Work"),
                            ],
                        ),
                        &self.address_state,
                    );
                    col.button(
                        12,
                        ButtonDesc::new(Rect::new(0.0, 0.0, 120.0, 32.0), "ReadyBtn")
                            .with_widget_icon(ready_icon),
                    );
                    col.button(
                        13,
                        ButtonDesc::new(Rect::new(0.0, 0.0, 120.0, 32.0), "PendingBtn")
                            .with_widget_icon(pending_icon),
                    );
                    col.tabs(
                        14,
                        TabsDesc::new(Rect::new(0.0, 0.0, 360.0, 40.0), &[]).with_items(vec![
                            TabItem::new(101, "ReadyTab").with_widget_icon(ready_icon),
                            TabItem::new(102, "PendingTab")
                                .with_icon(ShellIcon::Folder)
                                .with_widget_icon(pending_icon),
                        ]),
                    );
                    col.context_menu(
                        15,
                        MenuDesc::new(
                            Rect::new(0.0, 0.0, 180.0, 80.0),
                            vec![
                                MenuItem::action("ReadyItem").with_widget_icon(ready_icon),
                                MenuItem::action("PendingItem").with_widget_icon(pending_icon),
                            ],
                        ),
                        &self.menu_state,
                    );
                });
            }
        }

        let mut tree = TreeViewState::new(28.0);
        tree.rebuild(
            &[TreeRow::new(501, 0, false), TreeRow::new(502, 0, false)],
            |_| Vec::new(),
        );
        let mut menu_state = ContextMenuState::new();
        menu_state.open_at(Vec2::new(60.0, 60.0), 12, None);

        let app = IconBridgeApp {
            tree_state: RefCell::new(tree),
            address_state: AddressBarState::new(),
            menu_state,
            ready_handle: None,
            pending_handle: None,
        };
        let mut rt = AppRuntime::new(app, Vec2::new(1000.0, 700.0));
        let ready_h =
            rt.asset_hub_mut()
                .upload_external_rgba8(16, 16, true, vec![255u8; 16 * 16 * 4]);
        let pending_h =
            rt.asset_hub_mut()
                .upload_external_rgba8(16, 16, true, vec![128u8; 16 * 16 * 4]);
        rt.asset_hub_mut()
            .commit_external_texture_slot(ready_h, Ok(63));

        // 校验辅助函数本身：已提交返回 GPU 槽位，未提交退化为 None，矢量与 None 原样透传
        assert_eq!(
            resolve_widget_icon(rt.asset_hub(), WidgetIcon::ExternalTexture(ready_h.0)),
            WidgetIcon::ExternalTexture(63)
        );
        assert_eq!(
            resolve_widget_icon(rt.asset_hub(), WidgetIcon::ExternalTexture(pending_h.0)),
            WidgetIcon::None
        );
        assert_eq!(
            resolve_widget_icon(rt.asset_hub(), WidgetIcon::Vector(ShellIcon::Folder)),
            WidgetIcon::Vector(ShellIcon::Folder)
        );

        {
            let a = rt.application_mut();
            a.ready_handle = Some(ready_h);
            a.pending_handle = Some(pending_h);
            let mut ts = a.tree_state.borrow_mut();
            ts.set_row_icon(501, WidgetIcon::ExternalTexture(ready_h.0));
            ts.set_row_icon(502, WidgetIcon::ExternalTexture(pending_h.0));
        }
        rt.invalidate();
        let _ = rt.frame(0.0);

        let scene = rt.scene();
        let none_raw = u32::from(unbound_theme::Token::NONE.raw());
        let has_image_slot = |node_key: u64, slot: u32| -> bool {
            scene.node_spans.get(&node_key).is_some_and(|&(s, e)| {
                scene.primitives[s as usize..e as usize].iter().any(|p| {
                    p.prim_type() == PrimType::Image
                        && p.border_token_or_tex == slot
                        && p.fill_token_or_uv == none_raw
                })
            })
        };
        let has_any_image = |node_key: u64| -> bool {
            scene.node_spans.get(&node_key).is_some_and(|&(s, e)| {
                scene.primitives[s as usize..e as usize]
                    .iter()
                    .any(|p| p.prim_type() == PrimType::Image)
            })
        };

        assert!(
            has_image_slot(501, 63),
            "tree row ready icon must bind slot 63"
        );
        assert!(
            !has_any_image(502),
            "tree row pending icon must not emit image"
        );
        assert!(
            has_image_slot(11, 63),
            "address bar root icon must bind slot 63"
        );
        assert!(
            has_image_slot(12, 63),
            "ready button icon must bind slot 63"
        );
        assert!(
            !has_any_image(13),
            "pending button icon must not emit image"
        );
        assert!(has_image_slot(14, 63), "tabs ready icon must bind slot 63");
        assert!(
            has_image_slot(mix_key(15, 0x4354_584D), 63),
            "context menu ready icon must bind slot 63"
        );
        assert!(
            !scene.primitives.iter().any(|p| {
                p.prim_type() == PrimType::Image && p.border_token_or_tex == pending_h.0 as u32
            }),
            "uncommitted external handle must never leak into GPU image primitives"
        );
    }

    #[test]
    fn tabview_sub_hit_states_are_isolated_and_close_add_context_events_round_trip() {
        use unbound_gpu::data::PrimType;
        use unbound_theme::Token;
        use unbound_widgets::{TabItem, TabViewStyle, TabWidthMode, TabsDesc, TabsTokens};

        const TABS_KEY: u64 = 7100;

        struct TabsInteractionApp {
            desc: TabsDesc,
            context_seen: Option<TabHitTarget>,
        }

        impl Application for TabsInteractionApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(
                &mut self,
                _: &mut (),
                event: AppEvent,
                ctx: &mut AppContext,
            ) -> EventOutcome {
                if let AppEvent::ContextMenu { .. } = event {
                    self.context_seen = ctx.tab_context_target(TABS_KEY);
                }
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                v.tabs(TABS_KEY, self.desc.clone());
            }
        }

        let desc = TabsDesc::new(Rect::new(0.0, 0.0, 640.0, 40.0), &[])
            .with_items(vec![
                TabItem::new(101, "Alpha"),
                TabItem::new(102, "Beta"),
                TabItem::new(103, "Gamma"),
                TabItem::new(104, "Pinned").pinned(),
            ])
            .with_addable(true)
            .with_style(TabViewStyle::ShellChrome)
            .with_width_mode(TabWidthMode::adaptive_default());

        let app = TabsInteractionApp {
            desc: desc.clone(),
            context_seen: None,
        };
        let mut rt = AppRuntime::new(app, Vec2::new(800.0, 200.0));
        let _ = rt.frame(0.0);

        let rect = rt.widget_rect(TABS_KEY).expect("tabs mounted rect");
        let mut laid_desc = desc.clone();
        laid_desc.rect = rect;

        let tab0_bg_point = laid_desc.label_rect(0, 0.0).center();
        let tab1_bg_point = laid_desc.label_rect(1, 0.0).center();
        let tab2_bg_point = laid_desc.label_rect(2, 0.0).center();
        let tab3_bg_point = laid_desc.label_rect(3, 0.0).center();
        let close1_rect = laid_desc.close_rect(1, 0.0).expect("tab 1 close rect");
        let close1_center = close1_rect.center();
        let add_rect = laid_desc.add_rect().expect("add button rect");
        let add_center = add_rect.center();

        let tokens = TabsTokens::standard();
        let tab_bg_fill = |rt: &AppRuntime<TabsInteractionApp>, idx: usize| -> u32 {
            let r = laid_desc.tab_rect(idx, 0.0);
            let scene = rt.scene();
            let (s, e) = scene.node_spans[&TABS_KEY];
            scene.primitives[s as usize..e as usize]
                .iter()
                .find(|p| {
                    p.prim_type() == PrimType::RoundedRect
                        && (p.local_pos[0] - r.origin.x).abs() < 1.0
                        && (p.size[0] - r.size.x).abs() < 1.0
                        && (p.size[1] - r.size.y).abs() < 1.0
                })
                .expect("tab background rect")
                .fill_token_or_uv
        };
        let close_bg_fill = |rt: &AppRuntime<TabsInteractionApp>, idx: usize| -> u32 {
            let r = laid_desc.close_rect(idx, 0.0).expect("close rect");
            let scene = rt.scene();
            let (s, e) = scene.node_spans[&TABS_KEY];
            scene.primitives[s as usize..e as usize]
                .iter()
                .find(|p| {
                    p.prim_type() == PrimType::RoundedRect
                        && (p.local_pos[0] - r.origin.x).abs() < 1.0
                        && (p.size[0] - r.size.x).abs() < 1.0
                        && (p.size[1] - r.size.y).abs() < 1.0
                })
                .expect("close button background rect")
                .fill_token_or_uv
        };
        let add_bg_fill = |rt: &AppRuntime<TabsInteractionApp>| -> u32 {
            let scene = rt.scene();
            let (s, e) = scene.node_spans[&TABS_KEY];
            scene.primitives[s as usize..e as usize]
                .iter()
                .find(|p| {
                    p.prim_type() == PrimType::RoundedRect
                        && (p.local_pos[0] - add_rect.origin.x).abs() < 1.0
                        && (p.size[0] - add_rect.size.x).abs() < 1.0
                        && (p.size[1] - add_rect.size.y).abs() < 1.0
                })
                .expect("add button background rect")
                .fill_token_or_uv
        };

        // 1. 悬停标签 1 主体：仅标签 1 呈现悬停底色，标签 2 与所有关闭按钮均保持静息。
        rt.pointer_move(tab1_bg_point);
        let _ = rt.frame(0.016);
        assert_eq!(tab_bg_fill(&rt, 0), u32::from(tokens.tab_active.raw()));
        assert_eq!(tab_bg_fill(&rt, 1), u32::from(tokens.tab_hover.raw()));
        assert_eq!(tab_bg_fill(&rt, 2), u32::from(tokens.tab.raw()));
        assert_eq!(close_bg_fill(&rt, 0), u32::from(Token::NONE.raw()));
        assert_eq!(close_bg_fill(&rt, 1), u32::from(Token::NONE.raw()));
        assert_eq!(add_bg_fill(&rt), u32::from(Token::NONE.raw()));

        // 2. 悬停并按下标签 1 的关闭按钮 `×`：仅标签 1 的 `×` 呈现 HOVER / PRESSED，不串扰其他标签。
        rt.pointer_move(close1_center);
        let _ = rt.frame(0.032);
        assert_eq!(close_bg_fill(&rt, 1), u32::from(Token::HOVER.raw()));
        assert_eq!(close_bg_fill(&rt, 0), u32::from(Token::NONE.raw()));
        assert_eq!(tab_bg_fill(&rt, 2), u32::from(tokens.tab.raw()));

        rt.pointer_down(close1_center, PointerButton::Left);
        let _ = rt.frame(0.048);
        assert_eq!(close_bg_fill(&rt, 1), u32::from(Token::PRESSED.raw()));
        assert_eq!(close_bg_fill(&rt, 0), u32::from(Token::NONE.raw()));

        // 3. 左键释放标签 1 的 `×`：发出关闭请求且不误切当前激活标签（仍为 0）。
        rt.pointer_up(close1_center, PointerButton::Left);
        assert_eq!(
            rt.context().take_tab_close_request(TABS_KEY),
            Some(TabHitTarget { index: 1, id: 102 })
        );
        assert_eq!(rt.context().take_tab_close_request(TABS_KEY), None);
        assert_eq!(rt.context().tab_active(TABS_KEY), Some(0));

        // 4. 鼠标中键点击标签 2：发出关闭请求且不误切激活标签；中键点击固定标签 3 不触发关闭。
        rt.pointer_move(tab2_bg_point);
        rt.pointer_down(tab2_bg_point, PointerButton::Middle);
        rt.pointer_up(tab2_bg_point, PointerButton::Middle);
        assert_eq!(
            rt.context().take_tab_close_request(TABS_KEY),
            Some(TabHitTarget { index: 2, id: 103 })
        );
        assert_eq!(rt.context().tab_active(TABS_KEY), Some(0));

        rt.pointer_move(tab3_bg_point);
        rt.pointer_down(tab3_bg_point, PointerButton::Middle);
        rt.pointer_up(tab3_bg_point, PointerButton::Middle);
        assert_eq!(rt.context().take_tab_close_request(TABS_KEY), None);

        // 5. 悬停、按下并左键点击 `+` 按钮：仅 `+` 高亮并置位新建请求。
        rt.pointer_move(add_center);
        let _ = rt.frame(0.064);
        assert_eq!(add_bg_fill(&rt), u32::from(Token::HOVER.raw()));
        rt.pointer_down(add_center, PointerButton::Left);
        let _ = rt.frame(0.080);
        assert_eq!(add_bg_fill(&rt), u32::from(Token::PRESSED.raw()));
        rt.pointer_up(add_center, PointerButton::Left);
        assert!(rt.context().take_tab_add_request(TABS_KEY));
        assert!(!rt.context().take_tab_add_request(TABS_KEY));

        // 6. 右键点击标签 1：触发 ContextMenu 时 tab_context_target 精确返回标签 1，且不误切激活标签。
        rt.pointer_move(tab1_bg_point);
        rt.pointer_down(tab1_bg_point, PointerButton::Right);
        rt.pointer_up(tab1_bg_point, PointerButton::Right);
        assert_eq!(
            rt.application().context_seen,
            Some(TabHitTarget { index: 1, id: 102 })
        );
        assert_eq!(
            rt.context().tab_context_target(TABS_KEY),
            Some(TabHitTarget { index: 1, id: 102 })
        );
        assert_eq!(
            rt.context().take_tab_context_target(TABS_KEY),
            Some(TabHitTarget { index: 1, id: 102 })
        );
        assert_eq!(rt.context().tab_context_target(TABS_KEY), None);
        assert_eq!(rt.context().tab_active(TABS_KEY), Some(0));

        // 7. 左键点击标签 1 主体：正常切换激活标签至 1；再点回标签 0 验证切换。
        rt.pointer_down(tab1_bg_point, PointerButton::Left);
        rt.pointer_up(tab1_bg_point, PointerButton::Left);
        assert_eq!(rt.context().tab_active(TABS_KEY), Some(1));
        rt.pointer_move(tab0_bg_point);
        rt.pointer_down(tab0_bg_point, PointerButton::Left);
        rt.pointer_up(tab0_bg_point, PointerButton::Left);
        assert_eq!(rt.context().tab_active(TABS_KEY), Some(0));
    }

    #[test]
    fn context_menu_win11_render_sub_hits_and_click_activation_round_trip() {
        use unbound_gpu::data::PrimType;
        use unbound_theme::Token;
        use unbound_widgets::{
            ContextMenuState, MenuDesc, MenuItem, WidgetIcon, CONTEXT_MENU_ITEM_HEIGHT,
            MENU_PANEL_PADDING, MENU_SEPARATOR_HEIGHT,
        };

        const MENU_KEY: u64 = 7200;

        struct ContextMenuTestApp {
            menu_state: ContextMenuState,
            ext_handle: Option<ExternalTextureHandle>,
            activated_log: Vec<usize>,
        }

        impl Application for ContextMenuTestApp {
            type State = ();
            fn init(&mut self, _: &mut AppContext) {}
            fn reduce(
                &mut self,
                _: &mut (),
                _event: AppEvent,
                ctx: &mut AppContext,
            ) -> EventOutcome {
                if let Some(idx) = ctx.take_context_menu_activated(MENU_KEY) {
                    self.activated_log.push(idx);
                    return EventOutcome::new(DirtyFlags::PAINT);
                }
                EventOutcome::new(DirtyFlags::NONE)
            }
            fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
                // 声明菜单锚点节点：ContextMenu 浮层按锚点存活，锚点缺失会被 prune_anchors 裁掉。
                // 注意键不能取 1（语义根保留键），这里用 100。
                v.button(
                    100,
                    unbound_widgets::ButtonDesc::new(Rect::new(0.0, 0.0, 80.0, 30.0), "Target"),
                );
                let icon = self
                    .ext_handle
                    .map_or(WidgetIcon::None, |h| WidgetIcon::ExternalTexture(h.0));
                let desc = MenuDesc::context_menu(
                    Rect::new(0.0, 0.0, 220.0, 0.0),
                    vec![
                        MenuItem::action("Open")
                            .with_widget_icon(icon)
                            .with_shortcut("Ctrl+O"),
                        MenuItem::separator(),
                        MenuItem::action("Disabled")
                            .disabled()
                            .with_shortcut("Ctrl+D"),
                        MenuItem::action("More").with_submenu(vec![MenuItem::action("Sub")]),
                        MenuItem::action("Delete").danger(true),
                    ],
                );
                v.context_menu(MENU_KEY, desc, &self.menu_state);
            }
        }

        let mut menu_state = ContextMenuState::new();
        menu_state.open_at(Vec2::new(80.0, 60.0), 100, None);
        let app = ContextMenuTestApp {
            menu_state,
            ext_handle: None,
            activated_log: Vec::new(),
        };
        let mut rt = AppRuntime::new(app, Vec2::new(1000.0, 700.0));
        let ext_h =
            rt.asset_hub_mut()
                .upload_external_rgba8(16, 16, true, vec![255u8; 16 * 16 * 4]);
        rt.asset_hub_mut()
            .commit_external_texture_slot(ext_h, Ok(88));
        rt.application_mut().ext_handle = Some(ext_h);
        rt.invalidate();
        let _ = rt.frame(0.0);
        // 浮层（portal 层）在首帧挂载后次帧才进入图元收集，需再跑一帧才能断言 span。
        let _ = rt.frame(0.0);

        let widget_key = mix_key(MENU_KEY, 0x4354_584D);
        let prim_count_idle = rt.scene().primitives.len();
        let span = *rt
            .scene()
            .node_spans
            .get(&widget_key)
            .expect("context menu primitive span");
        let menu_prims = &rt.scene().primitives[span.0 as usize..span.1 as usize];

        // 1. 校验标签字形、快捷键字形、危险项前景色、禁用项前景色与外部彩色图标槽位。
        assert!(menu_prims.iter().any(|p| p.prim_type() == PrimType::Glyph
            && p.fill_token_or_uv == u32::from(Token::TEXT_PRIMARY.raw())));
        assert!(menu_prims.iter().any(|p| p.prim_type() == PrimType::Glyph
            && p.fill_token_or_uv == u32::from(Token::TEXT_SECONDARY.raw())));
        assert!(menu_prims.iter().any(|p| p.prim_type() == PrimType::Glyph
            && p.fill_token_or_uv == u32::from(Token::ERROR.raw())));
        assert!(menu_prims.iter().any(|p| p.prim_type() == PrimType::Glyph
            && p.fill_token_or_uv == u32::from(Token::TEXT_DISABLED.raw())));
        assert!(menu_prims.iter().any(|p| p.prim_type() == PrimType::Image
            && p.border_token_or_tex == 88
            && p.fill_token_or_uv == u32::from(Token::NONE.raw())));

        // 精确计算各行中心点（与 Win11 4px 内边距、32px 项高与 9px 分隔线对齐）。
        let row0_center = Vec2::new(
            80.0 + 110.0,
            60.0 + MENU_PANEL_PADDING + CONTEXT_MENU_ITEM_HEIGHT * 0.5,
        );
        let sep_center = Vec2::new(
            80.0 + 110.0,
            60.0 + MENU_PANEL_PADDING + CONTEXT_MENU_ITEM_HEIGHT + MENU_SEPARATOR_HEIGHT * 0.5,
        );
        let dis_center = Vec2::new(
            80.0 + 110.0,
            60.0 + MENU_PANEL_PADDING
                + CONTEXT_MENU_ITEM_HEIGHT
                + MENU_SEPARATOR_HEIGHT
                + CONTEXT_MENU_ITEM_HEIGHT * 0.5,
        );
        let danger_center = Vec2::new(
            80.0 + 110.0,
            60.0 + MENU_PANEL_PADDING
                + CONTEXT_MENU_ITEM_HEIGHT * 3.0
                + MENU_SEPARATOR_HEIGHT
                + CONTEXT_MENU_ITEM_HEIGHT * 0.5,
        );

        let pill_fill_at = |rt: &AppRuntime<ContextMenuTestApp>, row_y: f32| -> u32 {
            let s = rt.scene();
            let sp = s.node_spans[&widget_key];
            s.primitives[sp.0 as usize..sp.1 as usize]
                .iter()
                .find(|p| {
                    p.prim_type() == PrimType::RoundedRect
                        && p.size[0] > 180.0
                        && p.size[1] < CONTEXT_MENU_ITEM_HEIGHT
                        && (p.local_pos[1] - (row_y - (CONTEXT_MENU_ITEM_HEIGHT - 4.0) * 0.5)).abs()
                            < 2.0
                })
                .expect("menu item pill primitive")
                .fill_token_or_uv
        };

        // 2. 悬停与按下项 0：仅项 0 切换为 HOVER / PRESSED，危险项保持透明，且图元总数恒定。
        rt.pointer_move(row0_center);
        let _ = rt.frame(0.016);
        assert_eq!(rt.scene().primitives.len(), prim_count_idle);
        assert_eq!(
            pill_fill_at(&rt, row0_center.y),
            u32::from(Token::HOVER.raw())
        );
        assert_eq!(
            pill_fill_at(&rt, danger_center.y),
            u32::from(Token::NONE.raw())
        );

        rt.pointer_down(row0_center, PointerButton::Left);
        let _ = rt.frame(0.032);
        assert_eq!(rt.scene().primitives.len(), prim_count_idle);
        assert_eq!(
            pill_fill_at(&rt, row0_center.y),
            u32::from(Token::PRESSED.raw())
        );
        assert_eq!(
            pill_fill_at(&rt, danger_center.y),
            u32::from(Token::NONE.raw())
        );

        // 3. 左键释放项 0：触发激活并被应用消费一次，二次读取返回 None。
        rt.pointer_up(row0_center, PointerButton::Left);
        assert_eq!(rt.application().activated_log, vec![0]);
        assert_eq!(rt.context().take_context_menu_activated(MENU_KEY), None);

        // 4. 点击分隔线与禁用项：不触发激活。
        rt.pointer_move(sep_center);
        rt.pointer_down(sep_center, PointerButton::Left);
        rt.pointer_up(sep_center, PointerButton::Left);
        assert_eq!(rt.application().activated_log, vec![0]);
        assert_eq!(rt.context().take_context_menu_activated(MENU_KEY), None);

        rt.pointer_move(dis_center);
        rt.pointer_down(dis_center, PointerButton::Left);
        rt.pointer_up(dis_center, PointerButton::Left);
        assert_eq!(rt.application().activated_log, vec![0]);
        assert_eq!(rt.context().take_context_menu_activated(MENU_KEY), None);

        // 5. 点击危险项（索引 4）：精确回传 Some(4)。
        rt.pointer_move(danger_center);
        rt.pointer_down(danger_center, PointerButton::Left);
        rt.pointer_up(danger_center, PointerButton::Left);
        assert_eq!(rt.application().activated_log, vec![0, 4]);
        assert_eq!(rt.context().take_context_menu_activated(MENU_KEY), None);
    }
}

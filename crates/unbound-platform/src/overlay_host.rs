//! 透明穿透覆盖层宿主、多窗口编排、遮挡零开销节流、OLE 文件拖拽桥接与防休眠守卫（阶段 1 WP2）。
//!
//! 面向两大宿主的核心平台能力：
//! * **下游渲染工具**：需要替代 Tauri 透明覆盖层宿主（`WS_EX_LAYERED` /
//!   `WS_EX_TRANSPARENT` / `WS_EX_NOACTIVATE`、按 UI 矩形动态切换鼠标穿透与拦截、
//!   紧跟被调试目标进程 HWND 的坐标与最小化联动），以及内置浮窗（Tasks / Performance /
//!   Settings）与主窗口在单 `VkDevice` 下的多窗口生命周期编排；
//! * **下游资产应用**：需要系统文件拖入与向外拖出（OLE `IDropTarget` / `IDropSource` / `CF_HDROP`
//!   策略层与内存测试替身）、窗口完全被遮挡时的 0% 渲染暂停，以及重度任务期间的屏幕防休眠守卫。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use unbound_math::{Rect, Vec2};

use crate::window::{ScreenRect, WindowConfig, WindowPosition};
use crate::windows::{WindowId, WindowRegistry};

/// 透明覆盖层与特殊浮窗的扩展窗口规格（在保持 [`WindowConfig`] 结构体字面量兼容的同时提供完整扩展）。
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayWindowSpec {
    /// 基础窗口配置。
    pub base: WindowConfig,
    /// 是否启用 DWM Alpha 合成透明背景。
    pub transparent: bool,
    /// 是否置顶（`HWND_TOPMOST`）。
    pub always_on_top: bool,
    /// 是否默认开启全窗口鼠标穿透（`WS_EX_TRANSPARENT`）。
    pub click_through: bool,
    /// 是否不在任务栏显示图标（`WS_EX_TOOLWINDOW`）。
    pub skip_taskbar: bool,
    /// 点击或显示时是否不抢占前台焦点（`WS_EX_NOACTIVATE`）。
    pub no_activate: bool,
    /// 所属父窗口逻辑 ID（跨平台：Win32 由父窗原点换算全局矩形，Wayland 映射为 `xdg_popup` 父表面）。
    pub owner_window: Option<WindowId>,
    /// 相对父窗口的锚点矩形（物理像素，用于越界浮层定位与贴边翻转）。
    pub anchor_rect: Option<ScreenRect>,
}

impl Default for OverlayWindowSpec {
    fn default() -> Self {
        Self {
            base: WindowConfig {
                decorations: false,
                resizable: false,
                ..WindowConfig::default()
            },
            transparent: true,
            always_on_top: true,
            click_through: true,
            skip_taskbar: true,
            no_activate: true,
            owner_window: None,
            anchor_rect: None,
        }
    }
}

impl OverlayWindowSpec {
    /// 创建面向下游渲染工具调试覆盖层的预设规格。
    #[must_use]
    pub fn debugger_overlay(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            base: WindowConfig::default()
                .with_title(title)
                .with_size(width, height),
            transparent: true,
            always_on_top: true,
            click_through: true,
            skip_taskbar: true,
            no_activate: true,
            owner_window: None,
            anchor_rect: None,
        }
    }

    /// 创建面向内置工具浮窗（如 Tasks / Performance / Settings）的预设规格。
    #[must_use]
    pub fn utility_floating(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            base: WindowConfig {
                title: title.into(),
                width,
                height,
                resizable: true,
                decorations: false,
                min_width: 240,
                min_height: 160,
                visible: true,
                refresh: crate::pacing::REFRESH_120,
                position: WindowPosition::CenterActiveMonitor,
                initial_position: None,
                initial_maximized: false,
                // 覆盖层走分层窗口的 `transparent` 开关，与交换链合成方式无关：
                // 把它改成预乘 alpha 会让覆盖层内容按另一套 alpha 语义参与合成。
                surface_alpha: crate::window::SurfaceAlpha::Opaque,
                backdrop: None,
                ..WindowConfig::default()
            },
            transparent: false,
            always_on_top: false,
            click_through: false,
            skip_taskbar: false,
            no_activate: false,
            owner_window: None,
            anchor_rect: None,
        }
    }

    /// 计算对应的 Win32 扩展窗口样式位（`WS_EX_*`）。
    #[must_use]
    pub const fn win32_ex_style_bits(&self) -> u32 {
        const WS_EX_TOPMOST: u32 = 0x0000_0008;
        const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
        const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
        const WS_EX_LAYERED: u32 = 0x0008_0000;
        const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

        let mut bits = 0u32;
        if self.transparent {
            bits |= WS_EX_LAYERED;
        }
        if self.always_on_top {
            bits |= WS_EX_TOPMOST;
        }
        if self.click_through {
            bits |= WS_EX_TRANSPARENT;
        }
        if self.skip_taskbar {
            bits |= WS_EX_TOOLWINDOW;
        }
        if self.no_activate {
            bits |= WS_EX_NOACTIVATE;
        }
        bits
    }
}

/// 透明覆盖层的动态鼠标穿透命中结果。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayHitDisposition {
    /// 指针位于交互式 UI 面板或小部件内，由覆盖层窗口捕获（对应 Win32 `HTCLIENT` 并移除 `WS_EX_TRANSPARENT`）。
    CaptureClient,
    /// 指针位于覆盖层透明空白区，穿透至下方的目标游戏/渲染窗口（对应 Win32 `HTTRANSPARENT`）。
    PassThrough,
}

/// 动态鼠标穿透判定器：根据当前帧挂载的交互式 UI 矩形列表实时决定指针是否穿透。
#[derive(Clone, Debug, Default)]
pub struct HitTestPassThroughPolicy {
    interactive_rects: Vec<Rect>,
    force_pass_through: bool,
}

impl HitTestPassThroughPolicy {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 更新当前帧可响应交互的 UI 区域列表（如 HUD 工具条、浮动面板、弹窗）。
    pub fn set_interactive_rects(&mut self, rects: Vec<Rect>) {
        self.interactive_rects = rects;
    }

    /// 强制开启或关闭全局穿透（例如按住快捷键完全隐藏交互层时）。
    pub fn set_force_pass_through(&mut self, force: bool) {
        self.force_pass_through = force;
    }

    /// 判断逻辑坐标 `point` 处的指针是否应被覆盖层捕获。
    #[must_use]
    pub fn evaluate(&self, point: Vec2) -> OverlayHitDisposition {
        if self.force_pass_through {
            return OverlayHitDisposition::PassThrough;
        }
        if self
            .interactive_rects
            .iter()
            .any(|rect| rect.contains_point(point))
        {
            OverlayHitDisposition::CaptureClient
        } else {
            OverlayHitDisposition::PassThrough
        }
    }
}

/// 目标进程窗口跟随状态机（用于下游渲染工具覆盖层与目标 HWND 同步位置、尺寸与最小化隐藏）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TargetWindowFollower {
    /// 被调试的目标窗口 `HWND`。
    pub target_hwnd: isize,
    /// 目标窗口在屏幕物理空间的最新客户区矩形。
    pub target_bounds: Rect,
    /// 目标窗口是否处于最小化状态。
    pub target_minimized: bool,
    /// 目标窗口是否处于前台激活状态。
    pub target_foreground: bool,
}

impl TargetWindowFollower {
    #[must_use]
    pub const fn new(target_hwnd: isize) -> Self {
        Self {
            target_hwnd,
            target_bounds: Rect::ZERO,
            target_minimized: false,
            target_foreground: false,
        }
    }

    /// 同步目标窗口最新几何与状态，返回覆盖层窗口是否应当可见以及其目标矩形。
    pub fn update_target_state(
        &mut self,
        bounds: Rect,
        minimized: bool,
        foreground: bool,
    ) -> (bool, Rect) {
        self.target_bounds = bounds;
        self.target_minimized = minimized;
        self.target_foreground = foreground;
        let should_show = !minimized && !bounds.is_empty();
        (should_show, bounds)
    }
}

pub use crate::pacing::RenderThrottleState;

/// 窗口遮挡轮询默认间隔（500ms）。
pub const DEFAULT_OCCLUSION_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Win32 Z 序枚举最大窗口层数上限，防止极端窗口数量下的轮询卡顿。
pub const MAX_OCCLUSION_Z_ORDER_LAYERS: usize = 64;

/// 窗口遮挡检测器：判断目标窗口是否最小化、被虚拟桌面隐藏（`DWMWA_CLOAKED`），
/// 或被 Z 序上方的不透明窗口矩形并集完全覆盖。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct OcclusionMonitor {
    minimized: bool,
    visible: bool,
    occluded: bool,
    focused: bool,
    cloaked: bool,
    poll_interval: Duration,
    last_probe: Option<Instant>,
}

impl Default for OcclusionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl OcclusionMonitor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            minimized: false,
            visible: true,
            occluded: false,
            focused: true,
            cloaked: false,
            poll_interval: DEFAULT_OCCLUSION_POLL_INTERVAL,
            last_probe: None,
        }
    }

    /// 使用自定义轮询间隔构造窗口遮挡监视器。
    #[must_use]
    pub const fn with_poll_interval(poll_interval: Duration) -> Self {
        Self {
            minimized: false,
            visible: true,
            occluded: false,
            focused: true,
            cloaked: false,
            poll_interval,
            last_probe: None,
        }
    }

    /// 返回当前配置的轮询间隔。
    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// 根据窗口矩形、最小化/可见/虚拟桌面隐藏状态（`cloaked`）与 Z 序上方不透明矩形集合更新遮挡状态。
    pub fn evaluate_occlusion_with_cloaked(
        &mut self,
        window_rect: Rect,
        minimized: bool,
        visible: bool,
        focused: bool,
        cloaked: bool,
        z_above_opaque_rects: &[Rect],
    ) -> RenderThrottleState {
        self.cloaked = cloaked;
        if cloaked {
            self.minimized = minimized;
            self.visible = visible;
            self.focused = focused;
            self.occluded = true;
            return RenderThrottleState::Suspended;
        }
        self.evaluate_occlusion(
            window_rect,
            minimized,
            visible,
            focused,
            z_above_opaque_rects,
        )
    }

    /// 按设定间隔轮询指定原生 `HWND` 的遮挡状态；仅在状态发生变化时返回 `Some(occluded)`。
    ///
    /// 非 Windows 平台安全降级为无操作（保持未遮挡）。
    pub fn poll_hwnd(&mut self, hwnd: isize, now: Instant) -> Option<bool> {
        if let Some(last) = self.last_probe {
            if now.saturating_duration_since(last) < self.poll_interval {
                return None;
            }
        }
        self.last_probe = Some(now);
        let next = probe_hwnd_occluded(hwnd);
        if next != self.occluded {
            self.occluded = next;
            Some(next)
        } else {
            None
        }
    }

    /// 返回最近一次评估中窗口是否处于虚拟桌面隐藏（`DWMWA_CLOAKED`）状态。
    #[must_use]
    pub const fn is_cloaked(&self) -> bool {
        self.cloaked
    }

    /// 根据窗口矩形与其Z序上方不透明窗口矩形列表更新遮挡状态。
    pub fn evaluate_occlusion(
        &mut self,
        window_rect: Rect,
        minimized: bool,
        visible: bool,
        focused: bool,
        z_above_opaque_rects: &[Rect],
    ) -> RenderThrottleState {
        self.minimized = minimized;
        self.visible = visible;
        self.focused = focused;
        if minimized || !visible || window_rect.is_empty() {
            self.occluded = true;
            return RenderThrottleState::Suspended;
        }
        self.occluded = is_rect_fully_covered(window_rect, z_above_opaque_rects);
        if self.occluded {
            RenderThrottleState::Suspended
        } else {
            // 遵循 ADR-0030 可见性驱动节流：窗口未最小化、未隐藏且未被完全遮挡时，
            // 即使失焦也维持 Active（是否显式启用失焦降帧由 FramePacer 策略控制）。
            RenderThrottleState::Active
        }
    }

    /// 直接同步遮挡标记（供平台事件状态机更新）。
    pub fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    /// 直接同步最小化标记（供平台事件状态机更新）。
    pub fn set_minimized(&mut self, minimized: bool) {
        self.minimized = minimized;
    }

    /// 直接同步可见性标记（供平台事件状态机更新）。
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// 直接同步焦点标记（供平台事件状态机更新）。
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    #[must_use]
    pub const fn is_occluded(&self) -> bool {
        self.occluded
    }

    #[must_use]
    pub const fn is_minimized(&self) -> bool {
        self.minimized
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }
}

/// 纯几何并集覆盖算法：判断目标矩形 `target` 是否被 `covers` 中所有矩形的并集 100% 覆盖。
#[must_use]
pub fn rects_cover(target: Rect, covers: &[Rect]) -> bool {
    is_rect_fully_covered(target, covers)
}

/// 探测指定原生窗口句柄（`HWND`）是否处于不可见、最小化、`DWMWA_CLOAKED`（虚拟桌面隐藏），
/// 或被 Z 序上方至多 64 层不透明顶层窗口的几何并集完全覆盖。
///
/// 非 Windows 平台安全降级返回 `false`。
#[must_use]
pub fn probe_hwnd_occluded(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::{HWND, RECT};
        use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        use ::windows::Win32::UI::WindowsAndMessaging::{
            GetWindow, GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible, GWL_EXSTYLE,
            GW_HWNDPREV, WS_EX_LAYERED, WS_EX_TRANSPARENT,
        };
        use core::ffi::c_void;

        if hwnd == 0 {
            return false;
        }
        let hwnd = HWND(hwnd as *mut c_void);
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return true;
            }
            // 检测目标窗口自身是否被 DWM 隐藏（例如位于其他虚拟桌面）。
            let mut target_cloaked: u32 = 0;
            let target_cloaked_ok = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&raw mut target_cloaked).cast::<c_void>(),
                core::mem::size_of::<u32>() as u32,
            );
            if target_cloaked_ok.is_ok() && target_cloaked != 0 {
                return true;
            }
            let mut target_rc = RECT::default();
            if GetWindowRect(hwnd, &mut target_rc).is_err() {
                return false;
            }
            let target = Rect::new(
                target_rc.left as f32,
                target_rc.top as f32,
                (target_rc.right - target_rc.left).max(0) as f32,
                (target_rc.bottom - target_rc.top).max(0) as f32,
            );
            if target.is_empty() {
                return true;
            }
            let mut opaque_covers = Vec::new();
            let mut cursor = hwnd;
            for _ in 0..MAX_OCCLUSION_Z_ORDER_LAYERS {
                let Ok(prev) = GetWindow(cursor, GW_HWNDPREV) else {
                    break;
                };
                if prev.0.is_null() {
                    break;
                }
                cursor = prev;
                if !IsWindowVisible(prev).as_bool() || IsIconic(prev).as_bool() {
                    continue;
                }
                let ex_style = GetWindowLongW(prev, GWL_EXSTYLE) as u32;
                if (ex_style & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0)) != 0 {
                    continue;
                }
                let mut cloaked: u32 = 0;
                let cloaked_ok = DwmGetWindowAttribute(
                    prev,
                    DWMWA_CLOAKED,
                    (&raw mut cloaked).cast::<c_void>(),
                    core::mem::size_of::<u32>() as u32,
                );
                if cloaked_ok.is_ok() && cloaked != 0 {
                    continue;
                }
                let mut rc = RECT::default();
                if GetWindowRect(prev, &mut rc).is_ok() {
                    let cover = Rect::new(
                        rc.left as f32,
                        rc.top as f32,
                        (rc.right - rc.left).max(0) as f32,
                        (rc.bottom - rc.top).max(0) as f32,
                    );
                    if !cover.is_empty() {
                        opaque_covers.push(cover);
                    }
                }
            }
            rects_cover(target, &opaque_covers)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

/// 递归判断矩形 `target` 是否被 `covers` 中的矩形并集 100% 覆盖。
#[must_use]
pub fn is_rect_fully_covered(target: Rect, covers: &[Rect]) -> bool {
    if target.is_empty() {
        return true;
    }
    for (idx, cover) in covers.iter().enumerate() {
        let clip = target.intersect(cover);
        if clip.is_empty() {
            continue;
        }
        if clip == target {
            return true;
        }
        let rest = &covers[idx + 1..];
        // 将 target 沿 clip 切分为最多 4 个不重叠剩余子矩形（上、下、左、右）
        let top = Rect::new(
            target.origin.x,
            target.origin.y,
            target.size.x,
            clip.origin.y - target.origin.y,
        );
        let bottom = Rect::new(
            target.origin.x,
            clip.max().y,
            target.size.x,
            target.max().y - clip.max().y,
        );
        let left = Rect::new(
            target.origin.x,
            clip.origin.y,
            clip.origin.x - target.origin.x,
            clip.size.y,
        );
        let right = Rect::new(
            clip.max().x,
            clip.origin.y,
            target.max().x - clip.max().x,
            clip.size.y,
        );
        return is_rect_fully_covered(top, rest)
            && is_rect_fully_covered(bottom, rest)
            && is_rect_fully_covered(left, rest)
            && is_rect_fully_covered(right, rest);
    }
    false
}

/// 屏幕防休眠守卫：通过引用计数追踪耗时任务（如下游资产应用批量缩略图烘焙或下游渲染工具长帧回放），
/// 在计数 > 0 时维持 `ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED`。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenSleepInhibitor {
    active_leases: u32,
}

impl ScreenSleepInhibitor {
    #[must_use]
    pub const fn new() -> Self {
        Self { active_leases: 0 }
    }

    /// 申请防休眠租约；返回是否从空闲态首次进入防休眠激活态。
    pub fn acquire(&mut self) -> bool {
        let was_zero = self.active_leases == 0;
        self.active_leases = self.active_leases.saturating_add(1);
        was_zero
    }

    /// 释放防休眠租约；返回是否所有租约均已归零（可恢复系统默认电源策略）。
    pub fn release(&mut self) -> bool {
        if self.active_leases == 0 {
            return false;
        }
        self.active_leases -= 1;
        self.active_leases == 0
    }

    /// 当前活跃的防休眠租约数量。
    #[must_use]
    pub const fn active_count(&self) -> u32 {
        self.active_leases
    }

    /// 当前是否要求保持屏幕常亮。
    #[must_use]
    pub const fn requires_display_on(&self) -> bool {
        self.active_leases > 0
    }
}

/// OLE / `CF_HDROP` 系统文件拖放操作类型。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FileDragEffect {
    #[default]
    None,
    Copy,
    Move,
    Link,
}

/// 跨窗口/外部程序文件拖入与拖出桥接器（含无头测试替身）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDragDropBridge {
    hovering_files: Vec<PathBuf>,
    hover_pos: Vec2,
    outgoing_drag_files: Vec<PathBuf>,
    effect: FileDragEffect,
}

impl FileDragDropBridge {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 外部文件进入窗口客户区（`IDropTarget::DragEnter`）。
    pub fn on_drag_enter(&mut self, files: Vec<PathBuf>, pos: Vec2) -> FileDragEffect {
        self.hovering_files = files;
        self.hover_pos = pos;
        self.effect = if self.hovering_files.is_empty() {
            FileDragEffect::None
        } else {
            FileDragEffect::Copy
        };
        self.effect
    }

    /// 外部文件在窗口上方移动（`IDropTarget::DragOver`）。
    pub fn on_drag_over(&mut self, pos: Vec2) -> FileDragEffect {
        self.hover_pos = pos;
        self.effect
    }

    /// 外部文件离开窗口（`IDropTarget::DragLeave`）。
    pub fn on_drag_leave(&mut self) {
        self.hovering_files.clear();
        self.effect = FileDragEffect::None;
    }

    /// 外部文件在窗口内释放（`IDropTarget::Drop`），返回落下的文件路径列表。
    pub fn on_drop(&mut self, pos: Vec2) -> Vec<PathBuf> {
        self.hover_pos = pos;
        self.effect = FileDragEffect::None;
        core::mem::take(&mut self.hovering_files)
    }

    /// 发起从应用内部（如下游资产应用资产网格）向外部 DCC 软件拖出文件的操作（`DoDragDrop`）。
    pub fn begin_external_file_drag(&mut self, files: Vec<PathBuf>) -> bool {
        if files.is_empty() {
            return false;
        }
        self.outgoing_drag_files = files;
        true
    }

    /// 结束向外部拖出文件操作。
    pub fn finish_external_file_drag(&mut self) -> Vec<PathBuf> {
        core::mem::take(&mut self.outgoing_drag_files)
    }

    /// 当前悬停的文件列表。
    #[must_use]
    pub fn hovering_files(&self) -> &[PathBuf] {
        &self.hovering_files
    }

    /// 当前鼠标悬停位置。
    #[must_use]
    pub const fn hover_pos(&self) -> Vec2 {
        self.hover_pos
    }
}

/// 多窗口共享设备编排池：在单 `VkDevice` + 共享纹理/字体图集下管理主窗口与多个独立浮窗。
#[derive(Clone, Debug)]
pub struct MultiWindowHostPool<C> {
    registry: WindowRegistry<(OverlayWindowSpec, OcclusionMonitor, C)>,
    next_id: u64,
}

impl<C> Default for MultiWindowHostPool<C> {
    fn default() -> Self {
        Self {
            registry: WindowRegistry::new(),
            next_id: 1,
        }
    }
}

impl<C> MultiWindowHostPool<C> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 生成并注册一个新窗口槽位，返回分配的 [`WindowId`]。
    pub fn spawn_window(&mut self, spec: OverlayWindowSpec, context: C) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.registry
            .insert(id, (spec, OcclusionMonitor::new(), context));
        id
    }

    /// 关闭指定窗口并返回其上下文以便回收其 Swapchain。
    pub fn close_window(&mut self, id: WindowId) -> Option<C> {
        self.registry.remove(id).map(|(_, _, ctx)| ctx)
    }

    /// 当前存活窗口数。
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.registry.len()
    }

    /// 主窗口 ID。
    #[must_use]
    pub fn primary_window(&self) -> Option<WindowId> {
        self.registry.primary()
    }

    /// 访问指定窗口的规格、遮挡监控与上下文。
    #[must_use]
    pub fn get(&self, id: WindowId) -> Option<&(OverlayWindowSpec, OcclusionMonitor, C)> {
        self.registry.get(id)
    }

    /// 可变访问指定窗口的规格、遮挡监控与上下文。
    pub fn get_mut(
        &mut self,
        id: WindowId,
    ) -> Option<&mut (OverlayWindowSpec, OcclusionMonitor, C)> {
        self.registry.get_mut(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_policy_switches_between_capture_and_passthrough() {
        let mut policy = HitTestPassThroughPolicy::new();
        policy.set_interactive_rects(vec![
            Rect::new(20.0, 20.0, 200.0, 48.0),
            Rect::new(400.0, 100.0, 300.0, 240.0),
        ]);
        assert_eq!(
            policy.evaluate(Vec2::new(50.0, 35.0)),
            OverlayHitDisposition::CaptureClient
        );
        assert_eq!(
            policy.evaluate(Vec2::new(260.0, 35.0)),
            OverlayHitDisposition::PassThrough
        );

        policy.set_force_pass_through(true);
        assert_eq!(
            policy.evaluate(Vec2::new(50.0, 35.0)),
            OverlayHitDisposition::PassThrough
        );
    }

    #[test]
    fn occlusion_monitor_detects_single_and_split_rect_coverage() {
        let win = Rect::new(100.0, 100.0, 400.0, 300.0);
        let mut monitor = OcclusionMonitor::new();

        // 两个相邻窗口拼合完全覆盖目标窗口
        let left_cover = Rect::new(80.0, 80.0, 220.0, 350.0);
        let right_cover = Rect::new(300.0, 80.0, 220.0, 350.0);
        let state = monitor.evaluate_occlusion(win, false, true, false, &[left_cover, right_cover]);
        assert_eq!(state, RenderThrottleState::PausedZeroCost);
        assert!(monitor.is_occluded());

        // 留出一条 10px 缝隙，则未完全遮挡；遵循 ADR-0030，失焦但仍可见保持 Active。
        let partial_right = Rect::new(310.0, 80.0, 220.0, 350.0);
        let state2 =
            monitor.evaluate_occlusion(win, false, true, false, &[left_cover, partial_right]);
        assert_eq!(state2, RenderThrottleState::Active);
        assert!(!monitor.is_occluded());

        // 最小化或隐藏时同样进入 Suspended 停绘。
        assert_eq!(
            monitor.evaluate_occlusion(win, true, true, true, &[]),
            RenderThrottleState::Suspended
        );
        assert_eq!(
            monitor.evaluate_occlusion(win, false, false, true, &[]),
            RenderThrottleState::Suspended
        );
    }

    #[test]
    fn sleep_inhibitor_ref_counts_leases() {
        let mut guard = ScreenSleepInhibitor::new();
        assert!(!guard.requires_display_on());
        assert!(guard.acquire());
        assert!(!guard.acquire());
        assert_eq!(guard.active_count(), 2);
        assert!(guard.requires_display_on());
        assert!(!guard.release());
        assert!(guard.release());
        assert!(!guard.requires_display_on());
    }

    #[test]
    fn file_drag_drop_bridge_handles_incoming_and_outgoing_drags() {
        let mut bridge = FileDragDropBridge::new();
        let files = vec![PathBuf::from("hero.fbx"), PathBuf::from("albedo.ktx2")];
        assert_eq!(
            bridge.on_drag_enter(files.clone(), Vec2::new(120.0, 80.0)),
            FileDragEffect::Copy
        );
        assert_eq!(bridge.hovering_files().len(), 2);
        let dropped = bridge.on_drop(Vec2::new(140.0, 90.0));
        assert_eq!(dropped, files);
        assert!(bridge.hovering_files().is_empty());

        assert!(bridge.begin_external_file_drag(vec![PathBuf::from("export.glb")]));
        assert_eq!(
            bridge.finish_external_file_drag(),
            vec![PathBuf::from("export.glb")]
        );
    }

    #[test]
    fn multi_window_host_pool_manages_primary_and_utility_windows() {
        let mut pool = MultiWindowHostPool::new();
        let main_id = pool.spawn_window(
            OverlayWindowSpec::utility_floating("Launcher", 1280, 800),
            "main_swapchain",
        );
        let tasks_id = pool.spawn_window(
            OverlayWindowSpec::utility_floating("Tasks", 480, 360),
            "tasks_swapchain",
        );
        assert_eq!(pool.window_count(), 2);
        assert_eq!(pool.primary_window(), Some(main_id));
        assert_eq!(pool.close_window(main_id), Some("main_swapchain"));
        assert_eq!(pool.primary_window(), Some(tasks_id));
    }

    #[test]
    fn occlusion_monitor_handles_rects_cover_cloaked_and_poll_interval() {
        let target = Rect::new(0.0, 0.0, 100.0, 100.0);
        // 两个半屏矩形并集完全覆盖 target。
        assert!(rects_cover(
            target,
            &[
                Rect::new(0.0, 0.0, 50.0, 100.0),
                Rect::new(50.0, 0.0, 50.0, 100.0),
            ]
        ));
        // 留下 1px 缝隙时不算完全覆盖。
        assert!(!rects_cover(
            target,
            &[
                Rect::new(0.0, 0.0, 49.0, 100.0),
                Rect::new(50.0, 0.0, 50.0, 100.0),
            ]
        ));

        let mut monitor = OcclusionMonitor::with_poll_interval(Duration::from_millis(500));
        assert_eq!(monitor.poll_interval(), DEFAULT_OCCLUSION_POLL_INTERVAL);

        // 虚拟桌面隐藏（DWMWA_CLOAKED）直接进入挂起态。
        assert_eq!(
            monitor.evaluate_occlusion_with_cloaked(target, false, true, true, true, &[]),
            RenderThrottleState::Suspended
        );
        assert!(monitor.is_occluded());
        assert!(monitor.is_cloaked());

        // 取消 cloaked 且无遮挡时恢复 Active。
        assert_eq!(
            monitor.evaluate_occlusion_with_cloaked(target, false, true, true, false, &[]),
            RenderThrottleState::Active
        );
        assert!(!monitor.is_occluded());
        assert!(!monitor.is_cloaked());

        // 非 Windows 平台（或无效 HWND 0）轮询安全降级为无变化且遵守节流间隔。
        let t0 = Instant::now();
        assert_eq!(monitor.poll_hwnd(0, t0), None);
        assert_eq!(monitor.poll_hwnd(0, t0 + Duration::from_millis(100)), None);
        assert_eq!(monitor.poll_hwnd(0, t0 + Duration::from_millis(600)), None);
        assert!(!probe_hwnd_occluded(0));
    }
}

/// 计算弹出菜单在屏幕坐标系中的安全放置矩形（优先锚点右下，越界时自动翻转并钳制到显示器工作区，ADR-0022）。
#[must_use]
pub fn place_popup_in_work_area(anchor_screen: Vec2, popup_size: Vec2, work_area: Rect) -> Rect {
    let w = popup_size.x.max(1.0).min(work_area.width().max(1.0));
    let h = popup_size.y.max(1.0).min(work_area.height().max(1.0));

    let mut x = anchor_screen.x;
    if x + w > work_area.max().x {
        x = anchor_screen.x - w;
    }
    x = x.clamp(
        work_area.origin.x,
        (work_area.max().x - w).max(work_area.origin.x),
    );

    let mut y = anchor_screen.y;
    if y + h > work_area.max().y {
        y = anchor_screen.y - h;
    }
    y = y.clamp(
        work_area.origin.y,
        (work_area.max().y - h).max(work_area.origin.y),
    );

    Rect::new(x, y, w, h)
}

/// 判断给定客户区坐标的弹出菜单是否越出宿主窗口边界，从而需要路由至独立无焦点 Popup 宿主（ADR-0022）。
#[must_use]
pub fn should_use_popup_host(popup_in_client: Rect, host_client_size: Vec2) -> bool {
    popup_in_client.origin.x < 0.0
        || popup_in_client.origin.y < 0.0
        || popup_in_client.max().x > host_client_size.x
        || popup_in_client.max().y > host_client_size.y
}

#[cfg(test)]
mod popup_placement_tests {
    use super::*;

    #[test]
    fn popup_flips_when_exceeding_bottom_right_and_handles_negative_monitors() {
        let work = Rect::new(-1920.0, 0.0, 1920.0, 1040.0);
        let placed =
            place_popup_in_work_area(Vec2::new(-50.0, 1000.0), Vec2::new(220.0, 180.0), work);
        assert!(placed.origin.x >= work.origin.x);
        assert!(placed.max().x <= work.max().x);
        assert!(placed.origin.y >= work.origin.y);
        assert!(placed.max().y <= work.max().y);
        assert!((placed.max().x - (-50.0)).abs() < 1e-4);
        assert!((placed.max().y - 1000.0).abs() < 1e-4);
    }

    #[test]
    fn overflow_detection_routes_only_clipping_menus_to_popup_host() {
        assert!(!should_use_popup_host(
            Rect::new(10.0, 10.0, 200.0, 150.0),
            Vec2::new(800.0, 600.0)
        ));
        assert!(should_use_popup_host(
            Rect::new(700.0, 10.0, 200.0, 150.0),
            Vec2::new(800.0, 600.0)
        ));
    }
}

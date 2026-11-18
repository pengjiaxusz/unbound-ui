//! 透明穿透覆盖层宿主、多窗口编排、遮挡零开销节流、OLE 文件拖拽桥接与防休眠守卫（阶段 1 WP2）。
//!
//! 面向两大宿主的核心平台能力：
//! * **`chahu-render-debugger`**：需要替代 Tauri 透明覆盖层宿主（`WS_EX_LAYERED` /
//!   `WS_EX_TRANSPARENT` / `WS_EX_NOACTIVATE`、按 UI 矩形动态切换鼠标穿透与拦截、
//!   紧跟被调试目标进程 HWND 的坐标与最小化联动），以及内置浮窗（Tasks / Performance /
//!   Settings）与主窗口在单 `VkDevice` 下的多窗口生命周期编排；
//! * **`dunting`**：需要系统文件拖入与向外拖出（OLE `IDropTarget` / `IDropSource` / `CF_HDROP`
//!   策略层与内存测试替身）、窗口完全被遮挡时的 0% 渲染暂停，以及重度任务期间的屏幕防休眠守卫。

use std::path::PathBuf;

use unbound_math::{Rect, Vec2};

use crate::window::{WindowConfig, WindowPosition};
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
    /// 所属父窗口或被跟随的目标进程窗口 `HWND`。
    pub owner_hwnd: Option<isize>,
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
            owner_hwnd: None,
        }
    }
}

impl OverlayWindowSpec {
    /// 创建面向 `chahu-render-debugger` 调试覆盖层的预设规格。
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
            owner_hwnd: None,
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
                // 覆盖层走分层窗口的 `transparent` 开关，与交换链合成方式无关：
                // 把它改成预乘 alpha 会让覆盖层内容按另一套 alpha 语义参与合成。
                surface_alpha: crate::window::SurfaceAlpha::Opaque,
            },
            transparent: false,
            always_on_top: false,
            click_through: false,
            skip_taskbar: false,
            no_activate: false,
            owner_hwnd: None,
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

/// 目标进程窗口跟随状态机（用于 `chahu-render-debugger` 覆盖层与目标 HWND 同步位置、尺寸与最小化隐藏）。
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

/// 窗口遮挡与空闲节流状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderThrottleState {
    /// 窗口可见且处于前台，按显示器完整刷新率运行。
    FullSpeed,
    /// 窗口可见但非前台且无动画，降频至指定 FPS。
    ThrottledFps(u32),
    /// 窗口最小化、隐藏或被上方不透明窗口 100% 遮挡，暂停 Vulkan 渲染提交（0% GPU 开销）。
    PausedZeroCost,
}

/// 窗口遮挡检测器：判断目标窗口矩形是否被前方不透明窗口集合完全覆盖。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct OcclusionMonitor {
    minimized: bool,
    visible: bool,
    occluded: bool,
    focused: bool,
}

impl OcclusionMonitor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            minimized: false,
            visible: true,
            occluded: false,
            focused: true,
        }
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
            return RenderThrottleState::PausedZeroCost;
        }
        self.occluded = is_rect_fully_covered(window_rect, z_above_opaque_rects);
        if self.occluded {
            RenderThrottleState::PausedZeroCost
        } else if focused {
            RenderThrottleState::FullSpeed
        } else {
            RenderThrottleState::ThrottledFps(30)
        }
    }

    #[must_use]
    pub const fn is_occluded(&self) -> bool {
        self.occluded
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

/// 屏幕防休眠守卫：通过引用计数追踪耗时任务（如 `dunting` 批量缩略图烘焙或 `chahu-render-debugger` 长帧回放），
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

    /// 发起从应用内部（如 `dunting` 资产网格）向外部 DCC 软件拖出文件的操作（`DoDragDrop`）。
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

        // 留出一条 10px 缝隙，则未完全遮挡
        let partial_right = Rect::new(310.0, 80.0, 220.0, 350.0);
        let state2 =
            monitor.evaluate_occlusion(win, false, true, false, &[left_cover, partial_right]);
        assert_eq!(state2, RenderThrottleState::ThrottledFps(30));
        assert!(!monitor.is_occluded());
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
}

//! lang-allow: 中文注释与文档
//! 幽灵窗（`Ghost`）与悬浮置顶浮岛（`FloatingIsland`）的宿主状态机与规格构建器（ADR 0039）。
//!
//! 与 [`unbound_platform::PopupHostRegistry`] 严格分工：
//! * `popup_host.rs` 专管依附父窗的轻量弹出菜单、下拉框与 ToolTip；
//! * `floating_host.rs` 协同 [`crate::MultiWindowCoordinator`] 管理拥有独立
//!   `AppRuntime`、`Surface`、`Swapchain` 与 `Renderer` 的顶层无激活幽灵窗与可拖拽浮岛。

use rustc_hash::FxHashMap;
use unbound_math::Vec2;
use unbound_platform::{
    FloatingPlacementStrategy, FloatingWindowCapabilities, FloatingWindowPolicy,
    RelativeMonitorAnchor, ScreenRect, SurfaceAlpha, WindowBackdropMaterial, WindowConfig,
    WindowId, WindowPosition, WindowRole,
};

/// 基于平台能力矩阵计算实际生效的浮窗策略。
///
/// 当平台（如无 `wlr-layer-shell` 的 Wayland 合成器）不支持某项能力时，
/// 对应策略位将确定性降级为 `false`，确保应用层可精确观测实际行为。
#[must_use]
pub fn effective_policy(
    policy: FloatingWindowPolicy,
    caps: FloatingWindowCapabilities,
) -> FloatingWindowPolicy {
    FloatingWindowPolicy {
        no_activate: policy.no_activate && caps.supports_no_activate,
        always_on_top: policy.always_on_top && caps.supports_topmost,
        skip_taskbar: policy.skip_taskbar && caps.supports_skip_taskbar,
        click_through: policy.click_through && caps.supports_click_through,
        draggable: policy.draggable,
    }
}

/// 单个幽灵窗或悬浮浮岛的运行时宿主状态快照。
#[derive(Clone, Debug, PartialEq)]
pub struct FloatingHostState {
    /// 窗口唯一标识。
    pub id: WindowId,
    /// 窗口角色（`Ghost` 或 `FloatingIsland`）。
    pub role: WindowRole,
    /// 应用侧请求的原始策略。
    pub requested_policy: FloatingWindowPolicy,
    /// 结合当前平台能力裁剪后实际生效的策略。
    pub effective_policy: FloatingWindowPolicy,
    /// 当前记录的窗口外框屏幕物理矩形。
    pub outer_rect: ScreenRect,
    /// 当前窗口是否可见。
    pub visible: bool,
    /// 窗口实例代次：仅在窗口被销毁重建时递增，运行期热切换保持不变。
    pub generation: u64,
    /// 最近一次边缘磁吸后记录的相对显示器锚点（用于跨屏或工作区变化后恢复）。
    pub last_snap_anchor: Option<RelativeMonitorAnchor>,
}

/// Default physical pixel offset applied from the pointer to a drag proxy ghost window.
pub const DEFAULT_DRAG_PROXY_CURSOR_OFFSET: (i32, i32) = (12, 12);

/// Computes the physical screen rectangle for a cross-screen drag proxy ghost window.
#[must_use]
pub fn compute_drag_proxy_rect(
    screen_physical: Vec2,
    size: (u32, u32),
    cursor_offset: (i32, i32),
) -> ScreenRect {
    let x = screen_physical.x.round() as i32 + cursor_offset.0;
    let y = screen_physical.y.round() as i32 + cursor_offset.1;
    ScreenRect::new(x, y, size.0.max(1), size.1.max(1))
}

/// Runtime lifecycle state of the cross-screen ghost window used to render a tear-out drag proxy.
#[derive(Clone, Debug, PartialEq)]
pub struct DragProxyGhostState {
    /// Platform window identifier bound once the ghost window is created.
    pub window_id: Option<WindowId>,
    /// Source window that owns the active cross-window drag session.
    pub source_window: WindowId,
    /// Visible title rendered inside the drag proxy ghost window.
    pub title: String,
    /// Current physical screen outer rectangle of the drag proxy ghost window.
    pub outer_rect: ScreenRect,
    /// Whether the drag proxy ghost window is currently visible.
    pub visible: bool,
    /// Whether a native ghost window creation request is currently in flight.
    pub spawn_pending: bool,
}

/// 管理所有已注册浮窗状态与平台能力的宿主控制器。
#[derive(Clone, Debug)]
pub struct FloatingHost {
    entries: FxHashMap<WindowId, FloatingHostState>,
    capabilities: FloatingWindowCapabilities,
    drag_proxy: Option<DragProxyGhostState>,
}

impl Default for FloatingHost {
    fn default() -> Self {
        Self::with_current_platform()
    }
}

impl FloatingHost {
    /// 使用显式指定的平台能力矩阵创建浮窗宿主。
    #[must_use]
    pub fn new(capabilities: FloatingWindowCapabilities) -> Self {
        Self {
            entries: FxHashMap::default(),
            capabilities,
            drag_proxy: None,
        }
    }

    /// 使用当前运行平台的默认能力矩阵创建浮窗宿主。
    #[must_use]
    pub fn with_current_platform() -> Self {
        Self::new(FloatingWindowCapabilities::current_platform())
    }

    /// 获取当前宿主持有的平台能力矩阵。
    #[must_use]
    pub const fn capabilities(&self) -> FloatingWindowCapabilities {
        self.capabilities
    }

    /// 更新平台能力矩阵（通常用于无头跨平台降级测试），并同步重算全部已注册浮窗的生效策略。
    pub fn set_capabilities(&mut self, capabilities: FloatingWindowCapabilities) {
        self.capabilities = capabilities;
        for state in self.entries.values_mut() {
            state.effective_policy = effective_policy(state.requested_policy, capabilities);
        }
    }

    /// 注册一个幽灵窗或悬浮浮岛；若同 `id` 曾存在则视为重建并递增 `generation`。
    pub fn register(
        &mut self,
        id: WindowId,
        role: WindowRole,
        policy: FloatingWindowPolicy,
        outer_rect: ScreenRect,
        visible: bool,
    ) -> &FloatingHostState {
        let next_generation = self
            .entries
            .get(&id)
            .map_or(1, |prev| prev.generation.saturating_add(1));
        let effective = effective_policy(policy, self.capabilities);
        let state = FloatingHostState {
            id,
            role,
            requested_policy: policy,
            effective_policy: effective,
            outer_rect,
            visible,
            generation: next_generation,
            last_snap_anchor: None,
        };
        self.entries.insert(id, state);
        self.entries.get(&id).expect("刚插入的浮窗状态条目必然存在")
    }

    /// 注销并移除指定浮窗的状态记录。
    pub fn unregister(&mut self, id: WindowId) -> Option<FloatingHostState> {
        if self
            .drag_proxy
            .as_ref()
            .is_some_and(|proxy| proxy.window_id == Some(id))
        {
            self.drag_proxy = None;
        }
        self.entries.remove(&id)
    }

    /// 查询指定浮窗的只读状态。
    #[must_use]
    pub fn get(&self, id: WindowId) -> Option<&FloatingHostState> {
        self.entries.get(&id)
    }

    /// 查询指定浮窗的可变状态。
    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut FloatingHostState> {
        self.entries.get_mut(&id)
    }

    /// 判断给定窗口是否由浮窗宿主托管。
    #[must_use]
    pub fn is_floating(&self, id: WindowId) -> bool {
        self.entries.contains_key(&id)
    }

    /// 当前托管的浮窗数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何托管中的浮窗。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 遍历全部托管中的浮窗状态。
    pub fn iter(&self) -> impl Iterator<Item = &FloatingHostState> {
        self.entries.values()
    }

    /// 返回当前全部托管中的浮窗 ID 列表。
    #[must_use]
    pub fn ids(&self) -> Vec<WindowId> {
        self.entries.keys().copied().collect()
    }

    /// 运行期热更新完整策略，不重建窗口（`generation` 保持不变）。
    pub fn update_policy(
        &mut self,
        id: WindowId,
        policy: FloatingWindowPolicy,
    ) -> Option<FloatingWindowPolicy> {
        let state = self.entries.get_mut(&id)?;
        state.requested_policy = policy;
        state.effective_policy = effective_policy(policy, self.capabilities);
        Some(state.effective_policy)
    }

    /// 运行期热切换可见性，不重建窗口（`generation` 保持不变）。
    pub fn set_visible(&mut self, id: WindowId, visible: bool) -> bool {
        let Some(state) = self.entries.get_mut(&id) else {
            return false;
        };
        state.visible = visible;
        true
    }

    /// 运行期热切换置顶状态，返回经平台能力裁剪后的实际生效值；`generation` 保持不变。
    pub fn set_always_on_top(&mut self, id: WindowId, topmost: bool) -> Option<bool> {
        let state = self.entries.get_mut(&id)?;
        state.requested_policy.always_on_top = topmost;
        state.effective_policy = effective_policy(state.requested_policy, self.capabilities);
        Some(state.effective_policy.always_on_top)
    }

    /// 运行期热切换鼠标穿透状态，返回经平台能力裁剪后的实际生效值；`generation` 保持不变。
    pub fn set_click_through(&mut self, id: WindowId, click_through: bool) -> Option<bool> {
        let state = self.entries.get_mut(&id)?;
        state.requested_policy.click_through = click_through;
        state.effective_policy = effective_policy(state.requested_policy, self.capabilities);
        Some(state.effective_policy.click_through)
    }

    /// 运行期热更新窗口外框矩形；若平台不支持全局定位则仅更新尺寸并保留原坐标。
    pub fn set_outer_rect(&mut self, id: WindowId, rect: ScreenRect) -> Option<ScreenRect> {
        let state = self.entries.get_mut(&id)?;
        if self.capabilities.supports_global_positioning {
            state.outer_rect = rect;
        } else {
            state.outer_rect.width = rect.width;
            state.outer_rect.height = rect.height;
        }
        Some(state.outer_rect)
    }

    /// 记录边缘磁吸结果与相对显示器锚点，不改变 `generation`。
    pub fn set_snap_anchor(
        &mut self,
        id: WindowId,
        rect: ScreenRect,
        anchor: Option<RelativeMonitorAnchor>,
    ) -> bool {
        let Some(state) = self.entries.get_mut(&id) else {
            return false;
        };
        if self.capabilities.supports_global_positioning {
            state.outer_rect = rect;
        } else {
            state.outer_rect.width = rect.width;
            state.outer_rect.height = rect.height;
        }
        state.last_snap_anchor = anchor;
        true
    }

    /// Returns the active cross-screen drag proxy ghost state, if any.
    #[must_use]
    pub fn drag_proxy(&self) -> Option<&DragProxyGhostState> {
        self.drag_proxy.as_ref()
    }

    /// Starts or refreshes the cross-screen drag proxy ghost lifecycle state.
    pub fn begin_drag_proxy(
        &mut self,
        source_window: WindowId,
        title: impl Into<String>,
        initial_rect: ScreenRect,
        visible: bool,
    ) -> &DragProxyGhostState {
        let existing_window = self.drag_proxy.as_ref().and_then(|p| p.window_id);
        let spawn_pending = existing_window.is_none();
        self.drag_proxy = Some(DragProxyGhostState {
            window_id: existing_window,
            source_window,
            title: title.into(),
            outer_rect: initial_rect,
            visible,
            spawn_pending,
        });
        if let Some(win_id) = existing_window {
            let _ = self.set_outer_rect(win_id, initial_rect);
            let _ = self.set_visible(win_id, visible);
        }
        self.drag_proxy
            .as_ref()
            .expect("drag proxy state was just inserted")
    }

    /// Binds a newly created ghost window identifier to the pending drag proxy state.
    pub fn bind_drag_proxy_window(&mut self, id: WindowId) -> bool {
        let Some(proxy) = self.drag_proxy.as_mut() else {
            return false;
        };
        if proxy.window_id.is_some() && proxy.window_id != Some(id) {
            return false;
        }
        proxy.window_id = Some(id);
        proxy.spawn_pending = false;
        let rect = proxy.outer_rect;
        let visible = proxy.visible;
        let _ = self.set_outer_rect(id, rect);
        let _ = self.set_visible(id, visible);
        true
    }

    /// Updates the cross-screen drag proxy rectangle and visibility without recreating the window.
    pub fn update_drag_proxy(
        &mut self,
        rect: ScreenRect,
        visible: bool,
    ) -> Option<(ScreenRect, bool, Option<WindowId>)> {
        let proxy = self.drag_proxy.as_mut()?;
        if self.capabilities.supports_global_positioning {
            proxy.outer_rect = rect;
        } else {
            proxy.outer_rect.width = rect.width;
            proxy.outer_rect.height = rect.height;
        }
        proxy.visible = visible;
        let applied_rect = proxy.outer_rect;
        let win_id = proxy.window_id;
        if let Some(id) = win_id {
            let _ = self.set_outer_rect(id, applied_rect);
            let _ = self.set_visible(id, visible);
        }
        Some((applied_rect, visible, win_id))
    }

    /// Ends the cross-screen drag proxy lifecycle and returns the bound ghost window id to close.
    pub fn end_drag_proxy(&mut self) -> Option<WindowId> {
        self.drag_proxy.take().and_then(|proxy| proxy.window_id)
    }
}

/// 幽灵窗与悬浮浮岛的声明式规格构建器。
#[derive(Clone, Debug, PartialEq)]
pub struct FloatingWindowSpec {
    /// 基础窗口配置。
    pub config: WindowConfig,
    /// 可选初始定位策略（创建前由协调器按显示器工作区求解）。
    pub placement: Option<FloatingPlacementStrategy>,
}

impl FloatingWindowSpec {
    /// 构造默认幽灵窗（`Ghost`）规格：无边框、预乘 Alpha、默认隐藏直至首帧就绪。
    #[must_use]
    pub fn ghost(title: impl Into<String>, width: u32, height: u32) -> Self {
        let mut config = WindowConfig::default()
            .with_title(title)
            .with_size(width, height)
            .ghost();
        config.decorations = false;
        config.resizable = false;
        config.surface_alpha = SurfaceAlpha::PreMultiplied;
        config.role = WindowRole::Ghost;
        config.floating_policy = FloatingWindowPolicy::ghost_default();
        config.visible = false;
        Self {
            config,
            placement: None,
        }
    }

    /// Builds a non-activating, click-through, always-on-top ghost window spec for cross-screen drag proxies.
    #[must_use]
    pub fn drag_proxy_ghost(
        title: impl Into<String>,
        width: u32,
        height: u32,
        screen_pos: (i32, i32),
    ) -> Self {
        let mut spec = Self::ghost(title, width.max(1), height.max(1));
        spec.config.role = WindowRole::Ghost;
        spec.config.floating_policy = FloatingWindowPolicy {
            no_activate: true,
            always_on_top: true,
            skip_taskbar: true,
            click_through: true,
            draggable: false,
        };
        spec.config.position = WindowPosition::Physical {
            x: screen_pos.0,
            y: screen_pos.1,
        };
        spec.config.visible = true;
        spec
    }

    /// 构造默认悬浮浮岛（`FloatingIsland`）规格：无边框、预乘 Alpha、置顶可拖拽且不抢焦点。
    #[must_use]
    pub fn floating_island(title: impl Into<String>, width: u32, height: u32) -> Self {
        let mut config = WindowConfig::default()
            .with_title(title)
            .with_size(width, height)
            .floating_island();
        config.decorations = false;
        config.surface_alpha = SurfaceAlpha::PreMultiplied;
        config.role = WindowRole::FloatingIsland;
        config.floating_policy = FloatingWindowPolicy::floating_island_default();
        config.visible = true;
        Self {
            config,
            placement: None,
        }
    }

    /// 附加初始定位策略。
    #[must_use]
    pub fn with_placement(mut self, placement: FloatingPlacementStrategy) -> Self {
        self.placement = Some(placement);
        self
    }

    /// 覆盖浮窗策略。
    #[must_use]
    pub fn with_policy(mut self, policy: FloatingWindowPolicy) -> Self {
        self.config.floating_policy = policy;
        self
    }

    /// 设置初始可见性。
    #[must_use]
    pub fn with_visible(mut self, visible: bool) -> Self {
        self.config.visible = visible;
        self
    }

    /// 设置是否允许用户调整大小。
    #[must_use]
    pub fn with_resizable(mut self, resizable: bool) -> Self {
        self.config.resizable = resizable;
        self
    }

    /// 设置窗口原生后景材质意图。
    #[must_use]
    pub fn with_backdrop(mut self, backdrop: Option<WindowBackdropMaterial>) -> Self {
        self.config.backdrop = backdrop;
        self
    }

    /// 拆分为底层 [`WindowConfig`] 与可选定位策略。
    #[must_use]
    pub fn build(self) -> (WindowConfig, Option<FloatingPlacementStrategy>) {
        (self.config, self.placement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_and_floating_island_specs_set_expected_defaults() {
        let ghost = FloatingWindowSpec::ghost("Preview Ghost", 360, 240);
        assert!(!ghost.config.decorations);
        assert_eq!(ghost.config.surface_alpha, SurfaceAlpha::PreMultiplied);
        assert_eq!(ghost.config.role, WindowRole::Ghost);
        assert!(!ghost.config.visible);
        assert!(ghost.config.floating_policy.no_activate);
        assert!(ghost.config.floating_policy.always_on_top);
        assert!(ghost.config.floating_policy.skip_taskbar);
        assert!(ghost.config.floating_policy.click_through);

        let island = FloatingWindowSpec::floating_island("Media Island", 320, 96);
        assert!(!island.config.decorations);
        assert_eq!(island.config.surface_alpha, SurfaceAlpha::PreMultiplied);
        assert_eq!(island.config.role, WindowRole::FloatingIsland);
        assert!(island.config.visible);
        assert!(island.config.floating_policy.no_activate);
        assert!(island.config.floating_policy.always_on_top);
        assert!(island.config.floating_policy.skip_taskbar);
        assert!(!island.config.floating_policy.click_through);
        assert!(island.config.floating_policy.draggable);
    }

    #[test]
    fn effective_policy_degrades_deterministically_across_platforms() {
        let requested = FloatingWindowPolicy::ghost_default();

        let win32 = effective_policy(requested, FloatingWindowCapabilities::win32_full());
        assert_eq!(win32, requested);

        let x11 = effective_policy(requested, FloatingWindowCapabilities::x11_default());
        assert!(x11.always_on_top);
        assert!(x11.skip_taskbar);
        assert!(x11.click_through);

        let wayland = effective_policy(
            requested,
            FloatingWindowCapabilities::wayland_xdg_fallback(),
        );
        assert!(!wayland.always_on_top);
        assert!(!wayland.no_activate);
        assert!(wayland.click_through);
    }

    #[test]
    fn hot_toggles_preserve_generation_while_re_register_increments_it() {
        let mut host = FloatingHost::new(FloatingWindowCapabilities::win32_full());
        let id = WindowId(42);
        let rect = ScreenRect::new(100, 120, 320, 180);

        let initial = host.register(
            id,
            WindowRole::FloatingIsland,
            FloatingWindowPolicy::floating_island_default(),
            rect,
            true,
        );
        assert_eq!(initial.generation, 1);

        assert_eq!(host.set_click_through(id, true), Some(true));
        assert_eq!(host.set_always_on_top(id, false), Some(false));
        assert!(host.set_visible(id, false));
        let moved = ScreenRect::new(240, 300, 320, 180);
        assert_eq!(host.set_outer_rect(id, moved), Some(moved));

        let after_hot_toggle = host.get(id).unwrap();
        assert_eq!(after_hot_toggle.generation, 1);
        assert!(after_hot_toggle.effective_policy.click_through);
        assert!(!after_hot_toggle.effective_policy.always_on_top);
        assert!(!after_hot_toggle.visible);
        assert_eq!(after_hot_toggle.outer_rect, moved);

        let recreated = host.register(
            id,
            WindowRole::FloatingIsland,
            FloatingWindowPolicy::floating_island_default(),
            rect,
            true,
        );
        assert_eq!(recreated.generation, 2);
    }

    #[test]
    fn wayland_fallback_ignores_global_position_changes_but_keeps_size() {
        let mut host = FloatingHost::new(FloatingWindowCapabilities::wayland_xdg_fallback());
        let id = WindowId(99);
        host.register(
            id,
            WindowRole::Ghost,
            FloatingWindowPolicy::ghost_default(),
            ScreenRect::new(0, 0, 300, 200),
            false,
        );

        let updated = host
            .set_outer_rect(id, ScreenRect::new(500, 400, 420, 260))
            .unwrap();
        assert_eq!(updated, ScreenRect::new(0, 0, 420, 260));
    }

    #[test]
    fn drag_proxy_ghost_spec_enforces_no_activate_click_through_topmost() {
        let spec = FloatingWindowSpec::drag_proxy_ghost("Tab Proxy", 220, 48, (-400, 120));
        assert!(!spec.config.decorations);
        assert!(!spec.config.resizable);
        assert_eq!(spec.config.surface_alpha, SurfaceAlpha::PreMultiplied);
        assert_eq!(spec.config.role, WindowRole::Ghost);
        assert!(spec.config.visible);
        assert!(spec.config.floating_policy.no_activate);
        assert!(spec.config.floating_policy.always_on_top);
        assert!(spec.config.floating_policy.skip_taskbar);
        assert!(spec.config.floating_policy.click_through);
        assert!(!spec.config.floating_policy.draggable);
        assert_eq!(
            spec.config.position,
            WindowPosition::Physical { x: -400, y: 120 }
        );
    }

    #[test]
    fn drag_proxy_lifecycle_tracks_position_visibility_and_cleanup() {
        let mut host = FloatingHost::new(FloatingWindowCapabilities::win32_full());
        let source = WindowId(1);
        let ghost = WindowId(77);
        let initial = compute_drag_proxy_rect(
            Vec2::new(640.0, 240.0),
            (196, 44),
            DEFAULT_DRAG_PROXY_CURSOR_OFFSET,
        );
        assert_eq!(initial, ScreenRect::new(652, 252, 196, 44));

        let started = host.begin_drag_proxy(source, "Inspector", initial, true);
        assert_eq!(started.source_window, source);
        assert!(started.spawn_pending);
        assert_eq!(started.window_id, None);

        host.register(
            ghost,
            WindowRole::Ghost,
            FloatingWindowPolicy::ghost_default(),
            initial,
            true,
        );
        assert!(host.bind_drag_proxy_window(ghost));
        assert!(!host.drag_proxy().unwrap().spawn_pending);
        assert_eq!(host.drag_proxy().unwrap().window_id, Some(ghost));

        let moved = ScreenRect::new(-320, 180, 196, 44);
        let updated = host.update_drag_proxy(moved, true).unwrap();
        assert_eq!(updated, (moved, true, Some(ghost)));
        assert_eq!(host.get(ghost).unwrap().outer_rect, moved);
        assert_eq!(host.get(ghost).unwrap().generation, 1);

        let hidden = host.update_drag_proxy(moved, false).unwrap();
        assert_eq!(hidden, (moved, false, Some(ghost)));
        assert!(!host.get(ghost).unwrap().visible);
        assert_eq!(host.get(ghost).unwrap().generation, 1);

        assert_eq!(host.end_drag_proxy(), Some(ghost));
        assert!(host.drag_proxy().is_none());
    }
}

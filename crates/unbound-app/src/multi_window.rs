//! 多窗口生命周期协调器与跨窗标签撕裂（tear-out）纯状态机（ADR-0023 / `GAP-UI-002`）。
//!
//! 基于 `unbound_platform::MultiWindowHostPool` 封装主/副窗口派生、聚焦、关闭与跨窗拖拽落点路由，
//! 保持纯 CPU 可无头测试，供后续 `MultiWindowRunner`（共享 `Arc<Gpu>` + 每窗独立 `Swapchain`）直接驱动。

use unbound_math::{Rect, Vec2};
use unbound_platform::{MultiWindowHostPool, OverlayWindowSpec, WindowId};

/// 应用层发往多窗口运行器的窗口管理指令。
#[derive(Clone, Debug, PartialEq)]
pub enum MultiWindowCommand {
    /// 派生一个新副窗口（共享同一 `VkDevice` 与图集）。
    SpawnWindow {
        /// 目标窗口规格。
        spec: OverlayWindowSpec,
        /// 业务侧自定义路由标签（如槽位转移 ID）。
        tag: String,
    },
    /// 关闭指定窗口（关闭副窗不影响主窗；关闭最后一个窗口触发应用退出）。
    CloseWindow(WindowId),
    /// 将焦点切换至指定窗口。
    FocusWindow(WindowId),
}

/// 跨窗拖拽投递事件载荷。
#[derive(Clone, Debug, PartialEq)]
pub struct CrossWindowDropEvent {
    /// 发起拖拽的源窗口标识。
    pub source_window: WindowId,
    /// 接收落点的目标窗口标识。
    pub target_window: WindowId,
    /// 释放时的屏幕绝对坐标。
    pub screen_pos: Vec2,
    /// 业务侧透传的拖拽载荷（例如序列化槽位 ID）。
    pub payload: String,
}

/// 活跃跨窗拖拽追踪态。
#[derive(Clone, Debug, PartialEq)]
pub struct CrossWindowDragState {
    /// 源窗口标识。
    pub source_window: WindowId,
    /// 业务载荷。
    pub payload: String,
    /// 当前屏幕绝对坐标。
    pub screen_pos: Vec2,
}

/// 单窗宿主元数据条目。
#[derive(Clone, Debug, PartialEq)]
pub struct WindowHostMeta {
    /// 业务路由标签。
    pub tag: String,
    /// 窗口当前在屏幕坐标系中的外框矩形。
    pub screen_rect: Rect,
}

/// 多窗口生命周期与跨窗拖拽协调器（纯状态机，可在无头环境完整回归）。
#[derive(Debug, Default)]
pub struct MultiWindowCoordinator {
    pool: MultiWindowHostPool<WindowHostMeta>,
    focused: Option<WindowId>,
    active_drag: Option<CrossWindowDragState>,
}

impl MultiWindowCoordinator {
    /// 创建空的多窗口协调器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册或派生一个窗口，返回分配的 [`WindowId`]。
    pub fn spawn_window(
        &mut self,
        spec: OverlayWindowSpec,
        tag: impl Into<String>,
        screen_rect: Rect,
    ) -> WindowId {
        let id = self.pool.spawn_window(
            spec,
            WindowHostMeta {
                tag: tag.into(),
                screen_rect,
            },
        );
        self.focused = Some(id);
        id
    }

    /// 更新指定窗口在屏幕坐标系中的矩形（窗口移动或缩放后调用）。
    pub fn set_window_rect(&mut self, id: WindowId, screen_rect: Rect) -> bool {
        if let Some((_, _, meta)) = self.pool.get_mut(id) {
            meta.screen_rect = screen_rect;
            true
        } else {
            false
        }
    }

    /// 关闭指定窗口；若全部窗口均已关闭返回 `true`（提示主循环退出）。
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let _ = self.pool.close_window(id);
        if self.focused == Some(id) {
            self.focused = self.pool.primary_window();
        }
        if self
            .active_drag
            .as_ref()
            .is_some_and(|d| d.source_window == id)
        {
            self.active_drag = None;
        }
        self.pool.window_count() == 0
    }

    /// 返回当前活跃窗口总数。
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.pool.window_count()
    }

    /// 返回当前聚焦窗口标识。
    #[must_use]
    pub const fn focused_window(&self) -> Option<WindowId> {
        self.focused
    }

    /// 启动一次跨窗拖拽（例如标签页拖出）。
    pub fn begin_cross_window_drag(
        &mut self,
        source_window: WindowId,
        payload: impl Into<String>,
        screen_pos: Vec2,
    ) {
        self.active_drag = Some(CrossWindowDragState {
            source_window,
            payload: payload.into(),
            screen_pos,
        });
    }

    /// 更新跨窗拖拽的屏幕坐标。
    pub fn update_cross_window_drag(&mut self, screen_pos: Vec2) {
        if let Some(drag) = &mut self.active_drag {
            drag.screen_pos = screen_pos;
        }
    }

    /// 结束跨窗拖拽；若释放点落在某已知窗口矩形内则产出 [`CrossWindowDropEvent`]。
    pub fn finish_cross_window_drag(
        &mut self,
        screen_pos: Vec2,
        target_window: Option<WindowId>,
    ) -> Option<CrossWindowDropEvent> {
        let drag = self.active_drag.take()?;
        let target = target_window?;
        self.pool.get(target)?;
        Some(CrossWindowDropEvent {
            source_window: drag.source_window,
            target_window: target,
            screen_pos,
            payload: drag.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_manages_primary_secondary_lifecycle_and_cross_window_drop() {
        let mut coord = MultiWindowCoordinator::new();
        let main_id = coord.spawn_window(
            OverlayWindowSpec::default(),
            "main",
            Rect::new(0.0, 0.0, 1280.0, 800.0),
        );
        let sub_id = coord.spawn_window(
            OverlayWindowSpec::utility_floating("Detached", 800, 600),
            "detached-tab-2",
            Rect::new(1300.0, 100.0, 800.0, 600.0),
        );
        assert_eq!(coord.window_count(), 2);
        assert_eq!(coord.focused_window(), Some(sub_id));

        // 模拟把标签从副窗拖回主窗
        coord.begin_cross_window_drag(sub_id, "slot:42", Vec2::new(1400.0, 200.0));
        coord.update_cross_window_drag(Vec2::new(400.0, 300.0));
        let drop_evt = coord
            .finish_cross_window_drag(Vec2::new(400.0, 300.0), Some(main_id))
            .expect("drop onto main window succeeds");
        assert_eq!(drop_evt.source_window, sub_id);
        assert_eq!(drop_evt.target_window, main_id);
        assert_eq!(drop_evt.payload, "slot:42");

        // 关闭副窗不退出主窗
        assert!(!coord.close_window(sub_id));
        assert_eq!(coord.window_count(), 1);
        assert_eq!(coord.focused_window(), Some(main_id));

        // 关闭最后一个主窗触发退出
        assert!(coord.close_window(main_id));
    }
}

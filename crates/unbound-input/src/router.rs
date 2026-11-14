//! 输入路由：命中测试 → 指针捕获 → 光标栈（`docs` §6.1、§6.2）。
//!
//! [`Router`] 负责将“系统通知指针移动”转化为“节点 42 正在被拖拽”。
//! 它统一持有三个必须保持一致的有状态组件：
//!
//! * 指针状态机（按下 / 拖拽 / 释放 / 双击）；
//! * 悬停目标，源自与按下相同的拾取结果；
//! * 光标栈，每帧根据拾取路径刷新。
//!
//!
//! 将三者集中于一处使光标具有**确定性**：它由决定点击目标的同一条
//! 有序拾取路径重新计算得出，因此缩放手柄在作为点击目标时绝不会错误显示
//! 工字光标。

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

use crate::cursor::{CursorKind, CursorSource, CursorStack};
use crate::hit::{pick, pick_stack, HitRegion, PORTAL_LAYER};
use crate::pointer::{PointerButton, PointerConfig, PointerEvent, PointerState};

/// 将指针输入路由到各命中区域。
#[derive(Clone, Debug)]
pub struct Router {
    regions: Vec<HitRegion>,
    pointer: PointerState,
    cursors: CursorStack,
    hover: Option<u64>,
    tool_cursor: CursorKind,
    /// 最近一次观测到的指针画布坐标。
    ///
    /// `PointerState::position` 仅在状态机收到事件时更新；平移、缩放或滚动
    /// 会使其过时。悬停与光标必须基于最新位置解析，因此每次观测——无论来自
    /// 系统事件还是相机驱动的 [`Router::refresh_at`]——都会记录在此处。
    ///
    ///
    /// `None` 表示指针尚未进入窗口：在状态机的原点占位值处拾取会凭空捏造
    /// 悬停目标。
    last_position: Option<Vec2>,
    /// 路由器最近一次收到的滚轮增量。
    ///
    /// 滚轮事件不会移动指针，因此增量需单独记录：依赖滚轮滚动的宿主
    /// 即使在指针静止时也需要知道滚轮转动了多少。
    ///
    last_wheel: Vec2,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(PointerConfig::default())
    }
}

impl Router {
    #[must_use]
    pub fn new(config: PointerConfig) -> Self {
        Self {
            regions: Vec::new(),
            pointer: PointerState::new(config),
            cursors: CursorStack::new(),
            hover: None,
            tool_cursor: CursorKind::Default,
            last_position: None,
            last_wheel: Vec2::ZERO,
        }
    }

    /// 替换本帧的可拾取区域集合。
    ///
    /// 内部向量会被复用，因此稳态帧无堆分配。由于新集合可能不再包含
    /// 原先位于指针下的节点，此处会重新拾取悬停目标与光标。
    ///
    pub fn set_regions(&mut self, regions: Vec<HitRegion>) {
        self.regions = regions;
        match self.last_position {
            Some(position) => self.refresh_at(position),
            None => self.refresh_cursor(),
        }
    }

    /// 当前已注册的可拾取区域列表。
    #[must_use]
    pub fn regions(&self) -> &[HitRegion] {
        &self.regions
    }

    /// 当前位于指针下方的节点（若有）。
    #[must_use]
    pub fn hover(&self) -> Option<u64> {
        self.hover
    }

    /// 基于当前帧解析出的待显示光标。
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        self.cursors.resolve()
    }

    /// 当前激活的画布工具所请求的光标。
    pub fn set_tool_cursor(&mut self, cursor: CursorKind) {
        self.tool_cursor = cursor;
        self.refresh_cursor();
    }

    /// 指针状态，供需要原始捕获/拖拽标志的手势使用。
    #[must_use]
    pub fn pointer(&self) -> &PointerState {
        &self.pointer
    }

    /// `point` 处最顶层的命中区域。
    #[must_use]
    pub fn pick(&self, point: Vec2) -> Option<&HitRegion> {
        pick(&self.regions, point)
    }

    /// `point` 处的所有命中区域，按从顶层到底层排序。
    ///
    /// 即“拾取路径”：首项是点击将命中的目标，其余项是其祖先节点。
    /// 滚轮事件会沿该路径查找最近的可滚动容器。
    ///
    #[must_use]
    pub fn pick_all(&self, point: Vec2) -> Vec<&HitRegion> {
        pick_stack(&self.regions, point)
    }

    /// 将滚轮事件路由到指针下方的区域。
    ///
    /// 返回的路径按从顶层到底层排序，宿主可沿其向上查找最近的可滚动祖先
    /// 并执行滚动。
    pub fn wheel(&mut self, delta: Vec2, point: Vec2) -> Vec<&HitRegion> {
        self.last_wheel = delta;
        self.last_position = Some(point);
        self.pointer.on_move(point);
        self.pick_all(point)
    }

    /// 路由器最近一次收到的滚轮增量。
    #[must_use]
    pub fn last_wheel(&self) -> Vec2 {
        self.last_wheel
    }

    /// 主键按下。
    ///
    /// 为不区分按键的宿主提供的便捷方法；需要支持上下文菜单或中键拖拽的
    /// 宿主应调用 [`Self::pointer_down_button`]。
    ///
    pub fn pointer_down(&mut self, position: Vec2, now: f64) -> Option<PointerEvent> {
        self.pointer_down_button(position, PointerButton::Left, now)
    }

    /// 任意按键按下。
    pub fn pointer_down_button(
        &mut self,
        position: Vec2,
        button: PointerButton,
        now: f64,
    ) -> Option<PointerEvent> {
        self.last_position = Some(position);
        let target = pick(&self.regions, position).map(|r| r.node);
        self.hover = target;
        let event = self.pointer.on_down(target, position, button, now);
        self.refresh_cursor();
        event
    }

    /// 指针移动。当存在活跃捕获时跳过命中测试——这正是捕获的意义所在。
    ///
    pub fn pointer_move(&mut self, position: Vec2) -> Option<PointerEvent> {
        self.last_position = Some(position);
        let event = self.pointer.on_move(position);
        if self.pointer.capture().is_none() {
            self.hover = pick(&self.regions, position).map(|r| r.node);
        }
        self.refresh_cursor();
        event
    }

    /// 主键释放。
    ///
    /// 若释放位置落在原按下区域之外，则不会产生点击，即使指针移动距离
    /// 尚未达到拖拽阈值亦然：当用户在其他元素上松手时，按钮不应被激活。
    ///
    ///
    pub fn pointer_up(&mut self, position: Vec2, now: f64) -> Vec<PointerEvent> {
        self.pointer_up_button(position, PointerButton::Left, now)
    }

    /// 任意按键释放。
    pub fn pointer_up_button(
        &mut self,
        position: Vec2,
        button: PointerButton,
        now: f64,
    ) -> Vec<PointerEvent> {
        self.last_position = Some(position);
        let pressed = self.pointer.pressed();
        let mut events = self.pointer.on_up(position, button, now);
        let released_on = pick(&self.regions, position).map(|r| r.node);
        self.hover = released_on;
        if pressed.is_some() {
            let clicked = events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. }));
            if released_on != pressed {
                events.retain(|event| {
                    !matches!(
                        event,
                        PointerEvent::Click { .. } | PointerEvent::DoubleClick { .. }
                    )
                });
                // 点击已被扣留，因此下一次按下应开启新的点击序列，
                // 而非与本次按下配对。
                self.pointer.reset_click_sequence();
            } else if !clicked {
                // 在原目标上结束的拖拽同样不派发点击；
                // 它绝不能与下一次按下配对构成双击。
                self.pointer.reset_click_sequence();
            }
        }
        self.refresh_cursor();
        events
    }

    /// 中止当前交互。
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        let event = self.pointer.cancel();
        match self.last_position {
            // 捕获期间悬停检测被冻结；现在指针可能位于任何位置，必须重新拾取。
            //
            Some(position) => self.refresh_at(position),
            None => self.refresh_cursor(),
        }
        event
    }

    /// 在不移动状态机的前提下，于 `position` 处重新拾取悬停目标与光标。
    ///
    /// 平移、缩放或场景重建会移动静止指针下方的几何体。命中区域位于画布空间，
    /// 因此必须在新的画布坐标处解析悬停与光标，而不能干等下一次系统光标事件。
    ///
    ///
    pub fn refresh_at(&mut self, position: Vec2) {
        self.last_position = Some(position);
        if self.pointer.capture().is_none() {
            self.hover = pick(&self.regions, position).map(|r| r.node);
        }
        self.refresh_cursor_at(position);
    }

    /// 根据当前拾取路径重新计算光标栈。
    fn refresh_cursor(&mut self) {
        self.refresh_cursor_at(self.last_position.unwrap_or(Vec2::ZERO));
    }

    /// 根据 `position` 处的拾取路径重新计算光标栈。
    ///
    /// 每个来源只遍历一趟，同来源内最顶层区域胜出。这使结果与区域顺序无关，
    /// 且无需临时向量，因此鼠标移动零分配。
    ///
    fn refresh_cursor_at(&mut self, position: Vec2) {
        self.cursors.clear();
        if self.pointer.capture().is_some() {
            let kind = if self.pointer.is_dragging() {
                CursorKind::Grabbing
            } else {
                CursorKind::Pointer
            };
            self.cursors.push(CursorSource::Capture, kind);
        }
        const SOURCES: [CursorSource; 5] = [
            CursorSource::Overlay,
            CursorSource::Handle,
            CursorSource::CanvasTool,
            CursorSource::Text,
            CursorSource::Default,
        ];
        for source in SOURCES {
            let mut best: Option<((u8, u32, u64), CursorKind)> = None;
            for region in &self.regions {
                let Some(kind) = region.cursor else { continue };
                let region_source = if region.layer >= PORTAL_LAYER {
                    CursorSource::Overlay
                } else {
                    kind.source()
                };
                if region_source != source || !region.hit(position) {
                    continue;
                }
                let rank = (region.layer, region.z, region.node);
                if best.is_none_or(|(current, _)| rank > current) {
                    best = Some((rank, kind));
                }
            }
            if let Some((_, kind)) = best {
                self.cursors.push(source, kind);
            }
        }
        // 仅当工具确实请求了光标时才占领工具槽位：
        // 在此处推入 `Default` 会压过普通悬停控件的光标。
        if self.tool_cursor != CursorKind::Default {
            self.cursors
                .push(CursorSource::CanvasTool, self.tool_cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit::Shape;
    use unbound_math::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn region(node: u64, bounds: Rect) -> HitRegion {
        HitRegion::new(node, 1, 0, bounds).with_cursor(CursorKind::Pointer)
    }

    /// 滚轮事件必须送达指针下方的区域。
    ///
    /// 若没有滚轮路由，输入层无法告知宿主滚轮在**何处**转动，
    /// 每个可滚动区域就只能自行猜测。
    #[test]
    fn wheel_events_are_routed_to_the_regions_under_the_pointer() {
        let mut router = Router::new(PointerConfig::default());
        router.set_regions(vec![
            region(1, rect(0.0, 0.0, 100.0, 100.0)),
            region(2, rect(0.0, 0.0, 50.0, 50.0)),
        ]);
        let path = router.wheel(Vec2::new(0.0, 40.0), Vec2::new(10.0, 10.0));
        assert_eq!(path.len(), 2, "both regions are under the pointer");
        // 最顶层区域排在最前：即点击会命中的区域。
        assert_eq!(path[0].node, 2);
        assert_eq!(path[1].node, 1);
        assert_eq!(router.last_wheel(), Vec2::new(0.0, 40.0));
        // 滚轮不会移动指针，但路由器会记住滚轮发生的位置。
        //
        assert_eq!(router.pointer().position(), Vec2::new(10.0, 10.0));
    }

    /// 在空白处滚动滚轮返回空路径。
    #[test]
    fn a_wheel_over_nothing_routes_nowhere() {
        let mut router = Router::new(PointerConfig::default());
        router.set_regions(vec![region(1, rect(0.0, 0.0, 10.0, 10.0))]);
        let path = router.wheel(Vec2::new(0.0, 1.0), Vec2::new(100.0, 100.0));
        assert!(path.is_empty());
    }

    fn router() -> Router {
        Router::default()
    }

    #[test]
    fn a_press_targets_the_topmost_region() {
        let mut r = router();
        r.set_regions(vec![
            region(1, rect(0.0, 0.0, 100.0, 100.0)),
            region(2, rect(50.0, 50.0, 100.0, 100.0)),
        ]);
        let event = r.pointer_down(Vec2::new(60.0, 60.0), 0.0).unwrap();
        assert_eq!(event.target(), Some(2));
        assert_eq!(r.hover(), Some(2));
    }

    #[test]
    fn moving_without_a_press_updates_hover_only() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));
        r.pointer_move(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), None);
    }

    #[test]
    fn a_hovered_resize_handle_sets_the_resize_cursor() {
        let mut r = router();
        let handle = HitRegion::new(9, 1, 10, rect(90.0, 40.0, 20.0, 20.0))
            .with_cursor(CursorKind::ResizeEastWest);
        r.set_regions(vec![region(1, rect(0.0, 0.0, 200.0, 200.0)), handle]);
        r.pointer_move(Vec2::new(95.0, 45.0));
        assert_eq!(r.cursor(), CursorKind::ResizeEastWest);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.cursor(), CursorKind::Pointer);
    }

    #[test]
    fn a_portal_overlay_wins_the_cursor_over_the_ui() {
        let mut r = router();
        let portal = HitRegion::new(2, PORTAL_LAYER, 0, rect(0.0, 0.0, 50.0, 50.0))
            .with_cursor(CursorKind::Text);
        r.set_regions(vec![region(1, rect(0.0, 0.0, 200.0, 200.0)), portal]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.cursor(), CursorKind::Text);
    }

    #[test]
    fn a_capture_keeps_the_grabbing_cursor_over_other_regions() {
        let mut r = router();
        r.set_regions(vec![
            region(1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 0, rect(200.0, 200.0, 100.0, 100.0)).with_cursor(CursorKind::Text),
        ]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(30.0, 10.0));
        assert!(r.pointer().is_dragging());
        assert_eq!(r.cursor(), CursorKind::Grabbing);
        // 划过文本区域：捕获依然拥有光标控制权。
        r.pointer_move(Vec2::new(250.0, 250.0));
        assert_eq!(r.cursor(), CursorKind::Grabbing);
        r.pointer_up(Vec2::new(250.0, 250.0), 0.5);
        assert_eq!(r.cursor(), CursorKind::Text);
    }

    #[test]
    fn a_full_drag_cycle_emits_start_drag_and_up() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        let down = r.pointer_down(Vec2::new(10.0, 10.0), 0.0).unwrap();
        assert!(matches!(down, PointerEvent::Down { .. }));
        let start = r.pointer_move(Vec2::new(20.0, 10.0)).unwrap();
        assert!(matches!(start, PointerEvent::DragStart { target: 1, .. }));
        let drag = r.pointer_move(Vec2::new(40.0, 10.0)).unwrap();
        assert!(matches!(drag, PointerEvent::Drag { target: 1, .. }));
        let up = r.pointer_up(Vec2::new(40.0, 10.0), 0.3);
        assert_eq!(up.len(), 1, "a drag must not emit a click");
    }

    #[test]
    fn a_click_on_a_rounded_button_ignores_the_corner() {
        let mut r = router();
        r.set_regions(vec![HitRegion::new(5, 1, 0, rect(0.0, 0.0, 100.0, 100.0))
            .with_shape(Shape::Rounded { radii: [30.0; 4] })]);
        let corner = r.pointer_down(Vec2::new(2.0, 2.0), 0.0).unwrap();
        assert_eq!(corner.target(), None);
        let centre = r.pointer_down(Vec2::new(50.0, 50.0), 0.1).unwrap();
        assert_eq!(centre.target(), Some(5));
    }

    #[test]
    fn cancel_releases_the_capture() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(50.0, 10.0));
        let event = r.cancel().unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 1 });
        assert_eq!(
            r.cursor(),
            CursorKind::Pointer,
            "the capture slot must be gone and the hovered widget takes over"
        );
    }

    #[test]
    fn the_tool_cursor_is_the_floor() {
        let mut r = router();
        r.set_regions(vec![]);
        r.set_tool_cursor(CursorKind::Crosshair);
        r.pointer_move(Vec2::new(1.0, 1.0));
        assert_eq!(r.cursor(), CursorKind::Crosshair);
    }

    #[test]
    fn replacing_the_regions_repicks_the_hover_at_the_same_pointer() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));

        r.set_regions(vec![region(2, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), Some(2), "the pointer stayed, the scene changed");

        r.set_regions(Vec::new());
        assert_eq!(r.hover(), None);
    }

    #[test]
    fn regions_alone_do_not_invent_a_hover() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), None, "the pointer never entered the window");
    }

    #[test]
    fn a_release_outside_the_press_target_is_not_a_click() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(99.0, 50.0), 0.0);
        // 超出边界两像素：低于三像素的拖拽阈值，因此除非路由器检查
        // 释放落点，否则状态机仍会将其视为点击。
        //
        let events = r.pointer_up(Vec2::new(101.0, 50.0), 0.1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. })),
            "releasing outside the target must not activate it: {events:?}"
        );
        assert_eq!(r.hover(), None);

        // 被扣留的点击绝不能污染下一次点击序列。
        let down = r.pointer_down(Vec2::new(50.0, 50.0), 0.2).unwrap();
        assert!(matches!(down, PointerEvent::Down { click_count: 1, .. }));
        let events = r.pointer_up(Vec2::new(50.0, 50.0), 0.3);
        assert!(matches!(
            events.last(),
            Some(PointerEvent::Click { count: 1, .. })
        ));
    }

    #[test]
    fn a_drag_does_not_pair_with_the_next_press_as_a_double_click() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(60.0, 60.0));
        let events = r.pointer_up(Vec2::new(60.0, 60.0), 0.1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. })),
            "a drag must not click even when it ends on the target"
        );

        let down = r.pointer_down(Vec2::new(10.0, 10.0), 0.2).unwrap();
        assert!(
            matches!(down, PointerEvent::Down { click_count: 1, .. }),
            "the drag must reset the click sequence, got {down:?}"
        );
    }

    #[test]
    fn cancelling_repicks_the_hover_after_a_drag() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(300.0, 300.0));
        assert_eq!(r.hover(), Some(1), "hover is frozen while captured");

        r.cancel();
        assert_eq!(r.hover(), None, "the pointer is no longer over the button");
    }

    #[test]
    fn a_scene_rebuild_after_a_camera_move_does_not_resurrect_the_old_hover() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));

        // 相机平移：同一个窗口坐标点现在对应画布坐标 (500, 500)。
        r.refresh_at(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), None);

        // 重建场景时必须在刷新后的坐标处重新拾取，而非指针曾经所在的
        // 过时状态机坐标。
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), None, "a stale hover must not come back");
        assert_eq!(r.cursor(), CursorKind::Default);
    }

    #[test]
    fn cancelling_after_a_camera_move_uses_the_refreshed_position() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        assert_eq!(r.hover(), Some(1));

        // 画布在被捕获的指针下方移动。
        r.refresh_at(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), Some(1), "hover stays frozen while captured");

        r.cancel();
        assert_eq!(
            r.hover(),
            None,
            "cancel must use the refreshed point, not the stale press point"
        );
    }
}

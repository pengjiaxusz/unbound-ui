//! 确定性、无 GPU 的 UI 交互测试驱动器。
//!
//! [`TestApp`] 持有每个交互组件共享的核心状态——节点世界、指针路由器、
//! 语义树、相机与焦点槽位——并以**窗口像素**暴露合成输入。
//! 测试永不直接接触画布坐标：驱动器会应用与窗口相同的逆相机变换，
//! 因此缩放或平移后的场景与实际所见完全一致。
//!
//!
//!
//! 驱动器刻意不感知渲染或布局。组件只需提供一个轻量测试夹具，
//! 根据派生状态构建场景（按钮示例见 `apps/showcase/src/testing.rs`），
//! 通过 [`TestApp::install`] 安装命中区域与语义树，随后即可通过
//! [`TestApp::check`] 运行五项几何检查器。
//!
//!
//! ```ignore
//! let mut app = TestApp::new((1280.0, 720.0));
//! let region = HitRegion::new(7, 1, 0, Rect::new(100.0, 100.0, 200.0, 40.0));
//! app.install(vec![region], Semantics::new(1));
//!
//! let report = app.click(Vec2::new(200.0, 120.0));
//! assert_eq!(report.clicked(), Some(7));
//! ```

use slotmap::Key;
use unbound_a11y::{A11yNode, Semantics};
use unbound_input::{CursorKind, HitRegion, PointerConfig, PointerEvent, Router};
use unbound_math::Vec2;
use unbound_world::{NodeId, UiWorld};

use crate::{Frame, Harness, Report};

/// 新建 [`TestApp`] 时创建的语义根节点。
pub const DEFAULT_ROOT: u64 = 1;

/// 单次合成点击产生的结果。
#[derive(Clone, Debug, Default)]
pub struct ClickReport {
    /// 按下时捕获指针的节点（如有）。
    pub pressed: Option<u64>,
    /// 路由器按顺序发出的全部事件。
    pub events: Vec<PointerEvent>,
}

impl ClickReport {
    /// 若本次按下演变为点击，则记录收到 `Click` 的节点。
    #[must_use]
    pub fn clicked(&self) -> Option<u64> {
        self.events.iter().find_map(|event| match event {
            PointerEvent::Click { target, .. } => Some(*target),
            _ => None,
        })
    }

    /// 路由器统计的连续点击次数（单击为 1）。
    #[must_use]
    pub fn click_count(&self) -> Option<u8> {
        self.events.iter().find_map(|event| match event {
            PointerEvent::Click { count, .. } => Some(*count),
            _ => None,
        })
    }

    /// 两次点击是否构成了双击。
    #[must_use]
    pub fn is_double_click(&self) -> bool {
        self.events
            .iter()
            .any(|event| matches!(event, PointerEvent::DoubleClick { .. }))
    }
}

/// 无头 UI 会话的共享状态。
pub struct TestApp {
    /// 节点世界；组件在此分配其标识。
    pub world: UiWorld,
    /// 命中区域、悬停与指针捕获状态。
    pub router: Router,
    /// 语义树。
    pub semantics: Semantics,
    /// 视口尺寸（窗口像素）。
    pub viewport: Vec2,
    /// 相机缩放倍率。
    pub zoom: f32,
    /// 相机平移量（窗口像素）。
    pub pan: Vec2,
    /// 当前聚焦的语义 ID（如有）。
    focus: Option<u64>,
    /// 上一次指针位置（窗口像素）。
    pointer: Option<Vec2>,
    /// 路由器用于为事件打时间戳的逻辑时钟。
    time: f64,
    /// 每次调用 [`install`](Self::install) 时递增。
    generation: u64,
}

impl TestApp {
    /// 使用 [`DEFAULT_ROOT`] 创建会话。
    #[must_use]
    pub fn new(viewport: (f32, f32)) -> Self {
        Self::with_root(viewport, DEFAULT_ROOT)
    }

    /// 使用显式语义根 ID 创建会话。
    #[must_use]
    pub fn with_root(viewport: (f32, f32), root: u64) -> Self {
        Self {
            world: UiWorld::new(),
            router: Router::new(PointerConfig::default()),
            semantics: Semantics::new(root),
            viewport: Vec2::new(viewport.0, viewport.1),
            zoom: 1.0,
            pan: Vec2::ZERO,
            focus: None,
            pointer: None,
            time: 0.0,
            generation: 0,
        }
    }

    /// 推进逻辑时钟（单位：秒）。
    ///
    /// 在两次点击之间调用可避免路由器将其配对为双击，
    /// 或将两个合成事件隔离开到不同帧中。
    pub fn advance(&mut self, seconds: f64) {
        self.time += seconds;
    }

    /// 当前逻辑时间（秒）。
    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// 迄今已安装的场景次数。
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 由 `zoom` 与 `pan` 隐含的相机锚点。
    #[must_use]
    pub fn anchor(&self) -> Vec2 {
        self.viewport * 0.5 - self.pan / self.zoom
    }

    /// 将窗口位置映射为画布坐标。
    ///
    /// 着色器按 `screen = (canvas - anchor) * zoom + viewport / 2` 映射，
    /// 因此逆变换为 `canvas = (screen - viewport / 2) / zoom + anchor`。
    #[must_use]
    pub fn screen_to_canvas(&self, screen: Vec2) -> Vec2 {
        (screen - self.viewport * 0.5) / self.zoom + self.anchor()
    }

    /// 将画布坐标映射回窗口坐标。
    #[must_use]
    pub fn canvas_to_screen(&self, canvas: Vec2) -> Vec2 {
        (canvas - self.anchor()) * self.zoom + self.viewport * 0.5
    }

    /// 按窗口空间增量平移相机。
    ///
    /// 会在新的画布点位重新拾取悬停与光标：指针未动，但其下方的画布已移动。
    ///
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
        self.refresh_pointer();
    }

    /// 围绕窗口空间锚点缩放，保持锚点下的画布位置不变。
    pub fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        let before = self.screen_to_canvas(anchor);
        self.zoom = (self.zoom * factor).clamp(0.25, 8.0);
        let anchor_canvas = before - (anchor - self.viewport * 0.5) / self.zoom;
        self.pan = (self.viewport * 0.5 - anchor_canvas) * self.zoom;
        self.refresh_pointer();
    }

    /// 将指针移动到指定窗口位置并更新悬停状态。
    ///
    /// 返回当前悬停的节点（如有）。
    pub fn move_to(&mut self, screen: Vec2) -> Option<u64> {
        self.pointer = Some(screen);
        self.router.pointer_move(self.screen_to_canvas(screen));
        self.router.hover()
    }

    /// 在当前指针位置按下鼠标左键。
    ///
    /// 返回路由器捕获的节点（如有）。
    pub fn pointer_down(&mut self) -> Option<u64> {
        let position = self.screen_to_canvas(self.pointer_position());
        self.router.pointer_down(position, self.time);
        self.router.pointer().pressed()
    }

    /// 在当前指针位置释放鼠标左键。
    pub fn pointer_up(&mut self) -> Vec<PointerEvent> {
        let position = self.screen_to_canvas(self.pointer_position());
        self.router.pointer_up(position, self.time)
    }

    /// 一步完成移动、按下与释放。
    pub fn click(&mut self, screen: Vec2) -> ClickReport {
        self.move_to(screen);
        let pressed = self.pointer_down();
        let events = self.pointer_up();
        ClickReport { pressed, events }
    }

    /// 按住左键从一个窗口位置拖拽到另一个位置。
    pub fn drag(&mut self, from: Vec2, to: Vec2) -> ClickReport {
        self.move_to(from);
        let pressed = self.pointer_down();
        self.move_to(to);
        let events = self.pointer_up();
        ClickReport { pressed, events }
    }

    /// 中止交互，如同窗口失去焦点。
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        self.router.cancel()
    }

    /// 查询指定窗口位置下的节点，不移动指针。
    #[must_use]
    pub fn pick(&self, screen: Vec2) -> Option<u64> {
        self.router
            .pick(self.screen_to_canvas(screen))
            .map(|region| region.node)
    }

    /// 路由器在当前指针位置请求的鼠标光标样式。
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        self.router.cursor()
    }

    /// 替换命中区域与语义树。
    ///
    /// 此步骤对应“帧已提交”：递增代际计数并将焦点重新应用到新树。
    ///
    ///
    /// 重建可能会移除当前持有指针捕获或焦点的节点。两者都会针对新场景
    /// 进行校验：指向已不存在节点的捕获会被取消（并返回 `CaptureLost` 事件），
    /// 而指向已消失节点的焦点会回退到根节点。
    /// 否则后续释放事件可能会向场景中已不存在的节点投递点击。
    ///
    ///
    pub fn install(
        &mut self,
        regions: Vec<HitRegion>,
        semantics: Semantics,
    ) -> Option<PointerEvent> {
        self.router.set_regions(regions);
        let capture_lost = if self
            .router
            .pointer()
            .capture()
            .is_some_and(|node| !self.router.regions().iter().any(|r| r.node == node))
        {
            self.router.cancel()
        } else {
            None
        };
        self.semantics = semantics;
        if self
            .focus
            .is_some_and(|key| key != self.semantics.root() && self.semantics.get(key).is_none())
        {
            self.focus = None;
        }
        self.apply_focus();
        self.generation = self.generation.wrapping_add(1);
        capture_lost
    }

    /// 聚焦指定节点并同步至语义树。
    pub fn set_focus(&mut self, node: Option<NodeId>) {
        self.focus = node.map(node_key);
        self.apply_focus();
    }

    /// 聚焦原始语义 ID，供驱动快照的测试使用。
    pub fn set_focus_key(&mut self, key: u64) {
        self.focus = Some(key);
        self.apply_focus();
    }

    /// 移除键盘焦点。
    ///
    /// 语义树回退到根节点，与窗口将焦点让渡给其他控件时的行为一致。
    ///
    pub fn clear_focus(&mut self) {
        self.focus = None;
        self.apply_focus();
    }

    /// 当前聚焦的语义 ID（如有）。
    #[must_use]
    pub fn focused_key(&self) -> Option<u64> {
        self.focus
    }

    /// 获取标识对应的语义节点。
    ///
    /// # Panic
    ///
    /// 当节点不在当前树中时 panic。
    #[must_use]
    pub fn semantics_node(&self, node: NodeId) -> &A11yNode {
        let key = node_key(node);
        self.semantics
            .get(key)
            .unwrap_or_else(|| panic!("node {key} is not in the semantic tree"))
    }

    /// 断言节点的无障碍标签。
    pub fn assert_label(&self, node: NodeId, expected: &str) {
        assert_eq!(
            self.semantics_node(node).label,
            expected,
            "label of {node:?}"
        );
    }

    /// 断言节点的无障碍值。
    pub fn assert_value(&self, node: NodeId, expected: &str) {
        assert_eq!(
            self.semantics_node(node).value,
            expected,
            "value of {node:?}"
        );
    }

    /// 断言当前聚焦的语义 ID。
    pub fn assert_focus(&self, expected: Option<NodeId>) {
        assert_eq!(
            self.focused_key(),
            expected.map(node_key),
            "focus must match"
        );
        assert_eq!(
            self.semantics.focus(),
            self.focused_key().unwrap_or_else(|| self.semantics.root())
        );
    }

    /// 对一帧运行五项几何检查器。
    #[must_use]
    pub fn check(&self, frame: &Frame) -> Report {
        Harness::run(frame)
    }

    /// 运行检查器，若发现任何违规即 panic。
    pub fn assert_clean(&self, frame: &Frame) -> Report {
        Harness::assert_clean(frame)
    }

    /// 当前指针位置（窗口像素）。
    fn pointer_position(&self) -> Vec2 {
        self.pointer
            .expect("move the pointer before pressing or releasing")
    }

    /// 当相机在指针下方移动后，重新拾取悬停与光标。
    fn refresh_pointer(&mut self) {
        if let Some(screen) = self.pointer {
            self.router.refresh_at(self.screen_to_canvas(screen));
        }
    }

    /// 将 `focus` 镜像到语义树（根节点表示“无焦点”）。
    fn apply_focus(&mut self) {
        let target = self.focus.unwrap_or_else(|| self.semantics.root());
        self.semantics.set_focus(target);
    }
}

/// 节点的语义 ID。
#[must_use]
pub fn node_key(node: NodeId) -> u64 {
    node.data().as_ffi()
}

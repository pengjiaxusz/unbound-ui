//! showcase 按钮的测试装置（test harness）。
//!
//! [`ButtonHarness`] 是构建在 [`unbound_headless::driver::TestApp`] 之上的薄控件层：
//! 它持有按钮标识、标签塑形器、激活计数以及最近构建的场景。
//! 每个交互辅助方法都会从路由器重新推导视觉状态，并仅在离散的 `(visual, count)` 对
//! 发生变化时才重建场景——这与窗口使用的守卫条件完全相同，因此测试会验证真实的
//! 重绘策略。
//!
//!
//! `tests/gallery.rs` 中的集成测试使用此装置；GPU 像素测试则使用
//! [`ButtonHarness::rebuild_with`] 来烘焙特定状态。

use unbound_headless::driver::{ClickReport, TestApp};
use unbound_headless::Report;
use unbound_input::PointerEvent;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_world::NodeId;

use crate::button::{self, ButtonDesc, ButtonVisualState};
use crate::gallery::{self, Labels};

/// 测试装置渲染所使用的虚拟视口。
pub const VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// 无头按钮会话。
pub struct ButtonHarness {
    /// 共享的交互会话。
    pub app: TestApp,
    /// 标签塑形器与字形缓存。
    pub labels: Labels,
    /// 几何信息与基础标签。
    pub desc: ButtonDesc,
    /// 按钮标识。
    pub node: NodeId,
    /// 按钮被激活的次数。
    pub count: u32,
    /// 按钮是否处于禁用状态。
    pub disabled: bool,
    /// 最近一次场景的字形。
    pub glyphs: Vec<GlyphQuad>,
    /// 最近一次场景。
    pub scene: gallery::Gallery,
    /// 当前烘焙进 `scene` 的视觉状态。
    baked: ButtonVisualState,
    /// 当前烘焙进 `scene` 的计数。
    baked_count: u32,
}

/// 在覆盖整个表面的**同色**矩形之上绘制一个圆角矩形的场景。
/// 正确的直接 alpha（straight-alpha）合成在边缘像素无论如何抗锯齿时都应不可见。
/// 旧的双重 alpha 混合会把阴影颜色折入每个部分覆盖的像素，
/// 导致圆角矩形边缘在原本平坦的表面上出现一道暗边——即
/// “同色却有可见接缝”的缺陷。
///
///
#[must_use]
pub fn same_colour_overlay_scene(viewport: (f32, f32)) -> gallery::Gallery {
    let palette = gallery::palette();
    let mut scene = gallery::Gallery::new();
    scene.camera = unbound_gpu::Camera::ui(viewport, 1.0);
    scene.rect(
        0.0,
        0.0,
        viewport.0,
        viewport.1,
        0.0,
        palette.surface,
        0,
        0.0,
        0.0,
        0,
    );
    scene.rect(
        40.0,
        40.0,
        viewport.0 - 80.0,
        viewport.1 - 80.0,
        28.0,
        palette.surface,
        0,
        0.0,
        0.0,
        1,
    );
    scene
}

impl Default for ButtonHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl ButtonHarness {
    /// 构建默认卡片并烘焙空闲状态场景。
    #[must_use]
    pub fn new() -> Self {
        let mut app = TestApp::new(VIEWPORT);
        let desc = ButtonDesc::new(gallery::button_rect(), "Clicked: 0");
        let node = button::insert_button(&mut app.world, desc.rect);
        let mut harness = Self {
            app,
            labels: Labels::new(),
            desc,
            node,
            count: 0,
            disabled: false,
            glyphs: Vec::new(),
            scene: gallery::Gallery::default(),
            baked: ButtonVisualState::default(),
            baked_count: 0,
        };
        harness.rebuild_with(ButtonVisualState::default(), 0);
        harness
    }

    /// 按钮的语义 ID。
    #[must_use]
    pub fn node_key(&self) -> u64 {
        button::node_u64(self.node)
    }

    /// 从路由器与焦点槽位推导出的视觉状态。
    #[must_use]
    pub fn visual(&self) -> ButtonVisualState {
        let focused = if self.app.focused_key() == Some(self.node_key()) {
            Some(self.node)
        } else {
            None
        };
        button::resolve_state(self.node, &self.app.router, focused, self.disabled)
    }

    /// 以窗口像素为单位的按钮中心坐标。
    #[must_use]
    pub fn screen_center(&self) -> Vec2 {
        self.app.canvas_to_screen(self.desc.rect.center())
    }

    /// 当离散状态改变时重建场景。
    pub fn sync(&mut self) {
        let visual = self.visual();
        if visual != self.baked || self.count != self.baked_count {
            self.rebuild_with(visual, self.count);
        }
    }

    /// 将特定状态烘焙到一个新场景中。
    ///
    /// GPU 测试通过此方法直接渲染空闲 / 悬停 / 按下状态，而无需驱动路由器；
    /// 交互测试应优先使用 [`sync`](Self::sync)。
    pub fn rebuild_with(&mut self, visual: ButtonVisualState, count: u32) -> &gallery::Gallery {
        let mut desc = ButtonDesc::new(self.desc.rect, format!("Clicked: {count}"));
        desc.radius = self.desc.radius;
        desc.font_size = self.desc.font_size;
        desc.z = self.desc.z;
        desc.variant = self.desc.variant;
        let (scene, glyphs, regions, semantics) = gallery::build_interactive(
            &mut self.labels,
            &gallery::palette(),
            VIEWPORT,
            &desc,
            button::node_u64(self.node),
            visual,
            count,
        );
        self.scene = scene;
        self.glyphs = glyphs;
        self.app.install(regions, semantics);
        self.baked = visual;
        self.baked_count = count;
        self.disabled = visual.disabled;
        &self.scene
    }

    /// 将指针移动到按钮中心。
    pub fn hover(&mut self) {
        self.hover_at(self.screen_center());
    }

    /// 将指针移动到窗口中的指定位置。
    pub fn hover_at(&mut self, screen: Vec2) {
        self.app.move_to(screen);
        self.sync();
    }

    /// 在当前指针位置按下鼠标左键。
    pub fn press(&mut self) {
        self.app.pointer_down();
        // 路由器一捕获按钮，窗口就会立即将其聚焦。
        if self.app.router.pointer().pressed() == Some(button::node_u64(self.node)) {
            self.app.set_focus(Some(self.node));
        }
        self.sync();
    }

    /// 在当前指针位置释放鼠标左键。
    pub fn release(&mut self) -> Vec<PointerEvent> {
        let events = self.app.pointer_up();
        if events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { target, .. } if *target == button::node_u64(self.node)))
        {
            self.count = self.count.wrapping_add(1);
        }
        self.sync();
        events
    }

    /// 一步完成悬停、按下和释放按钮。
    pub fn click(&mut self) -> ClickReport {
        let report = self.app.click(self.screen_center());
        if report.pressed == Some(button::node_u64(self.node)) {
            self.app.set_focus(Some(self.node));
        }
        if report.clicked() == Some(button::node_u64(self.node)) {
            self.count = self.count.wrapping_add(1);
        }
        self.sync();
        report
    }

    /// 像窗口中按 `Enter` / `Space` 一样激活按钮。
    pub fn activate(&mut self) {
        self.count = self.count.wrapping_add(1);
        self.sync();
    }

    /// 切换禁用状态并重新烘焙场景。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        self.sync();
    }

    /// 迄今已安装的场景数量。
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.app.generation()
    }

    /// 聚焦按钮并重新烘焙焦点环。
    pub fn focus(&mut self) {
        self.app.set_focus(Some(self.node));
        self.sync();
    }

    /// 清除焦点并重新烘焙场景。
    pub fn blur(&mut self) {
        self.app.set_focus(None);
        self.sync();
    }

    /// 当前烘焙进语义树中的无障碍标签。
    #[must_use]
    pub fn label(&self) -> &str {
        &self.app.semantics_node(self.node).label
    }

    /// 当前烘焙进语义树中的无障碍值。
    #[must_use]
    pub fn value(&self) -> &str {
        &self.app.semantics_node(self.node).value
    }

    /// 几何校验器所检查的帧。
    #[must_use]
    pub fn frame(&self) -> unbound_headless::Frame {
        let mut desc = ButtonDesc::new(self.desc.rect, format!("Clicked: {}", self.baked_count));
        desc.radius = self.desc.radius;
        desc.font_size = self.desc.font_size;
        desc.z = self.desc.z;
        desc.variant = self.desc.variant;
        gallery::button_frame(VIEWPORT, &desc, &self.glyphs)
    }

    /// 在按钮帧上运行五项几何校验器。
    pub fn assert_clean(&self) -> Report {
        self.app.assert_clean(&self.frame())
    }

    /// 按钮所占用的画布矩形。
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.desc.rect
    }
}

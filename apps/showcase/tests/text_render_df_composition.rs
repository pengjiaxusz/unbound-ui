// 展台文本渲染控制台：距离场与合成组调节控件及互斥规则集成的无头测试。

use std::ops::{Deref, DerefMut};

use showcase::pages::text_render::{
    resolve_feature, TextFeatureKey, PART_COMP_LAYER_DIRECT, PART_COMP_LAYER_ISOLATED,
    PART_COMP_LINEAR, PART_COMP_SRGB, PART_DF_MTSDF, PART_DF_OFF, PART_DF_SDF,
};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::AppRuntime;
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_text::{BlendSpace, DistanceFieldMode, LayerMode, PhaseMode, TextCapabilities};

const VIEWPORT: (f32, f32) = (1440.0, 900.0);

struct App {
    runtime: AppRuntime<GalleryApp>,
    viewport: (f32, f32),
}

impl Deref for App {
    type Target = GalleryApp;

    fn deref(&self) -> &GalleryApp {
        self.runtime.application()
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut GalleryApp {
        self.runtime.application_mut()
    }
}

impl App {
    fn new() -> Self {
        Self {
            runtime: AppRuntime::new(GalleryApp::new(), Vec2::new(VIEWPORT.0, VIEWPORT.1)),
            viewport: VIEWPORT,
        }
    }

    fn paint(&mut self, dt: f32) {
        if let Some(node) = self.runtime.application_mut().take_focus_request() {
            self.runtime.focus_node(Some(node));
        }
        if self.runtime.application_mut().take_blur_request() {
            self.runtime.focus_node(None);
        }
        self.runtime.frame(dt);
    }

    fn rebuild(&mut self, viewport: (f32, f32)) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.runtime.resize(Vec2::new(viewport.0, viewport.1));
        }
        self.runtime.invalidate();
        self.paint(1.0 / 60.0);
    }

    fn click_node(&mut self, node: u64) {
        let rect = self
            .regions
            .iter()
            .find(|region| region.node == node)
            .map(|region| region.bounds)
            .unwrap_or_else(|| panic!("no hit region for node {node:#x}"));
        let center = rect.origin + rect.size * 0.5;
        self.runtime.pointer_move(center);
        self.runtime.advance(0.5);
        self.runtime.pointer_down(center, PointerButton::Left);
        self.runtime.pointer_up(center, PointerButton::Left);
        self.paint(1.0 / 60.0);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.state.theme = ThemeMode::Dark;
    app.rebuild(VIEWPORT);
    app
}

/// 验收条件 1、2：距离场与合成组控制台调节控件交互与互斥规则生效。
#[test]
fn text_render_distance_field_and_composition_controls() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 默认初始状态
    assert_eq!(
        app.state.text_render.settings.distance_field,
        DistanceFieldMode::Off
    );
    assert!(!app.state.text_render.settings.use_sdf);
    assert_eq!(app.state.text_render.settings.blend_space, BlendSpace::Srgb);
    assert_eq!(app.state.text_render.settings.layer_mode, LayerMode::Direct);

    // 1. 点击 SDF 按钮：启用距离场，验证互斥规则（相位锁定为 1）
    let sdf_node = target::id(Target::Demo(page_index, 0, PART_DF_SDF));
    app.click_node(sdf_node);
    assert_eq!(
        app.state.text_render.settings.distance_field,
        DistanceFieldMode::Sdf
    );
    assert!(app.state.text_render.settings.use_sdf);
    assert_eq!(
        app.state.text_render.settings.phase_mode,
        PhaseMode::One,
        "SDF mode must lock phase mode to 1"
    );

    // 验证互斥规则在能力查询中生效：8 相位与 4 相位被禁用
    let caps = TextCapabilities::probe();
    let res8 = resolve_feature(
        TextFeatureKey::PhaseEight,
        &caps,
        &app.state.text_render.settings,
    );
    assert!(res8.disabled);
    assert_eq!(
        res8.reason,
        Some("Phase locked to 1 when distance field is active")
    );

    // 2. 点击 Off 按钮：关闭距离场
    let off_node = target::id(Target::Demo(page_index, 0, PART_DF_OFF));
    app.click_node(off_node);
    assert_eq!(
        app.state.text_render.settings.distance_field,
        DistanceFieldMode::Off
    );
    assert!(!app.state.text_render.settings.use_sdf);

    // 3. 点击 MTSDF 按钮：因为未接线被灰化，点击不能开启 MTSDF
    let mtsdf_node = target::id(Target::Demo(page_index, 0, PART_DF_MTSDF));
    app.click_node(mtsdf_node);
    assert_ne!(
        app.state.text_render.settings.distance_field,
        DistanceFieldMode::Mtsdf,
        "MTSDF button is disabled and must not change settings"
    );

    // 4. 合成组：混合空间切换 (sRGB <-> Linear)
    let linear_node = target::id(Target::Demo(page_index, 0, PART_COMP_LINEAR));
    app.click_node(linear_node);
    assert_eq!(
        app.state.text_render.settings.blend_space,
        BlendSpace::Linear
    );

    let srgb_node = target::id(Target::Demo(page_index, 0, PART_COMP_SRGB));
    app.click_node(srgb_node);
    assert_eq!(app.state.text_render.settings.blend_space, BlendSpace::Srgb);

    // 5. 合成组：图层模式切换 (Direct <-> Isolated)
    let isolated_node = target::id(Target::Demo(page_index, 0, PART_COMP_LAYER_ISOLATED));
    app.click_node(isolated_node);
    assert_eq!(
        app.state.text_render.settings.layer_mode,
        LayerMode::Isolated
    );

    let direct_node = target::id(Target::Demo(page_index, 0, PART_COMP_LAYER_DIRECT));
    app.click_node(direct_node);
    assert_eq!(app.state.text_render.settings.layer_mode, LayerMode::Direct);
}

//! 亚克力材质在展厅宿主里的端到端行为。
//!
//! 亚克力要真的落到系统：选择「亚克力 + 系统原生」后，宿主解析出的计划必须
//! 携带亚克力系统材质请求，清屏 alpha 必须归零让材质从透明表面后透出，而宿主
//! 向 DWM 请求的原始取值必须是亚克力对应的那一个。
//!
//! 同时，亚克力在材质种类里明确没有自绘变体：选中「亚克力 + 引擎自绘」必须
//! 带回退原因，且绝不静默地造出一份亚克力外观。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::backdrop_host::{window_backdrop_request, WindowBackdropRequest};
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::AppRuntime;
use unbound_backdrop::{
    BackdropCompositeAlpha, BackdropSource, DwmMaterial, FallbackReason, MaterialKind,
};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_platform::WindowBackdropMaterial;

const VIEWPORT: (f32, f32) = (1440.0, 900.0);

/// 像窗口宿主一样通过运行时驱动外壳。
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

    /// 将外壳请求的任意焦点交给运行时，然后绘制一帧。
    fn paint(&mut self, dt: f32) {
        if let Some(node) = self.runtime.application_mut().take_focus_request() {
            self.runtime.focus_node(Some(node));
        }
        if self.runtime.application_mut().take_blur_request() {
            self.runtime.focus_node(None);
        }
        self.runtime.frame(dt);
    }

    fn resize(&mut self, viewport: (f32, f32)) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.runtime.resize(Vec2::new(viewport.0, viewport.1));
        }
    }

    fn rebuild(&mut self, viewport: (f32, f32)) {
        self.resize(viewport);
        self.runtime.invalidate();
        self.paint(1.0 / 60.0);
    }

    fn pointer_move(&mut self, pos: Vec2) {
        self.runtime.pointer_move(pos);
    }

    fn pointer_down(&mut self, pos: Vec2) {
        self.runtime.pointer_move(pos);
        // 向前拨动时钟以防两次点击被配对为双击。
        self.runtime.advance(0.5);
        self.runtime.pointer_down(pos, PointerButton::Left);
    }

    fn pointer_up(&mut self, pos: Vec2) {
        self.runtime.pointer_up(pos, PointerButton::Left);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.state.theme = ThemeMode::Dark;
    app.rebuild(VIEWPORT);
    app
}

fn rect_of(app: &App, node: u64) -> Rect {
    app.regions
        .iter()
        .find(|region| region.node == node)
        .map(|region| region.bounds)
        .unwrap_or_else(|| panic!("no hit region for node {node:#x}"))
}

/// 两个矩形是否互相压住。
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.origin.x < b.max().x
        && b.origin.x < a.max().x
        && a.origin.y < b.max().y
        && b.origin.y < a.max().y
}

fn click_node(app: &mut App, node: u64) {
    let rect = rect_of(app, node);
    let point = rect.center();
    app.pointer_move(point);
    app.pointer_down(point);
    app.pointer_up(point);
    app.rebuild(VIEWPORT);
}

/// 后景页材质行某个材质档的节点。
fn material_node(kind: MaterialKind) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        MATERIAL_PART_BASE + kind.index() as u16,
    ))
}

/// 后景页来源行某个来源的节点。
fn source_node(source: BackdropSource) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        SOURCE_PART_BASE + source.index() as u16,
    ))
}

/// 打开后景页并注入一档平台能力。
fn backdrop_app(windows_build: Option<u32>, premultiplied: bool) -> App {
    let mut app = app();
    app.set_backdrop_capabilities(windows_build, premultiplied);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);
    app
}

/// 亚克力走原生来源时，宿主必须向系统请求亚克力材质并把清屏 alpha 归零。
#[test]
fn the_acrylic_native_source_requests_the_acrylic_system_value() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, material_node(MaterialKind::Acrylic));
    click_node(&mut app, source_node(BackdropSource::Native));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Acrylic,
        "clicking the acrylic button must update the user choice"
    );
    assert_eq!(
        app.state.backdrop_selection.source,
        BackdropSource::Native,
        "clicking the native source must update the user choice"
    );
    assert_eq!(
        app.state.backdrop_plan.material_kind,
        MaterialKind::Acrylic,
        "the host must keep the acrylic material kind"
    );
    assert_eq!(
        app.state.backdrop_plan.source,
        BackdropSource::Native,
        "the host must keep the native source"
    );
    assert_eq!(
        app.state.backdrop_plan.material,
        DwmMaterial::Acrylic,
        "the host must request the acrylic system material"
    );
    assert_eq!(
        app.state.backdrop_plan.clear_alpha, 0.0,
        "the acrylic native path must clear to a fully transparent surface"
    );
    assert!(
        app.root_background_transparent(),
        "the root background must be transparent so the acrylic material shows through"
    );
    assert_eq!(
        app.state.backdrop_plan.fallback, None,
        "the acrylic native path must not report a fallback"
    );
    assert_eq!(
        app.state.backdrop_plan.unavailable, None,
        "the acrylic native path must not report an unavailable reason"
    );

    // 宿主向 DWM 请求的原始取值必须是亚克力对应的那一个。
    let (request, lock) = window_backdrop_request(&app.state.backdrop_plan);
    let WindowBackdropRequest::Dwm(material) = request else {
        panic!("the system acrylic material must stay on the DWM channel");
    };
    assert_eq!(
        material,
        WindowBackdropMaterial::Acrylic,
        "the host must ask the system for the acrylic material"
    );
    assert_eq!(
        material.dwm_backdrop_type_raw(),
        3,
        "the acrylic material must use the transient-window backdrop value"
    );
    assert!(
        lock,
        "a native acrylic material must keep the inactive-window lock"
    );
}

/// 平台层的原始取值必须保持公开文档记载的数值。
#[test]
fn the_platform_backdrop_values_keep_their_documented_raw_numbers() {
    assert_eq!(
        WindowBackdropMaterial::System.dwm_backdrop_type_raw(),
        0,
        "the system material must use the automatic backdrop value"
    );
    assert_eq!(
        WindowBackdropMaterial::None.dwm_backdrop_type_raw(),
        1,
        "the disabled material must use the none backdrop value"
    );
    assert_eq!(
        WindowBackdropMaterial::Mica.dwm_backdrop_type_raw(),
        2,
        "the mica material must use the main-window backdrop value"
    );
    assert_eq!(
        WindowBackdropMaterial::Acrylic.dwm_backdrop_type_raw(),
        3,
        "the acrylic material must use the transient-window backdrop value"
    );
    assert_eq!(
        WindowBackdropMaterial::Tabbed.dwm_backdrop_type_raw(),
        4,
        "the tabbed material must use the tabbed-window backdrop value"
    );
}

/// 亚克力没有自绘变体：选中非法组合必须带回退原因，且不得静默产出亚克力外观。
#[test]
fn the_acrylic_self_drawn_source_reports_the_missing_self_drawn_variant() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, material_node(MaterialKind::Acrylic));
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));

    let fallback = app
        .state
        .backdrop_combination
        .fallback
        .expect("the missing self-drawn variant must be reported");
    assert_eq!(
        fallback,
        FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic
        },
        "the readout must name the acrylic material as the one without a self-drawn variant"
    );
    assert_eq!(fallback.label(), "No self-drawn variant");
    assert_eq!(
        fallback.description(),
        "The Acrylic material has no self-drawn variant, so the combination fell back"
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native,
        "the fallback must switch to the native source when the system can host it"
    );

    // 机器承载不了原生路径时：回退到标准云母自绘，且没有亚克力系统材质请求。
    app.set_backdrop_capabilities(Some(19045), true);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_combination.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic
        }),
        "the fallback reason must survive the capability change"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::Mica,
        "the fallback must switch to the standard mica material"
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn,
        "the fallback must keep the self-drawn source"
    );
    assert_eq!(
        app.state.backdrop_plan.material,
        DwmMaterial::None,
        "the self-drawn fallback must not request any system material"
    );
    assert_eq!(
        app.state.backdrop_plan.clear_alpha, 1.0,
        "the self-drawn fallback must keep the clear alpha opaque"
    );
    assert!(
        !app.root_background_transparent(),
        "the self-drawn fallback must not fake a transparent root"
    );
}

/// 材质行与来源行的按钮互不重叠，且各自保留可访问性身份。
#[test]
fn the_acrylic_buttons_keep_their_identity_and_do_not_overlap() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, material_node(MaterialKind::Acrylic));

    let acrylic_rect = rect_of(&app, material_node(MaterialKind::Acrylic));
    for kind in MaterialKind::ALL {
        let node = material_node(kind);
        let rect = rect_of(&app, node);
        for source in BackdropSource::ALL {
            assert!(
                !rects_overlap(rect, rect_of(&app, source_node(source))),
                "the {kind:?} button must not overlap the {source:?} button"
            );
        }
        let a11y = app
            .semantics
            .get(node)
            .expect("the material button must be in the accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Material: {} — {}", kind.label(), kind.description()),
            "the material button must be named after its axis and material"
        );
        assert_eq!(
            a11y.selected,
            kind == MaterialKind::Acrylic,
            "only the picked material must be marked as selected"
        );
    }

    for source in BackdropSource::ALL {
        let node = source_node(source);
        let a11y = app
            .semantics
            .get(node)
            .expect("the source button must be in the accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Source: {} — {}", source.label(), source.description()),
            "the source button must be named after its axis and source"
        );
        assert_eq!(
            a11y.selected,
            source == BackdropSource::Auto,
            "only the picked source must be marked as selected"
        );
    }

    assert!(
        acrylic_rect.size.x > 0.0 && acrylic_rect.size.y > 0.0,
        "the acrylic button must occupy a real area"
    );
}

/// 在云母与亚克力之间切换不改变表面合成方式，因此不重建交换链。
#[test]
fn switching_between_mica_and_acrylic_keeps_the_surface_composition() {
    let mut app = backdrop_app(Some(26100), true);
    assert_eq!(
        app.state.backdrop_plan.composition,
        BackdropCompositeAlpha::PreMultiplied,
        "a premultiplied-capable host must keep the premultiplied surface"
    );

    click_node(&mut app, material_node(MaterialKind::Acrylic));
    assert_eq!(
        app.state.backdrop_plan.composition,
        BackdropCompositeAlpha::PreMultiplied,
        "switching to acrylic must not rebuild the swapchain"
    );
    assert_eq!(
        app.state.backdrop_plan.clear_alpha, 0.0,
        "the acrylic native path must clear to a fully transparent surface"
    );

    click_node(&mut app, material_node(MaterialKind::Mica));
    assert_eq!(
        app.state.backdrop_plan.composition,
        BackdropCompositeAlpha::PreMultiplied,
        "switching back to mica must not rebuild the swapchain"
    );
    assert_eq!(
        app.state.backdrop_plan.material,
        DwmMaterial::Mica,
        "the mica material must request the mica system material"
    );
}

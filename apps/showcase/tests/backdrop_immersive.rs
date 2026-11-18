//! 沉浸式后景实验档与逐档门槛的展台集成测试。
//!
//! 这些用例从「用户点击」一路检查到「宿主回写的计划」，钉住三件事：
//!
//! 1. 承载不了的材质按钮是禁用的，点击也不会改变选择；
//! 2. 选中按钮不会被误画成禁用态（#32 遗留缺陷）；
//! 3. 沉浸式后景的系统取值未确认时，失败原因必须出现在读出面板状态里。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::AppRuntime;
use unbound_backdrop::{BackdropSource, MaterialKind, UnavailableReason};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};

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

fn click_node(app: &mut App, node: u64) {
    let rect = rect_of(app, node);
    let point = rect.center();
    app.pointer_move(point);
    app.pointer_down(point);
    app.pointer_up(point);
    app.rebuild(VIEWPORT);
}

fn material_node(kind: MaterialKind) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        MATERIAL_PART_BASE + kind.index() as u16,
    ))
}

fn source_node(source: BackdropSource) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        SOURCE_PART_BASE + source.index() as u16,
    ))
}

/// 打开后景页，机器能力给定。
fn backdrop_app(windows_build: Option<u32>, premultiplied: bool) -> App {
    let mut app = app();
    app.set_backdrop_capabilities(windows_build, premultiplied);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);
    app
}

fn is_disabled(app: &App, node: u64) -> bool {
    app.semantics
        .get(node)
        .unwrap_or_else(|| panic!("node {node:#x} must be in the accessibility tree"))
        .disabled
}

/// 承载不了的材质按钮必须带禁用态，点击也不改变选择。
#[test]
fn a_tier_the_machine_cannot_host_is_disabled_and_ignores_clicks() {
    // 在合格构建上把来源切到原生：沉浸式后景没有确认的系统取值，必须禁用。
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::Native));

    assert!(
        is_disabled(&app, material_node(MaterialKind::Immersive)),
        "the immersive tier must be disabled on the native source"
    );
    assert!(
        !is_disabled(&app, material_node(MaterialKind::Mica)),
        "a capable build must keep standard Mica enabled"
    );

    let before = app.state.backdrop_selection;
    click_node(&mut app, material_node(MaterialKind::Immersive));
    assert_eq!(
        app.state.backdrop_selection, before,
        "clicking a disabled tier must not move the selection"
    );
}

/// 未知构建号下所有依赖原生的档都必须禁用；自绘回退能兜住的档仍可选。
#[test]
fn an_unknown_build_disables_native_only_tiers() {
    let mut app = backdrop_app(None, true);
    click_node(&mut app, source_node(BackdropSource::Native));

    for material in [
        MaterialKind::FollowSystem,
        MaterialKind::Mica,
        MaterialKind::MicaAlt,
        MaterialKind::Acrylic,
        MaterialKind::Immersive,
    ] {
        assert!(
            is_disabled(&app, material_node(material)),
            "{material:?} must be disabled on the native source with an unknown build"
        );
    }
    assert!(
        !is_disabled(&app, material_node(MaterialKind::None)),
        "the no-material tier must never be disabled"
    );

    // 自动来源下 Mica 仍有自绘回退，必须可选。
    click_node(&mut app, source_node(BackdropSource::Auto));
    assert!(
        !is_disabled(&app, material_node(MaterialKind::Mica)),
        "auto must keep Mica selectable through its self-drawn fallback"
    );
    assert!(
        is_disabled(&app, material_node(MaterialKind::Acrylic)),
        "auto must disable Acrylic because it has no self-drawn fallback"
    );
}

/// 自绘来源不因缺少自绘变体而禁用任何档。
#[test]
fn the_self_drawn_source_keeps_the_immersive_tier_selectable() {
    let mut app = backdrop_app(None, false);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));

    assert!(
        !is_disabled(&app, material_node(MaterialKind::Immersive)),
        "the self-drawn source must not disable the immersive tier for lacking a variant"
    );

    click_node(&mut app, material_node(MaterialKind::Immersive));
    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Immersive,
        "the immersive tier must be selectable on the self-drawn source"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::Mica,
        "the illegal self-drawn combination must fall back to Mica"
    );
}

/// 选中的材质按钮必须被标记为选中，且绝不被误画成禁用态（#32 遗留缺陷）。
#[test]
fn the_selected_material_button_is_selected_and_not_disabled() {
    let app = backdrop_app(Some(26100), true);
    let node = material_node(app.state.backdrop_selection.material);
    let a11y = app
        .semantics
        .get(node)
        .expect("the selected material must be in the accessibility tree");
    assert_eq!(a11y.role, Role::Button);
    assert!(a11y.selected, "the picked material must be marked selected");
    assert!(
        !a11y.disabled,
        "the picked material must not be drawn as disabled"
    );
}

/// 沉浸式的系统取值未确认时，读出面板状态里必须带上原因，且计划保持不透明。
#[test]
fn the_readout_surfaces_the_unconfirmed_immersive_channel() {
    let mut app = backdrop_app(Some(26100), true);

    // 先在自绘来源下选中沉浸式（允许），再切回原生来源观察回写结果。
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    click_node(&mut app, material_node(MaterialKind::Immersive));
    click_node(&mut app, source_node(BackdropSource::Native));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Immersive,
        "the source switch must not silently rewrite the user choice"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::None,
        "the unconfirmed channel must resolve to no material"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable,
        Some(UnavailableReason::SystemChannelUnconfirmed),
        "the readout must surface the unconfirmed system channel"
    );
    assert_eq!(
        app.state.backdrop_plan.unavailable,
        Some(UnavailableReason::SystemChannelUnconfirmed),
        "the plan must carry the unconfirmed system channel"
    );
    assert_eq!(
        app.state.backdrop_plan.clear_alpha, 1.0,
        "a plan without a system material must stay opaque"
    );
}

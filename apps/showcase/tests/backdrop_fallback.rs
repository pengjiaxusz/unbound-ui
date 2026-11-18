//! 展台上「组合非法」与「机器能力不足」的分离。
//!
//! 两类情况必须以两种不同的样子出现：
//!
//! 1. 显式自绘来源撞上没有自绘变体的材质时，按钮**仍可选**，选中后读出面板
//!    给出「没有自绘变体」的回退结果，按钮上另有可见的非法标记；
//! 2. 机器承载不了的组合进入**禁用态**，点击不改变选择，也不带非法标记。
//!
//! 本文件从真实点击一路检查到宿主回写的组合、计划与无障碍标签。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::AppRuntime;
use unbound_backdrop::{BackdropSource, FallbackReason, MaterialKind, UnavailableReason};
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

fn label_of(app: &App, node: u64) -> &str {
    app.semantics
        .get(node)
        .unwrap_or_else(|| panic!("node {node:#x} must be in the accessibility tree"))
        .label
        .as_str()
}

/// 自绘来源下，没有自绘变体的材质仍然可选——这不是机器能力不足。
#[test]
fn the_self_drawn_source_keeps_variantless_materials_selectable() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));

    for material in [MaterialKind::Acrylic, MaterialKind::Immersive] {
        let node = material_node(material);
        let a11y = app
            .semantics
            .get(node)
            .expect("the material button must be in the accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert!(
            !a11y.disabled,
            "{material:?} must stay selectable on the self-drawn source"
        );
        assert!(
            !a11y.label.contains("Invalid combination"),
            "{material:?} must not carry the illegal marker before it is chosen"
        );
    }
}

/// 选中非法组合后，读出面板状态给出「没有自绘变体」的回退，按钮带可见标记。
#[test]
fn an_illegal_acrylic_self_drawn_choice_reports_the_missing_variant() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    click_node(&mut app, material_node(MaterialKind::Acrylic));

    assert_eq!(
        app.state.backdrop_selection,
        unbound_backdrop::BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::SelfDrawn),
        "the illegal combination must stay selectable and become the user choice"
    );
    assert_eq!(
        app.state.backdrop_combination.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic
        }),
        "the readout must explain the missing self-drawn variant"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable, None,
        "an illegal combination must not claim a capability shortfall"
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native,
        "the fallback must keep the acrylic material and switch to the native source"
    );
    assert_eq!(
        app.state.backdrop_plan.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic
        }),
        "the plan must carry the same fallback the readout shows"
    );
    assert_eq!(
        app.state.backdrop_plan.unavailable, None,
        "the plan must not mix the capability reason into the illegal combination"
    );

    let node = material_node(MaterialKind::Acrylic);
    let a11y = app
        .semantics
        .get(node)
        .expect("the illegal material button must be in the accessibility tree");
    assert!(
        a11y.selected,
        "the illegal combination must be marked selected"
    );
    assert!(
        !a11y.disabled,
        "the illegal combination must not be drawn as disabled"
    );
    assert!(
        a11y.label.contains("Invalid combination"),
        "the selected illegal button must carry a visible marker beyond the fallback text"
    );
    assert!(
        a11y.label.contains("No self-drawn variant"),
        "the marker must name the missing self-drawn variant"
    );
}

/// 沉浸式后景在自绘来源下也可选，回退到标准云母的自绘变体。
#[test]
fn an_illegal_immersive_self_drawn_choice_falls_back_to_mica() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    click_node(&mut app, material_node(MaterialKind::Immersive));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Immersive,
        "the immersive tier must be selectable on the self-drawn source"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::Mica,
        "the illegal immersive combination must fall back to the Mica self-drawn variant"
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn,
        "the fallback must keep the self-drawn source when no native channel exists"
    );
    assert_eq!(
        app.state.backdrop_combination.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Immersive
        }),
        "the readout must name the missing immersive self-drawn variant"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable, None,
        "the unconfirmed system channel must not be mixed into the illegal combination"
    );

    let a11y = app
        .semantics
        .get(material_node(MaterialKind::Immersive))
        .expect("the illegal immersive button must be in the accessibility tree");
    assert!(a11y.selected);
    assert!(!a11y.disabled);
    assert!(a11y.label.contains("Invalid combination"));
}

/// 原生来源下，同一批材质按机器能力进入禁用态，且不带非法标记。
#[test]
fn the_native_source_disables_the_same_materials_for_capability_reasons() {
    // 构建号未知：原生路径对所有材质都不成立。
    let mut app = backdrop_app(None, true);
    click_node(&mut app, source_node(BackdropSource::Native));

    for material in [
        MaterialKind::FollowSystem,
        MaterialKind::Mica,
        MaterialKind::MicaAlt,
        MaterialKind::Acrylic,
        MaterialKind::Immersive,
    ] {
        let node = material_node(material);
        assert!(
            is_disabled(&app, node),
            "{material:?} must be disabled on the native source with an unknown build"
        );
        assert!(
            !label_of(&app, node).contains("Invalid combination"),
            "{material:?} is disabled by capability, not by an illegal combination"
        );
    }

    // 禁用按钮的点击不改变选择。
    let before = app.state.backdrop_selection;
    click_node(&mut app, material_node(MaterialKind::Acrylic));
    assert_eq!(
        app.state.backdrop_selection, before,
        "clicking a capability-disabled tier must not move the selection"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable,
        Some(UnavailableReason::BuildUnknown),
        "the readout must report the capability reason"
    );
    assert_eq!(
        app.state.backdrop_combination.fallback, None,
        "the capability reason must not be reported as a fallback"
    );
}

/// 能力足够时亚克力走原生可用；沉浸式仍因系统取值未确认而禁用。
#[test]
fn the_native_source_only_disables_what_the_machine_cannot_host() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::Native));

    assert!(
        !is_disabled(&app, material_node(MaterialKind::Acrylic)),
        "a capable host must keep acrylic selectable on the native source"
    );
    assert!(
        is_disabled(&app, material_node(MaterialKind::Immersive)),
        "the unconfirmed immersive channel must stay disabled on the native source"
    );

    click_node(&mut app, material_node(MaterialKind::Acrylic));
    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Acrylic,
        "clicking a capability-enabled tier must update the selection"
    );
    assert_eq!(
        app.state.backdrop_combination.fallback, None,
        "a legal native combination must not report a fallback"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable, None,
        "a capability-enabled combination must not report an unavailable reason"
    );
    assert!(
        !label_of(&app, material_node(MaterialKind::Acrylic)).contains("Invalid combination"),
        "a legal native combination must not carry the illegal marker"
    );
}

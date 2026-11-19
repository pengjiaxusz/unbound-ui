//! 亚克力自定义色调子档：独立通道、独立门槛。
//!
//! 这一档走未公开的 accent policy 通道，而不是 DWM 系统材质通道。验收要点：
//! 展台能选中它；它的可用性只由自己的能力字段决定；不可用时禁用并给出专属原因；
//! 它绝不改变其它材质的系统材质请求。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::MATERIAL_PART_BASE;
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::appearance::{backdrop_channel_request, BackdropChannel};
use unbound_app::AppRuntime;
use unbound_backdrop::{
    native_available, unavailable_reason, BackdropSource, DwmMaterial, MaterialKind,
    UnavailableReason, DEFAULT_ACRYLIC_TINT_ARGB,
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

/// 能力齐备时，展台能选中该子档，且计划走 accent 通道。
#[test]
fn the_tinted_acrylic_tier_is_selectable_on_the_showcase() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.set_backdrop_accent_capability(true);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    click_node(&mut app, material_node(MaterialKind::AcrylicTint));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::AcrylicTint,
        "the showcase must be able to select the tinted acrylic tier"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::AcrylicTint
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native
    );

    let plan = &app.state.backdrop_plan;
    assert_eq!(
        plan.material,
        DwmMaterial::None,
        "the tinted acrylic tier must not claim a DWM material value"
    );
    assert_eq!(
        plan.accent_tint,
        Some(DEFAULT_ACRYLIC_TINT_ARGB),
        "the tinted acrylic tier must carry its own accent tint"
    );
    assert_eq!(
        plan.clear_alpha, 0.0,
        "the system compositor draws the material, so the clear must be transparent"
    );
    assert!(
        app.root_background_transparent(),
        "the root background must be transparent for the accent material to show"
    );

    let (request, lock) = backdrop_channel_request(plan);
    assert_eq!(
        request,
        BackdropChannel::AccentTint(DEFAULT_ACRYLIC_TINT_ARGB),
        "the host must emit exactly one accent tint request"
    );
    assert!(lock, "a native material must keep the inactive-window lock");
}

/// 未公开导出不可用时，该子档禁用并给出专属原因，点击不改选择。
#[test]
fn the_tinted_acrylic_tier_is_disabled_without_its_own_capability() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.set_backdrop_accent_capability(false);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    let node = material_node(MaterialKind::AcrylicTint);
    let a11y = app
        .semantics
        .get(node)
        .expect("the disabled tier must still be in the accessibility tree");
    assert_eq!(a11y.role, Role::Button);
    assert!(
        a11y.disabled,
        "the tinted acrylic tier must be disabled when its channel is unavailable"
    );

    let before = app.state.backdrop_selection.material;
    click_node(&mut app, node);
    assert_eq!(
        app.state.backdrop_selection.material, before,
        "clicking the disabled tier must not change the selection"
    );

    assert_eq!(
        unavailable_reason(
            MaterialKind::AcrylicTint,
            &app.state.backdrop_capabilities()
        ),
        Some(UnavailableReason::AccentPolicyUnavailable),
        "the disabled tier must expose its own unavailable reason"
    );
}

/// 该子档的通道关闭时，其它材质的系统材质请求一个字节都不受影响。
#[test]
fn the_tinted_acrylic_channel_does_not_touch_the_system_material_requests() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.set_backdrop_accent_capability(false);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    for (kind, expected) in [
        (MaterialKind::FollowSystem, WindowBackdropMaterial::System),
        (MaterialKind::Mica, WindowBackdropMaterial::Mica),
        (MaterialKind::MicaAlt, WindowBackdropMaterial::Tabbed),
        (MaterialKind::Acrylic, WindowBackdropMaterial::Acrylic),
    ] {
        app.state.set_backdrop_material(kind);
        app.state.set_backdrop_source(BackdropSource::Native);
        app.rebuild(VIEWPORT);

        assert!(
            native_available(kind, &app.state.backdrop_capabilities()),
            "{kind:?} must stay native while the accent channel is off"
        );
        let plan = &app.state.backdrop_plan;
        assert_eq!(
            plan.accent_tint, None,
            "{kind:?} must never carry an accent tint"
        );
        let (request, _) = backdrop_channel_request(plan);
        assert_eq!(
            request,
            BackdropChannel::Dwm(expected),
            "{kind:?} must keep its own DWM system material request"
        );
    }
}

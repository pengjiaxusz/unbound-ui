//! 窗口后景材质集成测试（去重合并自原 5 个独立 `backdrop_*` 模块与 `shell.rs` 后景能力用例）。
//!
//! 覆盖全部材质种类（FollowSystem / None / Mica / MicaAlt / Acrylic / AcrylicTint / Immersive）、
//! 来源轴独立性（Auto / Native / SelfDrawn）、非法组合回退原因、平台能力不可用禁用与读出状态，
//! 以及无重叠布局与无障碍互斥选中语义。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::appearance::{backdrop_channel_request, BackdropChannel};
use unbound_app::AppRuntime;
use unbound_backdrop::{
    native_available, unavailable_reason, BackdropCompositeAlpha, BackdropSelection,
    BackdropSource, DwmMaterial, FallbackReason, MaterialKind, UnavailableReason, WallpaperTexture,
    DEFAULT_ACRYLIC_TINT_ARGB,
};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_platform::{ScreenRect, WindowBackdropMaterial};

const VIEWPORT: (f32, f32) = (1440.0, 900.0);

/// 像窗口宿主一样通过运行时驱动外壳的共用脚手架。
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

/// 打开后景页并注入指定平台能力与主屏几何。
fn backdrop_app(windows_build: Option<u32>, premultiplied: bool) -> App {
    let mut app = App::new();
    app.state.theme = ThemeMode::Dark;
    app.state.window_screen_rect = Some(ScreenRect::new(0, 0, 1280, 720));
    app.set_backdrop_capabilities(windows_build, premultiplied);
    app.navigate(PageId::Backdrop);
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

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.origin.x < b.max().x
        && a.max().x > b.origin.x
        && a.origin.y < b.max().y
        && a.max().y > b.origin.y
}

fn click_node(app: &mut App, node: u64) {
    let point = rect_of(app, node).center();
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

/// 平台能力（Windows 构建号与预乘合成支持）决定有效材质来源及原生回退路径。
#[test]
fn backdrop_capabilities_drive_effective_combination() {
    let mut app = backdrop_app(Some(22621), true);
    assert_eq!(app.state.windows_build, Some(22621));
    assert!(app.state.backdrop_native_available());
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native
    );

    app.state.set_backdrop_material(MaterialKind::Mica);
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.backdrop_combination.material, MaterialKind::Mica);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native
    );

    // Win10：原生不可用，回退到自绘。
    app.set_backdrop_capabilities(Some(19045), true);
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.windows_build, Some(19045));
    assert!(!app.state.backdrop_native_available());
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn
    );

    // 驱动不支持预乘：即使构建号达标也必须回退。
    app.set_backdrop_capabilities(Some(26100), false);
    app.rebuild(VIEWPORT);
    assert!(!app.state.backdrop_native_available());
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn
    );

    // 未知构建号（非 Windows）：同样回退。
    app.set_backdrop_capabilities(None, true);
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.windows_build, None);
    assert!(!app.state.backdrop_native_available());
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn
    );
}

/// 亚克力走原生来源时，宿主必须向 DWM 请求亚克力材质并保持透明清屏与非激活锁。
#[test]
fn acrylic_native_source_requests_acrylic_dwm_channel() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, material_node(MaterialKind::Acrylic));
    click_node(&mut app, source_node(BackdropSource::Native));

    assert_eq!(app.state.backdrop_selection.material, MaterialKind::Acrylic);
    assert_eq!(app.state.backdrop_selection.source, BackdropSource::Native);
    assert_eq!(app.state.backdrop_plan.material_kind, MaterialKind::Acrylic);
    assert_eq!(app.state.backdrop_plan.source, BackdropSource::Native);
    assert_eq!(app.state.backdrop_plan.material, DwmMaterial::Acrylic);
    assert_eq!(app.state.backdrop_plan.clear_alpha, 0.0);
    assert!(app.root_background_transparent());
    assert_eq!(app.state.backdrop_plan.fallback, None);
    assert_eq!(app.state.backdrop_plan.unavailable, None);

    let (request, lock) = backdrop_channel_request(&app.state.backdrop_plan);
    assert_eq!(
        request,
        BackdropChannel::Dwm(WindowBackdropMaterial::Acrylic)
    );
    assert!(lock, "native acrylic must keep the inactive-window lock");
}

/// 亚克力没有自绘变体：在自绘来源下选择后必须给出回退原因、可见非法标记并在无原生能力时回退标准云母。
#[test]
fn illegal_acrylic_self_drawn_reports_missing_variant_and_falls_back() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    click_node(&mut app, material_node(MaterialKind::Acrylic));

    assert_eq!(
        app.state.backdrop_selection,
        BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::SelfDrawn)
    );
    let fallback = app
        .state
        .backdrop_combination
        .fallback
        .expect("missing self-drawn variant must be reported");
    assert_eq!(
        fallback,
        FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic
        }
    );
    assert_eq!(fallback.label(), "No self-drawn variant");
    assert_eq!(
        fallback.description(),
        "The Acrylic material has no self-drawn variant, so the combination fell back"
    );
    assert_eq!(app.state.backdrop_combination.unavailable, None);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native
    );
    assert_eq!(app.state.backdrop_plan.fallback, Some(fallback));
    assert_eq!(app.state.backdrop_plan.unavailable, None);

    let a11y = app
        .semantics
        .get(material_node(MaterialKind::Acrylic))
        .expect("illegal material button must remain in accessibility tree");
    assert!(a11y.selected);
    assert!(!a11y.disabled);
    assert!(a11y.label.contains("Invalid combination"));
    assert!(a11y.label.contains("No self-drawn variant"));

    // 能力降级到 Win10 后：回退至标准云母自绘且清屏恢复不透明。
    app.set_backdrop_capabilities(Some(19045), true);
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.backdrop_combination.fallback, Some(fallback));
    assert_eq!(app.state.backdrop_combination.material, MaterialKind::Mica);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn
    );
    assert_eq!(app.state.backdrop_plan.material, DwmMaterial::None);
    assert_eq!(app.state.backdrop_plan.clear_alpha, 1.0);
    assert!(!app.root_background_transparent());
}

/// 自绘来源下无自绘变体的材质保持可选，选中沉浸式后回退至标准云母自绘并附带非法组合标记。
#[test]
fn self_drawn_source_keeps_variantless_tiers_selectable_and_immersive_falls_back_to_mica() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));

    for material in [MaterialKind::Acrylic, MaterialKind::Immersive] {
        let node = material_node(material);
        let a11y = app
            .semantics
            .get(node)
            .expect("material button must be in accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert!(!a11y.disabled);
        assert!(!a11y.label.contains("Invalid combination"));
    }

    click_node(&mut app, material_node(MaterialKind::Immersive));
    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Immersive
    );
    assert_eq!(app.state.backdrop_combination.material, MaterialKind::Mica);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn
    );
    assert_eq!(
        app.state.backdrop_combination.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Immersive
        })
    );
    assert_eq!(app.state.backdrop_combination.unavailable, None);

    let a11y = app
        .semantics
        .get(material_node(MaterialKind::Immersive))
        .expect("illegal immersive button must be in accessibility tree");
    assert!(a11y.selected);
    assert!(!a11y.disabled);
    assert!(a11y.label.contains("Invalid combination"));
}

/// 未知构建号下原生来源按能力禁用全部原生材质并暴露 `BuildUnknown`，自动来源下仅保留可自绘回退的云母。
#[test]
fn unknown_build_disables_native_only_tiers_with_capability_reason() {
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
        assert!(is_disabled(&app, node));
        assert!(!label_of(&app, node).contains("Invalid combination"));
    }
    assert!(!is_disabled(&app, material_node(MaterialKind::None)));

    let before = app.state.backdrop_selection;
    click_node(&mut app, material_node(MaterialKind::Acrylic));
    assert_eq!(app.state.backdrop_selection, before);
    assert_eq!(
        app.state.backdrop_combination.unavailable,
        Some(UnavailableReason::BuildUnknown)
    );
    assert_eq!(app.state.backdrop_combination.fallback, None);

    // 切回自动来源：Mica 有自绘回退因此可选，Acrylic 无自绘回退仍禁用。
    click_node(&mut app, source_node(BackdropSource::Auto));
    assert!(!is_disabled(&app, material_node(MaterialKind::Mica)));
    assert!(is_disabled(&app, material_node(MaterialKind::Acrylic)));
}

/// 能力齐备的原生来源仅禁用未确认系统通道的沉浸式材质，并屏蔽对禁用档位的点击。
#[test]
fn capable_native_source_enables_supported_tiers_and_disables_unconfirmed_immersive() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, source_node(BackdropSource::Native));

    assert!(!is_disabled(&app, material_node(MaterialKind::Mica)));
    assert!(!is_disabled(&app, material_node(MaterialKind::Acrylic)));
    assert!(is_disabled(&app, material_node(MaterialKind::Immersive)));

    let before = app.state.backdrop_selection;
    click_node(&mut app, material_node(MaterialKind::Immersive));
    assert_eq!(app.state.backdrop_selection, before);

    click_node(&mut app, material_node(MaterialKind::Acrylic));
    assert_eq!(app.state.backdrop_selection.material, MaterialKind::Acrylic);
    assert_eq!(app.state.backdrop_combination.fallback, None);
    assert_eq!(app.state.backdrop_combination.unavailable, None);
    assert!(!label_of(&app, material_node(MaterialKind::Acrylic)).contains("Invalid combination"));
}

/// 沉浸式在自绘来源下被选中后切回原生来源时，读出面板与计划必须暴露未确认通道原因且保持不透明。
#[test]
fn unconfirmed_immersive_channel_surfaces_reason_when_switching_to_native() {
    let mut app = backdrop_app(Some(26100), true);

    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    click_node(&mut app, material_node(MaterialKind::Immersive));
    click_node(&mut app, source_node(BackdropSource::Native));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::Immersive
    );
    assert_eq!(app.state.backdrop_combination.material, MaterialKind::None);
    assert_eq!(
        app.state.backdrop_combination.unavailable,
        Some(UnavailableReason::SystemChannelUnconfirmed)
    );
    assert_eq!(
        app.state.backdrop_plan.unavailable,
        Some(UnavailableReason::SystemChannelUnconfirmed)
    );
    assert_eq!(app.state.backdrop_plan.clear_alpha, 1.0);
}

/// 亚克力自定义色调子档在能力齐备时走独立 accent policy 通道，不占用 DWM 材质值。
#[test]
fn tinted_acrylic_tier_is_selectable_and_uses_accent_channel() {
    let mut app = backdrop_app(Some(26100), true);
    app.set_backdrop_accent_capability(true);
    app.rebuild(VIEWPORT);

    click_node(&mut app, material_node(MaterialKind::AcrylicTint));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::AcrylicTint
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
    assert_eq!(plan.material, DwmMaterial::None);
    assert_eq!(plan.accent_tint, Some(DEFAULT_ACRYLIC_TINT_ARGB));
    assert_eq!(plan.clear_alpha, 0.0);
    assert!(app.root_background_transparent());

    let (request, lock) = backdrop_channel_request(plan);
    assert_eq!(
        request,
        BackdropChannel::AccentTint(DEFAULT_ACRYLIC_TINT_ARGB)
    );
    assert!(lock);
}

/// accent policy 通道关闭时，自定义色调亚克力禁用并暴露专属原因，且绝不影响其他 DWM 材质请求。
#[test]
fn tinted_acrylic_disabled_without_accent_capability_and_isolates_dwm_materials() {
    let mut app = backdrop_app(Some(26100), true);
    app.set_backdrop_accent_capability(false);
    app.rebuild(VIEWPORT);

    let node = material_node(MaterialKind::AcrylicTint);
    let a11y = app
        .semantics
        .get(node)
        .expect("disabled tinted acrylic button must be in accessibility tree");
    assert_eq!(a11y.role, Role::Button);
    assert!(a11y.disabled);

    let before = app.state.backdrop_selection.material;
    click_node(&mut app, node);
    assert_eq!(app.state.backdrop_selection.material, before);
    assert_eq!(
        unavailable_reason(
            MaterialKind::AcrylicTint,
            &app.state.backdrop_capabilities()
        ),
        Some(UnavailableReason::AccentPolicyUnavailable)
    );

    for (kind, expected) in [
        (MaterialKind::FollowSystem, WindowBackdropMaterial::System),
        (MaterialKind::Mica, WindowBackdropMaterial::Mica),
        (MaterialKind::MicaAlt, WindowBackdropMaterial::Tabbed),
        (MaterialKind::Acrylic, WindowBackdropMaterial::Acrylic),
    ] {
        app.state.set_backdrop_material(kind);
        app.state.set_backdrop_source(BackdropSource::Native);
        app.rebuild(VIEWPORT);

        assert!(native_available(kind, &app.state.backdrop_capabilities()));
        assert_eq!(app.state.backdrop_plan.accent_tint, None);
        let (request, _) = backdrop_channel_request(&app.state.backdrop_plan);
        assert_eq!(request, BackdropChannel::Dwm(expected));
    }
}

/// 云母变体在原生路径下请求 `DwmMaterial::MicaAlt`，与标准云母互不串味，且来源轴切换不改材质选择。
#[test]
fn mica_alt_native_path_and_source_axis_isolation() {
    let mut app = backdrop_app(Some(26100), true);

    click_node(&mut app, material_node(MaterialKind::Mica));
    click_node(&mut app, source_node(BackdropSource::Native));
    let mica = app.state.backdrop_plan.clone();

    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    let mica_alt = app.state.backdrop_plan.clone();

    assert_eq!(mica.material, DwmMaterial::Mica);
    assert_eq!(mica_alt.material, DwmMaterial::MicaAlt);
    assert_eq!(mica_alt.clear_alpha, 0.0);
    assert!(mica_alt.root_background_transparent);
    assert!(mica_alt.texture.is_none());
    assert_eq!(mica_alt.fallback, None);
    assert_eq!(mica_alt.unavailable, None);
    assert_eq!(mica.clear_alpha, mica_alt.clear_alpha);
    assert_eq!(mica.composition, mica_alt.composition);

    for (source, expected_material) in [
        (BackdropSource::Auto, DwmMaterial::MicaAlt),
        (BackdropSource::Native, DwmMaterial::MicaAlt),
        (BackdropSource::SelfDrawn, DwmMaterial::None),
    ] {
        click_node(&mut app, source_node(source));
        assert_eq!(app.state.backdrop_selection.source, source);
        assert_eq!(app.state.backdrop_selection.material, MaterialKind::MicaAlt);
        assert_eq!(
            app.state.backdrop_combination.material,
            MaterialKind::MicaAlt
        );
        assert_eq!(app.state.backdrop_plan.material, expected_material);
    }
}

/// 云母变体在自绘路径下与标准云母共享全部纹理参数（仅材质配方不同），且不请求系统材质。
#[test]
fn mica_alt_self_drawn_path_matches_standard_mica_recipe() {
    let mut app = backdrop_app(Some(26100), true);

    click_node(&mut app, material_node(MaterialKind::Mica));
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    let mica = app.state.backdrop_plan.clone();

    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    let mica_alt = app.state.backdrop_plan.clone();

    assert_eq!(mica.source, BackdropSource::SelfDrawn);
    assert_eq!(mica_alt.source, BackdropSource::SelfDrawn);
    if let (Some(mica_texture), Some(mica_alt_texture)) = (mica.texture, mica_alt.texture) {
        assert_eq!(mica_texture.material, MaterialKind::Mica);
        assert_eq!(mica_alt_texture.material, MaterialKind::MicaAlt);
        assert_eq!(
            WallpaperTexture {
                material: MaterialKind::Mica,
                ..mica_alt_texture
            },
            mica_texture
        );
    }
    if unbound_backdrop::primary_screen_rect().is_some() {
        assert!(mica_alt.texture.is_some());
    }
    assert_eq!(mica.clear_alpha, 1.0);
    assert_eq!(mica.clear_alpha, mica_alt.clear_alpha);
    assert_eq!(
        mica.root_background_transparent,
        mica_alt.root_background_transparent
    );
    assert_eq!(mica_alt.material, DwmMaterial::None);
    assert_eq!(mica_alt.fallback, None);
    assert!(app.mica.is_enabled());
}

/// 在云母、云母变体与亚克力之间切换不改变预乘表面合成方式，避免重建交换链。
#[test]
fn switching_native_materials_keeps_premultiplied_swapchain_composition() {
    let mut app = backdrop_app(Some(26100), true);
    assert_eq!(
        app.state.backdrop_plan.composition,
        BackdropCompositeAlpha::PreMultiplied
    );

    for (kind, expected_dwm) in [
        (MaterialKind::MicaAlt, DwmMaterial::MicaAlt),
        (MaterialKind::Acrylic, DwmMaterial::Acrylic),
        (MaterialKind::Mica, DwmMaterial::Mica),
    ] {
        click_node(&mut app, material_node(kind));
        assert_eq!(
            app.state.backdrop_plan.composition,
            BackdropCompositeAlpha::PreMultiplied
        );
        assert_eq!(app.state.backdrop_plan.clear_alpha, 0.0);
        assert_eq!(app.state.backdrop_plan.material, expected_dwm);
    }
}

/// 材质行与来源行按钮互不重叠、命名完整，且选中项互斥高亮、绝不被误标为禁用态。
#[test]
fn material_and_source_rows_do_not_overlap_and_expose_exclusive_selection() {
    let mut app = backdrop_app(Some(26100), true);
    click_node(&mut app, material_node(MaterialKind::MicaAlt));

    let mut material_rects = Vec::new();
    for kind in MaterialKind::ALL {
        let node = material_node(kind);
        let rect = rect_of(&app, node);
        assert!(rect.size.x > 0.0 && rect.size.y > 0.0);
        material_rects.push(rect);

        let a11y = app
            .semantics
            .get(node)
            .expect("material button must be in accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Material: {} — {}", kind.label(), kind.description())
        );
        assert_eq!(a11y.selected, kind == MaterialKind::MicaAlt);
        if kind == MaterialKind::MicaAlt {
            assert!(!a11y.disabled);
        }
    }

    let mut source_rects = Vec::new();
    for source in BackdropSource::ALL {
        let node = source_node(source);
        let rect = rect_of(&app, node);
        assert!(rect.size.x > 0.0 && rect.size.y > 0.0);
        source_rects.push(rect);

        let a11y = app
            .semantics
            .get(node)
            .expect("source button must be in accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Source: {} — {}", source.label(), source.description())
        );
        assert_eq!(a11y.selected, source == BackdropSource::Auto);
    }

    for pair in material_rects.windows(2) {
        assert!(!rects_overlap(pair[0], pair[1]));
    }
    for pair in source_rects.windows(2) {
        assert!(!rects_overlap(pair[0], pair[1]));
    }
    for material in &material_rects {
        for source in &source_rects {
            assert!(!rects_overlap(*material, *source));
        }
    }
}

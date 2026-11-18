//! 云母变体（Mica Alt）的展台行为检查。
//!
//! 云母变体是独立的一档材质：它有自己的系统材质取值（`DWMSBT_TABBEDWINDOW` = 4），
//! 有自己的门槛与回退行为，但在自绘来源下必须与标准云母表现一致。
//! 这套测试盯住三件事：
//!
//! 1. 展台能选出云母变体，且读出面板跟着更新；
//! 2. 云母变体走原生来源时，请求的确实是云母变体自己的系统材质；
//! 3. 云母变体与标准云母在自绘来源下行为一致，材质轴与来源轴互不干扰。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::AppRuntime;
use unbound_backdrop::{BackdropCompositeAlpha, BackdropSource, DwmMaterial, MaterialKind};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_platform::{ScreenRect, WindowBackdropMaterial};

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

/// 材质行上某个材质档对应的节点编号。
fn material_node(kind: MaterialKind) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        MATERIAL_PART_BASE + kind.index() as u16,
    ))
}

/// 来源行上某个来源对应的节点编号。
fn source_node(source: BackdropSource) -> u64 {
    target::id(Target::Demo(
        PageId::Backdrop.index() as u8,
        0,
        SOURCE_PART_BASE + source.index() as u16,
    ))
}

/// 打开后景页，并把机器能力设成承载得住原生路径。
///
/// 窗口屏幕矩形由宿主在移动与缩放时写入：这里给一个已知的矩形，
/// 让自绘路径的纹理配方有确定的几何输入。
fn backdrop_app() -> App {
    let mut app = app();
    app.state.window_screen_rect = Some(ScreenRect::new(0, 0, 1280, 720));
    app.set_backdrop_capabilities(Some(26100), true);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);
    app
}

/// 点选云母变体，并确认读出面板与宿主计划都跟着走。
#[test]
fn clicking_the_mica_alt_button_updates_the_selection_and_the_readout() {
    let mut app = backdrop_app();

    click_node(&mut app, material_node(MaterialKind::MicaAlt));

    assert_eq!(
        app.state.backdrop_selection.material,
        MaterialKind::MicaAlt,
        "clicking the Mica Alt button must update the user choice"
    );
    assert_eq!(
        app.state.backdrop_selection.source,
        BackdropSource::Auto,
        "picking a material must not move the source axis"
    );
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::MicaAlt,
        "the host must write back the effective material"
    );
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native,
        "a capable host must resolve the Mica Alt material to the native source"
    );
    assert_eq!(
        app.state.backdrop_plan.material_kind,
        MaterialKind::MicaAlt,
        "the plan must carry the effective material"
    );
    assert_eq!(
        app.state.backdrop_plan.source,
        BackdropSource::Native,
        "the plan must carry the effective source"
    );
}

/// 云母变体走原生来源时，宿主必须请求云母变体自己的系统材质，并把清屏 alpha 压到 0。
#[test]
fn the_mica_alt_native_path_requests_the_mica_alt_system_material() {
    let mut app = backdrop_app();
    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    click_node(&mut app, source_node(BackdropSource::Native));

    let plan = &app.state.backdrop_plan;
    assert_eq!(
        plan.material,
        DwmMaterial::MicaAlt,
        "the native Mica Alt path must request the Mica Alt system material"
    );
    assert_eq!(
        plan.clear_alpha, 0.0,
        "the native Mica Alt path must clear to a fully transparent surface"
    );
    assert!(
        plan.root_background_transparent,
        "the native Mica Alt path must let the root background show through"
    );
    assert!(
        plan.texture.is_none(),
        "the native Mica Alt path must not generate a self-drawn texture"
    );
    assert_eq!(
        plan.fallback, None,
        "a capable host must not report a fallback for Mica Alt"
    );
    assert_eq!(
        plan.unavailable, None,
        "a capable host must not report an unavailable reason for Mica Alt"
    );
}

/// 云母变体与标准云母在原生来源下都请求各自的系统材质，互不串味。
#[test]
fn the_mica_alt_native_path_differs_from_standard_mica_only_in_the_material() {
    let mut app = backdrop_app();

    click_node(&mut app, material_node(MaterialKind::Mica));
    click_node(&mut app, source_node(BackdropSource::Native));
    let mica = app.state.backdrop_plan.clone();

    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    let mica_alt = app.state.backdrop_plan.clone();

    assert_eq!(
        mica.material,
        DwmMaterial::Mica,
        "standard Mica must request the standard Mica system material"
    );
    assert_eq!(
        mica_alt.material,
        DwmMaterial::MicaAlt,
        "Mica Alt must request the Mica Alt system material"
    );
    assert_eq!(
        mica.clear_alpha, mica_alt.clear_alpha,
        "both native paths must clear to the same transparency"
    );
    assert_eq!(
        mica.root_background_transparent, mica_alt.root_background_transparent,
        "both native paths must share the root transparency"
    );
    assert_eq!(
        mica.composition, mica_alt.composition,
        "both native paths must share the platform baseline composition"
    );
}

/// 云母变体走自绘来源时，必须与标准云母完全一致：同样的纹理配方、同样的清屏 alpha、
/// 同样的根透明性，且都不请求任何系统材质。
#[test]
fn the_mica_alt_self_drawn_path_matches_standard_mica() {
    let mut app = backdrop_app();

    click_node(&mut app, material_node(MaterialKind::Mica));
    click_node(&mut app, source_node(BackdropSource::SelfDrawn));
    let mica = app.state.backdrop_plan.clone();

    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    let mica_alt = app.state.backdrop_plan.clone();

    assert_eq!(
        mica.source,
        BackdropSource::SelfDrawn,
        "the self-drawn source must stay on the self-drawn path"
    );
    assert_eq!(
        mica_alt.source,
        BackdropSource::SelfDrawn,
        "the self-drawn source must stay on the self-drawn path for Mica Alt"
    );
    assert_eq!(
        mica.texture, mica_alt.texture,
        "Mica Alt must use the same self-drawn texture recipe as standard Mica"
    );
    // 纹理配方依赖宿主几何：主屏矩形可知时纹理必须在场。
    if unbound_backdrop::primary_screen_rect().is_some() {
        assert!(
            mica_alt.texture.is_some(),
            "the self-drawn path must produce a texture"
        );
    }
    assert_eq!(
        mica.clear_alpha, mica_alt.clear_alpha,
        "both self-drawn paths must clear to the same opacity"
    );
    assert_eq!(
        mica.clear_alpha, 1.0,
        "the self-drawn path must not clear to a transparent surface"
    );
    assert_eq!(
        mica.root_background_transparent, mica_alt.root_background_transparent,
        "both self-drawn paths must share the root transparency"
    );
    assert_eq!(
        mica_alt.material,
        DwmMaterial::None,
        "the self-drawn path must not request any system material"
    );
    assert_eq!(
        mica_alt.fallback, None,
        "the self-drawn path must not report a fallback"
    );
    assert!(
        app.mica.is_enabled(),
        "the Mica Alt self-drawn path must keep generating the wallpaper texture"
    );
}

/// 来源轴的三个按钮各自独立：改来源不动材质，也不改变材质带来的系统材质请求。
#[test]
fn the_source_row_moves_the_source_axis_without_moving_mica_alt() {
    let mut app = backdrop_app();
    click_node(&mut app, material_node(MaterialKind::MicaAlt));

    for (source, expected_material) in [
        (BackdropSource::Auto, DwmMaterial::MicaAlt),
        (BackdropSource::Native, DwmMaterial::MicaAlt),
        (BackdropSource::SelfDrawn, DwmMaterial::None),
    ] {
        click_node(&mut app, source_node(source));
        assert_eq!(
            app.state.backdrop_selection.source, source,
            "clicking a source button must update the user choice for {source:?}"
        );
        assert_eq!(
            app.state.backdrop_selection.material,
            MaterialKind::MicaAlt,
            "picking a source must not move the material axis for {source:?}"
        );
        assert_eq!(
            app.state.backdrop_combination.material,
            MaterialKind::MicaAlt,
            "the effective material must stay Mica Alt for {source:?}"
        );
        assert_eq!(
            app.state.backdrop_plan.material, expected_material,
            "the system material request must follow the source for {source:?}"
        );
    }
}

/// 材质行与来源行各占一段部件编号，两行按钮互不重叠。
#[test]
fn the_two_rows_do_not_overlap_and_keep_their_accessibility_identity() {
    let mut app = backdrop_app();
    click_node(&mut app, material_node(MaterialKind::MicaAlt));

    let mut material_rects = Vec::new();
    for kind in MaterialKind::ALL {
        let node = material_node(kind);
        let rect = rect_of(&app, node);
        material_rects.push(rect);
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
    }

    let mut source_rects = Vec::new();
    for source in BackdropSource::ALL {
        let node = source_node(source);
        let rect = rect_of(&app, node);
        source_rects.push(rect);
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
    }

    for pair in material_rects.windows(2) {
        assert!(
            !rects_overlap(pair[0], pair[1]),
            "the material row must not overlap itself"
        );
    }
    for pair in source_rects.windows(2) {
        assert!(
            !rects_overlap(pair[0], pair[1]),
            "the source row must not overlap itself"
        );
    }
    for material in &material_rects {
        for source in &source_rects {
            assert!(
                !rects_overlap(*material, *source),
                "the material row and the source row must not overlap"
            );
        }
    }
}

/// 选中的云母变体按钮必须被标记为选中，其它材质按钮不得被误标。
#[test]
fn the_picked_mica_alt_button_is_the_only_selected_material() {
    let mut app = backdrop_app();
    click_node(&mut app, material_node(MaterialKind::MicaAlt));

    for kind in MaterialKind::ALL {
        let node = material_node(kind);
        let a11y = app
            .semantics
            .get(node)
            .expect("the material button must be in the accessibility tree");
        assert_eq!(
            a11y.selected,
            kind == MaterialKind::MicaAlt,
            "only the picked material must be marked as selected for {kind:?}"
        );
    }
}

/// 在云母与云母变体之间来回切换绝不重建交换链：合成方式只由平台基线决定。
#[test]
fn switching_between_mica_and_mica_alt_keeps_the_swap_chain() {
    let mut app = backdrop_app();
    click_node(&mut app, material_node(MaterialKind::Mica));
    let composition = app.state.backdrop_plan.composition;
    assert_eq!(
        composition,
        BackdropCompositeAlpha::PreMultiplied,
        "a premultiplied-capable host must keep the premultiplied surface"
    );

    click_node(&mut app, material_node(MaterialKind::MicaAlt));
    assert_eq!(
        app.state.backdrop_plan.composition, composition,
        "switching to Mica Alt must not rebuild the swapchain"
    );
    assert_eq!(
        app.state.backdrop_plan.clear_alpha, 0.0,
        "the Mica Alt native path must clear to a transparent surface"
    );

    click_node(&mut app, material_node(MaterialKind::Mica));
    assert_eq!(
        app.state.backdrop_plan.composition, composition,
        "switching back to Mica must not rebuild the swapchain"
    );
}

/// 平台层的系统材质取值必须保持文档语义：云母变体是标签页材质，取值 4。
#[test]
fn the_platform_raw_values_keep_their_documented_meaning() {
    assert_eq!(
        WindowBackdropMaterial::System.dwm_backdrop_type_raw(),
        0,
        "DWMSBT_AUTO must stay 0"
    );
    assert_eq!(
        WindowBackdropMaterial::None.dwm_backdrop_type_raw(),
        1,
        "DWMSBT_NONE must stay 1"
    );
    assert_eq!(
        WindowBackdropMaterial::Mica.dwm_backdrop_type_raw(),
        2,
        "DWMSBT_MAINWINDOW must stay 2"
    );
    assert_eq!(
        WindowBackdropMaterial::Acrylic.dwm_backdrop_type_raw(),
        3,
        "DWMSBT_TRANSIENTWINDOW must stay 3"
    );
    assert_eq!(
        WindowBackdropMaterial::Tabbed.dwm_backdrop_type_raw(),
        4,
        "the Mica Alt system material must stay DWMSBT_TABBEDWINDOW = 4"
    );
}

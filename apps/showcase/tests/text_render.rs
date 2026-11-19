//! 文本渲染控制台骨架、状态栏与能力灰化机制的无头集成测试。
//! lang-allow: CJK page metadata assertions and specimen samples

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::text_render::{
    resolve_feature, FeatureResolution, TextConsoleGroup, TextFeatureKey, FEATURE_RULES,
    FONT_FAMILY_OPTIONS, FONT_FAMILY_PART_BASE, FONT_ITALIC_PART, FONT_SIZE_OPTIONS,
    FONT_SIZE_PART_BASE, FONT_WEIGHT_OPTIONS, FONT_WEIGHT_PART_BASE, GROUP_PART_BASE,
    PART_BACKEND_COSMIC, PART_BACKEND_DW, PART_BIDI_OFF, PART_BIDI_ON, PART_EMOJI_BACKEND_COSMIC,
    PART_EMOJI_BACKEND_DW, PART_EMOJI_BACKEND_FOLLOW, PART_EMOJI_PRES_COLOR, PART_EMOJI_PRES_MONO,
    PART_EMOJI_SOURCE_BUILTIN, PART_EMOJI_SOURCE_GRID, PART_EMOJI_SOURCE_SYSTEM,
    PART_LETTER_SPACING_0, PART_LETTER_SPACING_1, PART_LETTER_SPACING_2, PART_LIGATURES_OFF,
    PART_LIGATURES_ON, PART_LINEBREAK_ANYWHERE, PART_LINEBREAK_NONE, PART_LINEBREAK_WORD,
    PART_LINE_HEIGHT_20, PART_LINE_HEIGHT_28, PART_LINE_HEIGHT_AUTO, PART_SHAPER_COSMIC,
    PART_SHAPER_NATIVE, PHASE_PART_BASE, STATUS_BAR_PART,
};
use showcase::pages::{CategoryId, PageId};
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::{AppRuntime, Locale};
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_text::{
    CapabilityStatus, EmojiBackend, EmojiPresentation, EmojiSource, LineBreakMode, PhaseMode,
    RasterBackend, TextCapabilities, TextRenderSettings, TextShaper,
};

const VIEWPORT: (f32, f32) = (1440.0, 1200.0);

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

/// 验收条件 1：页面出现在展台导航并具备完整中英文元数据与搜索索引。
#[test]
fn text_render_page_metadata_and_navigation() {
    assert_eq!(PageId::TextRender.category(), CategoryId::System);
    assert!(
        CategoryId::System.pages().contains(&PageId::TextRender),
        "text render page must be registered in CategoryId::System"
    );
    assert_eq!(
        PageId::ALL[PageId::TextRender.index()],
        PageId::TextRender,
        "PageId::ALL must place TextRender at its indexed position"
    );

    let info = PageId::TextRender.info();
    assert!(!info.title.is_empty());
    assert!(!info.short.is_empty());
    assert!(!info.description.is_empty());
    assert!(!info.crates.is_empty());
    assert!(!info.keywords.is_empty());

    let zh = PageId::TextRender.localized_info(Locale::ZhCn);
    assert_ne!(zh.title, info.title);
    assert_ne!(zh.short, info.short);
    assert!(!zh.description.is_empty());

    // 搜索可达性测试：中英文关键词、拼音检索均应匹配
    assert!(PageId::TextRender.matches("Text Render"));
    assert!(PageId::TextRender.matches("directwrite"));
    assert!(PageId::TextRender.matches("文本渲染"));
    assert!(PageId::TextRender.matches("guangshanhua")); // 拼音：光栅化
}

/// 验收条件 2、4：无头渲染、状态栏实际生效值读取与无障碍语义树。
#[test]
fn text_render_page_headless_render_and_accessibility() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    // 状态栏必须读取真实生效值，而非选择值
    assert_eq!(
        app.state.text_render.report.backend,
        RasterBackend::default()
    );
    assert_eq!(app.state.text_render.report.phases, 8);
    assert_eq!(
        app.state.text_render.report.emoji_path,
        "Swash (Color RGBA8)"
    );
    assert_eq!(app.state.text_render.report.fallback_count, 0);

    let page_index = PageId::TextRender.index() as u8;

    // 验证六组折叠标题按钮的无障碍语义：Role::Button，且具有 expanded 展开状态
    for (i, group) in TextConsoleGroup::ALL.iter().enumerate() {
        let part = GROUP_PART_BASE + i as u16;
        let node_id = target::id(Target::Demo(page_index, 0, part));

        let a11y_node = app
            .semantics
            .get(node_id)
            .unwrap_or_else(|| panic!("missing a11y node for group {}", group.label()));

        assert_eq!(a11y_node.role, Role::Button);
        assert_eq!(
            a11y_node.expanded,
            Some(true),
            "group {} must be initially expanded in semantics tree",
            group.label()
        );
        assert!(a11y_node.label.contains(group.label()));
    }

    // 验证状态栏具备无障碍语义（Role::Status / 可聚焦）
    let status_node_id = target::id(Target::Demo(page_index, 0, STATUS_BAR_PART));
    let status_node = app
        .semantics
        .get(status_node_id)
        .expect("missing a11y node for status bar");
    assert_eq!(status_node.role, Role::Status);
    assert!(status_node.label.contains("Effective Report"));
}

/// 验收条件 2、3：六组折叠状态可切换且重绘不丢失。
#[test]
fn text_render_collapse_state_persistence_and_toggle() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;
    let font_group_node = target::id(Target::Demo(page_index, 0, GROUP_PART_BASE));

    // 初始状态：Font 分组展开
    assert!(app.state.text_render.group_expanded[0]);

    // 点击 Font 分组折叠标题
    app.click_node(font_group_node);
    assert!(
        !app.state.text_render.group_expanded[0],
        "Font group must be collapsed after click"
    );

    // 重绘页面，验证折叠状态保持
    app.rebuild(VIEWPORT);
    assert!(
        !app.state.text_render.group_expanded[0],
        "Font group must stay collapsed after rebuild"
    );

    // 验证语义树展开标志同步更新为 false
    let a11y_node = app
        .semantics
        .get(font_group_node)
        .expect("a11y node exists");
    assert_eq!(a11y_node.expanded, Some(false));

    // 再次点击，重新展开
    app.click_node(font_group_node);
    assert!(app.state.text_render.group_expanded[0]);
    app.rebuild(VIEWPORT);
    let a11y_node = app
        .semantics
        .get(font_group_node)
        .expect("a11y node exists");
    assert_eq!(a11y_node.expanded, Some(true));

    // 点击状态栏切换回退详情展开状态
    let status_node_id = target::id(Target::Demo(page_index, 0, STATUS_BAR_PART));
    assert!(!app.state.text_render.fallback_details_expanded);
    app.click_node(status_node_id);
    assert!(app.state.text_render.fallback_details_expanded);
}

/// 验收条件 3：表驱动三态能力灰化机制（Unavailable 与 NotCompiled 必然禁用）。
#[test]
fn table_driven_capability_disabled_unavailable_and_not_compiled() {
    let default_settings = TextRenderSettings::default();

    // 1. 测试 Unavailable 状态必然导致控件禁用，且禁用理由与底层能力一致
    let mut caps = TextCapabilities::probe();
    caps.directwrite = CapabilityStatus::Unavailable("Custom DirectWrite failure");
    let res = resolve_feature(TextFeatureKey::BackendDirectWrite, &caps, &default_settings);
    assert_eq!(
        res,
        FeatureResolution {
            disabled: true,
            reason: Some("Custom DirectWrite failure"),
        }
    );

    // 2. 测试 NotCompiled 状态必然导致控件禁用，且理由为构建未包含
    caps.directwrite = CapabilityStatus::NotCompiled;
    let res = resolve_feature(TextFeatureKey::BackendDirectWrite, &caps, &default_settings);
    assert_eq!(
        res,
        FeatureResolution {
            disabled: true,
            reason: Some("Feature not included in this build"),
        }
    );

    // 3. 测试 MTSDF 明确未接线
    let res = resolve_feature(TextFeatureKey::DistanceFieldMtsdf, &caps, &default_settings);
    assert_eq!(
        res,
        FeatureResolution {
            disabled: true,
            reason: Some("Pipeline not wired up"),
        }
    );

    // 4. 测试 Hinting 在当前后端不可用
    let res = resolve_feature(TextFeatureKey::Hinting, &caps, &default_settings);
    assert!(res.disabled);
    assert_eq!(
        res.reason,
        Some("Hinting is not supported by current backends")
    );

    // 5. 测试子像素抗锯齿（ADR 0013 灰度约束）
    let res = resolve_feature(
        TextFeatureKey::SubpixelAntialiasing,
        &caps,
        &default_settings,
    );
    assert!(res.disabled);
    assert_eq!(
        res.reason,
        Some("Mutually exclusive with transparent layers and background blending")
    );
}

/// 验收条件 3：表驱动能力上下文约束与互斥规则。
#[test]
fn table_driven_capability_contextual_constraints() {
    let mut caps = TextCapabilities::probe();
    caps.subpixel_phase_8 = CapabilityStatus::Available;
    caps.subpixel_phase_4 = CapabilityStatus::Available;

    // 当未开启距离场时，8 相位和 4 相位均可用
    let mut settings = TextRenderSettings {
        use_sdf: false,
        ..Default::default()
    };
    let res8 = resolve_feature(TextFeatureKey::PhaseEight, &caps, &settings);
    assert!(!res8.disabled);
    assert_eq!(res8.reason, None);

    let res4 = resolve_feature(TextFeatureKey::PhaseFour, &caps, &settings);
    assert!(!res4.disabled);
    assert_eq!(res4.reason, None);

    // 当开启距离场时，按规格相位必须锁定为 1，8 相位与 4 相位必须被互斥规则禁用
    settings.use_sdf = true;
    let res8_sdf = resolve_feature(TextFeatureKey::PhaseEight, &caps, &settings);
    assert!(res8_sdf.disabled);
    assert_eq!(
        res8_sdf.reason,
        Some("Phase locked to 1 when distance field is active")
    );

    let res4_sdf = resolve_feature(TextFeatureKey::PhaseFour, &caps, &settings);
    assert!(res4_sdf.disabled);
    assert_eq!(
        res4_sdf.reason,
        Some("Phase locked to 1 when distance field is active")
    );

    // 1 相位在距离场下仍然可用
    let res1_sdf = resolve_feature(TextFeatureKey::PhaseOne, &caps, &settings);
    assert!(!res1_sdf.disabled);
}

/// 验收条件 3、5：表驱动规则全量覆盖性与绝对无漏网断言。
#[test]
fn table_driven_rules_comprehensive_coverage() {
    // 构造全 Unavailable 的能力探测模型
    let mock_unavailable = TextCapabilities {
        directwrite: CapabilityStatus::Unavailable("mock unavailable"),
        cosmic: CapabilityStatus::Unavailable("mock unavailable"),
        sdf: CapabilityStatus::Unavailable("mock unavailable"),
        mtsdf: CapabilityStatus::Unavailable("mock unavailable"),
        subpixel_phase_8: CapabilityStatus::Unavailable("mock unavailable"),
        subpixel_phase_4: CapabilityStatus::Unavailable("mock unavailable"),
        subpixel_phase_1: CapabilityStatus::Unavailable("mock unavailable"),
        color_glyphs: CapabilityStatus::Unavailable("mock unavailable"),
        mono_fallback: CapabilityStatus::Unavailable("mock unavailable"),
        mono_directwrite: CapabilityStatus::Unavailable("mock unavailable"),
        color_directwrite: CapabilityStatus::Unavailable("mock unavailable"),
        hinting: CapabilityStatus::Unavailable("mock unavailable"),
        subpixel_antialiasing: CapabilityStatus::Unavailable("mock unavailable"),
    };

    let settings = TextRenderSettings::default();

    // 遍历 FEATURE_RULES，验证凡依赖系统特性的规则在 mock_unavailable 下必然被禁用
    for rule in FEATURE_RULES {
        let status = (rule.query_capability)(&mock_unavailable);
        if !status.is_available() {
            let res = resolve_feature(rule.key, &mock_unavailable, &settings);
            assert!(
                res.disabled,
                "feature rule {:?} must resolve to disabled when its underlying capability is unavailable",
                rule.key
            );
            assert!(
                res.reason.is_some(),
                "feature rule {:?} must provide a clear reason when disabled",
                rule.key
            );
        }
    }
}

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 验收条件 1、2：像素组内 DirectWrite 与 Swash 可操作切换，状态栏同步反映实际生效后端。
#[test]
fn text_render_backend_interactive_toggle_and_report_sync() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;
    let cosmic_btn_node = target::id(Target::Demo(page_index, 0, PART_BACKEND_COSMIC));
    let dw_btn_node = target::id(Target::Demo(page_index, 0, PART_BACKEND_DW));

    // 1. 点击切换到 Cosmic (Swash) 后端
    app.click_node(cosmic_btn_node);
    assert_eq!(
        app.state.text_render.report.backend,
        RasterBackend::Cosmic,
        "effective backend must switch to Cosmic"
    );
    assert_eq!(
        app.state.text_render.settings.raster_backend,
        RasterBackend::Cosmic
    );

    // 2. 点击切换回 DirectWrite 后端
    #[cfg(windows)]
    {
        app.click_node(dw_btn_node);
        assert_eq!(
            app.state.text_render.report.backend,
            RasterBackend::DirectWrite,
            "effective backend must switch to DirectWrite on Windows"
        );
        assert_eq!(
            app.state.text_render.settings.raster_backend,
            RasterBackend::DirectWrite
        );
    }
}

/// 验收条件 2：字体组四项（家族、字重、字号、斜体）三态呈现均为可用（Available）。
#[test]
fn font_group_tri_state_capabilities_available() {
    let caps = TextCapabilities::probe();
    let settings = TextRenderSettings::default();

    let font_keys = [
        TextFeatureKey::FontFamily,
        TextFeatureKey::FontWeight,
        TextFeatureKey::FontSize,
        TextFeatureKey::FontItalic,
    ];

    for key in font_keys {
        let res = resolve_feature(key, &caps, &settings);
        assert!(
            !res.disabled,
            "font feature {key:?} must be available in tri-state query"
        );
        assert_eq!(
            res.reason, None,
            "available font feature must not have a disabled reason"
        );
    }
}

/// 验收条件 1、2：字体组四项控件交互、状态更新与右侧实时预览渲染。
#[test]
fn font_group_controls_interaction_and_live_preview() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 验证初始状态：默认字体设置
    assert_eq!(app.state.text_render.settings.family, None);
    assert_eq!(app.state.text_render.settings.weight, 400);
    assert_eq!(app.state.text_render.settings.size, 14.0);
    assert!(!app.state.text_render.settings.italic);

    // 1. 切换字体家族：点击 FONT_FAMILY_OPTIONS 中的 Cascadia (索引 2)
    assert!(FONT_FAMILY_OPTIONS.len() > 2);
    let cascadia_node = target::id(Target::Demo(page_index, 0, FONT_FAMILY_PART_BASE + 2));
    app.click_node(cascadia_node);
    assert_eq!(
        app.state.text_render.settings.family.as_deref(),
        FONT_FAMILY_OPTIONS[2].1,
        "font family must update to Cascadia Mono"
    );

    // 2. 切换字重：点击 FONT_WEIGHT_OPTIONS 中的 Bold 700 (索引 3)
    assert!(FONT_WEIGHT_OPTIONS.len() > 3);
    let bold_node = target::id(Target::Demo(page_index, 0, FONT_WEIGHT_PART_BASE + 3));
    app.click_node(bold_node);
    assert_eq!(
        app.state.text_render.settings.weight, FONT_WEIGHT_OPTIONS[3].1,
        "font weight must update to 700"
    );

    // 3. 切换字号：点击 FONT_SIZE_OPTIONS 中的 24px (索引 3)
    assert!(FONT_SIZE_OPTIONS.len() > 3);
    let size_node = target::id(Target::Demo(page_index, 0, FONT_SIZE_PART_BASE + 3));
    app.click_node(size_node);
    assert_eq!(
        app.state.text_render.settings.size, FONT_SIZE_OPTIONS[3].1,
        "font size must update to 24.0"
    );

    // 4. 切换斜体：点击 FONT_ITALIC_PART 开关
    let italic_node = target::id(Target::Demo(page_index, 0, FONT_ITALIC_PART));
    app.click_node(italic_node);
    assert!(
        app.state.text_render.settings.italic,
        "italic setting must toggle to true"
    );

    // 5. 重绘后验证图元发射与预览渲染正常
    app.rebuild(VIEWPORT);
    let primitives_count = app.scene.primitives.len();
    assert!(
        primitives_count > 0,
        "primitives must be emitted under modified preview style"
    );
}

/// 验收条件 1：预览样式修改不影响其他页面文本。
#[test]
fn font_preview_style_isolation_from_other_pages() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 修改预览样式：设置大字号、粗体、斜体
    let size_node = target::id(Target::Demo(page_index, 0, FONT_SIZE_PART_BASE + 3));
    app.click_node(size_node);
    let bold_node = target::id(Target::Demo(page_index, 0, FONT_WEIGHT_PART_BASE + 3));
    app.click_node(bold_node);
    let italic_node = target::id(Target::Demo(page_index, 0, FONT_ITALIC_PART));
    app.click_node(italic_node);

    assert_eq!(app.state.text_render.settings.size, 24.0);
    assert_eq!(app.state.text_render.settings.weight, 700);
    assert!(app.state.text_render.settings.italic);

    // 导航到排版页面
    app.navigate(PageId::Typography);
    app.rebuild(VIEWPORT);
    let typography_primitives = app.scene.primitives.len();
    assert!(
        typography_primitives > 0,
        "typography page must render without error"
    );

    // 导航到按钮页面
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);
    let buttons_primitives = app.scene.primitives.len();
    assert!(
        buttons_primitives > 0,
        "buttons page must render without error"
    );

    // 导航回文本渲染控制台：预览设置依然保持
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.text_render.settings.size, 24.0);
    assert_eq!(app.state.text_render.settings.weight, 700);
    assert!(app.state.text_render.settings.italic);
}

/// 验收条件 1、2：展台控制台像素组亚像素相位档位运行期切换（8 / 4 / 1）与状态栏同步。
#[test]
fn text_render_phase_mode_runtime_switching_and_status_sync() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 1. 默认初始状态：8 相位
    assert_eq!(app.state.text_render.settings.phase_mode, PhaseMode::Eight);
    assert_eq!(app.state.text_render.report.phases, 8);
    let status_node_id = target::id(Target::Demo(page_index, 0, STATUS_BAR_PART));
    let status_node = app
        .semantics
        .get(status_node_id)
        .expect("status bar a11y node exists");
    assert!(
        status_node.label.contains("8"),
        "status bar a11y label must indicate 8 phases"
    );

    // 验证相位选项 8/4/1 控件均正常呈现于语义树中且处于可用状态
    for i in 0..3 {
        let opt_node_id = target::id(Target::Demo(page_index, 0, PHASE_PART_BASE + i));
        let opt_node = app
            .semantics
            .get(opt_node_id)
            .unwrap_or_else(|| panic!("missing a11y node for phase option {i}"));
        assert_eq!(opt_node.role, Role::RadioButton);
        if i == 0 {
            assert!(opt_node.selected);
        } else {
            assert!(!opt_node.selected);
        }
    }

    // 2. 点击切换到 4 档位（PHASE_PART_BASE + 1）
    let phase_4_node = target::id(Target::Demo(page_index, 0, PHASE_PART_BASE + 1));
    app.click_node(phase_4_node);
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.text_render.settings.phase_mode, PhaseMode::Four);
    assert_eq!(
        app.state.text_render.report.phases, 4,
        "effective report must update to 4 phases"
    );
    assert_eq!(app.labels.cache.phase_mode(), PhaseMode::Four);
    let opt_node_4 = app.semantics.get(phase_4_node).expect("a11y node exists");
    assert!(opt_node_4.selected);

    // 3. 点击切换到 1 档位（PHASE_PART_BASE + 2）
    let phase_1_node = target::id(Target::Demo(page_index, 0, PHASE_PART_BASE + 2));
    app.click_node(phase_1_node);
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.text_render.settings.phase_mode, PhaseMode::One);
    assert_eq!(
        app.state.text_render.report.phases, 1,
        "effective report must update to 1 phase"
    );
    assert_eq!(app.labels.cache.phase_mode(), PhaseMode::One);
    let opt_node_1 = app.semantics.get(phase_1_node).expect("a11y node exists");
    assert!(opt_node_1.selected);

    // 4. 点击切回 8 档位（PHASE_PART_BASE + 0）
    let phase_8_node = target::id(Target::Demo(page_index, 0, PHASE_PART_BASE));
    app.click_node(phase_8_node);
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.text_render.settings.phase_mode, PhaseMode::Eight);
    assert_eq!(
        app.state.text_render.report.phases, 8,
        "effective report must restore to 8 phases"
    );
    assert_eq!(app.labels.cache.phase_mode(), PhaseMode::Eight);
    let opt_node_8 = app.semantics.get(phase_8_node).expect("a11y node exists");
    assert!(opt_node_8.selected);
}

/// 验收条件 1、2、3：初始化失败静默回退 swash，状态栏显示实际后端并计数递增，切页不复位。
#[test]
fn text_render_init_failure_fallback_and_counter_persistence() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;
    let cosmic_btn_node = target::id(Target::Demo(page_index, 0, PART_BACKEND_COSMIC));
    let dw_btn_node = target::id(Target::Demo(page_index, 0, PART_BACKEND_DW));
    let status_node_id = target::id(Target::Demo(page_index, 0, STATUS_BAR_PART));

    // 初始状态下先切到 Cosmic 确保当前不在 DirectWrite
    app.click_node(cosmic_btn_node);
    assert_eq!(
        app.state.text_render.report.backend,
        RasterBackend::Cosmic,
        "pre-check: must switch to Cosmic initially"
    );
    let initial_fallback_count = app.state.text_render.report.fallback_count;

    // 注入 DirectWrite 强制初始化失败环境
    std::env::set_var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL", "1");

    // 点击切换至 DirectWrite
    app.click_node(dw_btn_node);

    // 清理环境变量
    std::env::remove_var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL");

    // 验证实际生效后端回退到 Cosmic
    assert_eq!(
        app.state.text_render.report.backend,
        RasterBackend::Cosmic,
        "effective backend must fallback to Cosmic on initialization failure"
    );

    // 回退计数必须单调递增
    let updated_fallback_count = app.state.text_render.report.fallback_count;
    assert_eq!(
        updated_fallback_count,
        initial_fallback_count + 1,
        "fallback count must increment upon fallback"
    );

    // 展开状态栏回退原因详情
    if !app.state.text_render.fallback_details_expanded {
        app.click_node(status_node_id);
    }
    assert!(app.state.text_render.fallback_details_expanded);
    assert!(app.state.text_render.report.last_fallback_reason.is_some());

    // 切页测试：切至 Buttons 页面后重绘
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);

    // 再切回 TextRender 页面并重绘
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    // 验证切页与重绘后回退计数不复位
    assert_eq!(
        app.state.text_render.report.fallback_count, updated_fallback_count,
        "fallback count must not reset when navigating across pages"
    );
}

/// 验收条件 1、2：排版组六项控件全部呈现、禁用项严格表驱动灰化并注明具体原因。
#[test]
fn typography_group_controls_accessibility_and_table_driven_disabled() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 1. 验证塑形器选项：Cosmic 可用，Native 禁用且标注未实现具体原因
    let cosmic_node = app
        .semantics
        .get(target::id(Target::Demo(page_index, 0, PART_SHAPER_COSMIC)))
        .expect("missing cosmic shaper node");
    assert!(!cosmic_node.disabled);

    let native_node = app
        .semantics
        .get(target::id(Target::Demo(page_index, 0, PART_SHAPER_NATIVE)))
        .expect("missing native shaper node");
    assert!(native_node.disabled);
    assert!(
        native_node
            .label
            .contains("Native and simple shapers not implemented"),
        "disabled native shaper must indicate concrete unimplemented reason, got: {}",
        native_node.label
    );

    // 点击禁用项 Native 不会改变设置，依然保持为 Cosmic
    app.click_node(target::id(Target::Demo(page_index, 0, PART_SHAPER_NATIVE)));
    assert_eq!(app.state.text_render.settings.shaper, TextShaper::Cosmic);

    // 2. 验证其余五项控件节点在语义树中全部存在
    let parts = [
        PART_LINEBREAK_WORD,
        PART_LINEBREAK_ANYWHERE,
        PART_LINEBREAK_NONE,
        PART_BIDI_ON,
        PART_BIDI_OFF,
        PART_LIGATURES_ON,
        PART_LIGATURES_OFF,
        PART_LETTER_SPACING_0,
        PART_LETTER_SPACING_1,
        PART_LETTER_SPACING_2,
        PART_LINE_HEIGHT_AUTO,
        PART_LINE_HEIGHT_20,
        PART_LINE_HEIGHT_28,
    ];
    for part in parts {
        let node_id = target::id(Target::Demo(page_index, 0, part));
        assert!(
            app.semantics.get(node_id).is_some(),
            "missing semantics node for part {part}"
        );
    }
}

/// 验收条件 1、4：排版组六项运行期切换、切页后依然保持、全局生效。
#[test]
fn typography_group_switching_updates_settings_and_persists_cross_page() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 1. 切换断行为 Anywhere
    app.click_node(target::id(Target::Demo(
        page_index,
        0,
        PART_LINEBREAK_ANYWHERE,
    )));
    assert_eq!(
        app.state.text_render.settings.line_break,
        LineBreakMode::Anywhere
    );

    // 2. 切换断行为 None
    app.click_node(target::id(Target::Demo(page_index, 0, PART_LINEBREAK_NONE)));
    assert_eq!(
        app.state.text_render.settings.line_break,
        LineBreakMode::None
    );

    // 3. 切换双向文本为 Off
    app.click_node(target::id(Target::Demo(page_index, 0, PART_BIDI_OFF)));
    assert!(!app.state.text_render.settings.bidi);

    // 4. 切换连字为 Off
    app.click_node(target::id(Target::Demo(page_index, 0, PART_LIGATURES_OFF)));
    assert!(!app.state.text_render.settings.ligatures);

    // 5. 切换字距为 2.0 px
    app.click_node(target::id(Target::Demo(
        page_index,
        0,
        PART_LETTER_SPACING_2,
    )));
    assert_eq!(app.state.text_render.settings.letter_spacing, 2.0);

    // 6. 切换行高为 28.0 px
    app.click_node(target::id(Target::Demo(page_index, 0, PART_LINE_HEIGHT_28)));
    assert_eq!(app.state.text_render.settings.line_height, 28.0);

    // 7. 切换页面到其他页面（如 Buttons），验证全局生效且设置保持
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);

    // 全局标签引擎 settings 必须已同步当前设置
    assert_eq!(app.labels.cache.settings().line_break, LineBreakMode::None);
    assert!(!app.labels.cache.settings().bidi);
    assert!(!app.labels.cache.settings().ligatures);
    assert_eq!(app.labels.cache.settings().letter_spacing, 2.0);
    assert_eq!(app.labels.cache.settings().line_height, 28.0);

    // 8. 重新导航回 TextRender 页面，验证各项设置保持不变
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    assert_eq!(
        app.state.text_render.settings.line_break,
        LineBreakMode::None
    );
    assert!(!app.state.text_render.settings.bidi);
    assert!(!app.state.text_render.settings.ligatures);
    assert_eq!(app.state.text_render.settings.letter_spacing, 2.0);
    assert_eq!(app.state.text_render.settings.line_height, 28.0);
}

/// 验收条件 1、4、5、6：Emoji 组控件交互、表驱动灰化、实际路径在状态栏反映且切页保持。
#[test]
fn emoji_group_controls_interaction_and_status_sync() {
    let mut app = app();
    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    let page_index = PageId::TextRender.index() as u8;

    // 1. 初始状态：Emoji 呈现为 Color，状态栏输出包含 Swash
    assert_eq!(
        app.state.text_render.settings.emoji_presentation,
        EmojiPresentation::Color
    );
    assert!(
        app.state.text_render.report.emoji_path.contains("Swash")
            || app.state.text_render.report.emoji_path.contains("Color"),
        "initial emoji path must indicate color: {}",
        app.state.text_render.report.emoji_path
    );

    // 验证初始控件节点在语义树中呈现
    let color_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_PRES_COLOR,
        )))
        .expect("color node exists");
    assert!(!color_node.disabled);

    let follow_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_BACKEND_FOLLOW,
        )))
        .expect("follow node exists");
    assert!(!follow_node.disabled);

    let cosmic_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_BACKEND_COSMIC,
        )))
        .expect("cosmic node exists");
    assert!(!cosmic_node.disabled);

    let system_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_SOURCE_SYSTEM,
        )))
        .expect("system node exists");
    assert!(!system_node.disabled);
    assert_eq!(
        app.state.text_render.settings.emoji_source,
        EmojiSource::System
    );

    // 2. 验证来源三档灰化项：Builtin 与 Grid 必须禁用并给出具体原因
    let builtin_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_SOURCE_BUILTIN,
        )))
        .expect("builtin node exists");
    assert!(builtin_node.disabled);
    assert!(
        builtin_node.label.contains("Bundled fonts not implemented"),
        "builtin source disabled reason: {}",
        builtin_node.label
    );

    let grid_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_SOURCE_GRID,
        )))
        .expect("grid node exists");
    assert!(grid_node.disabled);
    assert!(
        grid_node.label.contains("Grid atlas fonts not implemented"),
        "grid source disabled reason: {}",
        grid_node.label
    );

    // 3. 在 Color 模式下，Emoji 光栅器 DirectWrite 必须禁用（未实现层遍历）
    let dw_node = app
        .semantics
        .get(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_BACKEND_DW,
        )))
        .expect("emoji dw node exists");
    assert!(
        dw_node.disabled,
        "Color + DirectWrite must be disabled in Color mode"
    );
    assert!(
        dw_node.label.contains("layer traversal"),
        "Color + DirectWrite disabled reason: {}",
        dw_node.label
    );

    // 4. 点击切换到 Mono 呈现
    app.click_node(target::id(Target::Demo(
        page_index,
        0,
        PART_EMOJI_PRES_MONO,
    )));
    app.rebuild(VIEWPORT);

    assert_eq!(
        app.state.text_render.settings.emoji_presentation,
        EmojiPresentation::Mono
    );
    assert!(
        app.state.text_render.report.emoji_path.contains("Mono"),
        "effective emoji path must update to Mono: {}",
        app.state.text_render.report.emoji_path
    );

    // 5. 切换到 Mono 后，在 Windows 上 Emoji 光栅器 DirectWrite 应变为可用
    #[cfg(windows)]
    {
        let dw_node = app
            .semantics
            .get(target::id(Target::Demo(
                page_index,
                0,
                PART_EMOJI_BACKEND_DW,
            )))
            .expect("emoji dw node exists");
        assert!(
            !dw_node.disabled,
            "Mono + DirectWrite must be enabled on Windows"
        );

        // 点击切换到 DirectWrite 光栅器
        app.click_node(target::id(Target::Demo(
            page_index,
            0,
            PART_EMOJI_BACKEND_DW,
        )));
        app.rebuild(VIEWPORT);

        assert_eq!(
            app.state.text_render.settings.emoji_backend,
            EmojiBackend::DirectWrite
        );
        assert_eq!(
            app.state.text_render.report.emoji_path,
            "DirectWrite (Mono R8)"
        );
    }

    // 6. 跨页持久化：切到 Buttons 页面再切回
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);

    assert_eq!(
        app.labels.cache.settings().emoji_presentation,
        EmojiPresentation::Mono
    );

    app.navigate(PageId::TextRender);
    app.rebuild(VIEWPORT);

    assert_eq!(
        app.state.text_render.settings.emoji_presentation,
        EmojiPresentation::Mono
    );
}

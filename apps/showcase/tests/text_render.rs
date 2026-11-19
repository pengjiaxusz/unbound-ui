//! 文本渲染控制台骨架、状态栏与能力灰化机制的无头集成测试。
//! lang-allow: CJK page metadata assertions and specimen samples

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::text_render::{
    resolve_feature, FeatureResolution, TextConsoleGroup, TextFeatureKey, FEATURE_RULES,
    GROUP_PART_BASE, PART_BIDI_OFF, PART_BIDI_ON, PART_LETTER_SPACING_0, PART_LETTER_SPACING_1,
    PART_LETTER_SPACING_2, PART_LIGATURES_OFF, PART_LIGATURES_ON, PART_LINEBREAK_ANYWHERE,
    PART_LINEBREAK_NONE, PART_LINEBREAK_WORD, PART_LINE_HEIGHT_20, PART_LINE_HEIGHT_28,
    PART_LINE_HEIGHT_AUTO, PART_SHAPER_COSMIC, PART_SHAPER_NATIVE, STATUS_BAR_PART,
};
use showcase::pages::{CategoryId, PageId};
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::{AppRuntime, Locale};
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_text::{
    CapabilityStatus, LineBreakMode, RasterBackend, TextCapabilities, TextRenderSettings,
    TextShaper,
};

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

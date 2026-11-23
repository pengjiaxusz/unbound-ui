//! Showcase 全量双语（en-US / zh-CN）切换回归门禁测试（#174）。
//! lang-allow: bilingual i18n regression gate tests; Chinese is expected zh-CN output

use unbound_a11y::Semantics;
use unbound_app::Locale;

use showcase::pages::{CategoryId, KnobSpec, PageId};
use showcase::shell::{GalleryApp, Interaction};
use showcase::target::{self, Target};

const VIEWPORT: (f32, f32) = (1280.0, 800.0);

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

fn collect_semantics_labels(semantics: &Semantics) -> Vec<String> {
    let mut labels = Vec::new();
    let mut stack = vec![semantics.root()];
    while let Some(id) = stack.pop() {
        if let Some(node) = semantics.get(id) {
            labels.push(node.label.clone());
        }
        for &child in semantics.children_of(id) {
            stack.push(child);
        }
    }
    labels
}

#[test]
fn all_pages_and_categories_translate_to_chinese() {
    for category in CategoryId::ALL {
        let en = category.localized_label(Locale::EnUs);
        let zh = category.localized_label(Locale::ZhCn);
        assert_ne!(en, zh, "Category {:?} 未切换为中文", category);
        assert!(
            contains_cjk(zh),
            "Category {:?} 中文标签应包含汉字: {zh}",
            category
        );
    }

    for page in PageId::ALL {
        let en = page.localized_info(Locale::EnUs);
        let zh = page.localized_info(Locale::ZhCn);
        assert!(
            contains_cjk(zh.title),
            "Page {:?} 中文标题应包含汉字: {}",
            page,
            zh.title
        );
        assert!(
            contains_cjk(zh.short),
            "Page {:?} 中文短标题应包含汉字: {}",
            page,
            zh.short
        );
        assert!(
            contains_cjk(zh.description),
            "Page {:?} 中文描述应包含汉字: {}",
            page,
            zh.description
        );
        assert_ne!(
            en.description, zh.description,
            "Page {:?} 描述未随 Locale::ZhCn 切换",
            page
        );
    }
}

#[test]
fn all_knob_specs_and_example_headers_translate_to_chinese() {
    let mut total_examples = 0usize;
    let mut total_knobs = 0usize;

    // 保留不翻译的数字、代码标识、字形样本或拼音检索词选项。
    let untranslatable_options: &[&str] = &[
        "1 k",
        "10 k",
        "100 k",
        "1 s",
        "5 s",
        "10 s",
        "1,234.5",
        "98,765.43",
        "1,284,567.89",
        "2",
        "3",
        "4",
        "5",
        "6",
        "32",
        "128",
        "512",
        "1024",
        "2048",
        "∞",
        "CJK",
        "DOM",
        "O G C Q S 0 8 @",
        "永 国 回 曲 弯",
        "app",
        "editor",
        "fallback",
        "voseqi",
        "cdviqq",
        "vsqgx",
        "chongming",
        "cmwl",
    ];

    for page in PageId::ALL {
        for (card_idx, example) in page.examples().iter().enumerate() {
            total_examples += 1;
            let (en_title, en_desc) = example.localized_header(Locale::EnUs);
            let (zh_title, zh_desc) = example.localized_header(Locale::ZhCn);

            assert!(
                contains_cjk(zh_title),
                "Page {:?} card {card_idx} 中文标题缺失汉字: {zh_title} (en: {en_title})",
                page
            );
            assert!(
                contains_cjk(zh_desc),
                "Page {:?} card {card_idx} 中文描述缺失汉字（疑似回退英文）: {zh_desc} (en: {en_desc})",
                page
            );
            assert_ne!(
                en_desc, zh_desc,
                "Page {:?} card {card_idx} 示例描述在 ZhCn 下仍与英文相同",
                page
            );

            for (knob_idx, knob) in example.knobs.iter().enumerate() {
                total_knobs += 1;
                let en_label = knob.localized_label(Locale::EnUs);
                let zh_label = knob.localized_label(Locale::ZhCn);
                assert!(
                    contains_cjk(zh_label),
                    "Page {:?} card {card_idx} knob {knob_idx} 中文标签缺失汉字: {zh_label} (en: {en_label})",
                    page
                );
                assert_ne!(
                    en_label, zh_label,
                    "Page {:?} card {card_idx} knob {knob_idx} 标签未切换: {en_label}",
                    page
                );

                if let Some(opts) = knob.options() {
                    let zh_opts = knob
                        .localized_options(Locale::ZhCn)
                        .expect("Segmented knob must return localized_options");
                    assert_eq!(opts.len(), zh_opts.len());
                    for (&en_opt, &zh_opt) in opts.iter().zip(zh_opts.iter()) {
                        if !untranslatable_options.contains(&en_opt) {
                            assert!(
                                contains_cjk(zh_opt),
                                "Page {:?} card {card_idx} knob {knob_idx} 选项未翻译为中文: en={en_opt:?} zh={zh_opt:?}",
                                page
                            );
                        }
                    }
                }
            }
        }
    }

    assert!(
        total_examples >= 150,
        "预期覆盖全部页面与组件示例（实际 {total_examples}）"
    );
    assert!(
        total_knobs >= 100,
        "预期覆盖全部公共 KnobSpec 旋钮（实际 {total_knobs}）"
    );
    // 抽检方向选项确保 Top/Bottom/Left/Right 翻译正确
    assert_eq!(KnobSpec::localized_option("Top", Locale::ZhCn), "上方");
    assert_eq!(KnobSpec::localized_option("Bottom", Locale::ZhCn), "下方");
    assert_eq!(KnobSpec::localized_option("Left", Locale::ZhCn), "左侧");
    assert_eq!(KnobSpec::localized_option("Right", Locale::ZhCn), "右侧");
}

#[test]
fn shell_chrome_editor_menu_and_window_captions_translate_to_chinese() {
    let mut app = GalleryApp::new();
    app.rebuild(VIEWPORT, Interaction::default());

    // 1. 英文模式下右键打开搜索框上下文菜单
    let search_pos = app.layout.search.center();
    app.open_editor_menu(Target::Search, search_pos, true);
    app.rebuild(VIEWPORT, Interaction::default());
    for (idx, expected) in [(0u8, "Cut"), (1, "Copy"), (2, "Paste"), (4, "Select all")] {
        let node = app
            .semantics
            .get(target::id(Target::EditorMenuItem(idx)))
            .expect("editor menu item a11y node must exist in EnUs");
        assert_eq!(node.label, expected);
    }

    // 2. 切换至简体中文后，编辑器右键菜单与窗口控制按钮无障碍标签均切换为中文
    app.state.set_locale(Locale::ZhCn);
    app.rebuild(VIEWPORT, Interaction::default());
    for (idx, expected) in [(0u8, "剪切"), (1, "复制"), (2, "粘贴"), (4, "全选")] {
        let node = app
            .semantics
            .get(target::id(Target::EditorMenuItem(idx)))
            .expect("editor menu item a11y node must exist in ZhCn");
        assert_eq!(node.label, expected);
    }

    let min_label = &app
        .semantics
        .get(target::id(Target::WindowMinimize))
        .expect("minimize a11y node")
        .label;
    let max_label = &app
        .semantics
        .get(target::id(Target::WindowMaximize))
        .expect("maximize a11y node")
        .label;
    let close_label = &app
        .semantics
        .get(target::id(Target::WindowClose))
        .expect("close a11y node")
        .label;
    assert_eq!(min_label, "最小化窗口");
    assert_eq!(max_label, "最大化窗口");
    assert_eq!(close_label, "关闭窗口");

    // 最大化状态下恢复标签切为“向下还原”
    app.set_window_maximized(true);
    app.state.bump();
    app.rebuild(VIEWPORT, Interaction::default());
    let restore_label = &app
        .semantics
        .get(target::id(Target::WindowMaximize))
        .expect("restore a11y node")
        .label;
    assert_eq!(restore_label, "向下还原");
}

#[test]
fn eleven_demo_pages_switch_semantics_to_chinese() {
    let mut app = GalleryApp::new();
    app.state.set_locale(Locale::ZhCn);

    // 验证 11 个已双语化页面均可在 ZhCn 下无崩溃完整构建，且交互控件语义已切为中文
    let pages_with_expected_cjk_labels: &[(PageId, &str)] = &[
        (PageId::Backdrop, "材质"),
        (PageId::Viewport, "无限画布视口"),
        (PageId::Marquee, "框选操作表面"),
        (PageId::Undo, "撤销"),
        (PageId::ListView, "列表滚动条"),
        (PageId::Buttons, "主操作按钮"),
        (PageId::Toggles, "完成后发送通知"),
        (PageId::TextField, "项目名称"),
        (PageId::Overlays, "打开菜单"),
        (PageId::DragDrop, "卡片 1"),
        (PageId::Springs, "重播"),
        (PageId::A11y, "保存"),
        (PageId::Telemetry, "无限制帧率"),
        (PageId::TextRender, "分组: 字体与样式"),
    ];

    for &(page, expected_substr) in pages_with_expected_cjk_labels {
        app.state.navigate(page);
        app.rebuild(VIEWPORT, Interaction::default());
        let labels = collect_semantics_labels(&app.semantics);
        let found = labels.iter().any(|label| label.contains(expected_substr));
        assert!(
            found,
            "Page {:?} 切至 Locale::ZhCn 后未找到预期中文无障碍标签片段 {:?}",
            page, expected_substr
        );
    }

    // 其余展示型页面同样需在 ZhCn 下正常渲染（含旋钮中文切换）
    for page in [
        PageId::Colors,
        PageId::Typography,
        PageId::Fonts,
        PageId::Elevation,
        PageId::Vector,
        PageId::Flex,
        PageId::Assets,
        PageId::Settings,
        PageId::Systems,
        PageId::Components,
    ] {
        app.state.navigate(page);
        app.rebuild(VIEWPORT, Interaction::default());
        assert!(
            !app.semantics.is_empty(),
            "Page {:?} 在 ZhCn 下应产出有效语义树",
            page
        );
    }
}

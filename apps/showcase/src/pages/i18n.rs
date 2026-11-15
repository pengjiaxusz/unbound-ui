//! 原生国际化（i18n）与多语言资源包教学页（I18n）。
//! zh-strings-allow: i18n 教学页的双语词条，中文是演示数据
//!
//! 演示并教授如何在 Unbound UI 中使用引擎原生 `I18nBundle`、`Locale`、`AppContext::i18n`
//! 与 `ViewBuilder::t / t_args / t_plural`：
//! 1. **实时词条查询、命名空间与参数插值沙盒**（支持 `{name}` / `{{name}}`、单复数 `.zero` / `.one` / `.other` 与缺失键回退演练）。
//! 2. **区域感知的数字千分位、货币、日期格式化与 RTL 镜像方向检测实验室**。
//! 3. **三步快速接入指南与完整声明式代码示例**（如何在 `init` 注册双语词条、在标题栏/按钮触发 `LocaleChanged`、在 `view` 中零样板调用翻译）。

use unbound_app::{I18nBundle, LayoutDirection};
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant};

pub const I18N_INFO: PageInfo = PageInfo {
    title: "Native i18n & Localization",
    short: "Native i18n",
    description: "Built-in zero-overhead internationalization engine: Locale presets (English & Simplified Chinese), fallback chains, {name} interpolation, pluralization, locale-aware numbers/dates, and live runtime switching.",
    crates: &["unbound-app/src/i18n.rs", "unbound-text"],
    keywords: "i18n l10n locale language translation english chinese zh-CN en-US bundle format plural interpolation rtl",
};

pub const I18N_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Interactive i18n Bundle & Interpolation Playground",
        "Test live key lookup, {name} / {{count}} parameter interpolation, plural rules (.zero / .one / .other), and automatic fallback to en-US when a key is missing.",
        &["unbound-app/src/i18n.rs"],
        r#"// 1. Register bilingual dictionary in Application::init
ctx.i18n_mut().register_bilingual(&[
    ("welcome", "Welcome back, {user}!", "欢迎回来，{user}！"),
    ("tasks.zero", "No pending tasks", "暂无待办任务"),
    ("tasks.one", "1 pending task", "1 项待办任务"),
    ("tasks.other", "{count} pending tasks", "{count} 项待办任务"),
]);

// 2. Translate & interpolate inside Application::view
let title = view.t_args("welcome", &[("user", "Peng")]);
let summary = view.t_plural("tasks", state.task_count);"#,
        "Interactive i18n sandbox with language toggle buttons, task counter buttons, and live translation readout.",
        &[
            KnobSpec::segmented("Namespace", &["app", "editor", "fallback"], 0),
            KnobSpec::slider("Task Count", 0.0, 12.0, 1.0, 3.0, ""),
            KnobSpec::toggle("Simulate Missing Zh Key", false),
        ],
        DemoKind::I18n { mode: 0 },
    )
    .with_height(310.0),
    ExampleDef::new(
        "Locale-Aware Formatting & RTL Direction Inspector",
        "Inspect how I18nBundle formats numbers (thousands/decimal separators), calendar dates (MM/DD/YYYY vs YYYY年MM月DD日), and layout reading direction (LTR vs RTL).",
        &["unbound-app/src/i18n.rs", "unbound-layout"],
        r#"let bundle = ctx.i18n();
let num_str = bundle.format_number(1284567.89, 2);
let date_str = bundle.format_date(2026, 11, 15);
let dir: LayoutDirection = bundle.direction(); // Ltr or Rtl"#,
        "Four locale comparison rows demonstrating number, currency, date, and layout direction rules.",
        &[
            KnobSpec::slider("Decimals", 0.0, 4.0, 1.0, 2.0, ""),
            KnobSpec::segmented("Sample Amount", &["1,234.5", "98,765.43", "1,284,567.89"], 2),
            KnobSpec::toggle("Force RTL Override", false),
        ],
        DemoKind::I18n { mode: 1 },
    )
    .with_height(280.0),
    ExampleDef::new(
        "3-Step Integration Guide for Your Application",
        "Step-by-step tutorial showing how any Unbound UI app connects to the native i18n system via AppContext, ViewBuilder, and title bar language selector.",
        &["unbound-app", "unbound-widgets"],
        r#"// Step 1: Initialize bundle in init(&mut self, ctx: &mut AppContext)
ctx.i18n_mut().register_bilingual(&[("app.title", "My App", "我的应用")]);

// Step 2: Switch language in reduce() or via AppRuntime::set_locale()
ctx.set_locale_enum(Locale::ZhCn); // or Locale::EnUs

// Step 3: Read in view(&mut self, state, view: &mut ViewBuilder)
view.title_bar(view.t("app.title"));"#,
        "Three interactive step cards explaining registration, runtime switching, and declarative view binding.",
        &[KnobSpec::segmented(
            "Step Focus",
            &["1. Register", "2. Switch", "3. Render"],
            0,
        )],
        DemoKind::I18n { mode: 2 },
    )
    .with_height(270.0),
];

/// 渲染原生 i18n 教学页的三个交互舞台。
pub fn i18n_demo(ctx: &mut DemoCtx<'_, '_>, mode: u8) {
    match mode {
        0 => draw_bundle_playground(ctx),
        1 => draw_formatting_inspector(ctx),
        _ => draw_integration_guide(ctx),
    }
}

fn draw_bundle_playground(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let zh = ctx.state.locale.is_zh();
    let ns_idx = (ctx.knob(0).round().max(0.0) as usize).min(2);
    let knob_tasks = ctx.knob(1).round().max(0.0) as usize;
    let simulate_missing = ctx.knob_on(2);
    let total_tasks = knob_tasks.saturating_add(ctx.state.demo.i18n_counter as usize);

    let mut bundle = I18nBundle::with_locale(ctx.state.locale);
    bundle.register_bilingual(&[
        (
            "app:welcome",
            "Welcome back, {user}! Active locale: {lang}",
            "欢迎回来，{user}！当前语言区域：{lang}",
        ),
        (
            "editor:welcome",
            "Shader Workspace · {user} ({lang})",
            "着色器工作区 · {user}（{lang}）",
        ),
        (
            "fallback:welcome",
            "Fallback Demo · {user} ({lang})",
            "回退演示 · {user}（{lang}）",
        ),
        (
            "tasks.zero",
            "0 pending tasks in queue",
            "队列中暂无待办任务",
        ),
        (
            "tasks.one",
            "1 pending task in queue",
            "队列中有 1 项待办任务",
        ),
        (
            "tasks.other",
            "{count} pending tasks in queue",
            "队列中有 {count} 项待办任务",
        ),
    ]);
    bundle.register_locale(
        "en-US",
        &[(
            "only_en_key",
            "Fallback to en-US: This key is only registered in English!",
        )],
    );
    if !simulate_missing {
        bundle.register_locale(
            "zh-CN",
            &[(
                "only_en_key",
                "已命中 zh-CN 词条：右侧开启“模拟缺失”可观察自动回退 en-US！",
            )],
        );
    }

    // 顶部控制栏：快速切换全局语言 + 增减计数器
    let top_bar = Rect::new(
        rect.origin.x + 12.0,
        rect.origin.y + 8.0,
        rect.size.x - 24.0,
        36.0,
    );
    let btn_en = Rect::new(top_bar.origin.x, top_bar.origin.y, 108.0, 32.0);
    let btn_zh = Rect::new(btn_en.max().x + 8.0, top_bar.origin.y, 108.0, 32.0);
    let btn_add = Rect::new(btn_zh.max().x + 14.0, top_bar.origin.y, 128.0, 32.0);
    let btn_reset = Rect::new(btn_add.max().x + 8.0, top_bar.origin.y, 88.0, 32.0);

    widgets::button(
        ctx.canvas,
        ctx.part(1),
        btn_en,
        "English",
        if !zh {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        },
        true,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        btn_zh,
        "简体中文",
        if zh {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        },
        true,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(3),
        btn_add,
        if zh { "+ 新增任务" } else { "+ Add Task" },
        ButtonVariant::Secondary,
        true,
    );
    if btn_reset.max().x <= top_bar.max().x {
        widgets::button(
            ctx.canvas,
            ctx.part(4),
            btn_reset,
            if zh { "重置" } else { "Reset" },
            ButtonVariant::Subtle,
            true,
        );
    }

    let ns = ["app", "editor", "fallback"][ns_idx];
    let welcome_key = format!("{ns}:welcome");
    let interpolated = bundle.format(
        &welcome_key,
        &[("user", "Jiaxu"), ("lang", ctx.state.locale.tag())],
    );
    let plural_str = bundle.format_plural("tasks", total_tasks);
    let fallback_str = bundle.t("only_en_key");

    let rows = [
        (
            if zh {
                "当前激活区域"
            } else {
                "Active Locale"
            },
            format!(
                "{} ({})  ·  Fallback: {}",
                ctx.state.locale.native_name(),
                ctx.state.locale.tag(),
                bundle.fallback_locale()
            ),
            palette.primary,
        ),
        (
            if zh {
                "命名空间与插值"
            } else {
                "Interpolation"
            },
            interpolated,
            palette.text,
        ),
        (
            if zh {
                "单复数规则 (Plural)"
            } else {
                "Pluralization"
            },
            plural_str,
            palette.success,
        ),
        (
            if zh {
                "缺失键回退链"
            } else {
                "Fallback Chain"
            },
            fallback_str.to_owned(),
            if simulate_missing {
                palette.warning
            } else {
                palette.text_muted
            },
        ),
    ];

    let start_y = top_bar.max().y + 12.0;
    let avail_h = (rect.max().y - start_y - 8.0).max(120.0);
    let row_h = (avail_h / rows.len() as f32).clamp(32.0, 52.0);

    for (idx, (label, val, color)) in rows.iter().enumerate() {
        let y = start_y + idx as f32 * row_h;
        let row_rect = Rect::new(
            rect.origin.x + 12.0,
            y,
            rect.size.x - 24.0,
            (row_h - 6.0).max(26.0),
        );
        ctx.canvas.rect(
            row_rect,
            8.0,
            palette.card_bg,
            palette.card_border,
            1.0,
            0.0,
        );
        let badge_w = 132.0_f32.min(row_rect.size.x * 0.35);
        widgets::badge(
            ctx.canvas,
            Rect::new(
                row_rect.origin.x + 10.0,
                row_rect.center().y - 10.0,
                badge_w,
                20.0,
            ),
            label,
            palette.primary,
            palette.primary_soft,
        );
        let text_rect = Rect::new(
            row_rect.origin.x + badge_w + 22.0,
            row_rect.origin.y,
            (row_rect.size.x - badge_w - 32.0).max(40.0),
            row_rect.size.y,
        );
        ctx.canvas.begin_clip(text_rect);
        ctx.canvas
            .text(val, text_rect, 11.5, *color, AlignX::Start, AlignY::Center);
        ctx.canvas.end_clip();
    }
}

fn draw_formatting_inspector(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let zh = ctx.state.locale.is_zh();
    let decimals = (ctx.knob(0).round().max(0.0) as usize).min(4);
    let amount = match ctx.knob(1).round() as usize {
        0 => 1234.5,
        1 => 98765.43,
        _ => 1284567.89,
    };
    let force_rtl = ctx.knob_on(2);

    let locales = [
        (
            "en-US",
            if zh {
                "英语 (美国)"
            } else {
                "English (US)"
            },
        ),
        (
            "zh-CN",
            if zh {
                "简体中文 (中国)"
            } else {
                "Simplified Chinese"
            },
        ),
        (
            "de-DE",
            if zh {
                "德语 (欧洲格式)"
            } else {
                "German (DE)"
            },
        ),
        (
            "ar-SA",
            if zh {
                "阿拉伯语 (RTL)"
            } else {
                "Arabic (RTL)"
            },
        ),
    ];

    let header_h = 28.0;
    let header_rect = Rect::new(
        rect.origin.x + 12.0,
        rect.origin.y + 8.0,
        rect.size.x - 24.0,
        header_h,
    );
    ctx.canvas
        .rect(header_rect, 6.0, palette.primary_soft, 0, 0.0, 0.0);
    ctx.canvas.text(
        if zh {
            "区域标签 (Locale)          数字格式化 (format_number)          日期格式化 (format_date)          布局方向"
        } else {
            "Locale Tag                 Formatted Number                    Formatted Date                    Direction"
        },
        Rect::new(
            header_rect.origin.x + 12.0,
            header_rect.origin.y,
            header_rect.size.x - 24.0,
            header_rect.size.y,
        ),
        10.0,
        palette.primary,
        AlignX::Start,
        AlignY::Center,
    );

    let start_y = header_rect.max().y + 8.0;
    let row_h = ((rect.max().y - start_y - 10.0) / locales.len() as f32).clamp(32.0, 48.0);

    for (idx, (tag, name)) in locales.iter().enumerate() {
        let mut b = I18nBundle::new(*tag);
        if force_rtl {
            b.set_rtl_override(Some(true));
        }
        let num = b.format_number(amount, decimals);
        let date = b.format_date(2026, 11, 15);
        let dir = b.direction();
        let is_current = *tag == ctx.state.locale.tag();

        let y = start_y + idx as f32 * row_h;
        let row_r = Rect::new(
            rect.origin.x + 12.0,
            y,
            rect.size.x - 24.0,
            (row_h - 5.0).max(26.0),
        );
        ctx.canvas.rect(
            row_r,
            7.0,
            if is_current {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            if is_current {
                palette.primary
            } else {
                palette.card_border
            },
            1.0,
            0.0,
        );

        let col_w = row_r.size.x / 4.0;
        ctx.canvas.text(
            &format!("{tag} · {name}"),
            Rect::new(
                row_r.origin.x + 10.0,
                row_r.origin.y,
                col_w - 12.0,
                row_r.size.y,
            ),
            10.5,
            if is_current {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            &num,
            Rect::new(
                row_r.origin.x + col_w,
                row_r.origin.y,
                col_w - 12.0,
                row_r.size.y,
            ),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            &date,
            Rect::new(
                row_r.origin.x + col_w * 2.0,
                row_r.origin.y,
                col_w - 12.0,
                row_r.size.y,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        let dir_label = match dir {
            LayoutDirection::Ltr => "LTR (->)",
            LayoutDirection::Rtl => "RTL (<-)",
        };
        widgets::badge(
            ctx.canvas,
            Rect::new(row_r.max().x - 84.0, row_r.center().y - 9.0, 72.0, 18.0),
            dir_label,
            if dir == LayoutDirection::Rtl {
                palette.warning
            } else {
                palette.success
            },
            if dir == LayoutDirection::Rtl {
                palette.warning_soft
            } else {
                palette.success_soft
            },
        );
    }
}

fn draw_integration_guide(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let zh = ctx.state.locale.is_zh();
    let active_step = (ctx.knob(0).round().max(0.0) as usize).min(2);

    let steps = if zh {
        [
            (
                "第 1 步：注册多语言词条",
                "在 Application::init 中调用 ctx.i18n_mut().register_bilingual(&[(key, en, zh)]) 或 register_locale()，默认激活 en-US 并以此作为安全回退。",
                "ctx.i18n_mut().register_bilingual(&[(\"title\", \"Hello\", \"你好\")]);",
            ),
            (
                "第 2 步：绑定标题栏或设置切换",
                "在标题栏下拉菜单或快捷键事件中调用 ctx.set_locale_enum(Locale::ZhCn) 或 runtime.set_locale(\"zh-CN\")，自动触发全局重排与重绘。",
                "ctx.set_locale_enum(Locale::ZhCn); // 派发 LocaleChanged",
            ),
            (
                "第 3 步：在视图中声明式消费",
                "在 Application::view 中直接通过 view.t(\"key\")、view.t_args(\"key\", &[...]) 或 view.t_plural(\"key\", n) 获取当前语言文本。",
                "view.text_primary(ui_key!(\"hdr\"), view.t(\"title\"), 16.0, 600);",
            ),
        ]
    } else {
        [
            (
                "Step 1: Register Dictionaries",
                "Call ctx.i18n_mut().register_bilingual(&[(key, en, zh)]) inside Application::init. English (en-US) is active and serves as the fallback by default.",
                "ctx.i18n_mut().register_bilingual(&[(\"title\", \"Hello\", \"你好\")]);",
            ),
            (
                "Step 2: Switch Locale at Runtime",
                "Call ctx.set_locale_enum(Locale::ZhCn) or runtime.set_locale(\"zh-CN\") from a title bar selector or shortcut to trigger instant relayout.",
                "ctx.set_locale_enum(Locale::ZhCn); // emits LocaleChanged",
            ),
            (
                "Step 3: Consume in ViewBuilder",
                "Inside Application::view, call view.t(\"key\"), view.t_args(\"key\", &[...]), or view.t_plural(\"key\", count) with zero boilerplate.",
                "view.text_primary(ui_key!(\"hdr\"), view.t(\"title\"), 16.0, 600);",
            ),
        ]
    };

    let gap = 10.0;
    let card_w = ((rect.size.x - 24.0 - gap * 2.0) / 3.0).max(120.0);
    let card_h = (rect.size.y - 20.0).max(120.0);

    for (idx, (title, desc, snippet)) in steps.iter().enumerate() {
        let card = Rect::new(
            rect.origin.x + 12.0 + idx as f32 * (card_w + gap),
            rect.origin.y + 10.0,
            card_w,
            card_h,
        );
        let highlighted = idx == active_step;
        ctx.canvas.rect(
            card,
            10.0,
            if highlighted {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            if highlighted {
                palette.primary
            } else {
                palette.card_border
            },
            if highlighted { 1.5 } else { 1.0 },
            0.0,
        );
        ctx.canvas.begin_clip(card);
        ctx.canvas.text(
            title,
            Rect::new(
                card.origin.x + 12.0,
                card.origin.y + 12.0,
                card.size.x - 24.0,
                20.0,
            ),
            12.0,
            if highlighted {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text_wrapped(
            desc,
            Rect::new(
                card.origin.x + 12.0,
                card.origin.y + 38.0,
                card.size.x - 24.0,
                card.size.y - 96.0,
            ),
            10.0,
            palette.text_muted,
        );
        let code_box = Rect::new(
            card.origin.x + 10.0,
            card.max().y - 46.0,
            card.size.x - 20.0,
            36.0,
        );
        ctx.canvas.rect(
            code_box,
            6.0,
            palette.code_bg,
            palette.card_border,
            1.0,
            0.0,
        );
        ctx.canvas.begin_clip(code_box);
        ctx.canvas.text(
            snippet,
            Rect::new(
                code_box.origin.x + 8.0,
                code_box.origin.y,
                code_box.size.x - 16.0,
                code_box.size.y,
            ),
            9.0,
            palette.code_text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.end_clip();
        ctx.canvas.end_clip();
    }
}

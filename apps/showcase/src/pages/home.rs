//! 首页：首屏横幅、能力卡片与完整控件索引。

use accesskit::Role;
use unbound_app::Locale;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageId, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const INFO: PageInfo = PageInfo {
    title: "Unbound UI Gallery",
    short: "Home",
    description: "A live tour of the engine: renderer, text, layout, input, canvas, i18n and telemetry. Every example on this page is the framework drawing itself.",
    crates: &["unbound-gpu", "unbound-vector", "unbound-layout", "unbound-app"],
    keywords: "overview hero gallery start i18n",
};

/// 首屏 CTA 跳转的目标页面。
pub const HERO_TARGETS: [PageId; 2] = [PageId::Buttons, PageId::Viewport];

/// 四项核心特性展示。
pub const CAPABILITIES: [(&str, &str, PageId, Icon); 4] = [
    (
        "Typography",
        "MTSDF glyphs at every zoom",
        PageId::Typography,
        Icon::Layers,
    ),
    (
        "Virtualized List",
        "100 000 rows, 12 active nodes",
        PageId::ListView,
        Icon::Grid,
    ),
    (
        "Infinite Canvas",
        "Quadtree culling and snapping",
        PageId::Viewport,
        Icon::Snap,
    ),
    (
        "GPU Pipeline",
        "Uber shader, 4 draw calls",
        PageId::Telemetry,
        Icon::Chart,
    ),
];

/// 按当前语言返回四项核心特性的本地化标题与副标题。
const fn localized_capability(index: usize, locale: Locale) -> (&'static str, &'static str) {
    match locale {
        Locale::EnUs => (CAPABILITIES[index].0, CAPABILITIES[index].1),
        Locale::ZhCn => match index {
            0 => ("字体与排版", "任意缩放比下的高精 MTSDF 字形"), // lang-ok: zh-CN capability copy
            1 => ("虚拟化列表", "10 万行数据集，仅 12 个活跃节点"), // lang-ok: zh-CN capability copy
            2 => ("无限画布", "四叉树空间剔除与磁吸对齐"), // lang-ok: zh-CN capability copy
            _ => ("GPU 渲染管线", "统一超级着色器，单帧 4 次绘制调用"), // lang-ok: zh-CN capability copy
        },
    }
}

/// 完整目录网格。
pub const INDEX_PAGES: [PageId; 12] = [
    PageId::Colors,
    PageId::Typography,
    PageId::Buttons,
    PageId::Toggles,
    PageId::Sliders,
    PageId::TextField,
    PageId::Flex,
    PageId::Overlays,
    PageId::ListView,
    PageId::Viewport,
    PageId::I18n,
    PageId::A11y,
];

pub const EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Hero banner",
        "A composed surface: layered rounded rectangles, analytic shadows and Bézier curves, all drawn from the same primitive stream as the rest of the shell.",
        &["unbound-vector", "unbound-gpu"],
        r#"let hero = Surface::new(Rect::new(0.0, 0.0, 980.0, 208.0))
    .radius(20.0)
    .fill(Token::HERO_START);
scene.push(hero);
for (i, curve) in ribbons.iter().enumerate() {
    scene.curve(curve, 3.0, Token::ON_HERO, z + i);
}"#,
        "Heading 'Unbound UI Gallery' level 1; two link buttons 'Explore components' and 'Open canvas studio'.",
        &[KnobSpec::segmented("Density", &["Comfortable", "Compact"], 0)],
        DemoKind::Hero,
    )
    .with_height(250.0),
    ExampleDef::new(
        "Featured capabilities",
        "Four entry points into the framework's differentiating layers. Selecting a card navigates the shell exactly like a navigation row.",
        &["unbound-text", "unbound-world", "unbound-canvas", "unbound-kpi"],
        r#"CapabilityCard {
    title: "Virtualized List",
    metric: "100 000 rows",
    target: PageId::ListView,
}"#,
        "Four navigation links; each exposes its destination as the accessible name.",
        &[],
        DemoKind::CapabilityGrid,
    )
    .with_height(240.0),
    ExampleDef::new(
        "All controls",
        "The component index. Every tile is a live navigation target, which makes the home page itself a dogfooded NavigationView.",
        &["unbound-abi", "unbound-input"],
        r#"for page in PageId::ALL {
    grid.push(ControlTile::new(page.title(), page.category()));
}"#,
        "Twelve links arranged in a four-column grid; reading order follows the grid.",
        &[KnobSpec::segmented("Columns", &["3", "4", "6"], 1)],
        DemoKind::ControlIndex,
    )
    .with_height(300.0),
];

/// 首屏横幅。
pub fn hero(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    // 密度旋钮（`0` = 舒适，`1` = 紧凑）和较窄的舞台都会收紧横幅。
    // 读取该旋钮才能让右侧的分段控件真正起作用。
    //
    let compact = rect.size.x < 720.0 || ctx.knob_on(0);

    ctx.canvas
        .rect(rect, 20.0, palette.hero_start, 0, 0.0, 24.0);
    // 光斑与丝带故意设计得比首屏更大；应将其裁剪到圆角表面内，
    // 而非让裁剪矩形切出一条生硬的竖线并把圆角裁成直角。
    //
    ctx.canvas.begin_clip_rounded(rect, 20.0);
    // 两个柔和的光斑无需逐像素着色器即可营造渐变效果。
    ctx.canvas.rect(
        Rect::new(
            rect.max().x - rect.size.x * 0.52,
            rect.origin.y - rect.size.y * 0.45,
            rect.size.x * 0.78,
            rect.size.y * 1.5,
        ),
        rect.size.y * 0.75,
        palette.hero_end,
        0,
        0.0,
        0.0,
    );
    ctx.canvas.rect(
        Rect::new(
            rect.max().x - rect.size.x * 0.3,
            rect.max().y - rect.size.y * 0.42,
            rect.size.x * 0.34,
            rect.size.y * 0.42,
        ),
        90.0,
        palette.secondary,
        0,
        0.0,
        0.0,
    );

    // 装饰性丝带曲线。
    //
    // 它们位于右下角：文案占据首屏左侧 58%，若丝带穿过段落，
    // 会把文字变成背景纹理而非可读句子。
    //
    //
    // 丝带从首屏*下方*起步，并在进入时被圆角表面裁剪。
    // 若从紫色光斑内边缘起步，它们会在半空中戛然而止，看起来像破损的
    // 裁剪矩形，而非有意为之的设计。
    //
    for index in 0..4 {
        let f = index as f32;
        ctx.canvas.curve(
            [
                rect.origin.x + rect.size.x * (0.58 + f * 0.03),
                rect.max().y + 8.0,
            ],
            [
                rect.origin.x + rect.size.x * 0.86,
                rect.origin.y + rect.size.y * 0.55 + f * 14.0,
            ],
            [
                rect.max().x + 14.0,
                rect.origin.y + rect.size.y * 0.88 - f * 12.0,
            ],
            2.0,
            palette.on_hero,
        );
    }
    ctx.canvas.end_clip();

    let pad_x = if compact { 20.0 } else { 28.0 };
    let pad_y = if compact { 16.0 } else { 24.0 };
    let left = Rect::new(
        rect.origin.x + pad_x,
        rect.origin.y + pad_y,
        rect.size.x * if compact { 0.72 } else { 0.58 },
        rect.size.y - pad_y * 2.0,
    );
    let badge_width = if compact { 92.0 } else { 116.0 };
    widgets::badge(
        ctx.canvas,
        Rect::new(left.origin.x, left.origin.y, badge_width, 22.0),
        "v0.1 · GALLERY",
        palette.on_hero,
        palette.hero_end,
    );
    let title_size = if compact { 34.0 } else { 44.0 };
    let title_y = left.origin.y + if compact { 28.0 } else { 32.0 };
    ctx.canvas.text(
        ctx.state.t("home.hero.title"),
        Rect::new(
            left.origin.x,
            title_y,
            left.size.x,
            if compact { 42.0 } else { 54.0 },
        ),
        title_size,
        palette.on_hero,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text_wrapped(
        ctx.state.t("home.hero.subtitle"),
        Rect::new(
            left.origin.x,
            title_y + if compact { 44.0 } else { 54.0 },
            left.size.x,
            left.size.y - if compact { 108.0 } else { 132.0 },
        ),
        if compact { 12.0 } else { 13.0 },
        palette.on_hero,
    );

    let cta_y = left.max().y - if compact { 30.0 } else { 34.0 };
    let primary = Rect::new(
        left.origin.x,
        cta_y,
        if compact { 148.0 } else { 168.0 },
        if compact { 30.0 } else { 34.0 },
    );
    widgets::button(
        ctx.canvas,
        ctx.part(1),
        primary,
        ctx.state.t("home.hero.cta_primary"),
        ButtonVariant::OnHero,
        true,
    );
    let secondary = Rect::new(primary.max().x + 10.0, cta_y, 154.0, 34.0);
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        secondary,
        ctx.state.t("home.hero.cta_secondary"),
        ButtonVariant::OnHeroSecondary,
        true,
    );
    ctx.canvas.semantic(
        Role::Heading,
        ctx.state.t("app.title"),
        Rect::new(left.origin.x, left.origin.y + 32.0, left.size.x, 54.0),
    );
}

/// 四张能力卡片。
pub fn capability_grid(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let columns = if rect.size.x >= 620.0 { 2 } else { 1 };
    let gap = 12.0;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = (rect.size.y - gap) * 0.5;
    for (index, (_, _, page, icon)) in CAPABILITIES.iter().enumerate() {
        let (title, subtitle) = localized_capability(index, ctx.state.locale);
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        let page_info = page.localized_info(ctx.state.locale);
        let label = format!("{title} — {}", page_info.title);
        widgets::feature_card(
            ctx.canvas,
            ctx.part(index as u16 + 1),
            cell,
            *icon,
            title,
            subtitle,
            &label,
            index,
        );
        ctx.control(index as u16 + 1, cell, Role::Link, &label, false);
    }
}

/// 完整控件索引。
pub fn control_index(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let columns = match ctx.knob(0) as usize {
        0 => 3,
        2 => 6,
        _ => 4,
    };
    let gap = 8.0;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = 46.0;
    for (index, page) in INDEX_PAGES.iter().enumerate() {
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        if cell.max().y > rect.max().y {
            continue;
        }
        let hover = ctx.canvas.visual(ctx.part(index as u16 + 1), false);
        ctx.canvas.rect(
            cell,
            9.0,
            if hover.is_hovered() {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            palette.card_border,
            1.0,
            0.0,
        );
        ctx.canvas.rect(
            Rect::new(cell.origin.x + 8.0, cell.origin.y + 8.0, 30.0, 30.0),
            8.0,
            palette.stage_bg,
            0,
            0.0,
            0.0,
        );
        let info = page.localized_info(ctx.state.locale);
        let initial = info.short.chars().next().unwrap_or('U').to_string();
        ctx.canvas.text(
            &initial,
            Rect::new(cell.origin.x + 8.0, cell.origin.y + 8.0, 30.0, 30.0),
            14.0,
            palette.primary,
            AlignX::Center,
            AlignY::Center,
        );
        ctx.canvas.text(
            info.short,
            Rect::new(
                cell.origin.x + 46.0,
                cell.origin.y + 7.0,
                cell.size.x - 52.0,
                15.0,
            ),
            11.5,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            info.crates.first().copied().unwrap_or("unbound"),
            Rect::new(
                cell.origin.x + 46.0,
                cell.origin.y + 23.0,
                cell.size.x - 52.0,
                13.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.control(index as u16 + 1, cell, Role::Link, info.title, false);
    }
}

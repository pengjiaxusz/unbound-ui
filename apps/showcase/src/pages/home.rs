//! Home: the hero, the capability cards and the full control index.

use accesskit::Role;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageId, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const INFO: PageInfo = PageInfo {
    title: "Unbound UI Gallery",
    short: "Home",
    description: "A live tour of the engine: renderer, text, layout, input, canvas and telemetry. Every example on this page is the framework drawing itself.",
    crates: &["unbound-gpu", "unbound-vector", "unbound-layout"],
    docs: "docs/06-tooling/showcase.md",
    keywords: "overview hero gallery start",
};

/// The pages the hero CTAs jump to.
pub const HERO_TARGETS: [PageId; 2] = [PageId::Buttons, PageId::Viewport];

/// The four featured capabilities.
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

/// The full catalogue grid.
pub const INDEX_PAGES: [PageId; 12] = [
    PageId::Colors,
    PageId::Typography,
    PageId::Buttons,
    PageId::Toggles,
    PageId::Sliders,
    PageId::TextField,
    PageId::Flex,
    PageId::Overlays,
    PageId::DragDrop,
    PageId::ListView,
    PageId::Viewport,
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

/// The hero banner.
pub fn hero(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let compact = rect.size.x < 720.0;

    ctx.canvas
        .rect(rect, 20.0, palette.hero_start, 0, 0.0, 24.0);
    // The blobs and ribbons are larger than the hero on purpose; clip them to
    // the rounded surface instead of letting a scissor rectangle cut a hard
    // vertical line and square off the corners.
    ctx.canvas.begin_clip_rounded(rect, 20.0);
    // Two soft blobs create the gradient without a per-pixel shader.
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

    // Decorative ribbon curves.
    for index in 0..4 {
        let f = index as f32;
        ctx.canvas.curve(
            [
                rect.origin.x + rect.size.x * 0.42,
                rect.max().y - 26.0 - f * 9.0,
            ],
            [
                rect.origin.x + rect.size.x * 0.72,
                rect.origin.y + 18.0 + f * 22.0,
            ],
            [rect.max().x - 12.0, rect.origin.y + 70.0 + f * 24.0],
            2.0,
            palette.on_hero,
        );
    }
    ctx.canvas.end_clip();

    let left = Rect::new(
        rect.origin.x + 28.0,
        rect.origin.y + 24.0,
        rect.size.x * if compact { 0.72 } else { 0.58 },
        rect.size.y - 48.0,
    );
    let badge_width = if compact { 92.0 } else { 116.0 };
    widgets::badge(
        ctx.canvas,
        Rect::new(left.origin.x, left.origin.y, badge_width, 22.0),
        "v0.1 · GALLERY",
        palette.on_hero,
        palette.hero_end,
    );
    ctx.canvas.text(
        "Unbound UI",
        Rect::new(left.origin.x, left.origin.y + 32.0, left.size.x, 54.0),
        if compact { 34.0 } else { 44.0 },
        palette.on_hero,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text_wrapped(
        "An engine-level UI framework: GPU primitives, MTSDF text, Taffy layout, an infinite canvas and a headless verifier — dogfooded by the application you are looking at.",
        Rect::new(
            left.origin.x,
            left.origin.y + 86.0,
            left.size.x,
            left.size.y - 132.0,
        ),
        13.0,
        palette.on_hero,
    );

    let cta_y = left.max().y - 34.0;
    let primary = Rect::new(left.origin.x, cta_y, 168.0, 34.0);
    widgets::button(
        ctx.canvas,
        ctx.part(1),
        primary,
        "Explore components",
        ButtonVariant::Primary,
        true,
    );
    let secondary = Rect::new(primary.max().x + 10.0, cta_y, 154.0, 34.0);
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        secondary,
        "Open canvas studio",
        ButtonVariant::Secondary,
        true,
    );
    ctx.canvas.semantic(
        Role::Heading,
        "Unbound UI Gallery",
        Rect::new(left.origin.x, left.origin.y + 32.0, left.size.x, 54.0),
    );
}

/// Four capability cards.
pub fn capability_grid(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let columns = if rect.size.x >= 620.0 { 2 } else { 1 };
    let gap = 12.0;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = (rect.size.y - gap) * 0.5;
    for (index, (title, subtitle, page, icon)) in CAPABILITIES.iter().enumerate() {
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        let hover = ctx.canvas.visual(ctx.part(index as u16 + 1), false);
        ctx.canvas.rect(
            cell,
            12.0,
            if hover.is_hovered() {
                palette.control_hover
            } else {
                palette.card_bg
            },
            palette.card_border,
            1.0,
            4.0,
        );
        ctx.canvas.rect(
            Rect::new(cell.origin.x + 14.0, cell.origin.y + 14.0, 34.0, 34.0),
            9.0,
            match index {
                0 => palette.primary_soft,
                1 => palette.success_soft,
                2 => palette.warning_soft,
                _ => palette.secondary_soft,
            },
            0,
            0.0,
            0.0,
        );
        widgets::icon(
            ctx.canvas,
            *icon,
            Rect::new(cell.origin.x + 14.0, cell.origin.y + 14.0, 34.0, 34.0),
            match index {
                0 => palette.primary,
                1 => palette.success,
                2 => palette.warning,
                _ => palette.secondary,
            },
            palette.card_bg,
        );
        ctx.canvas.text(
            title,
            Rect::new(
                cell.origin.x + 58.0,
                cell.origin.y + 15.0,
                cell.size.x - 70.0,
                20.0,
            ),
            14.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text_wrapped(
            subtitle,
            Rect::new(
                cell.origin.x + 58.0,
                cell.origin.y + 36.0,
                cell.size.x - 72.0,
                cell.size.y - 48.0,
            ),
            11.5,
            palette.text_muted,
        );
        ctx.canvas.text(
            "→",
            Rect::new(cell.max().x - 30.0, cell.max().y - 30.0, 18.0, 18.0),
            15.0,
            palette.primary,
            AlignX::Center,
            AlignY::Center,
        );
        ctx.control(
            index as u16 + 1,
            cell,
            Role::Link,
            &format!("{title} — {}", page.info().title),
            false,
        );
    }
}

/// The complete control index.
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
        let info = page.info();
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

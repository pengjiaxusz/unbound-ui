//! Design Guidance: colour, type, elevation and vectors.

use unbound_math::f16::f16_to_f32;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::theme;
use crate::widgets;

pub const COLORS_INFO: PageInfo = PageInfo {
    title: "Color palette",
    short: "Color palette",
    description: "Semantic tokens are resolved once per theme into a 512-entry lookup table; primitives only carry a token index. Switching theme uploads no geometry.",
    crates: &["unbound-theme", "unbound-math"],
    docs: "docs/03-core/theme.md",
    keywords: "color token palette theme light dark contrast semantic",
};

pub const TYPOGRAPHY_INFO: PageInfo = PageInfo {
    title: "Typography",
    short: "Typography",
    description: "MTSDF glyph caches keep stems sharp from 9 pt captions to 60 pt display type, including extreme zoom and CJK fallback.",
    crates: &["unbound-text", "mtsdf.comp"],
    docs: "docs/03-core/text.md",
    keywords: "typography type mtsdf sdf font glyph scale sharpening",
};

pub const ELEVATION_INFO: PageInfo = PageInfo {
    title: "Elevation & clipping",
    short: "Elevation",
    description: "Rounded-rect clipping and analytic shadows share the same surface primitive. Radius, blur and border are all uniforms — no extra draw calls.",
    crates: &["unbound-gpu", "unbound-vector"],
    docs: "docs/03-core/vector.md",
    keywords: "elevation shadow radius corner clipping border surface",
};

pub const VECTOR_INFO: PageInfo = PageInfo {
    title: "Vector & curves",
    short: "Vector",
    description: "Bézier ribbons are expanded on the GPU. The same curve primitive powers the hero banner, focus rings and the infinite canvas minimap.",
    crates: &["unbound-vector", "uber.frag"],
    docs: "docs/03-core/vector.md",
    keywords: "vector bezier curve shape antialias gpu",
};

pub const COLOR_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Semantic swatches",
        "The twelve tokens the shell consumes. Labels show both the semantic role and the numeric slot stored in every primitive.",
        &["unbound-theme"],
        r#"let lut = theme::lut(ThemeMode::Dark);
let chip = Rect::new(x, y, 148.0, 72.0);
scene.rect(chip, 10.0, Token::CARD_BG, Token::CARD_BORDER, 1.0, 0.0);"#,
        "A definition list of twelve colour tokens; the swatch exposes the token name as its description.",
        &[
            KnobSpec::segmented("Columns", &["2", "3", "4"], 2),
            KnobSpec::toggle("Show slot ids", true),
        ],
        DemoKind::Palette { compare: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Light / dark parity",
        "Both themes are generated from the same slot layout, so a component that renders correctly in one theme renders correctly in the other.",
        &["unbound-theme"],
        r#"let light = palette();
let dark = palette();
assert_eq!(light.primary, dark.primary); // same slot, different value"#,
        "Two labelled groups, 'Light' and 'Dark', each listing the same six semantic roles.",
        &[
            KnobSpec::segmented("Role", &["Surface", "Text", "Status"], 0),
            KnobSpec::check("Hex labels", true),
        ],
        DemoKind::Palette { compare: true },
    )
    .with_height(260.0),
];

pub const TYPOGRAPHY_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Type scale",
        "A seven-step ramp. The body size is a live knob; every label re-shapes through the same cache, so dragging never invalidates atlases.",
        &["unbound-text", "unbound-text/src/sdf.rs"],
        r#"let ramp = [
    ("Display", 40.0), ("Title", 28.0), ("Subtitle", 20.0),
    ("Body", 15.0), ("Caption", 12.0), ("Mono", 11.0),
];
for (name, size) in ramp { scene.text(name, point, size, Token::TEXT); }"#,
        "A six-level heading and paragraph hierarchy; each row exposes its point size.",
        &[
            KnobSpec::slider("Body size", 12.0, 22.0, 1.0, 15.0, "pt"),
            KnobSpec::toggle("Ligatures", true),
        ],
        DemoKind::Typography { scaling: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Extreme scaling",
        "The glyph quad carries a screen-space derivative, so 400 % zoom reveals smooth MTSDF contours instead of magnified bitmaps.",
        &["unbound-text", "mtsdf.comp"],
        r#"camera.set_scale(4.0);
scene.text("Vector at any zoom", point, 26.0, Token::TEXT);"#,
        "A magnified text specimen; the zoom badge is exposed as a status.",
        &[
            KnobSpec::slider("Scale", 100.0, 400.0, 25.0, 200.0, "%"),
            KnobSpec::segmented("Fallback", &["Latin", "CJK", "Mixed"], 0),
        ],
        DemoKind::Typography { scaling: true },
    )
    .with_height(240.0),
];

pub const ELEVATION_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Elevation ladder",
        "Three analytic shadow levels. Blur and radius are single-precision uniforms, so smooth shadows cost the same as a flat rectangle.",
        &["unbound-gpu", "uber.frag"],
        r#"for level in 1..=3 {
    scene.rect(card, radius, Token::CARD_BG, Token::BORDER, 1.0, blur * level);
}"#,
        "Three cards labelled 'Low', 'Medium' and 'High' elevation.",
        &[
            KnobSpec::slider("Corner radius", 0.0, 32.0, 1.0, 16.0, "px"),
            KnobSpec::slider("Shadow blur", 0.0, 40.0, 1.0, 18.0, "px"),
            KnobSpec::check("Hairline border", true),
        ],
        DemoKind::Elevation { clipping: false },
    )
    .with_height(280.0),
    ExampleDef::new(
        "Rounded clipping",
        "Child content is clipped by the surface's SDF, including the shadow pass. Toggle overflow to see the clip boundary.",
        &["unbound-gpu"],
        r#"let surface = surface(rect, radius).clip_children();
scene.push(surface);
scene.rect(child_overflowing, 8.0, Token::PRIMARY, 0, 0.0, 0.0);"#,
        "A card containing an oversized child, with the clip radius exposed as a slider.",
        &[
            KnobSpec::slider("Clip radius", 0.0, 40.0, 1.0, 20.0, "px"),
            KnobSpec::toggle("Show overflow", false),
        ],
        DemoKind::Elevation { clipping: true },
    )
    .with_height(240.0),
];

pub const VECTOR_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Bézier ribbons",
        "Quadratic segments are stroked by the uber shader. The curvature knob moves the control point; stroke width stays pixel-stable under zoom.",
        &["unbound-vector", "uber.frag"],
        r#"scene.curve([24.0, 120.0], [320.0, 12.0], [620.0, 120.0], width, Token::PRIMARY);"#,
        "Five labelled curves; the control-point toggle reveals the construction geometry.",
        &[
            KnobSpec::slider("Stroke", 1.0, 8.0, 0.5, 3.0, "px"),
            KnobSpec::slider("Curvature", 0.0, 100.0, 1.0, 62.0, "%"),
            KnobSpec::check("Control points", false),
        ],
        DemoKind::Vector { shapes: false },
    )
    .with_height(260.0),
    ExampleDef::new(
        "Shape language",
        "Rounded surfaces, rings and ribbons share one primitive. This is the vocabulary the icons and the hero banner are built from.",
        &["unbound-vector"],
        r#"VectorShape::ring(center, radius, width)
    .fill(Token::PRIMARY_SOFT)
    .stroke(Token::PRIMARY, 2.0);"#,
        "A composition of four vector shapes with text labels.",
        &[
            KnobSpec::slider("Stroke", 1.0, 8.0, 0.5, 2.5, "px"),
            KnobSpec::toggle("Fill shapes", true),
        ],
        DemoKind::Vector { shapes: true },
    )
    .with_height(240.0),
];

fn hex(color: [u16; 4]) -> String {
    let channel = |bits: u16| {
        let value = f16_to_f32(bits);
        let linear = value.clamp(0.0, 1.0);
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    };
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

/// Colour tokens, optionally in light / dark parity mode.
pub fn palette(ctx: &mut DemoCtx<'_, '_>, compare: bool) {
    if compare {
        palette_compare(ctx);
    } else {
        palette_grid(ctx);
    }
}

fn palette_grid(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let columns = match ctx.knob(0) as usize {
        0 => 2,
        1 => 3,
        _ => 4,
    };
    let show_slots = ctx.knob_on(1);
    let gap = 10.0;
    let swatches = palette.swatches();
    let rows = swatches.len().div_ceil(columns);
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = ((rect.size.y - gap * (rows - 1) as f32) / rows as f32).clamp(56.0, 82.0);
    let table = theme::lut(ctx.state.theme.is_dark());
    for (index, (name, token)) in swatches.iter().enumerate() {
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
        ctx.canvas
            .rect(cell, 10.0, palette.card_bg, palette.card_border, 1.0, 2.0);
        let chip = Rect::new(
            cell.origin.x + 8.0,
            cell.origin.y + 8.0,
            cell.size.x - 16.0,
            30.0,
        );
        ctx.canvas.rect(chip, 6.0, *token, 0, 0.0, 0.0);
        ctx.canvas.text(
            name,
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + 42.0,
                cell.size.x - 20.0,
                14.0,
            ),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        let detail = if show_slots {
            format!("token {token} · {}", hex(table.entries()[*token as usize]))
        } else {
            hex(table.entries()[*token as usize])
        };
        ctx.canvas.text(
            &detail,
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + 56.0,
                cell.size.x - 20.0,
                12.0,
            ),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
    }
}

fn palette_compare(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let light = theme::lut(false);
    let dark = theme::lut(true);
    let roles: &[(&str, u16)] = match ctx.knob(0) as usize {
        1 => &[
            ("Text", palette.text),
            ("Muted", palette.text_muted),
            ("Primary", palette.primary),
            ("Border", palette.border),
        ],
        _ => &[
            ("Background", palette.background),
            ("Surface", palette.surface),
            ("Card", palette.card_bg),
            ("Stage", palette.stage_bg),
        ],
    };
    let show_hex = ctx.knob_on(1);

    let panel_w = (rect.size.x - 12.0) * 0.5;
    for (side, (label, table)) in [("Light", &light), ("Dark", &dark)].iter().enumerate() {
        let panel = Rect::new(
            rect.origin.x + side as f32 * (panel_w + 12.0),
            rect.origin.y,
            panel_w,
            rect.size.y,
        );
        // Both panels use the current shell's typography tokens so text stays
        // legible while the swatches show the corresponding theme value.
        ctx.canvas
            .rect(panel, 12.0, palette.card_bg, palette.card_border, 1.0, 4.0);
        ctx.canvas.text(
            label,
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 10.0,
                panel.size.x - 28.0,
                20.0,
            ),
            14.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            if *label == "Light" {
                "ThemeMode::Light"
            } else {
                "ThemeMode::Dark"
            },
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 30.0,
                panel.size.x - 28.0,
                14.0,
            ),
            10.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        for (index, (role, token)) in roles.iter().enumerate() {
            let row = Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 54.0 + index as f32 * 34.0,
                panel.size.x - 28.0,
                28.0,
            );
            ctx.canvas.rect(row, 7.0, *token, 0, 0.0, 0.0);
            let color = table.entries()[*token as usize];
            ctx.canvas.text(
                role,
                Rect::new(row.origin.x + 10.0, row.origin.y, 70.0, row.size.y),
                11.0,
                palette.text,
                AlignX::Start,
                AlignY::Center,
            );
            if show_hex {
                ctx.canvas.text(
                    &hex(color),
                    Rect::new(row.max().x - 80.0, row.origin.y, 72.0, row.size.y),
                    9.5,
                    palette.text,
                    AlignX::End,
                    AlignY::Center,
                );
            }
        }
    }
}

/// The type ramp, live.
pub fn typography(ctx: &mut DemoCtx<'_, '_>, scaling: bool) {
    if scaling {
        typography_scaling(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let body = ctx.knob(0);
    let ligatures = ctx.knob_on(1);
    let rows: [(&str, f32, &str); 5] = [
        ("Display", 34.0, "Unbound"),
        ("Title", 24.0, "Render everything"),
        ("Subtitle", 18.0, "GPU primitives, no DOM"),
        ("Body", body, "The quick brown fox jumps over the lazy dog."),
        ("Caption", 11.0, "MTSDF 512² atlas · 4× supersampled"),
    ];
    let mut y = rect.origin.y;
    for (name, size, sample) in rows {
        ctx.canvas.text(
            name,
            Rect::new(rect.origin.x, y, 74.0, size),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            sample,
            Rect::new(rect.origin.x + 82.0, y, rect.size.x - 82.0, size + 6.0),
            size,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        y += size + 15.0;
    }
    let sample = if ligatures {
        "office · affine · fi fl"
    } else {
        "office · affine"
    };
    ctx.canvas.divider(rect.origin.x, y + 2.0, rect.size.x);
    ctx.canvas.text(
        sample,
        Rect::new(rect.origin.x, y + 12.0, rect.size.x * 0.56, 24.0),
        15.0,
        palette.primary,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.text(
        "SDF / MTSDF",
        Rect::new(rect.max().x - 160.0, y + 12.0, 160.0, 24.0),
        10.0,
        palette.text_muted,
        AlignX::End,
        AlignY::Center,
    );
}

fn typography_scaling(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let scale = ctx.knob(0) / 100.0;
    let fallback = ctx.knob(1) as usize;
    let sample = match fallback {
        1 => "无界界面 · 矢量字形",
        2 => "Unbound 无界 · 400%",
        _ => "Unbound UI",
    };
    let size = (30.0 * scale).clamp(24.0, 120.0);
    ctx.canvas
        .rect(rect, 14.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.text(
        sample,
        Rect::new(
            rect.origin.x + 18.0,
            rect.origin.y,
            rect.size.x - 36.0,
            rect.size.y - 18.0,
        ),
        size,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    widgets::badge(
        ctx.canvas,
        Rect::new(rect.origin.x + 18.0, rect.max().y - 34.0, 96.0, 22.0),
        &format!("{:.0} % zoom", scale * 100.0),
        palette.primary,
        palette.primary_soft,
    );
    ctx.canvas.text(
        "screen-space derivatives / 1 px stem",
        Rect::new(
            rect.origin.x + 126.0,
            rect.max().y - 34.0,
            rect.size.x - 144.0,
            22.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

/// Elevation and clipping.
pub fn elevation(ctx: &mut DemoCtx<'_, '_>, clipping: bool) {
    if clipping {
        elevation_clipping(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let radius = ctx.knob(0);
    let blur = ctx.knob(1);
    let hairline = ctx.knob_on(2);
    for level in 0..3 {
        let card = Rect::new(
            rect.origin.x + level as f32 * (rect.size.x - 24.0) / 3.0 + 8.0,
            rect.origin.y + 18.0 + level as f32 * 8.0,
            (rect.size.x - 40.0) / 3.0,
            rect.size.y - 62.0,
        );
        ctx.canvas.rect(
            card,
            radius,
            palette.card_bg,
            if hairline { palette.card_border } else { 0 },
            if hairline { 1.0 } else { 0.0 },
            blur * (level as f32 + 1.0) * 0.55,
        );
        ctx.canvas.text(
            ["Low", "Medium", "High"][level],
            Rect::new(
                card.origin.x + 14.0,
                card.origin.y + 14.0,
                card.size.x - 28.0,
                18.0,
            ),
            13.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            &format!("{:.0} px blur", blur * (level as f32 + 1.0) * 0.55),
            Rect::new(
                card.origin.x + 14.0,
                card.origin.y + 34.0,
                card.size.x - 28.0,
                14.0,
            ),
            10.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.rect(
            Rect::new(card.origin.x + 14.0, card.max().y - 28.0, 38.0, 8.0),
            4.0,
            palette.primary_soft,
            0,
            0.0,
            0.0,
        );
    }
}

fn elevation_clipping(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let radius = ctx.knob(0);
    let overflow = ctx.knob_on(1);
    let card = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 12.0,
        rect.size.x - 48.0,
        rect.size.y - 24.0,
    );
    ctx.canvas.rect(
        card,
        radius,
        palette.card_bg,
        palette.card_border,
        1.0,
        10.0,
    );
    // A gradient bar intentionally larger than the surface.
    let child = Rect::new(
        card.origin.x - 18.0,
        card.origin.y + 34.0,
        card.size.x + 36.0,
        42.0,
    );
    ctx.canvas.begin_clip(card);
    ctx.canvas.rect(child, 0.0, palette.primary, 0, 0.0, 0.0);
    ctx.canvas.rect(
        Rect::new(
            child.origin.x + 20.0,
            child.origin.y + 10.0,
            child.size.x * 0.42,
            22.0,
        ),
        11.0,
        palette.hero_end,
        0,
        0.0,
        0.0,
    );
    ctx.canvas.text(
        "clipped by the surface SDF",
        Rect::new(
            card.origin.x + 18.0,
            card.origin.y + 42.0,
            card.size.x - 36.0,
            26.0,
        ),
        13.0,
        palette.on_hero,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.end_clip();
    ctx.canvas.text(
        if overflow {
            "overflow: children may paint outside the radius"
        } else {
            "overflow: clip"
        },
        Rect::new(
            card.origin.x + 16.0,
            card.max().y - 36.0,
            card.size.x - 32.0,
            20.0,
        ),
        10.5,
        if overflow {
            palette.warning
        } else {
            palette.text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    if overflow {
        ctx.canvas.rect(
            Rect::new(card.max().x - 42.0, card.origin.y + 66.0, 58.0, 26.0),
            8.0,
            palette.warning,
            0,
            0.0,
            6.0,
        );
    }
    widgets::badge(
        ctx.canvas,
        Rect::new(card.max().x - 96.0, card.origin.y + 12.0, 84.0, 20.0),
        "SDF clip",
        palette.primary,
        palette.primary_soft,
    );
}

/// Bézier ribbons and the shape vocabulary.
pub fn vector(ctx: &mut DemoCtx<'_, '_>, shapes: bool) {
    if shapes {
        vector_shapes(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let width = ctx.knob(0);
    let curvature = ctx.knob(1) / 100.0;
    let controls = ctx.knob_on(2);
    for index in 0..5 {
        let f = index as f32;
        let y = rect.origin.y + 24.0 + f * (rect.size.y - 48.0) / 5.0;
        let lift = (rect.size.y * 0.32) * curvature * if index % 2 == 0 { 1.0 } else { -1.0 };
        let p0 = [rect.origin.x + 10.0, y];
        let p1 = [rect.origin.x + rect.size.x * 0.52, y - lift];
        let p2 = [rect.max().x - 10.0, y];
        ctx.canvas.curve(p0, p1, p2, width, palette.primary);
        if controls {
            ctx.canvas.rect(
                Rect::new(p1[0] - 3.0, p1[1] - 3.0, 6.0, 6.0),
                3.0,
                palette.warning,
                0,
                0.0,
                0.0,
            );
            ctx.canvas.rect(
                Rect::new(p0[0] - 2.0, p0[1] - 2.0, 4.0, 4.0),
                2.0,
                palette.text_muted,
                0,
                0.0,
                0.0,
            );
            ctx.canvas.rect(
                Rect::new(p2[0] - 2.0, p2[1] - 2.0, 4.0, 4.0),
                2.0,
                palette.text_muted,
                0,
                0.0,
                0.0,
            );
        }
        ctx.canvas.text(
            &format!("c{index}"),
            Rect::new(rect.max().x - 28.0, y - 8.0, 24.0, 16.0),
            9.5,
            palette.text_muted,
            AlignX::End,
            AlignY::Center,
        );
    }
}

fn vector_shapes(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let stroke = ctx.knob(0);
    let fill = ctx.knob_on(1);
    let c = rect.center();
    let ring = Rect::new(c.x - 54.0, c.y - 54.0, 108.0, 108.0);
    ctx.canvas.rect(
        ring,
        54.0,
        if fill { palette.primary_soft } else { 0 },
        0,
        0.0,
        0.0,
    );
    ctx.canvas.stroke(ring, 54.0, palette.primary, stroke);
    ctx.canvas.rect(
        Rect::new(c.x - 26.0, c.y - 26.0, 52.0, 52.0),
        14.0,
        if fill { palette.secondary } else { 0 },
        0,
        0.0,
        0.0,
    );
    ctx.canvas.curve(
        [rect.origin.x + 12.0, c.y + 64.0],
        [c.x, rect.origin.y + 8.0],
        [rect.max().x - 12.0, c.y - 64.0],
        stroke * 1.4,
        palette.hero_end,
    );
    for (index, label) in ["ring", "rounded", "ribbon"].iter().enumerate() {
        ctx.canvas.text(
            label,
            Rect::new(
                rect.origin.x + 12.0 + index as f32 * 92.0,
                rect.max().y - 24.0,
                82.0,
                14.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

//! Multilingual fonts: script coverage, curve sharpness and BiDi sandbox.
//!
//! The type pages prove that one size ramp shares a cache; this page proves
//! that one *shaping* pipeline can carry twelve writing systems, including the
//! mark-heavy ones (Arabic harakat, Thai tone marks, Devanagari matras,
//! Vietnamese double accents) and the right-to-left ones.

use unbound_math::Rect;
use unbound_text::TextStyle;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets;

pub const FONTS_INFO: PageInfo = PageInfo {
    title: "Multilingual fonts",
    short: "Fonts & Scripts",
    description: "Twelve writing systems shaped by one pipeline: HarfBuzz mark offsets, BiDi levels and the system fallback chain are all live in this page.",
    crates: &["unbound-text", "cosmic-text", "rustybuzz"],
    docs: "docs/03-core/text.md",
    keywords: "font multilingual script cjk arabic hebrew thai devanagari cyrillic greek rtl fallback shaping",
};

pub const FONTS_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Global script matrix",
        "Twelve writing systems rendered at one size. The badge on the right is not a guess: it is the family the shaped run actually hit, read back from the font database.",
        &["unbound-text", "unicode-bidi"],
        r#"let style = TextStyle { font_size: 15.0, weight: 400, ..Default::default() };
let families = labels.resolved_families("天地玄黄", &style);
canvas.text_styled("天地玄黄", row, &style, palette.text, AlignX::Start, AlignY::Center);"#,
        "A definition list of twelve scripts; each row exposes the resolved fallback families as its description.",
        &[
            KnobSpec::segmented(
                "Script group",
                &[
                    "All (12)",
                    "East Asian (CJK)",
                    "European & Cyrillic",
                    "Complex & RTL",
                ],
                0,
            ),
            KnobSpec::slider("Font size", 11.0, 26.0, 1.0, 15.0, "pt"),
            KnobSpec::segmented("Weight", &["Regular 400", "Medium 500", "Bold 700"], 0),
        ],
        DemoKind::ScriptMatrix,
    )
    .with_height(560.0),
    ExampleDef::new(
        "Curve & stem sharpness",
        "The glyph atlas is a pure transparent gutter around the raster, so a curve's antialiased edge keeps its gradient. This stage is the zoom test: O, G, C, Q, S and their CJK counterparts.",
        &["unbound-text", "swash"],
        r#"// The gutter is transparent; coverage is copied verbatim.
let padding = 2; // texels, never an extruded edge ramp"#,
        "A seven-step size waterfall over corner-sensitive characters, with a five-weight row underneath.",
        &[
            KnobSpec::segmented(
                "Specimen",
                &["O G C Q S 0 8 @", "永 国 回 曲 弯", "Mixed pangram"],
                0,
            ),
            KnobSpec::toggle("Italic slant", false),
            KnobSpec::slider("Tracking / Scale", 100.0, 300.0, 10.0, 100.0, "%"),
        ],
        DemoKind::CurveInspector,
    )
    .with_height(500.0),
    ExampleDef::new(
        "Live multilingual & BiDi sandbox",
        "One run, three sizes. Numbers inside Arabic text keep their own direction and the fallback chain is listed under the line.",
        &["unbound-text", "unicode-bidi"],
        r#"shape("نص حكيم 1234 abc", &style); // UBA resolves levels per run
let families = labels.resolved_families(text, &style);"#,
        "A three-size specimen of the selected preset; the custom preset is a live text input that accepts IME composition.",
        &[KnobSpec::segmented(
            "Preset",
            &[
                "CJK + Latin",
                "Arabic BiDi + numbers",
                "Math & symbols",
                "Custom input",
            ],
            0,
        )],
        DemoKind::BidiSandbox,
    )
    .with_height(330.0),
];

/// One preview row of the script matrix.
struct Script {
    tag: &'static str,
    name: &'static str,
    sample: &'static str,
    /// 1 = East Asian, 2 = European / Cyrillic, 3 = complex / RTL.
    group: u8,
}

const SCRIPTS: [Script; 12] = [
    Script {
        tag: "Hans",
        name: "简体中文",
        sample: "天地玄黄，宇宙洪荒。敏捷的棕色狐狸跳过懒狗 0123456789",
        group: 1,
    },
    Script {
        tag: "Hant",
        name: "繁體中文",
        sample: "視野無限，窗明几淨。無界圖形引擎支援多語系文字排版",
        group: 1,
    },
    Script {
        tag: "Jpan",
        name: "日本語",
        sample: "いろはにほへと ちりぬるを。祇園精舎の鐘の声",
        group: 1,
    },
    Script {
        tag: "Hang",
        name: "한국어",
        sample: "다람쥐 헌 쳇바퀴에 타고파. 모든 인류는 자유로우며",
        group: 1,
    },
    Script {
        tag: "Latn",
        name: "English",
        sample: "Sphinx of black quartz, judge my vow. 0123456789 & fi fl",
        group: 2,
    },
    Script {
        tag: "Latn+",
        name: "Français · Deutsch · Español",
        sample: "Voix ambiguë d’un cœur · Zwölf Boxkämpfer · ¡Qué pingüino!",
        group: 2,
    },
    Script {
        tag: "Viet",
        name: "Tiếng Việt",
        sample: "Trăm năm trong cõi người ta, chữ tài chữ mệnh khéo là",
        group: 2,
    },
    Script {
        tag: "Cyrl",
        name: "Русский",
        sample: "Съешь же ещё этих мягких французских булок, да выпей чаю.",
        group: 2,
    },
    Script {
        tag: "Grek",
        name: "Ελληνικά",
        sample: "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία · αβγδεζηθικλμνξοπρστυφχψω",
        group: 2,
    },
    Script {
        tag: "Arab",
        name: "العربية",
        sample: "نص حكيم له سر قاطع وذو شأن عظيم مكتوب على ثوب أخضر",
        group: 3,
    },
    Script {
        tag: "Hebr",
        name: "עברית",
        sample: "דג סקרן שט בים מאוכזב ולפתע מצא חברה — 12345",
        group: 3,
    },
    Script {
        tag: "Deva",
        name: "हिन्दी · ไทย",
        sample: "श्वेतार्क · क्ति · भाषाไทย: เป็นมนุษย์สุดประเสริฐเลิศคุณค่า",
        group: 3,
    },
];

fn style(size: f32, weight: u16, italic: bool) -> TextStyle {
    TextStyle {
        font_size: size,
        weight,
        italic,
        ..TextStyle::default()
    }
}

/// Card 1: the twelve-script matrix.
pub fn script_matrix(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let group = ctx.knob(0) as usize;
    let size = ctx.knob(1).clamp(11.0, 26.0);
    let weight = match ctx.knob(2) as usize {
        1 => 500,
        2 => 700,
        _ => 400,
    };
    let rows: Vec<&Script> = SCRIPTS
        .iter()
        .filter(|script| group == 0 || script.group as usize == group)
        .collect();
    if rows.is_empty() {
        return;
    }
    let row_h = ((rect.size.y - 8.0) / rows.len() as f32).clamp(26.0, 56.0);
    let label_w = 118.0;
    for (index, script) in rows.iter().enumerate() {
        let y = rect.origin.y + index as f32 * row_h;
        if y + row_h > rect.max().y + 1.0 {
            break;
        }
        if index % 2 == 1 {
            ctx.canvas.rect(
                Rect::new(rect.origin.x, y, rect.size.x, row_h),
                0.0,
                palette.card_bg,
                0,
                0.0,
                0.0,
            );
        }
        let text_style = style(size.min(row_h - 6.0).max(10.0), weight, false);
        ctx.canvas.text(
            &format!("{} · {}", script.tag, script.name),
            Rect::new(rect.origin.x + 8.0, y, label_w, row_h),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        // The family badge is measured before the sample is placed, so a long
        // fallback chain never gets painted over the specimen.
        let families = ctx
            .canvas
            .labels
            .resolved_families(script.sample, &text_style);
        let badge = match families.len() {
            0 => String::new(),
            1 => families[0].clone(),
            n => format!("{} +{}", families[0], n - 1),
        };
        let badge_w = if badge.is_empty() {
            0.0
        } else {
            ctx.canvas.labels.measure(&badge, 9.0).x + 16.0
        };
        let sample_w = rect.size.x - label_w - badge_w - 26.0;
        let sample = Rect::new(rect.origin.x + 8.0 + label_w, y, sample_w.max(40.0), row_h);
        // Glyph quads are not clipped to the box they were shaped for, so a
        // long sample would paint over the badge on a narrow (high zoom) card.
        ctx.canvas.begin_clip(sample);
        ctx.canvas.text_styled(
            script.sample,
            sample,
            &text_style,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.end_clip();
        if !badge.is_empty() {
            widgets::badge_sized(
                ctx.canvas,
                Rect::new(
                    rect.max().x - badge_w - 8.0,
                    y + (row_h - 18.0) * 0.5,
                    badge_w,
                    18.0,
                ),
                &badge,
                palette.text_muted,
                palette.card_bg,
                9.0,
            );
        }
    }
}

/// Card 2: the corner-sensitive size and weight waterfall.
pub fn curve_inspector(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let specimen = match ctx.knob(0) as usize {
        1 => "永 国 回 曲 弯",
        2 => "O0Oo 永 Gg Qq 8B 无界",
        _ => "O G C Q S 0 8 @",
    };
    let italic = ctx.knob_on(1);
    let percent = ctx.knob(2).clamp(100.0, 300.0);
    let factor = 0.7 + (percent - 100.0) / 200.0 * 0.6;
    let mut y = rect.origin.y + 4.0;
    for base in [10.0f32, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0] {
        let size = (base * factor).clamp(8.0, 64.0);
        let row = Rect::new(rect.origin.x, y, rect.size.x, size * 1.32);
        if row.max().y > rect.max().y {
            break;
        }
        ctx.canvas.text(
            &format!("{base:.0}pt"),
            Rect::new(rect.origin.x, y, 36.0, size * 1.32),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        let specimen_rect = Rect::new(rect.origin.x + 40.0, y, rect.size.x - 44.0, size * 1.32);
        ctx.canvas.begin_clip(specimen_rect);
        ctx.canvas.text_styled(
            specimen,
            specimen_rect,
            &style(size, 400, italic),
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.end_clip();
        y += size * 1.32 + 2.0;
    }
    if y + 32.0 >= rect.max().y {
        return;
    }
    ctx.canvas.divider(rect.origin.x, y + 4.0, rect.size.x);
    y += 14.0;
    let weights: [(u16, &str); 5] = [
        (300, "Light 300"),
        (400, "Regular 400"),
        (500, "Medium 500"),
        (600, "SemiBold 600"),
        (700, "Bold 700"),
    ];
    let row_h = ((rect.max().y - y) / weights.len() as f32).min(30.0);
    for (weight, name) in weights {
        ctx.canvas.text(
            name,
            Rect::new(rect.origin.x, y, 86.0, row_h),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        let sample_rect = Rect::new(rect.origin.x + 90.0, y, rect.size.x - 94.0, row_h);
        ctx.canvas.begin_clip(sample_rect);
        ctx.canvas.text_styled(
            "Unbound 无界 · Съешь · العربية",
            sample_rect,
            &style(15.0, weight, italic),
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.end_clip();
        y += row_h;
    }
}

/// Card 3: the three-size BiDi specimen plus an IME input.
pub fn bidi_sandbox(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let preset = ctx.knob(0) as usize;
    let custom = preset == 3;
    let text: &str = match preset {
        0 => "无界界面 · Unbound UI · 0123456789",
        1 => "نص حكيم له سر قاطع 1234 abc · שלום עולם",
        2 => "∑ ∫ √ ∞ ≈ ≠ ≤ ≥ → ∀ ∃ ∈ ∇ ∂ © ® ™",
        _ => {
            if ctx.state.demo.text.is_empty() {
                "Click below, then type with the IME…"
            } else {
                ctx.state.demo.text.as_str()
            }
        }
    };
    let placeholder = custom && ctx.state.demo.text.is_empty();
    let color = if placeholder {
        palette.text_muted
    } else {
        palette.text
    };
    let mut y = rect.origin.y + 4.0;
    for (size, label) in [(12.0f32, "12 pt"), (15.0, "15 pt"), (28.0, "28 pt")] {
        let row_h = size * 1.5;
        ctx.canvas.text(
            label,
            Rect::new(rect.origin.x, y, 42.0, row_h),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        let row = Rect::new(rect.origin.x + 46.0, y, rect.size.x - 50.0, row_h);
        let (quads, _) = ctx.canvas.layout_text_styled(
            text,
            row,
            &style(size, 400, false),
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.begin_clip(row);
        ctx.canvas.glyphs(&quads, color);
        ctx.canvas.end_clip();
        y += row_h + 2.0;
    }
    let families = ctx
        .canvas
        .labels
        .resolved_families(text, &style(15.0, 400, false));
    let resolved = if families.is_empty() {
        "no system font matched".to_string()
    } else {
        families.join("  +  ")
    };
    let resolved_rect = Rect::new(rect.origin.x, y + 2.0, rect.size.x, 16.0);
    ctx.canvas.begin_clip(resolved_rect);
    ctx.canvas.text(
        &resolved,
        resolved_rect,
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.end_clip();
    if !custom {
        return;
    }
    let field = Rect::new(rect.origin.x, y + 24.0, rect.size.x, 34.0);
    if field.max().y > rect.max().y {
        return;
    }
    let focused = ctx.state.demo.typing;
    ctx.canvas.rect(
        field,
        8.0,
        palette.card_bg,
        if focused {
            palette.primary
        } else {
            palette.border
        },
        if focused { 2.0 } else { 1.0 },
        0.0,
    );
    ctx.canvas.text(
        text,
        Rect::new(
            field.origin.x + 12.0,
            field.origin.y,
            field.size.x - 24.0,
            field.size.y,
        ),
        15.0,
        color,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.control(
        1,
        field,
        accesskit::Role::TextInput,
        "Font sandbox input",
        false,
    );
}

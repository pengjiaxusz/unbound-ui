//! Basic Input: buttons, selection controls, sliders and text fields.

use accesskit::Role;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const BUTTONS_INFO: PageInfo = PageInfo {
    title: "Buttons",
    short: "Buttons",
    description: "Four visual weights with normal, hover, pressed, focused and disabled states. Press feedback is a real router hit, not a screenshot.",
    crates: &["unbound-input", "unbound-abi"],
    docs: "docs/03-core/input.md",
    keywords: "button primary secondary subtle icon four states hover pressed focus disabled",
};

pub const TOGGLES_INFO: PageInfo = PageInfo {
    title: "Toggle & selection",
    short: "Selection",
    description: "Switch, checkbox, radio group and segmented pill. Every control shares the same stable identity and semantics contract.",
    crates: &["unbound-input", "unbound-a11y"],
    docs: "docs/03-core/input.md",
    keywords: "toggle switch checkbox radio segmented selection control",
};

pub const SLIDERS_INFO: PageInfo = PageInfo {
    title: "Sliders & progress",
    short: "Sliders",
    description: "Continuous values with pointer capture, plus determinate and indeterminate progress driven by the animation clock.",
    crates: &["unbound-input", "unbound-anim"],
    docs: "docs/03-core/input.md",
    keywords: "slider range progress bar indeterminate drag pointer",
};

pub const TEXT_INFO: PageInfo = PageInfo {
    title: "Text field",
    short: "Text field",
    description: "A focused text surface with caret blinking, drag selection and an IME candidate strip. Typing is handled by the shell's key reducer.",
    crates: &["unbound-text", "unbound-input"],
    docs: "docs/03-core/text.md",
    keywords: "text field input caret selection ime keyboard shaping",
};

pub const BUTTON_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Button variants",
        "Primary, secondary, subtle and icon buttons share one hit-test identity scheme. Click the stage button — the counter proves the event round-trip.",
        &["unbound-input", "unbound-abi"],
        r#"Button::new("Primary")
    .variant(Variant::Primary)
    .on_click(Message::Increment)
    .disabled(state.locked)"#,
        "Four buttons expose their label as the accessible name; disabled buttons are removed from the tab order.",
        &[
            KnobSpec::segmented("Variant", &["Primary", "Secondary", "Subtle", "Icon"], 0),
            KnobSpec::toggle("Disabled", false),
            KnobSpec::check("Focus ring", false),
        ],
        DemoKind::Buttons { icons: false },
    )
    .with_height(250.0),
    ExampleDef::new(
        "Icon buttons",
        "The icon vocabulary used by the title bar and navigation. Icons are painted geometrically, so they inherit the theme and pixel grid.",
        &["unbound-vector", "unbound-input"],
        r#"IconButton::new(Icon::Search)
    .size(40.0)
    .selected(index == active)
    .on_click(Message::SelectIcon(index))"#,
        "Eight toggle buttons; the selected icon is exposed through `aria-pressed`.",
        &[
            KnobSpec::slider("Size", 28.0, 56.0, 2.0, 38.0, "px"),
            KnobSpec::toggle("Disabled", false),
        ],
        DemoKind::Buttons { icons: true },
    )
    .with_height(220.0),
];

pub const TOGGLE_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Switch, check and radio",
        "Three selection models with the same focus contract. Radio buttons participate in a group; the switch and checkbox are independent.",
        &["unbound-input", "unbound-a11y"],
        r#"ToggleSwitch { checked: true, on_change: Message::ToggleA }
CheckBox { checked: false, label: "Notify on completion" }
RadioGroup { selected: 0, options: ["Automatic", "Manual"] }"#,
        "A switch, a checkbox and a three-way radio group; arrow keys move radio selection.",
        &[
            KnobSpec::toggle("Disabled", false),
            KnobSpec::segmented("Density", &["Comfortable", "Compact"], 0),
        ],
        DemoKind::Selection { segmented: false },
    )
    .with_height(270.0),
    ExampleDef::new(
        "Segmented control",
        "A single-choice pill with a sliding selection surface. The same primitive is used for the title bar's zoom and theme switches.",
        &["unbound-input", "unbound-anim"],
        r#"SegmentedControl::new(["Day", "Week", "Month"])
    .selected(state.pill)
    .on_change(Message::SetRange)"#,
        "A three-option tab list; selection follows click and keyboard activation.",
        &[
            KnobSpec::segmented("Options", &["3", "4", "5"], 0),
            KnobSpec::toggle("Full width", true),
        ],
        DemoKind::Selection { segmented: true },
    )
    .with_height(220.0),
];

pub const SLIDER_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Continuous sliders",
        "Drag anywhere on the track; pointer capture keeps the thumb under the cursor even when it leaves the row.",
        &["unbound-input", "unbound-input/src/router.rs"],
        r#"Slider::new("Volume", 0.0, 1.0)
    .value(state.volume)
    .on_input(Message::SetVolume)"#,
        "Three labelled sliders; each exposes its current value and min/max range.",
        &[KnobSpec::toggle("Show ticks", true)],
        DemoKind::Sliders { progress: false },
    )
    .with_height(240.0),
    ExampleDef::new(
        "Progress",
        "Determinate progress reflects a bound value; indeterminate progress is paced by the animation clock and composited without a new draw call.",
        &["unbound-anim", "unbound-gpu"],
        r#"ProgressBar::new(0.64)
    .determinate(true)
    .animate(true)"#,
        "A progress indicator exposes `value` and `max`; the paused state is announced.",
        &[
            KnobSpec::segmented("Mode", &["Determinate", "Indeterminate"], 0),
            KnobSpec::toggle("Pause", false),
        ],
        DemoKind::Sliders { progress: true },
    )
    .with_height(220.0),
];

pub const TEXT_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Focused text field",
        "Click the field and type. The shell routes characters into a real buffer; the caret blinks on the animation clock.",
        &["unbound-text", "unbound-input"],
        r#"TextField::new("Project name")
    .value(&mut state.text)
    .focused(true)
    .on_input(Message::TextChanged)"#,
        "A focused text input; the caret is exposed as a text selection, not a separate element.",
        &[
            KnobSpec::toggle("Read only", false),
            KnobSpec::check("Show IME", true),
        ],
        DemoKind::TextField { selection: false },
    )
    .with_height(240.0),
    ExampleDef::new(
        "Selection & IME",
        "A word selection with a real highlight rect, plus the candidate strip an IME would place under the caret.",
        &["unbound-text/src/selection.rs", "unbound-text/src/shaping.rs"],
        r#"selection = Selection::word_at(caret);
scene.rect(selection.bounds(), 0.0, Token::SELECTION_FILL, 0, 0.0, 0.0);"#,
        "The selected range is exposed through the text input's `selection` property.",
        &[
            KnobSpec::segmented("Selection", &["Word", "Line", "None"], 0),
            KnobSpec::toggle("Read only", false),
        ],
        DemoKind::TextField { selection: true },
    )
    .with_height(240.0),
];

fn button_variant(index: usize) -> ButtonVariant {
    match index {
        1 => ButtonVariant::Secondary,
        2 => ButtonVariant::Subtle,
        3 => ButtonVariant::Icon,
        _ => ButtonVariant::Primary,
    }
}

/// Button gallery. `icons` selects the icon-grid variant.
pub fn buttons(ctx: &mut DemoCtx<'_, '_>, icons: bool) {
    if icons {
        icon_buttons(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let variant = button_variant(ctx.knob(0) as usize);
    let disabled = ctx.knob_on(1);
    let show_focus = ctx.knob_on(2);
    let main = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 24.0,
        (rect.size.x * 0.44).clamp(150.0, 260.0),
        44.0,
    );
    let visual = widgets::button(
        ctx.canvas,
        ctx.part(1),
        main,
        match variant {
            ButtonVariant::Primary => "Primary action",
            ButtonVariant::Secondary => "Secondary",
            ButtonVariant::Subtle => "Subtle action",
            ButtonVariant::Icon => "+",
            ButtonVariant::OnHero => "On hero",
            ButtonVariant::OnHeroSecondary => "On hero, secondary",
        },
        variant,
        !disabled,
    );
    if show_focus {
        ctx.canvas
            .stroke(main.inflate(2.0), 10.0, palette.focus_ring, 2.0);
    }
    let label = match visual {
        crate::canvas::Visual::Hover => "hover",
        crate::canvas::Visual::Pressed => "pressed",
        crate::canvas::Visual::Disabled => "disabled",
        crate::canvas::Visual::Idle => "normal",
    };
    ctx.canvas.text(
        &format!("state: {label}"),
        Rect::new(main.origin.x, main.max().y + 8.0, main.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text(
        &format!("{} click(s)", ctx.state.demo.clicks),
        Rect::new(main.origin.x, main.max().y + 28.0, main.size.x, 20.0),
        12.0,
        palette.primary,
        AlignX::Start,
        AlignY::Top,
    );

    let row_y = rect.origin.y + 92.0;
    for (index, (label, variant)) in [
        ("Primary", ButtonVariant::Primary),
        ("Secondary", ButtonVariant::Secondary),
        ("Subtle", ButtonVariant::Subtle),
    ]
    .iter()
    .enumerate()
    {
        let btn = Rect::new(
            rect.origin.x + 24.0 + index as f32 * 118.0,
            row_y,
            108.0,
            34.0,
        );
        widgets::button(
            ctx.canvas,
            ctx.part(10 + index as u16),
            btn,
            label,
            *variant,
            !disabled,
        );
    }
    ctx.canvas.text(
        "Click any button to exercise the router → reducer → rebuild loop.",
        Rect::new(
            rect.origin.x + 24.0,
            rect.max().y - 30.0,
            rect.size.x - 48.0,
            18.0,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

fn icon_buttons(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let size = ctx.knob(0);
    let disabled = ctx.knob_on(1);
    let icons = [
        Icon::Menu,
        Icon::Search,
        Icon::Sun,
        Icon::Moon,
        Icon::Grid,
        Icon::Chart,
        Icon::Snap,
        Icon::Layers,
    ];
    let gap = 10.0;
    for (index, icon) in icons.iter().enumerate() {
        let col = index % 4;
        let row = index / 4;
        let cell = Rect::new(
            rect.origin.x + 12.0 + col as f32 * (size + gap),
            rect.origin.y + 18.0 + row as f32 * (size + gap),
            size,
            size,
        );
        let selected = ctx.state.demo.selected_icon == index;
        let visual = ctx.canvas.visual(ctx.part(10 + index as u16), disabled);
        ctx.canvas.rect(
            cell,
            10.0,
            if selected {
                palette.primary_soft
            } else if visual.is_hovered() {
                palette.control_hover
            } else {
                palette.card_bg
            },
            if selected { 0 } else { palette.card_border },
            1.0,
            0.0,
        );
        widgets::icon(
            ctx.canvas,
            *icon,
            cell,
            if disabled {
                palette.text_muted
            } else if selected {
                palette.primary
            } else {
                palette.text
            },
            palette.card_bg,
        );
        if !disabled {
            ctx.control(
                10 + index as u16,
                cell,
                Role::Button,
                &format!("Icon {index}"),
                selected,
            );
        }
    }
    ctx.canvas.text(
        "Geometric icons · 2 px optical grid",
        Rect::new(
            rect.origin.x + 12.0,
            rect.max().y - 22.0,
            rect.size.x - 24.0,
            16.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

/// Selection controls.
pub fn selection(ctx: &mut DemoCtx<'_, '_>, segmented: bool) {
    if segmented {
        segmented_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let disabled = ctx.knob_on(0);
    let compact = ctx.knob(1) > 0.5;
    let row_h = if compact { 30.0 } else { 38.0 };
    let left = Rect::new(
        rect.origin.x + 18.0,
        rect.origin.y + 12.0,
        rect.size.x * 0.52 - 26.0,
        rect.size.y - 24.0,
    );

    let switch = Rect::new(left.origin.x, left.origin.y, left.size.x, row_h);
    widgets::toggle_switch(
        ctx.canvas,
        ctx.part(1),
        switch,
        "Wi-Fi",
        ctx.state.demo.toggle_a,
    );
    let check = Rect::new(left.origin.x, left.origin.y + row_h, left.size.x, row_h);
    widgets::checkbox(
        ctx.canvas,
        ctx.part(2),
        check,
        "Notify on completion",
        ctx.state.demo.toggle_b,
    );
    for (index, label) in ["Automatic", "Ask me", "Never"].iter().enumerate() {
        let row = Rect::new(
            left.origin.x,
            left.origin.y + row_h * 2.0 + row_h * index as f32,
            left.size.x,
            row_h,
        );
        widgets::radio(
            ctx.canvas,
            ctx.part(10 + index as u16),
            row,
            label,
            ctx.state.demo.radio_choice == index,
        );
    }
    if disabled {
        ctx.canvas.text(
            "disabled: pointer events suppressed",
            Rect::new(left.origin.x, left.max().y - 18.0, left.size.x, 14.0),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
    }

    let panel = Rect::new(
        rect.origin.x + rect.size.x * 0.55,
        rect.origin.y + 16.0,
        rect.size.x * 0.45 - 18.0,
        rect.size.y - 32.0,
    );
    ctx.canvas
        .rect(panel, 12.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.text(
        "Selection group",
        Rect::new(
            panel.origin.x + 14.0,
            panel.origin.y + 10.0,
            panel.size.x - 28.0,
            18.0,
        ),
        12.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    let pills = ["Day", "Week", "Month"];
    let pill_rect = Rect::new(
        panel.origin.x + 14.0,
        panel.origin.y + 36.0,
        panel.size.x - 28.0,
        32.0,
    );
    let ids = [ctx.part(20), ctx.part(21), ctx.part(22)];
    widgets::segmented(
        ctx.canvas,
        &ids,
        pill_rect,
        &pills,
        ctx.state.demo.pill_choice,
    );
    ctx.canvas.text(
        &format!("selected: {}", pills[ctx.state.demo.pill_choice.min(2)]),
        Rect::new(
            panel.origin.x + 14.0,
            pill_rect.max().y + 10.0,
            panel.size.x - 28.0,
            16.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
}

fn segmented_page(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let count = match ctx.knob(0) as usize {
        1 => 4,
        2 => 5,
        _ => 3,
    };
    let full_width = ctx.knob_on(1);
    let all_options = ["Day", "Week", "Month", "Quarter", "Year"];
    let labels = &all_options[..count];
    let ids: Vec<u64> = (0..count as u16)
        .map(|index| ctx.part(30 + index))
        .collect();
    let width = if full_width {
        rect.size.x - 48.0
    } else {
        (count as f32 * 86.0).min(rect.size.x - 48.0)
    };
    let pill = Rect::new(rect.origin.x + 24.0, rect.origin.y + 28.0, width, 38.0);
    widgets::segmented(
        ctx.canvas,
        &ids,
        pill,
        labels,
        ctx.state.demo.pill_choice.min(count - 1),
    );
    ctx.canvas.text(
        &format!("SegmentedControl · {} options", count),
        Rect::new(pill.origin.x, pill.max().y + 14.0, pill.size.x, 18.0),
        11.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    let indicator = Rect::new(
        pill.origin.x
            + pill.size.x / count as f32 * ctx.state.demo.pill_choice.min(count - 1) as f32
            + 6.0,
        pill.max().y + 44.0,
        pill.size.x / count as f32 - 12.0,
        4.0,
    );
    ctx.canvas
        .rect(indicator, 2.0, palette.primary, 0, 0.0, 0.0);
    ctx.canvas.text(
        "selection indicator animates with unbound-anim",
        Rect::new(pill.origin.x, indicator.max().y + 6.0, pill.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
}

/// Sliders and progress.
pub fn sliders(ctx: &mut DemoCtx<'_, '_>, progress: bool) {
    if progress {
        progress_bar(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let ticks = ctx.knob_on(0);
    let labels = ["Volume", "Brightness", "Playback speed"];
    for (index, label) in labels.iter().enumerate() {
        let row_y = rect.origin.y + 18.0 + index as f32 * 58.0;
        ctx.canvas.text(
            label,
            Rect::new(rect.origin.x + 18.0, row_y, rect.size.x - 120.0, 16.0),
            12.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            &format!("{:.0} %", ctx.state.demo.sliders[index] * 100.0),
            Rect::new(rect.max().x - 92.0, row_y, 74.0, 16.0),
            11.0,
            palette.text_muted,
            AlignX::End,
            AlignY::Top,
        );
        let track = Rect::new(rect.origin.x + 18.0, row_y + 26.0, rect.size.x - 36.0, 18.0);
        let node = ctx.part(1 + index as u16);
        widgets::slider(
            ctx.canvas,
            node,
            track,
            ctx.state.demo.sliders[index],
            0.0,
            1.0,
        );
        ctx.canvas.range(node, track, 0.0, 1.0, 0.01);
        if ticks {
            for tick in 0..=4 {
                let x = track.origin.x + track.size.x * tick as f32 / 4.0;
                ctx.canvas.rect(
                    Rect::new(x - 1.0, track.max().y + 2.0, 2.0, 4.0),
                    1.0,
                    palette.stage_grid,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
    }
    ctx.canvas.text(
        "Pointer capture keeps the thumb tracking after it leaves the row.",
        Rect::new(
            rect.origin.x + 18.0,
            rect.max().y - 24.0,
            rect.size.x - 36.0,
            16.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

fn progress_bar(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let indeterminate = ctx.knob(0) > 0.5;
    let paused = ctx.knob_on(1);
    let value = if paused {
        0.64
    } else {
        (ctx.state.demo.elapsed * 0.18).rem_euclid(1.0)
    };
    let track = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 36.0,
        rect.size.x - 48.0,
        12.0,
    );
    widgets::badge(
        ctx.canvas,
        Rect::new(track.origin.x, rect.origin.y + 8.0, 90.0, 20.0),
        if indeterminate {
            "Indeterminate"
        } else {
            "Determinate"
        },
        palette.primary,
        palette.primary_soft,
    );
    ctx.canvas.text(
        &format!("{:.0} %", value * 100.0),
        Rect::new(track.max().x - 54.0, rect.origin.y + 6.0, 54.0, 20.0),
        12.0,
        palette.text,
        AlignX::End,
        AlignY::Center,
    );
    ctx.canvas
        .rect(track, 6.0, palette.control_track, 0, 0.0, 0.0);
    if indeterminate {
        let span = track.size.x * 0.32;
        let travel = (track.size.x + span) * 0.5;
        let phase = (ctx.state.demo.elapsed * 0.8).rem_euclid(1.0);
        let x = track.origin.x - span + travel * phase;
        let bar = Rect::new(x, track.origin.y, span, track.size.y);
        ctx.canvas.begin_clip(track);
        ctx.canvas.rect(bar, 6.0, palette.primary, 0, 0.0, 0.0);
        ctx.canvas.end_clip();
    } else {
        ctx.canvas.rect(
            Rect::new(
                track.origin.x,
                track.origin.y,
                track.size.x * value,
                track.size.y,
            ),
            6.0,
            palette.primary,
            0,
            0.0,
            0.0,
        );
    }
    ctx.canvas.text(
        if paused {
            "paused · progress frozen"
        } else {
            "paced by the animation clock"
        },
        Rect::new(track.origin.x, track.max().y + 12.0, track.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text(
        "4 ms frame budget · no layout invalidation",
        Rect::new(track.origin.x, track.max().y + 34.0, track.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
}

/// Text fields, selection and IME.
pub fn text_field(ctx: &mut DemoCtx<'_, '_>, selection: bool) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let read_only = ctx.knob_on(if selection { 1 } else { 0 });
    let field = Rect::new(
        rect.origin.x + 20.0,
        if selection {
            rect.origin.y + 18.0
        } else {
            rect.origin.y + 30.0
        },
        rect.size.x - 40.0,
        46.0,
    );
    let focused = ctx.state.demo.typing;
    ctx.canvas.rect(
        field,
        9.0,
        palette.card_bg,
        if focused {
            palette.primary
        } else {
            palette.border
        },
        if focused { 2.0 } else { 1.0 },
        if focused { 6.0 } else { 0.0 },
    );
    let text_rect = Rect::new(
        field.origin.x + 14.0,
        field.origin.y,
        field.size.x - 28.0,
        field.size.y,
    );
    let text = if ctx.state.demo.text.is_empty() {
        "Project name"
    } else {
        ctx.state.demo.text.as_str()
    };
    let color = if ctx.state.demo.text.is_empty() {
        palette.text_muted
    } else {
        palette.text
    };
    let ink = ctx
        .canvas
        .text(text, text_rect, 15.0, color, AlignX::Start, AlignY::Center);
    let select_mode = if selection { ctx.knob(0) as usize } else { 2 };
    if select_mode < 2 && !ctx.state.demo.text.is_empty() {
        let selected = if select_mode == 0 {
            ctx.state.demo.text.split_whitespace().next().unwrap_or("")
        } else {
            ctx.state.demo.text.as_str()
        };
        let width = ctx.canvas.labels.measure(selected, 15.0).x;
        ctx.canvas.rect(
            Rect::new(
                ink.origin.x - 2.0,
                ink.origin.y - 2.0,
                width + 4.0,
                ink.size.y + 4.0,
            ),
            3.0,
            palette.selection_fill,
            0,
            0.0,
            0.0,
        );
        ctx.canvas.text(
            selected,
            Rect::new(ink.origin.x, ink.origin.y, width, ink.size.y),
            15.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
    }
    if focused && (ctx.state.demo.elapsed * 2.0).rem_euclid(2.0) < 1.2 {
        ctx.canvas.rect(
            Rect::new(
                ink.max().x + 2.0,
                field.origin.y + 12.0,
                2.0,
                field.size.y - 24.0,
            ),
            1.0,
            palette.primary,
            0,
            0.0,
            0.0,
        );
    }
    ctx.canvas.text(
        "Click, then type · Backspace deletes · Esc releases focus",
        Rect::new(field.origin.x, field.max().y + 8.0, field.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    if selection {
        let ime = Rect::new(field.origin.x, field.max().y + 34.0, 220.0, 34.0);
        ctx.canvas
            .rect(ime, 8.0, palette.overlay_bg, palette.card_border, 1.0, 6.0);
        ctx.canvas.text(
            "1 无界",
            Rect::new(ime.origin.x + 12.0, ime.origin.y, 70.0, ime.size.y),
            12.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            "2 界面",
            Rect::new(ime.origin.x + 82.0, ime.origin.y, 70.0, ime.size.y),
            12.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            "3 排版",
            Rect::new(ime.origin.x + 152.0, ime.origin.y, 60.0, ime.size.y),
            12.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            "IME candidate strip · shaping happens before commit",
            Rect::new(ime.origin.x, ime.max().y + 8.0, rect.size.x - 40.0, 16.0),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
    }
    if read_only {
        widgets::badge(
            ctx.canvas,
            Rect::new(field.max().x - 78.0, field.origin.y + 12.0, 66.0, 20.0),
            "read only",
            palette.text_muted,
            palette.stage_bg,
        );
    }
    if !read_only {
        ctx.control(1, field, Role::TextInput, "Project name", false);
    }
}

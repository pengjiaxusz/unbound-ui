//! Small, reusable gallery controls.
//!
//! These are not a second widget framework: each function is a pure renderer
//! over [`Canvas`]. Identity, hover, press and focus all come from the caller,
//! which is exactly the same contract the original button established.

use accesskit::Role;
use unbound_input::CursorKind;
use unbound_math::Rect;
use unbound_theme::Token;
use unbound_widgets::button::{
    emit_button_with_tokens, ButtonDesc, ButtonTokens, ButtonVariant as EngineButtonVariant,
};
use unbound_widgets::checkbox::{emit_checkbox, CheckboxDesc, CheckboxTokens};
use unbound_widgets::radio::{emit_radio, RadioDesc, RadioTokens};
use unbound_widgets::segmented::{
    emit_segmented_frame, emit_segmented_segment, SegmentedDesc, SegmentedTokens,
};
use unbound_widgets::slider::{emit_slider, SliderDesc, SliderTokens};
use unbound_widgets::switch::{emit_switch, SwitchDesc, SwitchTokens};
use unbound_widgets::{ControlState, WidgetCatalogId};

use crate::canvas::{quads_bounds, AlignX, AlignY, Canvas, Visual};
use crate::target::Target;

/// The visual weight of a button.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Subtle,
    Icon,
    /// A call to action on the saturated hero surface.
    ///
    /// A filled blue button on a blue gradient has almost no contrast, so
    /// the primary action is a solid white pill with the brand colour as its
    /// label, and the secondary action is a dark translucent pill with a
    /// hairline border.
    OnHero,
    /// The secondary call to action on the hero surface.
    OnHeroSecondary,
}

/// A gallery button. Returns the visual state it painted.
///
/// The control itself lives in the engine layer: the shell only supplies the
/// geometry, the shaped label and the per-variant colour slots, so the five
/// states and the focus ring come from [`emit_button_with_tokens`] instead of
/// a second hand-drawn implementation.
pub fn button(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    variant: ButtonVariant,
    enabled: bool,
) -> Visual {
    let state = canvas.control_state(node, !enabled, false);
    let font_size = if variant == ButtonVariant::Icon {
        15.0
    } else {
        13.0
    };
    let (mut quads, _) = canvas.layout_text(label, rect, font_size, AlignX::Center, AlignY::Center);
    let mut desc = ButtonDesc::new(rect, label)
        .with_variant(carrier_variant(variant))
        .with_shadow(0.0)
        .with_centre_label(false)
        .with_focus_ring_gap(2.0);
    desc.font_size = font_size;
    let output = emit_button_with_tokens(node, &desc, state, button_tokens(variant), &mut quads);
    canvas.widget_output(WidgetCatalogId::Button, node, &output);
    if !enabled {
        // The engine drops the hit region of a disabled button; the gallery
        // keeps its disabled buttons absorbing clicks like the hand-drawn
        // version did.
        canvas.control(
            node,
            rect,
            Role::Button,
            label,
            CursorKind::Pointer,
            true,
            false,
        );
    }
    visual_of(state)
}

/// The engine variant used as a carrier.
///
/// The colour slots are passed explicitly, so the carrier never decides the
/// palette; it only keeps the descriptor readable.
fn carrier_variant(variant: ButtonVariant) -> EngineButtonVariant {
    match variant {
        ButtonVariant::Primary => EngineButtonVariant::Primary,
        ButtonVariant::Secondary | ButtonVariant::OnHeroSecondary => EngineButtonVariant::Secondary,
        ButtonVariant::Subtle => EngineButtonVariant::Subtle,
        ButtonVariant::Icon => EngineButtonVariant::Icon,
        ButtonVariant::OnHero => EngineButtonVariant::OnHero,
    }
}

/// The colour slots each gallery variant paints with.
///
/// These are the exact slots the hand-drawn button used, so the migration
/// keeps the page pixel-identical: the engine's own defaults differ in three
/// places (Primary's border, every `label_disabled`, and Secondary's disabled
/// fill) and those are overridden here.
fn button_tokens(variant: ButtonVariant) -> ButtonTokens {
    let focus_ring = Token::FOCUS_RING;
    match variant {
        ButtonVariant::Primary => ButtonTokens {
            fill: Token::PRIMARY,
            fill_hover: Token::PRIMARY_HOVER,
            fill_pressed: Token::PRIMARY_PRESSED,
            fill_disabled: Token::DISABLED,
            border: Token::NONE,
            label: Token::ON_PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::Secondary => ButtonTokens {
            fill: Token::CARD_BG,
            fill_hover: Token::CONTROL_HOVER,
            fill_pressed: Token::CONTROL_HOVER,
            fill_disabled: Token::CARD_BG,
            border: Token::CARD_BORDER,
            label: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::Subtle | ButtonVariant::Icon => ButtonTokens {
            fill: Token::NONE,
            fill_hover: Token::CONTROL_HOVER,
            fill_pressed: Token::NAV_SELECTED,
            fill_disabled: Token::NONE,
            border: Token::NONE,
            label: Token::PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::OnHero => ButtonTokens {
            fill: Token::ON_HERO,
            fill_hover: Token::ON_HERO,
            fill_pressed: Token::PRIMARY_SOFT,
            fill_disabled: Token::ON_HERO,
            border: Token::NONE,
            label: Token::PRIMARY,
            label_disabled: Token::PRIMARY,
            focus_ring,
        },
        ButtonVariant::OnHeroSecondary => ButtonTokens {
            fill: Token::HERO_SCRIM,
            fill_hover: Token::HERO_SCRIM,
            fill_pressed: Token::HERO_SCRIM,
            fill_disabled: Token::HERO_SCRIM,
            border: Token::ON_HERO,
            label: Token::ON_HERO,
            label_disabled: Token::ON_HERO,
            focus_ring,
        },
    }
}

/// A row with a label and a switch.
///
/// The control itself lives in the engine layer
/// ([`unbound_widgets::switch`]): the shell only supplies the row geometry
/// and the shaped label.
pub fn toggle_switch(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    on: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, on);
    let desc = SwitchDesc::new(rect, label).with_on(on);
    let (quads, _) = canvas.layout_text(
        label,
        Rect::new(
            rect.origin.x,
            rect.origin.y,
            (rect.size.x - 56.0).max(0.0),
            rect.size.y,
        ),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_switch(node, &desc, state, &quads, SwitchTokens::standard());
    canvas.widget_output(WidgetCatalogId::Switch, node, &output);
    visual_of(state)
}

/// Translate a control state into the shell's own visual vocabulary.
fn visual_of(state: ControlState) -> Visual {
    if state.disabled {
        return Visual::Disabled;
    }
    match state.phase {
        unbound_widgets::PointerPhase::Idle => Visual::Idle,
        unbound_widgets::PointerPhase::Hovered => Visual::Hover,
        unbound_widgets::PointerPhase::Pressed => Visual::Pressed,
    }
}

/// A checkbox row.
pub fn checkbox(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    checked: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, checked);
    let desc = CheckboxDesc::new(rect, label).with_checked(checked);
    let (mark, _) = canvas.layout_text("✓", desc.box_rect(), 12.0, AlignX::Center, AlignY::Center);
    let (quads, _) = canvas.layout_text(
        label,
        desc.label_rect(),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_checkbox(
        node,
        &desc,
        state,
        &mark,
        &quads,
        CheckboxTokens::standard(),
    );
    canvas.widget_output(WidgetCatalogId::Checkbox, node, &output);
    visual_of(state)
}

/// A radio-button row.
pub fn radio(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    selected: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, selected);
    let desc = RadioDesc::new(rect, label).with_selected(selected);
    let (quads, _) = canvas.layout_text(
        label,
        desc.label_rect(),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_radio(node, &desc, state, &quads, RadioTokens::standard());
    canvas.widget_output(WidgetCatalogId::Radio, node, &output);
    visual_of(state)
}

/// A horizontal slider. `value` is in `[min, max]`.
pub fn slider(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    value: f32,
    min: f32,
    max: f32,
) -> Visual {
    let state = canvas.control_state(node, false, false);
    let desc = SliderDesc::new(rect, "Slider", value, min, max);
    let output = emit_slider(node, &desc, state, &[], SliderTokens::standard());
    canvas.widget_output(WidgetCatalogId::Slider, node, &output);
    visual_of(state)
}

/// A segmented pill control.
///
/// The slots are an even grid and the highlight is inset by two pixels on
/// every side, so the label never touches the frame: a 43 px slot holds
/// `"100%"` with seven pixels of breathing room on each side.
pub fn segmented(
    canvas: &mut Canvas<'_>,
    ids: &[u64],
    rect: Rect,
    labels: &[&str],
    selected: usize,
    disabled: bool,
    thumb: Option<f32>,
) {
    let mut desc = SegmentedDesc::new(rect, labels).with_selected(selected);
    if let Some(thumb) = thumb {
        desc = desc.with_thumb(thumb);
    }
    let tokens = SegmentedTokens::standard();
    let frame = emit_segmented_frame(&desc, tokens);
    canvas.widget_output(WidgetCatalogId::Segmented, 0, &frame);
    for index in 0..labels.len() {
        let state = canvas.control_state(ids[index], disabled, index == selected);
        let (quads, _) = canvas.layout_text(
            labels[index],
            desc.segment_rect(index),
            12.0,
            AlignX::Center,
            AlignY::Center,
        );
        let output = emit_segmented_segment(ids[index], &desc, index, state, &quads, tokens);
        canvas.control_output(ids[index], &output);
    }
}

/// A navigation row: selection pill, label and optional trailing hint.
pub fn nav_row(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    selected: bool,
    indent: f32,
) {
    let start = canvas.span_start();
    let palette = *canvas.palette;
    let visual = canvas.visual(node, false);
    // The row always paints two primitives — a background and the accent
    // bar — whatever its state. The background's slot is the pane's own
    // colour while idle, and the accent bar fades out, so neither state
    // changes the number of primitives the row owns.
    let background = if selected {
        palette.nav_selected
    } else if visual.is_hovered() {
        palette.nav_hover
    } else {
        palette.nav_bg
    };
    canvas.rect(rect, 8.0, background, 0, 0.0, 0.0);
    canvas.rect_faded(
        Rect::new(
            rect.origin.x - 10.0,
            rect.origin.y + 8.0,
            3.0,
            rect.size.y - 16.0,
        ),
        2.0,
        palette.primary,
        if selected { 1.0 } else { 0.0 },
    );
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + indent,
            rect.origin.y,
            rect.size.x - indent,
            rect.size.y,
        ),
        13.0,
        if selected {
            palette.nav_text
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        rect,
        Role::Link,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    canvas.record_span(node, start);
}

/// A small pill badge.
pub fn badge(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    label: &str,
    foreground: u16,
    background: u16,
) -> Rect {
    badge_sized(canvas, rect, label, foreground, background, 11.0)
}

/// A pill badge at an explicit font size.
///
/// The width always comes from the same font size the text is painted at, so
/// the caller can advance its cursor by the returned rectangle without the
/// pills overlapping.
pub fn badge_sized(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    label: &str,
    foreground: u16,
    background: u16,
    size: f32,
) -> Rect {
    let measured = canvas.labels.measure(label, size);
    let width = (measured.x + 16.0).max(28.0);
    let rect = Rect::new(rect.origin.x, rect.origin.y, width, rect.size.y);
    canvas.rect(rect, rect.size.y * 0.5, background, 0, 0.0, 0.0);
    canvas.text(
        label,
        rect,
        size,
        foreground,
        AlignX::Center,
        AlignY::Center,
    );
    rect
}

/// The icon vocabulary the shell paints geometrically.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Search,
    Sun,
    Moon,
    Snap,
    Chart,
    Grid,
    Layers,
    ChevronDown,
    ChevronRight,
    Check,
    Close,
    Plus,
    Play,
    Undo,
    Redo,
}

/// Paint an icon centred in `rect`.
pub fn icon(canvas: &mut Canvas<'_>, kind: Icon, rect: Rect, color: u16, background: u16) {
    let c = rect.center();
    match kind {
        Icon::Menu => {
            for i in 0..3 {
                canvas.rect(
                    Rect::new(c.x - 8.0, c.y - 6.0 + i as f32 * 5.0, 16.0, 2.0),
                    1.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::Search => {
            canvas.stroke(Rect::new(c.x - 9.0, c.y - 9.0, 13.0, 13.0), 6.5, color, 2.0);
            canvas.rect(
                Rect::new(c.x + 3.0, c.y + 3.0, 6.0, 2.0),
                1.0,
                color,
                0,
                0.0,
                0.0,
            );
        }
        Icon::Sun => {
            canvas.rect(
                Rect::new(c.x - 6.0, c.y - 6.0, 12.0, 12.0),
                6.0,
                color,
                0,
                0.0,
                0.0,
            );
            for (dx, dy, w, h) in [
                (0.0, -10.0, 2.0, 4.0),
                (0.0, 6.0, 2.0, 4.0),
                (-10.0, 0.0, 4.0, 2.0),
                (6.0, 0.0, 4.0, 2.0),
            ] {
                canvas.rect(Rect::new(c.x + dx, c.y + dy, w, h), 1.0, color, 0, 0.0, 0.0);
            }
        }
        Icon::Moon => {
            canvas.rect(
                Rect::new(c.x - 8.0, c.y - 8.0, 16.0, 16.0),
                8.0,
                color,
                0,
                0.0,
                0.0,
            );
            canvas.rect(
                Rect::new(c.x - 2.0, c.y - 9.0, 11.0, 14.0),
                7.0,
                background,
                0,
                0.0,
                0.0,
            );
        }
        Icon::Snap => {
            for row in 0..3 {
                for col in 0..3 {
                    canvas.rect(
                        Rect::new(
                            c.x - 8.0 + col as f32 * 6.0,
                            c.y - 8.0 + row as f32 * 6.0,
                            3.0,
                            3.0,
                        ),
                        1.0,
                        color,
                        0,
                        0.0,
                        0.0,
                    );
                }
            }
        }
        Icon::Chart => {
            for (i, height) in [6.0f32, 12.0, 9.0].iter().enumerate() {
                canvas.rect(
                    Rect::new(c.x - 8.0 + i as f32 * 6.0, c.y + 6.0 - height, 4.0, *height),
                    1.5,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::Grid => {
            for row in 0..2 {
                for col in 0..2 {
                    canvas.rect(
                        Rect::new(
                            c.x - 8.0 + col as f32 * 9.0,
                            c.y - 8.0 + row as f32 * 9.0,
                            7.0,
                            7.0,
                        ),
                        2.0,
                        color,
                        0,
                        0.0,
                        0.0,
                    );
                }
            }
        }
        Icon::Layers => {
            for i in 0..3 {
                canvas.rect(
                    Rect::new(
                        c.x - 8.0 + i as f32 * 2.0,
                        c.y - 6.0 + i as f32 * 5.0,
                        12.0,
                        4.0,
                    ),
                    2.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::ChevronDown => {
            canvas.text("⌄", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::ChevronRight => {
            canvas.text("›", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Check => {
            canvas.text("✓", rect, 14.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Close => {
            canvas.text("×", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Plus => {
            canvas.text("+", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Play => {
            canvas.text("▶", rect, 12.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Undo => {
            canvas.text("↶", rect, 17.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Redo => {
            canvas.text("↷", rect, 17.0, color, AlignX::Center, AlignY::Center);
        }
    }
}

/// A clickable icon button.
pub fn icon_button(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    kind: Icon,
    label: &str,
    selected: bool,
) {
    let palette = *canvas.palette;
    let visual = canvas.visual(node, false);
    let fill = match visual {
        Visual::Hover => palette.control_hover,
        Visual::Pressed => palette.nav_selected,
        _ => {
            if selected {
                palette.nav_selected
            } else {
                0
            }
        }
    };
    canvas.rect(rect, 8.0, fill, 0, 0.0, 0.0);
    icon(canvas, kind, rect, palette.text, palette.card_bg);
    if canvas.focused(node) {
        canvas.stroke(rect.inflate(1.0), 9.0, palette.focus_ring, 2.0);
    }
    canvas.control(
        node,
        rect,
        Role::Button,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
}

/// The primitive count of a glyph run, used by the telemetry page.
#[must_use]
pub fn glyph_count(quads: &[unbound_text::GlyphQuad]) -> usize {
    quads_bounds(quads).map_or(0, |_| quads.len())
}

/// The demo-part identity helper.
#[must_use]
pub fn demo_id(page: u8, card: u8, part: u16) -> u64 {
    crate::target::id(Target::Demo(page, card, part))
}

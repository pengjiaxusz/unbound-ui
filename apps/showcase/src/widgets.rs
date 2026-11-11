//! Small, reusable gallery controls.
//!
//! These are not a second widget framework: each function is a pure renderer
//! over [`Canvas`]. Identity, hover, press and focus all come from the caller,
//! which is exactly the same contract the original button established.

use accesskit::Role;
use unbound_input::CursorKind;
use unbound_math::Rect;

use crate::canvas::{quads_bounds, AlignX, AlignY, Canvas, Visual};
use crate::target::Target;

/// The visual weight of a button.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Subtle,
    Icon,
}

/// A gallery button. Returns the visual state it painted.
pub fn button(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    variant: ButtonVariant,
    enabled: bool,
) -> Visual {
    let visual = canvas.visual(node, !enabled);
    let palette = *canvas.palette;
    let (fill, border, text) = match variant {
        ButtonVariant::Primary => (
            match visual {
                Visual::Hover => palette.primary_hover,
                Visual::Pressed => palette.primary_pressed,
                Visual::Disabled => palette.disabled_fill,
                Visual::Idle => palette.primary,
            },
            0,
            if enabled {
                palette.on_primary
            } else {
                palette.text_muted
            },
        ),
        ButtonVariant::Secondary => (
            match visual {
                Visual::Hover | Visual::Pressed => palette.control_hover,
                _ => palette.card_bg,
            },
            palette.card_border,
            if enabled {
                palette.text
            } else {
                palette.text_muted
            },
        ),
        ButtonVariant::Subtle | ButtonVariant::Icon => (
            match visual {
                Visual::Hover => palette.control_hover,
                Visual::Pressed => palette.nav_selected,
                _ => 0,
            },
            0,
            if enabled {
                palette.primary
            } else {
                palette.text_muted
            },
        ),
    };
    canvas.rect(rect, 8.0, fill, border, 1.0, 0.0);
    if canvas.focused(node) {
        canvas.stroke(rect.inflate(2.0), 10.0, palette.focus_ring, 2.0);
    }
    if !label.is_empty() {
        canvas.text(
            label,
            rect,
            if variant == ButtonVariant::Icon {
                15.0
            } else {
                13.0
            },
            text,
            AlignX::Center,
            AlignY::Center,
        );
    }
    canvas.control(
        node,
        rect,
        Role::Button,
        label,
        CursorKind::Pointer,
        !enabled,
        matches!(visual, Visual::Pressed),
    );
    visual
}

/// A row with a label and a switch.
pub fn toggle_switch(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    on: bool,
) -> Visual {
    let visual = canvas.visual(node, false);
    let palette = *canvas.palette;
    let track = Rect::new(
        rect.max().x - 44.0,
        rect.origin.y + (rect.size.y - 24.0) * 0.5,
        44.0,
        24.0,
    );
    canvas.rect(
        track,
        12.0,
        if on {
            palette.primary
        } else {
            palette.control_track
        },
        0,
        0.0,
        0.0,
    );
    let knob_x = if on {
        track.max().x - 20.0
    } else {
        track.origin.x + 4.0
    };
    canvas.rect(
        Rect::new(knob_x, track.origin.y + 4.0, 16.0, 16.0),
        8.0,
        palette.control_thumb,
        0,
        0.0,
        2.0,
    );
    canvas.text(
        label,
        Rect::new(
            rect.origin.x,
            rect.origin.y,
            rect.size.x - 56.0,
            rect.size.y,
        ),
        13.0,
        if visual.is_hovered() {
            palette.text
        } else {
            palette.text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        rect,
        Role::Switch,
        label,
        CursorKind::Pointer,
        false,
        on,
    );
    visual
}

/// A checkbox row.
pub fn checkbox(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    checked: bool,
) -> Visual {
    let visual = canvas.visual(node, false);
    let palette = *canvas.palette;
    let box_rect = Rect::new(
        rect.origin.x,
        rect.origin.y + (rect.size.y - 18.0) * 0.5,
        18.0,
        18.0,
    );
    canvas.rect(
        box_rect,
        4.0,
        if checked {
            palette.primary
        } else if visual.is_hovered() {
            palette.control_hover
        } else {
            palette.card_bg
        },
        if checked { 0 } else { palette.border },
        1.0,
        0.0,
    );
    if checked {
        canvas.text(
            "✓",
            box_rect,
            12.0,
            palette.on_primary,
            AlignX::Center,
            AlignY::Center,
        );
    }
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + 28.0,
            rect.origin.y,
            rect.size.x - 28.0,
            rect.size.y,
        ),
        13.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        rect,
        Role::CheckBox,
        label,
        CursorKind::Pointer,
        false,
        checked,
    );
    visual
}

/// A radio-button row.
pub fn radio(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    selected: bool,
) -> Visual {
    let visual = canvas.visual(node, false);
    let palette = *canvas.palette;
    let circle = Rect::new(
        rect.origin.x,
        rect.origin.y + (rect.size.y - 18.0) * 0.5,
        18.0,
        18.0,
    );
    canvas.rect(
        circle,
        9.0,
        palette.card_bg,
        if selected {
            palette.primary
        } else {
            palette.border
        },
        2.0,
        0.0,
    );
    if selected {
        canvas.rect(circle.inflate(-5.0), 4.0, palette.primary, 0, 0.0, 0.0);
    }
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + 28.0,
            rect.origin.y,
            rect.size.x - 28.0,
            rect.size.y,
        ),
        13.0,
        if visual.is_hovered() {
            palette.text
        } else {
            palette.text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        rect,
        Role::RadioButton,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    visual
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
    let visual = canvas.visual(node, false);
    let palette = *canvas.palette;
    let t = ((value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0);
    let track = Rect::new(
        rect.origin.x,
        rect.origin.y + rect.size.y * 0.5 - 2.0,
        rect.size.x,
        4.0,
    );
    canvas.rect(track, 2.0, palette.control_track, 0, 0.0, 0.0);
    canvas.rect(
        Rect::new(
            track.origin.x,
            track.origin.y,
            track.size.x * t,
            track.size.y,
        ),
        2.0,
        palette.primary,
        0,
        0.0,
        0.0,
    );
    let thumb = Rect::new(
        track.origin.x + track.size.x * t - 8.0,
        track.origin.y + track.size.y * 0.5 - 8.0,
        16.0,
        16.0,
    );
    canvas.rect(
        thumb,
        8.0,
        palette.control_thumb,
        palette.border,
        1.0,
        if visual.is_pressed() { 8.0 } else { 4.0 },
    );
    if canvas.focused(node) {
        canvas.stroke(thumb.inflate(3.0), 11.0, palette.focus_ring, 2.0);
    }
    canvas.control(
        node,
        rect,
        Role::Slider,
        "Slider",
        CursorKind::Pointer,
        false,
        false,
    );
    visual
}

/// A segmented pill control.
pub fn segmented(
    canvas: &mut Canvas<'_>,
    ids: &[u64],
    rect: Rect,
    labels: &[&str],
    selected: usize,
) {
    let palette = *canvas.palette;
    canvas.rect(rect, 9.0, palette.control_track, 0, 0.0, 0.0);
    let count = labels.len().max(1);
    let width = rect.size.x / count as f32;
    for (index, label) in labels.iter().enumerate() {
        let segment = Rect::new(
            rect.origin.x + index as f32 * width + 3.0,
            rect.origin.y + 2.0,
            width - 6.0,
            rect.size.y - 4.0,
        );
        let active = index == selected;
        let visual = canvas.visual(ids[index], false);
        if active {
            canvas.rect(
                segment,
                7.0,
                palette.primary_soft,
                palette.primary,
                1.0,
                0.0,
            );
        } else if visual.is_hovered() {
            canvas.rect(segment, 7.0, palette.control_hover, 0, 0.0, 0.0);
        }
        canvas.text(
            label,
            segment,
            12.0,
            if active {
                palette.primary
            } else {
                palette.text_muted
            },
            AlignX::Center,
            AlignY::Center,
        );
        canvas.control(
            ids[index],
            segment,
            Role::Tab,
            (*label).to_string(),
            CursorKind::Pointer,
            false,
            active,
        );
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
    let palette = *canvas.palette;
    let visual = canvas.visual(node, false);
    if selected {
        canvas.rect(rect, 8.0, palette.nav_selected, 0, 0.0, 0.0);
        canvas.rect(
            Rect::new(
                rect.origin.x - 10.0,
                rect.origin.y + 8.0,
                3.0,
                rect.size.y - 16.0,
            ),
            2.0,
            palette.primary,
            0,
            0.0,
            0.0,
        );
    } else if visual.is_hovered() {
        canvas.rect(rect, 8.0, palette.nav_hover, 0, 0.0, 0.0);
    }
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

//! A select control, composed from the popover and menu primitives.
//!
//! The select owns no menu of its own: [`SelectDesc::menu`] builds a
//! [`MenuDesc`] anchored under the field, and the caller feeds the resulting
//! entry into [`crate::OverlayStack`]. That keeps dropdown placement and
//! dismissal in one place for every popup in the system.
//!
//! The keyboard contract is the one users expect: `Up`/`Down` move the
//! highlight, `Enter` chooses it, `Escape` closes without choosing.

use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::menu::{MenuItem, MenuState};
use crate::overlay::place_anchored_popup;
use crate::primitive::{focus_ring, instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of a select field.
pub const SELECT_LAYER: u8 = 1;
/// The height of one option row.
pub const SELECT_ROW_HEIGHT: f32 = 30.0;
const CHEVRON_HALF_WIDTH: f32 = 4.0;
/// Half the height of the disclosure chevron.
const CHEVRON_HALF_HEIGHT: f32 = 3.0;
/// Stroke width of the disclosure chevron.
const CHEVRON_STROKE: f32 = 1.5;

/// A select field.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectDesc {
    /// The closed field rectangle.
    pub rect: Rect,
    /// The option labels.
    pub options: Vec<String>,
    /// The accessible label.
    pub label: String,
    /// The text shown while nothing is selected.
    pub placeholder: String,
    /// Indices of the options that cannot be chosen.
    pub disabled_options: Vec<usize>,
    /// Optional initial selection index for declarative builders.
    pub initial_selection: Option<usize>,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
}

impl SelectDesc {
    #[must_use]
    pub fn new(rect: Rect, options: Vec<String>) -> Self {
        Self {
            rect,
            options,
            label: String::new(),
            placeholder: String::new(),
            disabled_options: Vec::new(),
            initial_selection: None,
            radius: 8.0,
            z: 730,
        }
    }

    /// The same field with a placeholder shown while nothing is selected.
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The same field with one option disabled.
    #[must_use]
    pub fn with_disabled_option(mut self, index: usize) -> Self {
        if !self.disabled_options.contains(&index) {
            self.disabled_options.push(index);
        }
        self
    }

    /// Whether an option cannot be chosen.
    #[must_use]
    pub fn option_disabled(&self, index: usize) -> bool {
        self.disabled_options.contains(&index)
    }

    /// The text the closed field displays.
    #[must_use]
    pub fn display_text<'a>(&'a self, state: &SelectState) -> &'a str {
        self.options
            .get(state.selected.unwrap_or(usize::MAX))
            .map_or(self.placeholder.as_str(), String::as_str)
    }

    /// The colour slot of the displayed text.
    ///
    /// A placeholder reads as secondary text; a real selection as primary
    /// text. `state.selected` is the only authority, never the label being
    /// empty, which keeps an option whose label is `""` from looking like a
    /// placeholder.
    #[must_use]
    pub fn text_token(&self, state: &SelectState, control: ControlState) -> Token {
        if control.disabled {
            Token::TEXT_DISABLED
        } else if state.selected.is_some() {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        }
    }

    /// The menu descriptor for the open dropdown.
    #[must_use]
    pub fn menu(&self, viewport: Rect) -> crate::menu::MenuDesc {
        let size = Vec2::new(
            self.rect.width(),
            self.options.len() as f32 * SELECT_ROW_HEIGHT,
        );
        let placed = place_anchored_popup(
            self.rect,
            size,
            viewport,
            crate::overlay::Placement::BottomStart,
        );
        let mut menu = crate::menu::MenuDesc::new(
            placed,
            self.options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    let item = MenuItem::action(option.clone());
                    if self.option_disabled(index) {
                        item.disabled()
                    } else {
                        item
                    }
                })
                .collect(),
        );
        menu.z = self.z + 1;
        menu.viewport = Some(viewport);
        menu.clamp_height();
        menu
    }
}

/// The open/closed and selection state of a select.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectState {
    /// Whether the dropdown is open.
    pub open: bool,
    /// The selected option.
    pub selected: Option<usize>,
    /// The highlighted option while open.
    pub highlighted: Option<usize>,
}

impl SelectState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the dropdown, highlighting the current selection.
    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = self.selected;
    }

    /// Close without choosing.
    pub fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
    }

    /// Toggle the dropdown.
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Move the highlight, wrapping and skipping disabled options.
    ///
    /// When every option is disabled the highlight stays where it is.
    pub fn step(&mut self, desc: &SelectDesc, forward: bool) {
        let count = desc.options.len();
        if count == 0 {
            return;
        }
        let start = self
            .highlighted
            .unwrap_or(if forward { count - 1 } else { 0 });
        for offset in 1..=count {
            let index = if forward {
                (start + offset) % count
            } else {
                (start + count - offset) % count
            };
            if !desc.option_disabled(index) {
                self.highlighted = Some(index);
                return;
            }
        }
    }

    /// Commit the highlighted option. Returns the chosen index.
    pub fn commit(&mut self) -> Option<usize> {
        let index = self.highlighted?;
        self.selected = Some(index);
        self.close();
        Some(index)
    }

    /// The selected label.
    #[must_use]
    pub fn label<'a>(&self, desc: &'a SelectDesc) -> Option<&'a str> {
        desc.options
            .get(self.selected?)
            .map(std::string::String::as_str)
    }

    /// The menu highlight for the open dropdown.
    #[must_use]
    pub fn menu_state(&self) -> MenuState {
        MenuState {
            highlighted: self.highlighted,
            ..MenuState::default()
        }
    }
}

/// A select bound to an identity.
#[derive(Clone, Debug)]
pub struct SelectWidget {
    /// The widget identity.
    pub node: u64,
    /// The descriptor.
    pub desc: SelectDesc,
    /// The state.
    pub state: SelectState,
}

impl WidgetDesc for SelectWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_select_with_glyphs(self.node, &self.desc, &self.state, state, glyphs)
    }
}

/// Emit the closed field: fill, border, the disclosure chevron, the caller's
/// glyphs and the focus ring.
#[must_use]
pub fn emit_select(
    node: u64,
    desc: &SelectDesc,
    state: &SelectState,
    control: ControlState,
) -> WidgetOutput {
    emit_select_with_glyphs(node, desc, state, control, &[])
}

/// Emit the closed field with the shaped label glyphs.
///
/// The glyphs are coloured with [`SelectDesc::text_token`], so an empty select
/// paints its placeholder in [`Token::TEXT_SECONDARY`] exactly like a text
/// field does.
#[must_use]
pub fn emit_select_with_glyphs(
    node: u64,
    desc: &SelectDesc,
    state: &SelectState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let fill = if control.disabled {
        Token::DISABLED
    } else if state.open || control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_RAISED
    };
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(rect, desc.radius, fill, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads: Vec<GpuPayload> = Vec::new();
    // The disclosure arrow: two strokes meeting at the apex, pointing down
    // while closed and up while open.
    let apex_y = if state.open {
        -CHEVRON_HALF_HEIGHT
    } else {
        CHEVRON_HALF_HEIGHT
    };
    let center = Vec2::new(rect.max().x - 16.0, rect.origin.y + rect.height() * 0.5);
    let left = Vec2::new(center.x - CHEVRON_HALF_WIDTH, center.y - apex_y);
    let apex = Vec2::new(center.x, center.y + apex_y);
    let right = Vec2::new(center.x + CHEVRON_HALF_WIDTH, center.y - apex_y);
    for (a, b) in [(left, apex), (apex, right)] {
        let (primitive, payload) = chevron_segment(a, b, Token::TEXT_SECONDARY, desc.z);
        primitives.push(primitive);
        payloads.push(payload);
    }

    let ink = desc.text_token(state, control);
    for quad in glyphs {
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
        let mut glyph = instance(
            quad.rect,
            [0.0; 4],
            ink,
            Token::NONE,
            0.0,
            0.0,
            if quad.color {
                PrimType::GlyphColor
            } else {
                PrimType::Glyph
            },
            SubElement::Content,
            desc.z,
        );
        glyph.border_token_or_tex = quad.layer;
        glyph.extra_payload_idx = payload_idx;
        if quad.msdf {
            glyph.prim_type_and_flags |= u32::from(unbound_gpu::data::PrimFlags::MSDF.bits()) << 16;
        }
        primitives.push(glyph);
    }

    primitives.push(focus_ring(
        rect.inflate(3.0),
        desc.radius + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, SELECT_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });
    let display = desc.display_text(state);
    let mut a11y = unbound_a11y::A11yNode::new(
        accesskit::Role::ComboBox,
        if desc.label.is_empty() {
            display.to_owned()
        } else {
            desc.label.clone()
        },
        rect,
    )
    .clickable()
    .disabled(control.disabled);
    a11y.value = display.to_owned();
    a11y.actions.insert(unbound_a11y::A11yAction::Expand);
    a11y.actions.insert(unbound_a11y::A11yAction::Collapse);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// One quadratic stroke of the disclosure chevron.
fn chevron_segment(a: Vec2, b: Vec2, color: Token, z: u16) -> (GpuPrimitiveInstance, GpuPayload) {
    let origin = Vec2::new(a.x.min(b.x), a.y.min(b.y));
    let size = Vec2::new((a.x - b.x).abs().max(1.0), (a.y - b.y).abs().max(1.0));
    let primitive = instance(
        Rect::new(origin.x, origin.y, size.x, size.y),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    let payload = GpuPayload::curve(
        [a.x - origin.x, a.y - origin.y],
        [(a.x + b.x) * 0.5 - origin.x, (a.y + b.y) * 0.5 - origin.y],
        [b.x - origin.x, b.y - origin.y],
        CHEVRON_STROKE,
    );
    (primitive, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SelectDesc {
        SelectDesc::new(
            Rect::new(20.0, 20.0, 200.0, 36.0),
            vec!["Apple".into(), "Banana".into(), "Cherry".into()],
        )
    }

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 600.0)
    }

    #[test]
    fn keyboard_navigation_wraps_and_commits() {
        let desc = desc();
        let mut state = SelectState::new();
        state.open();
        state.step(&desc, true);
        assert_eq!(state.highlighted, Some(0));
        state.step(&desc, true);
        assert_eq!(state.highlighted, Some(1));
        assert_eq!(state.commit(), Some(1));
        assert_eq!(state.selected, Some(1));
        assert!(!state.open);
        assert_eq!(state.label(&desc), Some("Banana"));
    }

    #[test]
    fn escape_closes_without_changing_the_selection() {
        let desc = desc();
        let mut state = SelectState::new();
        state.selected = Some(2);
        state.open();
        assert_eq!(state.highlighted, Some(2));
        state.close();
        assert_eq!(state.selected, Some(2));
        assert_eq!(state.label(&desc), Some("Cherry"));
    }

    #[test]
    fn the_dropdown_is_placed_under_the_field_and_inside_the_viewport() {
        let desc = desc();
        let menu = desc.menu(viewport());
        assert_eq!(menu.rect.origin.x, 20.0);
        assert_eq!(menu.rect.origin.y, 60.0);
        assert_eq!(menu.rect.height(), 90.0);
        assert!(viewport().contains_rect(&menu.rect));
    }

    #[test]
    fn a_dropdown_near_the_bottom_flips_up() {
        let mut desc = desc();
        desc.rect = Rect::new(20.0, 540.0, 200.0, 36.0);
        let menu = desc.menu(viewport());
        assert!(menu.rect.max().y <= desc.rect.origin.y);
        assert!(viewport().contains_rect(&menu.rect));
    }

    #[test]
    fn the_closed_field_is_pickable_and_exposes_combobox_semantics() {
        let desc = desc();
        let mut state = SelectState::new();
        state.selected = Some(1);
        let output = emit_select(4, &desc, &state, ControlState::default());
        assert_eq!(output.primitives.len(), 5);
        assert!(output.hit.is_some());
        let a11y = output.a11y.as_ref().unwrap();
        assert_eq!(a11y.role, accesskit::Role::ComboBox);
        assert_eq!(a11y.label, "Banana");
        assert_eq!(a11y.value, "Banana");
        assert!(output.focusable);
    }

    #[test]
    fn a_disabled_select_is_not_pickable() {
        let desc = desc();
        let state = SelectState::new();
        let disabled = ControlState {
            disabled: true,
            ..ControlState::default()
        };
        let output = emit_select(4, &desc, &state, disabled);
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

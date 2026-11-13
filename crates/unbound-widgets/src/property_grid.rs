//! Inspector-style property grid: sections, modified markers and resets.
//!
//! The grid is the read-mostly half of an inspector. It does not own the
//! model either: [`PropertyDesc`] carries the current value *and* the default,
//! so "is this property modified?" and "reset" are pure comparisons. A reset
//! is reported through [`PropertyGridState::take_reset`] and the application
//! writes the default back into its own model.
//!
//! Sections collapse independently. Collapsed rows are skipped by the layout,
//! not merely painted transparent, so a 300-property document emits only the
//! rows the user can see.

use accesskit::Role;
use rustc_hash::FxHashSet;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// How a property is edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PropertyKind {
    /// A free-form string.
    #[default]
    Text,
    /// A number with an optional unit.
    Number {
        /// The unit shown after the value.
        unit: String,
    },
    /// A boolean switch.
    Bool,
    /// A colour, shown as a hex string.
    Color,
    /// One of a fixed set of strings.
    Select(Vec<String>),
}

impl PropertyKind {
    /// The role the value cell exposes.
    #[must_use]
    pub fn role(&self) -> Role {
        match self {
            Self::Text => Role::TextInput,
            Self::Number { .. } => Role::SpinButton,
            Self::Bool => Role::CheckBox,
            Self::Color => Role::ColorWell,
            Self::Select(_) => Role::ComboBox,
        }
    }
}

/// One property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// A string.
    Text(String),
    /// A number, rendered with the caller's precision.
    Number(f64),
    /// A boolean.
    Bool(bool),
    /// A colour in `#rrggbb` form.
    Color(String),
}

impl PropertyValue {
    /// The text painted in the value column.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Text(text) | Self::Color(text) => text.clone(),
            Self::Number(number) => {
                let text = format!("{number:.2}");
                let trimmed = text.trim_end_matches('0').trim_end_matches('.');
                trimmed.to_owned()
            }
            Self::Bool(value) => {
                if *value {
                    "开".to_owned()
                } else {
                    "关".to_owned()
                }
            }
        }
    }
}

/// One property row.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDesc {
    /// The label in the leading column.
    pub label: String,
    /// The current value.
    pub value: PropertyValue,
    /// The value a reset restores.
    pub default: PropertyValue,
    /// The editor kind.
    pub kind: PropertyKind,
    /// An optional tooltip / a11y description.
    pub description: String,
}

impl PropertyDesc {
    /// A property with `value` as its own default.
    #[must_use]
    pub fn new(label: impl Into<String>, value: PropertyValue) -> Self {
        Self {
            label: label.into(),
            default: value.clone(),
            value,
            kind: PropertyKind::default(),
            description: String::new(),
        }
    }

    /// The same property with an editor kind.
    #[must_use]
    pub fn with_kind(mut self, kind: PropertyKind) -> Self {
        self.kind = kind;
        self
    }

    /// The same property with an explicit default.
    #[must_use]
    pub fn with_default(mut self, default: PropertyValue) -> Self {
        self.default = default;
        self
    }

    /// The same property with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Whether the value differs from the default.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.value != self.default
    }

    /// The value text, including the unit for numbers.
    #[must_use]
    pub fn value_text(&self) -> String {
        let mut text = self.value.display();
        if let PropertyKind::Number { unit } = &self.kind {
            if !unit.is_empty() {
                text.push(' ');
                text.push_str(unit);
            }
        }
        text
    }
}

/// One collapsible group of properties.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertySection {
    /// The section heading.
    pub title: String,
    /// The rows, in order.
    pub rows: Vec<PropertyDesc>,
}

impl PropertySection {
    /// A section with no rows yet.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    /// The same section with a row appended.
    #[must_use]
    pub fn with_row(mut self, property: PropertyDesc) -> Self {
        self.rows.push(property);
        self
    }
}

/// The pure descriptor of a property grid.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyGridDesc {
    /// The whole grid rectangle.
    pub rect: Rect,
    /// The accessible name.
    pub label: String,
    /// The leading column width.
    pub label_width: f32,
    /// The uniform row height.
    pub row_height: f32,
    /// The section heading height.
    pub header_height: f32,
    /// The sections.
    pub sections: Vec<PropertySection>,
    /// The draw order key.
    pub z: u16,
}

impl PropertyGridDesc {
    /// A grid with a 140 px label column and 26 px rows.
    #[must_use]
    pub fn new(rect: Rect, sections: Vec<PropertySection>) -> Self {
        Self {
            rect,
            label: String::new(),
            label_width: 140.0,
            row_height: 26.0,
            header_height: 28.0,
            sections,
            z: 620,
        }
    }

    /// The same grid with an accessible name.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Total number of rows, collapsed sections included.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.sections.iter().map(|section| section.rows.len()).sum()
    }

    /// The reset button of a value cell.
    #[must_use]
    pub fn reset_rect(&self, row: Rect) -> Rect {
        let size = (self.row_height * 0.5).min(16.0);
        Rect::new(
            row.max().x - size - 6.0,
            row.origin.y + (row.size.y - size) * 0.5,
            size,
            size,
        )
    }

    /// The value cell of a row.
    #[must_use]
    pub fn value_rect(&self, row: Rect) -> Rect {
        Rect::new(
            row.origin.x + self.label_width,
            row.origin.y,
            (row.size.x - self.label_width).max(0.0),
            row.size.y,
        )
    }
}

/// The laid-out rectangle of one visible row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyRowLayout {
    /// The section index.
    pub section: usize,
    /// The row index inside the section.
    pub row: usize,
    /// The whole row.
    pub rect: Rect,
    /// The label cell.
    pub label_rect: Rect,
    /// The value cell.
    pub value_rect: Rect,
    /// The reset affordance.
    pub reset_rect: Rect,
    /// The section heading, when this row is the first of its section.
    pub header: Option<Rect>,
}

/// The mutable state of a property grid.
#[derive(Clone, Debug, Default)]
pub struct PropertyGridState {
    collapsed: FxHashSet<usize>,
    selected: Option<(usize, usize)>,
    reset: Option<(usize, usize)>,
    hovered: Option<(usize, usize)>,
}

impl PropertyGridState {
    /// Everything expanded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle a section and return its new collapsed state.
    pub fn toggle_section(&mut self, section: usize) -> bool {
        if !self.collapsed.remove(&section) {
            self.collapsed.insert(section);
            true
        } else {
            false
        }
    }

    /// Whether a section is collapsed.
    #[must_use]
    pub fn is_collapsed(&self, section: usize) -> bool {
        self.collapsed.contains(&section)
    }

    /// Select a cell.
    pub fn select(&mut self, section: usize, row: usize) {
        self.selected = Some((section, row));
    }

    /// The selected cell.
    #[must_use]
    pub fn selected(&self) -> Option<(usize, usize)> {
        self.selected
    }

    /// Mark a row as hovered.
    pub fn hover(&mut self, cell: Option<(usize, usize)>) {
        self.hovered = cell;
    }

    /// The hovered cell.
    #[must_use]
    pub fn hovered(&self) -> Option<(usize, usize)> {
        self.hovered
    }

    /// Request a reset; the caller drains it with [`Self::take_reset`].
    pub fn reset_row(&mut self, section: usize, row: usize) {
        self.reset = Some((section, row));
    }

    /// Take the pending reset request.
    pub fn take_reset(&mut self) -> Option<(usize, usize)> {
        self.reset.take()
    }

    /// Lay out every visible row inside `desc`.
    #[must_use]
    pub fn layout(&self, desc: &PropertyGridDesc) -> Vec<PropertyRowLayout> {
        let mut out = Vec::new();
        let mut y = desc.rect.origin.y;
        for (section_index, section) in desc.sections.iter().enumerate() {
            if y >= desc.rect.max().y {
                break;
            }
            let header = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.header_height);
            y += desc.header_height;
            if self.is_collapsed(section_index) {
                out.push(PropertyRowLayout {
                    section: section_index,
                    row: usize::MAX,
                    rect: header,
                    label_rect: header,
                    value_rect: header,
                    reset_rect: header,
                    header: Some(header),
                });
                continue;
            }
            for (row_index, _) in section.rows.iter().enumerate() {
                if y >= desc.rect.max().y {
                    break;
                }
                let rect = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.row_height);
                out.push(PropertyRowLayout {
                    section: section_index,
                    row: row_index,
                    rect,
                    label_rect: Rect::new(
                        rect.origin.x,
                        rect.origin.y,
                        desc.label_width,
                        rect.size.y,
                    ),
                    value_rect: desc.value_rect(rect),
                    reset_rect: desc.reset_rect(rect),
                    header: (row_index == 0).then_some(header),
                });
                y += desc.row_height;
            }
        }
        out
    }
}

/// The colour slots a property grid paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyGridTokens {
    /// The grid surface.
    pub background: Token,
    /// A section heading.
    pub header: Token,
    /// The label text.
    pub label: Token,
    /// The value text.
    pub value: Token,
    /// The modified marker.
    pub modified: Token,
    /// The row rule.
    pub rule: Token,
    /// The selected cell.
    pub selected: Token,
}

impl PropertyGridTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            background: Token::SURFACE,
            header: Token::HEADER_BG,
            label: Token::TEXT_SECONDARY,
            value: Token::TEXT_PRIMARY,
            modified: Token::PRIMARY,
            rule: Token::DIVIDER,
            selected: Token::SELECTION,
        }
    }
}

/// Emit the visible rows and their section headings.
#[must_use]
pub fn emit_property_grid(
    node: u64,
    desc: &PropertyGridDesc,
    state: &PropertyGridState,
    row_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let tokens = PropertyGridTokens::standard();
    let layout = state.layout(desc);
    let mut primitives = Vec::with_capacity(4 + layout.len() * 3 + row_glyphs.len());
    let mut payloads = Vec::with_capacity(row_glyphs.len());

    primitives.push(surface(desc.rect, 0.0, tokens.background, desc.z));
    for row in &layout {
        if let Some(header) = row.header {
            primitives.push(surface(header, 0.0, tokens.header, desc.z + 1));
        }
        if row.row == usize::MAX {
            primitives.push(line(
                Rect::new(
                    row.rect.origin.x,
                    row.rect.max().y - 1.0,
                    row.rect.size.x,
                    1.0,
                ),
                tokens.rule,
                desc.z + 2,
            ));
            continue;
        }
        if state.selected() == Some((row.section, row.row)) {
            primitives.push(surface(row.rect, 0.0, tokens.selected, desc.z + 1));
        }
        primitives.push(line(
            Rect::new(
                row.rect.origin.x,
                row.rect.max().y - 1.0,
                row.rect.size.x,
                1.0,
            ),
            tokens.rule,
            desc.z + 2,
        ));
        if let Some(property) = desc
            .sections
            .get(row.section)
            .and_then(|section| section.rows.get(row.row))
        {
            if property.is_modified() {
                let dot = Rect::new(
                    row.label_rect.origin.x + 6.0,
                    row.label_rect.origin.y + (row.label_rect.size.y - 5.0) * 0.5,
                    5.0,
                    5.0,
                );
                primitives.push(surface(dot, 2.5, tokens.modified, desc.z + 3));
            }
            // The reset-button wash keeps a fixed slot: hovering only changes
            // its opacity, never the primitive count.
            let mut reset = surface(row.reset_rect.inflate(3.0), 4.0, tokens.header, desc.z + 3);
            reset.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
                reset.depth_u16(),
                if state.hovered() == Some((row.section, row.row)) {
                    1.0
                } else {
                    0.0
                },
            );
            primitives.push(reset);
        }
    }

    glyphs(
        row_glyphs,
        tokens.value,
        desc.z + 4,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        desc.rect,
        0.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 5,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).clickable();
    a11y.description = format!("{} 个属性", desc.row_count());
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Default),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
    }
}

/// A solid line primitive.
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PropertyGridDesc {
        PropertyGridDesc::new(
            Rect::new(0.0, 0.0, 300.0, 200.0),
            vec![
                PropertySection::new("Transform")
                    .with_row(PropertyDesc::new("X", PropertyValue::Number(10.0)))
                    .with_row(
                        PropertyDesc::new("Y", PropertyValue::Number(20.0))
                            .with_default(PropertyValue::Number(0.0)),
                    ),
                PropertySection::new("Appearance")
                    .with_row(PropertyDesc::new("Name", PropertyValue::Text("Box".into()))),
            ],
        )
        .with_label("Inspector")
    }

    #[test]
    fn modification_is_a_comparison_with_the_default() {
        let desc = desc();
        assert!(!desc.sections[0].rows[0].is_modified());
        assert!(desc.sections[0].rows[1].is_modified());
    }

    #[test]
    fn numbers_include_the_unit() {
        let property = PropertyDesc::new("Width", PropertyValue::Number(12.5)).with_kind(
            PropertyKind::Number {
                unit: "px".to_owned(),
            },
        );
        assert_eq!(property.value_text(), "12.5 px");
    }

    #[test]
    fn collapsing_a_section_skips_its_rows() {
        let desc = desc();
        let mut state = PropertyGridState::new();
        let expanded = state.layout(&desc);
        assert_eq!(expanded.len(), 3, "three rows");
        state.toggle_section(0);
        let collapsed = state.layout(&desc);
        assert_eq!(
            collapsed.len(),
            2,
            "collapsed header plus the other section"
        );
        assert_eq!(collapsed[0].row, usize::MAX);
    }

    #[test]
    fn rows_are_stacked_under_their_header() {
        let desc = desc();
        let state = PropertyGridState::new();
        let layout = state.layout(&desc);
        let first = layout[0];
        assert_eq!(first.rect.origin.y, desc.header_height);
        assert_eq!(
            layout[1].rect.origin.y,
            desc.header_height + desc.row_height
        );
        assert_eq!(
            layout[2].header.map(|header| header.origin.y),
            Some(desc.header_height + desc.row_height * 2.0)
        );
    }

    #[test]
    fn reset_requests_are_drained_once() {
        let mut state = PropertyGridState::new();
        state.reset_row(0, 1);
        assert_eq!(state.take_reset(), Some((0, 1)));
        assert_eq!(state.take_reset(), None);
    }

    #[test]
    fn the_value_cell_starts_after_the_label_column() {
        let desc = desc();
        let state = PropertyGridState::new();
        let row = state.layout(&desc)[1];
        assert_eq!(row.label_rect.size.x, 140.0);
        assert_eq!(row.value_rect.origin.x, 140.0);
        assert!(row.reset_rect.max().x <= desc.rect.max().x);
    }

    #[test]
    fn the_emitter_reports_a_group_with_the_row_count() {
        let desc = desc();
        let state = PropertyGridState::new();
        let output = emit_property_grid(9, &desc, &state, &[], ControlState::default());
        let a11y = output.a11y.expect("group");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.description, "3 个属性");
        assert!(output.hit.is_some());
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let mut counts = Vec::new();
        for hover in [None, Some((0, 1))] {
            let mut state = PropertyGridState::new();
            state.hover(hover);
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_property_grid(9, &desc, &state, &[], control);
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

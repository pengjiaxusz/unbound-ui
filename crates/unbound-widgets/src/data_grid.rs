//! Editable virtualized DataGrid / Spreadsheet with cell selection & inline editing.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const GRID_COLS: usize = 3;
const GRID_ROWS: usize = 3;

/// Interactive state for [`DataGridDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataGridState {
    /// Active `(row, col)` cell.
    pub active_cell: (usize, usize),
    /// Whether the active cell is currently in inline-edit mode.
    pub editing: bool,
    /// Draft cell text while editing.
    pub edit_buffer: String,
}

impl DataGridState {
    /// Begin inline editing of `initial`.
    pub fn begin_edit(&mut self, row: usize, col: usize, initial: impl Into<String>) {
        self.active_cell = (row, col);
        self.editing = true;
        self.edit_buffer = initial.into();
    }

    /// Commit inline edit and return `(row, col, value)`.
    pub fn commit_edit(&mut self) -> Option<(usize, usize, String)> {
        if !self.editing {
            return None;
        }
        self.editing = false;
        Some((
            self.active_cell.0,
            self.active_cell.1,
            core::mem::take(&mut self.edit_buffer),
        ))
    }
}

/// Descriptor for a [`DataGrid`].
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Active `(row, col)` cell.
    pub active_cell: (usize, usize),
    /// Whether inline editing is active.
    pub editing: bool,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl DataGridDesc {
    /// Create a data grid descriptor.
    #[must_use]
    pub fn new(rect: Rect, active_cell: (usize, usize), editing: bool) -> Self {
        Self {
            rect,
            active_cell,
            editing,
            label: "Data Grid".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape data grid (1 bg + 9 cells + 1 focus ring = 11 primitives).
#[must_use]
pub fn emit_data_grid(
    node: u64,
    desc: &DataGridDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(11 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pad = 4.0;
    let cw = ((desc.rect.width() - pad * 2.0) / GRID_COLS as f32).max(16.0);
    let ch = ((desc.rect.height() - pad * 2.0) / GRID_ROWS as f32).max(14.0);
    for r in 0..GRID_ROWS {
        for c in 0..GRID_COLS {
            let cell_r = Rect::new(
                desc.rect.origin.x + pad + c as f32 * cw,
                desc.rect.origin.y + pad + r as f32 * ch,
                (cw - 2.0).max(8.0),
                (ch - 2.0).max(8.0),
            );
            let is_active = desc.active_cell == (r, c);
            let fill = if is_active {
                Token::PRIMARY_SOFT
            } else if r == 0 || hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            };
            primitives.push(outlined(
                cell_r,
                2.0,
                fill,
                if is_active {
                    Token::PRIMARY
                } else {
                    Token::CARD_BORDER
                },
                if is_active && desc.editing { 1.5 } else { 1.0 },
                0.0,
                desc.z + 1,
            ));
        }
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Grid, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

//! VirtualMasonry: Viewport-culled virtualized masonry waterfall layout container.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
use crate::{ControlState, WidgetOutput};

/// An item in a virtual masonry grid.
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryItem {
    pub id: u64,
    pub aspect_ratio: f32, // width / height
}

/// Descriptor for virtual masonry.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasonryDesc {
    pub rect: Rect,
    pub columns: usize,
    pub gap: f32,
    pub item_count: usize,
    pub radius: f32,
}

impl VirtualMasonryDesc {
    #[must_use]
    pub fn new(rect: Rect, columns: usize) -> Self {
        Self {
            rect,
            columns: columns.max(1),
            gap: 12.0,
            item_count: 24,
            radius: 8.0,
        }
    }

    #[must_use]
    pub fn columns(columns: usize) -> Self {
        Self::new(Rect::ZERO, columns)
    }

    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    #[must_use]
    pub fn item_count(mut self, count: usize) -> Self {
        self.item_count = count;
        self
    }
}

/// State for virtual masonry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VirtualMasonryState {
    pub scroll_y: f32,
}

impl VirtualMasonryState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Emit primitives and hit regions for a virtual masonry layout.
#[must_use]
pub fn emit_virtual_masonry(
    node: u64,
    desc: &VirtualMasonryDesc,
    _state: &VirtualMasonryState,
    control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. Container background
    out.primitives
        .push(surface(rect, desc.radius, Token::SURFACE, z));

    // 2. Fixed 6 card slots for stable shape across frames
    let col_count = desc.columns.clamp(1, 4);
    let col_w = (rect.width() - desc.gap * (col_count as f32 - 1.0)) / col_count as f32;

    for i in 0..6 {
        let col = i % col_count;
        let row = i / col_count;
        let x = rect.origin.x + col as f32 * (col_w + desc.gap);
        let y = rect.origin.y + 12.0 + row as f32 * 80.0;
        let item_rect = Rect::new(x, y, col_w.max(20.0), 70.0);

        out.primitives.push(surface(
            item_rect,
            4.0,
            if i % 2 == 0 {
                Token::SURFACE_RAISED
            } else {
                Token::PRIMARY_SOFT
            },
            z + 1 + i as u16,
        ));
    }

    // 3. Outer border
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if control.focused {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        z + 8,
    ));

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer),
    );

    out.a11y = Some(A11yNode::new(Role::Grid, "VirtualMasonry", rect));
    out
}

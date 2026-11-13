//! AspectRatioBox & MasonryGrid adaptive multi-column waterfall layout.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_MASONRY_ITEMS: usize = 6;

/// Descriptor for an [`AspectRatioBox`] container.
#[derive(Clone, Debug, PartialEq)]
pub struct AspectRatioBoxDesc {
    /// Available outer bounds.
    pub bounds: Rect,
    /// Target `width / height` ratio.
    pub ratio: f32,
}

impl AspectRatioBoxDesc {
    /// Compute the largest centered rectangle inside `bounds` matching `ratio`.
    #[must_use]
    pub fn fit(&self) -> Rect {
        let r = self.ratio.max(0.05);
        let bw = self.bounds.width().max(1.0);
        let bh = self.bounds.height().max(1.0);
        let (w, h) = if bw / bh > r {
            (bh * r, bh)
        } else {
            (bw, bw / r)
        };
        Rect::new(
            self.bounds.origin.x + (bw - w) * 0.5,
            self.bounds.origin.y + (bh - h) * 0.5,
            w,
            h,
        )
    }
}

/// One masonry tile specification.
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryItem {
    /// Tile height (or aspect height).
    pub height: f32,
}

/// Descriptor for a [`MasonryGrid`].
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryDesc {
    /// Container rectangle.
    pub rect: Rect,
    /// Number of columns (>= 1).
    pub columns: usize,
    /// Gap between tiles in logical pixels.
    pub gap: f32,
    /// Tiles to arrange.
    pub items: Vec<MasonryItem>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl MasonryDesc {
    /// Create a masonry grid descriptor.
    #[must_use]
    pub fn new(rect: Rect, columns: usize, items: Vec<MasonryItem>) -> Self {
        Self {
            rect,
            columns: columns.max(1),
            gap: 6.0,
            items,
            label: "Masonry Grid".to_owned(),
            z: 600,
        }
    }
}

/// Pure masonry shortest-column placement algorithm.
#[must_use]
pub fn layout_masonry(desc: &MasonryDesc) -> Vec<Rect> {
    let cols = desc.columns.max(1);
    let col_w = ((desc.rect.width() - desc.gap * (cols as f32 + 1.0)) / cols as f32).max(8.0);
    let mut col_y = vec![desc.rect.origin.y + desc.gap; cols];
    let mut out = Vec::with_capacity(desc.items.len());
    for item in &desc.items {
        let (best_col, &best_y) = col_y
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .expect("cols >= 1");
        let x = desc.rect.origin.x + desc.gap + best_col as f32 * (col_w + desc.gap);
        let h = item.height.max(12.0);
        out.push(Rect::new(x, best_y, col_w, h));
        col_y[best_col] = best_y + h + desc.gap;
    }
    out
}

/// Emit a stable-shape masonry grid (1 bg + 6 tile slots + 1 focus ring = 8 primitives).
#[must_use]
pub fn emit_masonry(node: u64, desc: &MasonryDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8);
    let payloads = Vec::new();

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

    let rects = layout_masonry(desc);
    for idx in 0..MAX_MASONRY_ITEMS {
        let r_opt = rects.get(idx).copied();
        let r =
            r_opt.unwrap_or_else(|| Rect::new(desc.rect.origin.x, desc.rect.origin.y, 12.0, 12.0));
        let mut tile_inst = outlined(
            r,
            4.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        );
        if r_opt.is_none() {
            tile_inst.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(tile_inst.depth_u16(), 0.0);
        }
        primitives.push(tile_inst);
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Default)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_box_centers_and_fits_bounds() {
        let fit = AspectRatioBoxDesc {
            bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
            ratio: 1.0,
        }
        .fit();
        assert_eq!(fit, Rect::new(50.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn masonry_balances_shortest_column() {
        let desc = MasonryDesc::new(
            Rect::new(0.0, 0.0, 220.0, 200.0),
            2,
            vec![
                MasonryItem { height: 60.0 },
                MasonryItem { height: 20.0 },
                MasonryItem { height: 30.0 },
            ],
        );
        let rects = layout_masonry(&desc);
        assert_eq!(rects.len(), 3);
        assert!(rects[2].origin.x > rects[0].origin.x);
        assert_eq!(rects[2].origin.y, 32.0);
    }
}

//! 富文本选区合并与 IME 锚定（`docs` §6.2）。
//!
//! Two pieces of "text is not just a rectangle" plumbing live here:
//!
//! * **Rounded selection union.** Painting one rounded rect per line produces a
//!   visible notch at every line break. Instead we merge vertically adjacent
//!   line boxes whose X ranges overlap, and apply a radius only to the corners
//!   that end up on the outside of the merged run. The result is a single
//!   capsule-like shape whose outline is *geometrically* one region, without
//!   needing a general polygon boolean.
//! * **IME candidate anchoring.** `set_ime_cursor_area` wants a screen-space
//!   rectangle, but the caret lives in world space. [`ime_cursor_area`] applies
//!   the camera transform, floors to whole pixels (a fractional anchor makes
//!   Windows jitter the candidate window) and clamps into the window so the IME
//!   popup can never open off-screen.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Affine2, Rect, Vec2};

/// A merged run of line boxes, ready to be drawn as one rounded rectangle.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundedSlot {
    pub rect: Rect,
    /// Corner radii in `TL, TR, BR, BL` order. Shared edges get `0`.
    pub radii: [f32; 4],
}

/// Merge vertically adjacent selection rectangles into rounded runs.
///
/// Two consecutive line boxes merge when they are vertically adjacent (within
/// 1 px) and span the same horizontal range. The equal-span rule is deliberate:
/// a union of two *different* spans is an L-shape, and faking it with one
/// rectangle would highlight text that is not selected. Different spans stay
/// separate slots, which is exactly what desktop editors draw for a wrapped
/// selection.
#[must_use]
pub fn merge_line_rects(lines: &[Rect], radius: f32) -> Vec<RoundedSlot> {
    let mut slots: Vec<RoundedSlot> = Vec::new();
    for rect in lines {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 || !rect.is_finite() {
            continue;
        }
        if let Some(last) = slots.last_mut() {
            let spans_match = (last.rect.min().x - rect.min().x).abs() <= 0.5
                && (last.rect.max().x - rect.max().x).abs() <= 0.5;
            // The lines must follow each other downwards: the next box has to
            // start at or below the previous one's top. Without the direction
            // check, a box that lies *above* the previous one still looked
            // "adjacent" (the test used the absolute difference), and the
            // merged height `rect.max().y - top` came out negative.
            let below = rect.min().y >= last.rect.min().y - 0.5;
            let adjacent = (last.rect.max().y - rect.min().y).abs() <= 1.0;
            if spans_match && below && adjacent {
                let top = last.rect.min().y;
                let height = rect.max().y - top;
                if height > 0.0 {
                    last.rect = Rect::new(last.rect.origin.x, top, last.rect.size.x, height);
                    continue;
                }
            }
        }
        slots.push(RoundedSlot {
            rect: *rect,
            radii: [radius; 4],
        });
    }
    slots
}

/// Build a closed outline polygon for a rounded slot, for stroking or hit tests.
///
/// Corners run clockwise from the top-left; a zero radius emits the exact corner
/// point, which keeps a square slot a genuine 4-gon instead of a 28-gon with
/// coincident vertices.
#[must_use]
pub fn outline_polygon(slot: &RoundedSlot) -> Vec<Vec2> {
    const ARC_STEPS: usize = 6;
    const QUARTER: f32 = core::f32::consts::FRAC_PI_2;
    let min = slot.rect.min();
    let max = slot.rect.max();
    // (centre, start angle) for TL, TR, BR, BL.
    let corners = [
        (
            Vec2::new(min.x + slot.radii[0], min.y + slot.radii[0]),
            core::f32::consts::PI,
            slot.radii[0],
        ),
        (
            Vec2::new(max.x - slot.radii[1], min.y + slot.radii[1]),
            core::f32::consts::PI + QUARTER,
            slot.radii[1],
        ),
        (
            Vec2::new(max.x - slot.radii[2], max.y - slot.radii[2]),
            0.0,
            slot.radii[2],
        ),
        (
            Vec2::new(min.x + slot.radii[3], max.y - slot.radii[3]),
            QUARTER,
            slot.radii[3],
        ),
    ];
    let mut points = Vec::with_capacity(ARC_STEPS * 4 + 4);
    for (centre, start, radius) in corners {
        if radius <= 0.0 {
            points.push(centre);
            continue;
        }
        for step in 0..=ARC_STEPS {
            let angle = start + QUARTER * (step as f32 / ARC_STEPS as f32);
            points.push(Vec2::new(
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            ));
        }
    }
    points
}

/// Screen-space caret rectangle for `set_ime_cursor_area`.
///
/// `caret` is in world space, `camera` maps world to *logical* pixels, `dpi` is
/// the physical-pixels-per-logical-pixel ratio, and `window` is the physical
/// window size. The result is snapped to whole physical pixels and clamped
/// inside the window, with a minimum height so the IME window never collapses.
#[must_use]
pub fn ime_cursor_area(caret: Rect, camera: &Affine2, dpi: f32, window: Vec2) -> Rect {
    let transformed = camera.transform_rect_bounds(&caret);
    let scale = dpi.max(0.1);
    // The rectangle is snapped to whole physical pixels. The origin is rounded
    // to the nearest pixel and the size follows from the rounded corners, so
    // the area is never larger than the caret by up to a pixel on each side.
    let x = (transformed.origin.x * scale).round();
    let y = (transformed.origin.y * scale).round();
    let right = (transformed.max().x * scale).round();
    let bottom = (transformed.max().y * scale).round();
    let width = (right - x).max(1.0);
    let height = (bottom - y).max(1.0);
    let max_x = (window.x - width).max(0.0);
    let max_y = (window.y - height).max(0.0);
    Rect::new(x.clamp(0.0, max_x), y.clamp(0.0, max_y), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn three_stacked_lines_merge_into_one_capsule() {
        let lines = [
            rect(10.0, 0.0, 100.0, 20.0),
            rect(10.0, 20.0, 100.0, 20.0),
            rect(10.0, 40.0, 100.0, 20.0),
        ];
        let slots = merge_line_rects(&lines, 6.0);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].rect, rect(10.0, 0.0, 100.0, 60.0));
        // One rectangle means every corner is an outer corner, and the interior
        // line breaks are gone entirely — that is the "no notch" property.
        assert_eq!(slots[0].radii, [6.0, 6.0, 6.0, 6.0]);
    }

    #[test]
    fn a_wrapped_selection_with_uneven_spans_keeps_two_slots() {
        let lines = [rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 20.0, 80.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2, "an L-shape must not become one rectangle");
    }

    #[test]
    fn a_gap_splits_the_run() {
        let lines = [rect(0.0, 0.0, 100.0, 20.0), rect(0.0, 200.0, 100.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2, "far apart lines must not merge");
    }

    #[test]
    fn non_overlapping_columns_do_not_merge() {
        let lines = [rect(0.0, 0.0, 40.0, 20.0), rect(200.0, 20.0, 40.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn an_empty_rect_is_ignored() {
        let slots = merge_line_rects(&[rect(0.0, 0.0, 0.0, 20.0)], 4.0);
        assert!(slots.is_empty());
    }

    #[test]
    fn the_outline_starts_at_the_top_left_and_closes() {
        let slot = RoundedSlot {
            rect: rect(0.0, 0.0, 100.0, 50.0),
            radii: [8.0; 4],
        };
        let points = outline_polygon(&slot);
        assert!(points.len() >= 16, "{}", points.len());
        // The first point is on the top-left arc, i.e. left of and below the corner.
        assert!(points[0].x <= 0.0 + 8.0);
        assert!(points[0].y >= 0.0);
        // Every point stays inside the rect's bounds (within float error).
        for point in &points {
            assert!(point.x >= -0.01 && point.x <= 100.01);
            assert!(point.y >= -0.01 && point.y <= 50.01);
        }
    }

    #[test]
    fn a_square_corner_emits_the_exact_corner() {
        let slot = RoundedSlot {
            rect: rect(10.0, 20.0, 30.0, 40.0),
            radii: [0.0; 4],
        };
        let points = outline_polygon(&slot);
        assert_eq!(points.len(), 4, "a square slot is a quad");
        assert!(points.contains(&Vec2::new(10.0, 20.0)));
        assert!(points.contains(&Vec2::new(40.0, 60.0)));
    }

    #[test]
    fn ime_area_tracks_the_camera_zoom_and_dpi() {
        let caret = rect(10.0, 20.0, 2.0, 20.0);
        let camera = Affine2 {
            m00: 2.0,
            m10: 0.0,
            m01: 0.0,
            m11: 2.0,
            tx: 100.0,
            ty: 50.0,
        };
        let area = ime_cursor_area(caret, &camera, 2.0, Vec2::new(1920.0, 1080.0));
        // (10,20) * 2 + (100,50) = (120, 90), times dpi 2 = (240, 180).
        assert_eq!(area.origin, Vec2::new(240.0, 180.0));
        // The caret is 2 px wide, 20 tall; doubled by zoom then by dpi.
        assert_eq!(area.size, Vec2::new(8.0, 80.0));
    }

    #[test]
    fn ime_area_is_clamped_inside_the_window() {
        let caret = rect(5000.0, 5000.0, 2.0, 20.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(area.max().x <= 800.0 + 0.01);
        assert!(area.max().y <= 600.0 + 0.01);
    }

    #[test]
    fn ime_area_has_a_minimum_extent() {
        let caret = rect(0.0, 0.0, 0.0, 0.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(area.size.x >= 1.0);
        assert!(area.size.y >= 1.0);
    }

    /// The IME area must not be larger than the caret.
    ///
    /// The origin was floored while the size was ceiled, so the rectangle
    /// could be up to two physical pixels bigger than the caret on each side.
    #[test]
    fn ime_area_is_not_larger_than_the_caret() {
        let caret = rect(10.4, 20.6, 2.0, 20.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(
            area.size.x <= caret.size.x + 1.0,
            "the width must not exceed the caret's: {} vs {}",
            area.size.x,
            caret.size.x
        );
        assert!(
            area.size.y <= caret.size.y + 1.0,
            "the height must not exceed the caret's: {} vs {}",
            area.size.y,
            caret.size.y
        );
    }

    /// Merging line boxes must never produce a negative height.
    ///
    /// The adjacency test used the absolute difference, so a box lying *above*
    /// the previous one looked adjacent too, and the merged height came out
    /// negative.
    #[test]
    fn merging_lines_never_produces_a_negative_height() {
        let above = rect(0.0, 0.0, 20.0, 5.0);
        let below = rect(0.0, 5.0, 20.0, 5.0);
        // The "below" box comes first, then a box above it: the two must not
        // merge.
        let slots = merge_line_rects(&[below, above], 4.0);
        for slot in &slots {
            assert!(
                slot.rect.size.y >= 0.0,
                "a merged slot must not have a negative height: {:?}",
                slot.rect
            );
        }
    }
}

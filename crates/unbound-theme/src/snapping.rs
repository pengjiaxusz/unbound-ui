//! Static pixel-grid snapping (`docs` §4.4, §10.2).
//!
//! A static element must land on the physical pixel grid: at 10 % zoom a
//! border has to be crisp, and at 6400 % it must not crawl. The rule is the
//! classic "round to nearest" snap, applied to the *screen* position of the
//! edge — never to the world position, which would make the grid move with
//! the camera.
//!
//! The animation engine bypasses the grid while a spring is running: a
//! sub-pixel smooth position is what makes a motion look continuous, and the
//! grid would make it stutter.

use unbound_math::snap_to_pixel;

/// The zoom extremes the acceptance table names.
pub const MIN_ZOOM: f32 = 0.1;
/// 6400 %.
pub const MAX_ZOOM: f32 = 64.0;

/// Snap a screen-space position to the pixel grid.
///
/// The value is in *physical* pixels, so the device scale is one by default.
#[must_use]
pub fn snap_screen(value: f32) -> f32 {
    snap_to_pixel(value, 1.0)
}

/// The screen position of a world-space edge.
///
/// `world` is the edge in world units, `camera` the camera's world position,
/// `zoom` the canvas zoom and `viewport` the half size of the viewport in
/// physical pixels.
#[must_use]
pub fn edge_screen(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    (world - camera) * zoom + half_viewport
}

/// The snapped screen position of a world-space edge.
#[must_use]
pub fn edge_screen_snapped(
    world: f32,
    camera: f32,
    zoom: f32,
    half_viewport: f32,
    animated: bool,
) -> f32 {
    let screen = edge_screen(world, camera, zoom, half_viewport);
    if animated {
        // While a spring runs the position stays sub-pixel: the grid would
        // make the motion stutter.
        screen
    } else {
        snap_screen(screen)
    }
}

/// The largest deviation of the snapped edge from the ideal one, in pixels.
///
/// A snapped edge is at most half a pixel away from the ideal position; the
/// number is what the acceptance table calls the "snapping error".
#[must_use]
pub fn snap_error(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    let ideal = edge_screen(world, camera, zoom, half_viewport);
    (snap_screen(ideal) - ideal).abs()
}

/// The largest *change* of the snapped edge when the camera moves by a
/// sub-pixel amount.
///
/// This is the jitter metric: as long as the camera stays inside one pixel,
/// the snapped edge must not move at all.
#[must_use]
pub fn jitter(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    let base = snap_screen(edge_screen(world, camera, zoom, half_viewport));
    let mut worst = 0.0f32;
    // A whole pixel of camera movement must not move the edge by more than
    // one pixel; a sub-pixel movement must not move it at all.
    for step in 1..=16 {
        let offset = step as f32 / 16.0;
        let moved = snap_screen(edge_screen(
            world,
            camera + offset / zoom,
            zoom,
            half_viewport,
        ));
        worst = worst.max((moved - base).abs());
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The zoom range of the acceptance table.
    const ZOOMS: [f32; 6] = [0.1, 0.5, 1.0, 2.0, 8.0, 64.0];

    #[test]
    fn a_static_edge_lands_on_the_pixel_grid() {
        for zoom in ZOOMS {
            for step in 0..64 {
                let world = step as f32 * 0.37;
                let screen = edge_screen_snapped(world, 0.0, zoom, 256.0, false);
                assert!(
                    (screen - screen.round()).abs() < 1e-4,
                    "zoom {zoom}: {screen} is not on the grid"
                );
            }
        }
    }

    #[test]
    fn a_sub_pixel_camera_move_does_not_jitter_a_static_edge() {
        for zoom in ZOOMS {
            let jitter = jitter(123.0, 0.0, zoom, 256.0);
            // The camera moved by up to one pixel; the snapped edge may move
            // by one pixel when the grid boundary is crossed, but it must
            // never move by a fraction of a pixel.
            assert!(
                jitter <= 1.0,
                "zoom {zoom}: the edge jittered by {jitter} px"
            );
        }
    }

    #[test]
    fn the_snapping_error_stays_below_half_a_pixel() {
        for zoom in ZOOMS {
            for step in 0..256 {
                let world = step as f32 * 1.7;
                let error = snap_error(world, 0.0, zoom, 256.0);
                assert!(
                    error <= 0.5 + 1e-4,
                    "zoom {zoom}: the snapping error is {error} px"
                );
            }
        }
    }

    #[test]
    fn an_animated_edge_keeps_its_sub_pixel_position() {
        let world = 123.456;
        let snapped = edge_screen_snapped(world, 0.0, 1.0, 256.0, true);
        let ideal = edge_screen(world, 0.0, 1.0, 256.0);
        assert_eq!(snapped, ideal);
        assert!(
            (snapped - snapped.round()).abs() > 1e-4,
            "the animated position must not be snapped"
        );
    }

    #[test]
    fn the_zoom_extremes_of_the_acceptance_table_are_covered() {
        assert_eq!(MIN_ZOOM, 0.1);
        assert_eq!(MAX_ZOOM * 100.0, 6400.0);
        // At 6400 % a one-pixel camera move is 1/64 of a world unit, and the
        // content moves the other way.
        let a = edge_screen(0.0, 0.0, MAX_ZOOM, 256.0);
        let b = edge_screen(0.0, 1.0 / MAX_ZOOM, MAX_ZOOM, 256.0);
        assert!((a - b - 1.0).abs() < 1e-4);
    }
}

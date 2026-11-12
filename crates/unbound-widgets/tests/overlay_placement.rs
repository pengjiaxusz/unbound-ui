//! Anchored popups must never leave the window.
//!
//! This is the placement half of the overlay contract: for every anchor on or
//! near an edge and every requested side, the produced rectangle has to be
//! fully contained in the viewport.

use unbound_math::{Rect, Vec2};
use unbound_widgets::{place_anchored_popup, Placement};

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 720.0)
}

#[test]
fn every_corner_and_edge_anchor_stays_inside_the_viewport() {
    let sizes = [Vec2::new(240.0, 180.0), Vec2::new(80.0, 60.0)];
    let anchors = [
        Rect::new(0.0, 0.0, 88.0, 32.0),
        Rect::new(640.0, 0.0, 88.0, 32.0),
        Rect::new(1192.0, 0.0, 88.0, 32.0),
        Rect::new(0.0, 344.0, 88.0, 32.0),
        Rect::new(1192.0, 344.0, 88.0, 32.0),
        Rect::new(0.0, 688.0, 88.0, 32.0),
        Rect::new(640.0, 688.0, 88.0, 32.0),
        Rect::new(1192.0, 688.0, 88.0, 32.0),
        Rect::new(-30.0, -20.0, 88.0, 32.0),
        Rect::new(1230.0, 710.0, 88.0, 32.0),
    ];
    let placements = [
        Placement::BottomStart,
        Placement::BottomEnd,
        Placement::TopStart,
        Placement::TopEnd,
        Placement::RightStart,
        Placement::LeftStart,
    ];
    for size in sizes {
        for anchor in anchors {
            for placement in placements {
                let popup = place_anchored_popup(anchor, size, viewport(), placement);
                assert!(
                    viewport().contains_rect(&popup),
                    "{placement:?} at {anchor:?} produced {popup:?}"
                );
            }
        }
    }
}

#[test]
fn a_preferred_side_is_kept_when_it_fits() {
    let anchor = Rect::new(400.0, 300.0, 100.0, 30.0);
    let popup = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 120.0),
        viewport(),
        Placement::BottomStart,
    );
    assert_eq!(popup.origin, Vec2::new(400.0, 334.0));
}

#[test]
fn flipping_only_happens_when_the_other_side_fits() {
    // A popup taller than the space above *and* below cannot flip; it is
    // shifted inside the viewport instead.
    let anchor = Rect::new(400.0, 20.0, 100.0, 30.0);
    let popup = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 700.0),
        viewport(),
        Placement::BottomStart,
    );
    assert_eq!(popup.origin.y, 20.0);
    assert_eq!(popup.height(), 700.0);
    assert!(viewport().contains_rect(&popup));
}

#[test]
fn a_popup_larger_than_the_viewport_is_clipped_to_it() {
    let anchor = Rect::new(10.0, 10.0, 20.0, 20.0);
    let popup = place_anchored_popup(
        anchor,
        Vec2::new(4096.0, 4096.0),
        viewport(),
        Placement::BottomEnd,
    );
    assert_eq!(popup, viewport());
}

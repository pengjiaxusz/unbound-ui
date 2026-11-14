//! 锚定弹窗绝不能超出窗口。
//!
//! 这是浮层契约中的定位部分：对于边缘或近边缘的任意锚点及任意请求方向，
//! 生成的矩形必须完全包含在视口内。
//!

use unbound_math::{Rect, Vec2};
use unbound_widgets::overlay::{OverlayEntry, OverlayStack, PlacedPopup};
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
    let popup: Rect = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 120.0),
        viewport(),
        Placement::BottomStart,
    );
    assert_eq!(popup.origin, Vec2::new(400.0, 334.0));
}

#[test]
fn flipping_only_happens_when_the_other_side_fits() {
    // 高于上下可用空间的弹窗无法翻转，而是被平移回视口内。
    //
    let anchor = Rect::new(400.0, 20.0, 100.0, 30.0);
    let popup: Rect = place_anchored_popup(
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
    let popup: Rect = place_anchored_popup(
        anchor,
        Vec2::new(4096.0, 4096.0),
        viewport(),
        Placement::BottomEnd,
    );
    assert_eq!(popup, viewport());
}

#[test]
fn all_twelve_placements_stay_inside_the_viewport() {
    let anchors = [
        Rect::new(0.0, 0.0, 88.0, 32.0),
        Rect::new(640.0, 0.0, 88.0, 32.0),
        Rect::new(1192.0, 0.0, 88.0, 32.0),
        Rect::new(0.0, 344.0, 88.0, 32.0),
        Rect::new(1192.0, 344.0, 88.0, 32.0),
        Rect::new(0.0, 688.0, 88.0, 32.0),
        Rect::new(640.0, 688.0, 88.0, 32.0),
        Rect::new(1192.0, 688.0, 88.0, 32.0),
    ];
    let placements = [
        Placement::BottomStart,
        Placement::BottomEnd,
        Placement::Bottom,
        Placement::TopStart,
        Placement::TopEnd,
        Placement::Top,
        Placement::RightStart,
        Placement::RightEnd,
        Placement::Right,
        Placement::LeftStart,
        Placement::LeftEnd,
        Placement::Left,
    ];
    for anchor in anchors {
        for placement in placements {
            let placed: PlacedPopup =
                place_anchored_popup(anchor, Vec2::new(240.0, 180.0), viewport(), placement);
            assert!(
                viewport().contains_rect(&placed.rect),
                "{placement:?} at {anchor:?} produced {:?}",
                placed.rect
            );
        }
    }
}

#[test]
fn a_centred_side_centres_on_the_anchor() {
    let anchor = Rect::new(400.0, 300.0, 100.0, 40.0);
    let below: PlacedPopup = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 100.0),
        viewport(),
        Placement::Bottom,
    );
    assert_eq!(below.rect.center().x, anchor.center().x);
    assert_eq!(below.chosen_placement, Placement::Bottom);

    let left: PlacedPopup =
        place_anchored_popup(anchor, Vec2::new(200.0, 100.0), viewport(), Placement::Left);
    assert_eq!(left.rect.center().y, anchor.center().y);
    assert_eq!(left.chosen_placement, Placement::Left);
}

#[test]
fn the_arrow_tip_sits_on_the_chosen_edge_and_points_at_the_anchor() {
    let anchor = Rect::new(100.0, 650.0, 120.0, 36.0);
    let placed: PlacedPopup = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 160.0),
        viewport(),
        Placement::Bottom,
    );
    // 下方空间不足，因此向上翻转，箭头位于底边。
    assert_eq!(placed.chosen_placement, Placement::Top);
    assert_eq!(placed.arrow_tip.y, placed.rect.max().y);
    assert_eq!(placed.arrow_tip.x, anchor.center().x);

    let side: PlacedPopup = place_anchored_popup(
        anchor,
        Vec2::new(200.0, 160.0),
        viewport(),
        Placement::Right,
    );
    assert_eq!(side.arrow_tip.x, side.rect.origin.x);
}

#[test]
fn a_clipped_anchor_hides_the_overlay_without_closing_it() {
    let mut stack = OverlayStack::new();
    stack.push(OverlayEntry::popover(
        1,
        10,
        Rect::new(0.0, 0.0, 100.0, 100.0),
    ));
    stack.push(OverlayEntry::menu(
        2,
        11,
        Rect::new(50.0, 50.0, 100.0, 100.0),
    ));
    assert_eq!(stack.top().map(|entry| entry.node), Some(2));

    assert!(stack.set_anchor_clipped(2, true));
    assert_eq!(stack.top().map(|entry| entry.node), Some(1));
    assert_eq!(stack.len(), 2, "clipping hides, it does not close");

    assert!(stack.set_anchor_clipped(2, false));
    assert_eq!(stack.top().map(|entry| entry.node), Some(2));
}

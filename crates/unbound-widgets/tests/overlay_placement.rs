//! 锚定弹窗与工具提示（Tooltip）绝不能超出窗口。
//!
//! 这是浮层契约中的定位部分：对于边缘或近边缘的任意锚点及任意请求方向，
//! 生成的矩形必须完全包含在视口（或安全内缩视口）内。

use unbound_math::{Rect, Vec2};
use unbound_widgets::overlay::{OverlayEntry, OverlayStack, PlacedPopup};
use unbound_widgets::{
    place_anchored_popup, place_tooltip_in_window, ControlState, Placement, TooltipDesc,
    TOOLTIP_VIEWPORT_MARGIN,
};

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

#[test]
fn tooltip_in_window_auto_flips_and_clamps_inside_safe_viewport() {
    let vp = viewport();
    let safe = Rect::new(
        TOOLTIP_VIEWPORT_MARGIN,
        TOOLTIP_VIEWPORT_MARGIN,
        vp.width() - TOOLTIP_VIEWPORT_MARGIN * 2.0,
        vp.height() - TOOLTIP_VIEWPORT_MARGIN * 2.0,
    );
    let anchors = [
        Rect::new(2.0, 2.0, 72.0, 28.0),
        Rect::new(1200.0, 2.0, 76.0, 28.0),
        Rect::new(2.0, 688.0, 72.0, 28.0),
        Rect::new(1200.0, 688.0, 76.0, 28.0),
        Rect::new(600.0, 4.0, 80.0, 28.0),
        Rect::new(600.0, 688.0, 80.0, 28.0),
        Rect::new(2.0, 340.0, 48.0, 28.0),
        Rect::new(1230.0, 340.0, 48.0, 28.0),
    ];
    let placements = [
        Placement::Top,
        Placement::Bottom,
        Placement::Left,
        Placement::Right,
        Placement::TopStart,
        Placement::TopEnd,
        Placement::BottomStart,
        Placement::BottomEnd,
        Placement::LeftStart,
        Placement::LeftEnd,
        Placement::RightStart,
        Placement::RightEnd,
    ];

    for anchor in anchors {
        for placement in placements {
            let placed = place_tooltip_in_window(
                anchor,
                Vec2::new(220.0, 64.0),
                vp,
                placement,
                6.0,
                TOOLTIP_VIEWPORT_MARGIN,
            );
            assert!(
                safe.contains_rect(&placed.rect),
                "tooltip {placement:?} at {anchor:?} escaped safe viewport: {:?}",
                placed.rect
            );
            let desc = TooltipDesc::new(anchor, vp, Vec2::new(220.0, 64.0))
                .with_label("Tooltip automatic viewport clamping")
                .with_gap(6.0)
                .with_viewport_margin(TOOLTIP_VIEWPORT_MARGIN)
                .with_placement(placement);
            assert!(
                safe.contains_rect(&desc.rect()),
                "TooltipDesc {placement:?} at {anchor:?} escaped safe viewport: {:?}",
                desc.rect()
            );
            // 气泡由阴影、背景与边框三个图元组成；未传入字形时载荷表为空。
            let (out, _entry) = desc.render_with_glyphs(77, 0, ControlState::default(), &[]);
            assert_eq!(out.primitives.len(), 3);
            assert!(out.payloads.is_empty());
        }
    }

    // 紧贴顶边请求 Top 时自动翻转至 Bottom
    let top_edge_anchor = Rect::new(560.0, 6.0, 96.0, 28.0);
    let top_desc = TooltipDesc::new(top_edge_anchor, vp, Vec2::new(180.0, 44.0))
        .with_label("Top flip")
        .with_gap(6.0)
        .with_viewport_margin(TOOLTIP_VIEWPORT_MARGIN)
        .with_placement(Placement::Top);
    assert_eq!(top_desc.chosen_placement(), Placement::Bottom);
    assert!(safe.contains_rect(&top_desc.rect()));

    // 紧贴底边请求 Bottom 时自动翻转至 Top
    let bottom_edge_anchor = Rect::new(560.0, 686.0, 96.0, 28.0);
    let bottom_desc = TooltipDesc::new(bottom_edge_anchor, vp, Vec2::new(180.0, 44.0))
        .with_label("Bottom flip")
        .with_gap(6.0)
        .with_viewport_margin(TOOLTIP_VIEWPORT_MARGIN)
        .with_placement(Placement::Bottom);
    assert_eq!(bottom_desc.chosen_placement(), Placement::Top);
    assert!(safe.contains_rect(&bottom_desc.rect()));
}

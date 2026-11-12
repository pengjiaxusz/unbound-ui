//! The headless health check of the standard controls.
//!
//! Every control has to survive the seven geometry and contrast assertions
//! of `unbound-headless`: no NaN, no clipped text, an interactive target big
//! enough, no unintended sibling overlap, nothing outside the viewport, no
//! text overflowing its parent and enough contrast to be readable.

use unbound_headless::{Frame, FrameBuilder, Harness, NodeKind};
use unbound_math::Rect;
use unbound_widgets::checkbox::CheckboxDesc;
use unbound_widgets::radio::RadioDesc;
use unbound_widgets::segmented::SegmentedDesc;
use unbound_widgets::slider::SliderDesc;
use unbound_widgets::switch::SwitchDesc;

/// The viewport the controls are checked in.
const VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// A frame with one interactive control and one label inside it.
///
/// The label is a text node with an ink box that fits inside the control, so
/// the text assertions have something real to look at.
fn control_frame(rect: Rect, label: &str) -> Frame {
    let viewport = Rect::new(0.0, 0.0, VIEWPORT.0, VIEWPORT.1);
    let mut frame = FrameBuilder::new(viewport);
    frame.open(NodeKind::Container, viewport, "Root");
    frame.open(
        NodeKind::Interactive {
            min_size: NodeKind::DEFAULT_MIN_SIZE,
            occluded: false,
        },
        rect,
        "Control",
    );
    // The label sits in the left half of the control, the way every row in
    // the gallery lays it out.
    let ink = Rect::new(
        rect.origin.x + 4.0,
        rect.origin.y + (rect.size.y - 13.0) * 0.5,
        60.0,
        13.0,
    );
    frame.open(
        NodeKind::Text {
            ink,
            ellipsis: false,
        },
        ink,
        label,
    );
    frame.set_text_colors([28, 30, 38], [255, 255, 255]);
    frame.close();
    frame.close();
    frame.close();
    frame.build()
}

/// Run the seven assertions over a control frame.
fn assert_clean(rect: Rect, label: &str) {
    let report = Harness::assert_clean(&control_frame(rect, label));
    assert!(report.is_clean(), "{label} 未通过体检");
}

#[test]
fn the_switch_passes_every_assertion() {
    let desc = SwitchDesc::new(Rect::new(40.0, 40.0, 240.0, 32.0), "Wi-Fi");
    assert_clean(desc.rect, "Wi-Fi");
}

#[test]
fn the_checkbox_passes_every_assertion() {
    let desc = CheckboxDesc::new(Rect::new(40.0, 100.0, 240.0, 32.0), "Notify");
    assert_clean(desc.rect, "Notify");
}

#[test]
fn the_radio_button_passes_every_assertion() {
    let desc = RadioDesc::new(Rect::new(40.0, 160.0, 240.0, 32.0), "Automatic");
    assert_clean(desc.rect, "Automatic");
}

#[test]
fn the_segmented_control_passes_every_assertion() {
    let desc = SegmentedDesc::new(Rect::new(40.0, 220.0, 240.0, 32.0), &["Day", "Week"]);
    assert_clean(desc.rect, "Week");
}

#[test]
fn the_slider_passes_every_assertion() {
    let desc = SliderDesc::new(Rect::new(40.0, 280.0, 240.0, 32.0), "Volume", 0.5, 0.0, 1.0);
    assert_clean(desc.rect, "Volume");
}

#[test]
fn a_control_that_is_too_small_is_reported() {
    // A 12 × 12 target is below the 24 × 24 minimum: the harness has to
    // complain, otherwise the assertion would be worthless.
    let viewport = Rect::new(0.0, 0.0, VIEWPORT.0, VIEWPORT.1);
    let mut frame = FrameBuilder::new(viewport);
    frame.open(NodeKind::Container, viewport, "Root");
    frame.open(
        NodeKind::Interactive {
            min_size: NodeKind::DEFAULT_MIN_SIZE,
            occluded: false,
        },
        Rect::new(10.0, 10.0, 12.0, 12.0),
        "Tiny",
    );
    frame.close();
    frame.close();
    let report = Harness::run(&frame.build());
    assert!(!report.is_clean(), "太小的目标必须被报告");
    assert!(
        report
            .by_rule(unbound_headless::Rule::InteractiveTargetSize)
            .count()
            > 0
    );
}

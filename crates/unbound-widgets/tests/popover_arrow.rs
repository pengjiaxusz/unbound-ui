//! 气泡框箭头与 Portal 子作用域测试。
//!
//! `show_arrow` 添加两条在 [`PlacedPopup::arrow_tip`] 交汇的贝塞尔边，
//! `emit_popover_scope` 则在气泡裁剪区域内收集子元素并将其提升到 Portal 命中测试层。
//!

use unbound_gpu::data::{PrimType, CLIP_NONE};
use unbound_math::{Rect, Vec2};
use unbound_widgets::overlay::Placement;
use unbound_widgets::popover::{emit_popover, emit_popover_scope, PopoverDesc, POPOVER_LAYER};
use unbound_widgets::{ButtonDesc, ButtonWidget, ControlState, SceneCollector, WidgetDesc};

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 720.0)
}

fn desc() -> PopoverDesc {
    PopoverDesc::new(
        Rect::new(400.0, 300.0, 100.0, 32.0),
        viewport(),
        Vec2::new(220.0, 140.0),
    )
}

#[test]
fn an_arrow_adds_two_bezier_edges_that_start_at_the_arrow_tip() {
    let desc = PopoverDesc {
        show_arrow: true,
        ..desc()
    };
    let output = emit_popover(1, 2, &desc, ControlState::default());

    assert_eq!(output.output.primitives.len(), 5, "3 surfaces + 2 edges");
    assert_eq!(output.output.payloads.len(), 2);

    let edges = &output.output.primitives[3..];
    assert!(edges.iter().all(|primitive| {
        primitive.prim_type_and_flags
            == unbound_gpu::data::GpuPrimitiveInstance::pack_type(
                PrimType::Curve,
                unbound_gpu::data::BlendMode::Normal,
                unbound_gpu::data::SubElement::Content as u16,
            )
    }));

    let placed = desc.placed();
    let tip = placed.arrow_tip;
    assert_eq!(tip.y, placed.rect.origin.y, "points up at the anchor");
    assert!(edges
        .iter()
        .any(|primitive| { primitive.local_pos[1] == tip.y && primitive.local_pos[0] <= tip.x }));
}

#[test]
fn an_arrow_is_off_by_default() {
    let output = emit_popover(1, 2, &desc(), ControlState::default());
    assert_eq!(output.output.primitives.len(), 3);
    assert!(output.output.payloads.is_empty());
}

#[test]
fn the_scope_clips_children_and_lifts_them_onto_the_portal_layer() {
    let desc = PopoverDesc {
        placement: Placement::Bottom,
        ..desc()
    };
    let placed = desc.placed();
    let child = ButtonWidget {
        node: 700,
        desc: ButtonDesc::new(
            Rect::new(
                placed.rect.origin.x + 20.0,
                placed.rect.origin.y + 20.0,
                120.0,
                32.0,
            ),
            "Apply",
        ),
    };
    let child_output = child.emit(ControlState::default(), &mut []);

    let mut collector = SceneCollector::new(1);
    let output = emit_popover_scope(
        &mut collector,
        1,
        2,
        &desc,
        ControlState::default(),
        [(child.key(), child_output)],
    );

    assert_eq!(collector.clips.len(), 1, "the popover clip stays open");
    assert_eq!(
        collector.clips[0].aabb_clip,
        [
            placed.rect.origin.x,
            placed.rect.origin.y,
            placed.rect.max().x,
            placed.rect.max().y,
        ]
    );

    // 容器自身不被裁剪，但子元素会被裁剪。
    let container = collector.span(1).expect("container emitted");
    assert!(container
        .iter()
        .all(|primitive| primitive.clip_idx == CLIP_NONE));
    let primitives = collector.span(child.key()).expect("child emitted");
    assert!(
        primitives
            .iter()
            .all(|primitive| primitive.clip_idx != CLIP_NONE),
        "every child primitive inherits the popover clip"
    );

    // 子元素被提升到基础界面之上参与命中测试。
    let region = collector
        .regions
        .iter()
        .find(|region| region.node == child.key())
        .expect("child hit region");
    assert_eq!(region.layer, POPOVER_LAYER);
    assert_eq!(
        output.output.hit.as_ref().map(|hit| hit.layer),
        Some(POPOVER_LAYER)
    );
}

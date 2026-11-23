//! 滚动视图与独立按需叠加滚动条集成测试：拾取、轨道翻页、滑块拖拽与裁剪/变换。
//!
//! 滚动视图是输出被拆分的控件：其自身图元（滚动条）不受任何裁剪，
//! 而所有**子**图元必须使用输出的裁剪与变换。独立叠加滚动条（`emit_scrollbar`）
//! 则完全不接管裁剪与变换，供任意复合组件直接挂载。

use unbound_headless::driver::TestApp;
use unbound_input::Router;
use unbound_math::{Rect, Vec2};
use unbound_widgets::button::ButtonDesc;
use unbound_widgets::scroll::{
    emit_scroll_view, emit_scrollbar, scrollbar_thumb_node, scrollbar_track_node,
    update_virtual_list, ScrollBarDesc, ScrollBarVisibility, ScrollState, ScrollViewDesc,
    ScrollbarAxis, SCROLLBAR_IDLE_OPACITY, SCROLLBAR_THUMB_IDLE_WIDTH, SCROLLBAR_THUMB_WIDTH,
    SCROLLBAR_TRACK_CAPSULE_OPACITY, SCROLLBAR_WIDTH,
};
use unbound_widgets::text_input::TextInputDesc;
use unbound_widgets::{ButtonWidget, ControlState, SceneCollector, TextInputWidget, WidgetDesc};

fn desc() -> ScrollViewDesc {
    ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 3000.0))
}

fn tab() -> ControlState {
    ControlState::default()
}

#[test]
fn the_wheel_over_the_viewport_targets_the_scroll_view() {
    let desc = desc();
    let state = ScrollState::new();
    let output = emit_scroll_view(90, &desc, &state, tab());

    let mut router = Router::default();
    router.set_regions(vec![
        output.output.hit.clone().expect("viewport hit"),
        output.hits.first().cloned().expect("thumb hit"),
    ]);

    let hits = router.wheel(Vec2::new(-120.0, 0.0), Vec2::new(200.0, 150.0));
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|hit| hit.node == 90));
}

#[test]
fn the_scrollbar_thumb_is_pickable_and_draggable() {
    let desc = desc();
    let mut state = ScrollState::new();
    let thumb = emit_scroll_view(90, &desc, &state, tab())
        .hits
        .first()
        .cloned()
        .expect("content overflows, so the thumb exists");

    let mut app = TestApp::new((800.0, 600.0));
    app.install(vec![thumb.clone()], unbound_a11y::Semantics::new(90));
    assert_eq!(app.click(thumb.bounds.center()).clicked(), Some(thumb.node));

    // 按下滑块开始拖拽；向下移动会滚动内容。
    let start = thumb.bounds.center().y;
    state.begin_drag(ScrollbarAxis::Vertical, start);
    state.drag_to(&desc, start + 40.0);
    assert!(state.offset().y > 0.0);
    state.drag_to(&desc, start + 100_000.0);
    assert_eq!(state.offset().y, desc.max_offset().y);
    state.end_drag();
    assert!(state.dragging().is_none());
}

#[test]
fn the_clip_and_transform_stay_in_sync_with_the_offset() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.jump_to(&desc, Vec2::new(0.0, 420.0));
    let output = emit_scroll_view(90, &desc, &state, tab());

    assert_eq!(output.clip.aabb_clip, [0.0, 0.0, 400.0, 300.0]);
    assert_eq!(output.transform.translation, [0.0, -420.0]);
    assert!(output
        .output
        .hit
        .as_ref()
        .is_some_and(|hit| hit.bounds.contains_rect(&desc.rect)));
}

#[test]
fn scrolling_a_virtual_list_reuses_slots_instead_of_rebinding() {
    let desc = desc();
    let mut state = ScrollState::new();
    let mut list = unbound_world::virtualize::VirtualList::new(28.0, 1.0);
    let first = update_virtual_list(&mut list, &desc, &state);
    let budget = first.slots;

    state.jump_to(&desc, Vec2::new(0.0, 14.0));
    let second = update_virtual_list(&mut list, &desc, &state);
    assert!(second.update.offset_only);
    assert_eq!(second.update.rebound, 0);
    assert_eq!(second.slots, budget);
}

#[test]
fn a_disabled_scroll_view_still_emits_but_is_not_pickable() {
    let desc = desc();
    let state = ScrollState::new();
    let disabled = ControlState {
        disabled: true,
        ..ControlState::default()
    };
    let output = emit_scroll_view(90, &desc, &state, disabled);
    assert!(output.output.hit.is_some(), "the viewport still clips");
    assert!(!output.hits.is_empty());
}

#[test]
fn controls_inside_scroll_view_inherit_clip_and_transform() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.jump_to(&desc, Vec2::new(0.0, 420.0));
    let scroll = emit_scroll_view(90, &desc, &state, tab());

    // 可见按钮位于内容空间 y = 460，输入框位于 y = 100。
    let button = ButtonWidget {
        node: 700,
        desc: ButtonDesc::new(Rect::new(20.0, 460.0, 120.0, 36.0), "Save"),
    };
    let field = TextInputWidget {
        node: 701,
        desc: TextInputDesc::new(Rect::new(20.0, 100.0, 200.0, 32.0)),
        edit: unbound_widgets::TextEditState::default(),
        visual: unbound_widgets::TextInputVisual::default(),
    };

    let mut collector = SceneCollector::new(1);
    collector.push_clip(scroll.clip, desc.rect);
    collector.push_transform(scroll.transform);
    collector.push(button.key(), button.emit(tab(), &mut []));
    collector.push(field.key(), field.emit(tab(), &mut []));
    collector.pop_transform();
    collector.pop_clip();

    assert_eq!(collector.clips.len(), 1);
    assert_eq!(collector.clips[0].aabb_clip, [0.0, 0.0, 400.0, 300.0]);
    assert_eq!(collector.transforms.len(), 2, "identity + scroll");

    for key in [button.key(), field.key()] {
        let primitives = collector.span(key).expect("widget emitted");
        assert!(!primitives.is_empty());
        assert!(
            primitives.iter().all(|prim| prim.clip_idx != u32::MAX),
            "every primitive must inherit the viewport clip"
        );
        assert!(
            primitives.iter().all(|prim| prim.transform_idx == 1),
            "every primitive must follow the scroll transform"
        );
    }

    // 可见按钮在其屏幕位置仍可被拾取。
    let visible = Vec2::new(80.0, 460.0 - 420.0 + 18.0);
    assert_eq!(
        unbound_input::pick(&collector.regions, visible).map(|region| region.node),
        Some(button.key())
    );

    // 滚出视口上方的输入框会坍缩为空区域，无法再被拾取。
    let outside = Vec2::new(80.0, 100.0 - 420.0);
    assert_eq!(
        unbound_input::pick(&collector.regions, outside).map(|region| region.node),
        None
    );
}

#[test]
fn clicking_the_track_outside_the_thumb_pages_by_one_viewport() {
    let desc = desc();
    let mut state = ScrollState::new();
    let track = desc.track_rect(false);

    let below = Vec2::new(track.center().x, track.max().y - 4.0);
    assert!(state.page_at_track_click(&desc, below), "forward page");
    assert_eq!(state.target().y, desc.rect.height());

    state.jump_to(&desc, Vec2::new(0.0, desc.rect.height()));
    let above = Vec2::new(track.center().x, track.origin.y + 2.0);
    assert!(state.page_at_track_click(&desc, above), "backward page");
    assert_eq!(state.target().y, 0.0);
}

#[test]
fn clicking_the_thumb_or_outside_the_track_does_not_page() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.jump_to(&desc, Vec2::new(0.0, 1500.0));
    let thumb = desc.thumb_rect(state.offset(), false);
    assert!(!state.page_at_track_click(&desc, thumb.center()));
    assert_eq!(state.target().y, 1500.0);

    assert!(!state.page_at_track_click(&desc, Vec2::new(-10.0, -10.0)));
    assert_eq!(state.target().y, 1500.0);
}

#[test]
fn scroll_into_view_targets_the_nearest_edge() {
    let desc = desc();
    let mut state = ScrollState::new();

    // 完全位于视口下方的子元素：使其底边与视口对齐。
    assert!(state.scroll_into_view(&desc, Rect::new(20.0, 460.0, 120.0, 36.0)));
    assert_eq!(state.target().y, 460.0 + 36.0 - desc.rect.height());

    // 完全位于视口上方的子元素：使其顶边与视口对齐。
    assert!(state.scroll_into_view(&desc, Rect::new(20.0, 50.0, 120.0, 36.0)));
    assert_eq!(state.target().y, 50.0);

    // 已可见的子元素：目标不移动。
    assert!(!state.scroll_into_view(&desc, Rect::new(20.0, 100.0, 120.0, 20.0)));
    assert_eq!(state.target().y, 50.0);
}

#[test]
fn a_fling_projects_the_release_velocity_into_the_target() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.fling(&desc, Vec2::new(0.0, 1200.0));
    assert_eq!(state.target().y, 1200.0 * 0.25);
    assert_eq!(state.offset().y, 0.0, "only the target moves");
    assert!(state.is_animating());

    // 猛烈甩动最终精确停在内容末尾。
    state.fling(&desc, Vec2::new(0.0, 1_000_000.0));
    assert_eq!(state.target().y, desc.max_offset().y);

    state.jump_to(&desc, Vec2::ZERO);
    state.fling(&desc, Vec2::new(0.0, -500.0));
    assert_eq!(state.target().y, 0.0);
}

#[test]
fn collect_children_inherits_the_scroll_clip_and_transform() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.jump_to(&desc, Vec2::new(0.0, 420.0));
    let scroll = emit_scroll_view(90, &desc, &state, tab());

    let button = ButtonWidget {
        node: 700,
        desc: ButtonDesc::new(Rect::new(20.0, 460.0, 120.0, 36.0), "Save"),
    };

    let mut collector = SceneCollector::new(1);
    scroll.collect_children(&mut collector, desc.rect, |collector| {
        collector.push(button.key(), button.emit(tab(), &mut []));
    });

    assert_eq!(collector.clips.len(), 1);
    assert_eq!(collector.transforms.len(), 2, "identity + scroll");
    let primitives = collector.span(button.key()).expect("button emitted");
    assert!(primitives
        .iter()
        .all(|primitive| primitive.clip_idx != u32::MAX));
    assert!(primitives
        .iter()
        .all(|primitive| primitive.transform_idx == 1));
}

#[test]
fn overlay_scrollbar_mounts_without_clip_or_transform_and_routes_thumb_and_track_hits() {
    let viewport = Rect::new(0.0, 0.0, 320.0, 240.0);
    let owner = 500_u64;
    let bar = ScrollBarDesc::vertical(viewport, 1200.0, 0.0);
    let out = emit_scrollbar(owner, &bar);
    let track = out.track_rect.expect("overflowing track rect");
    let thumb = out.thumb_rect.expect("overflowing thumb rect");

    let mut collector = SceneCollector::new(1);
    out.push_into(&mut collector, owner);

    // 独立叠加滚动条不推入额外裁剪与变换。
    assert!(collector.clips.is_empty());
    assert!(collector.transforms.is_empty(), "no extra transform pushed");
    assert_eq!(collector.regions.len(), 2, "thumb hit + track hit");

    // 点击滑块区域优先命中滑块节点；点击滑块下方轨道命中轨道节点并触发翻页。
    assert_eq!(
        unbound_input::pick(&collector.regions, thumb.center()).map(|r| r.node),
        Some(scrollbar_thumb_node(owner, ScrollbarAxis::Vertical))
    );
    let below_thumb = Vec2::new(track.center().x, track.max().y - 4.0);
    assert_eq!(
        unbound_input::pick(&collector.regions, below_thumb).map(|r| r.node),
        Some(scrollbar_track_node(owner, ScrollbarAxis::Vertical))
    );

    let paged = bar
        .page_offset_for_track_click(below_thumb)
        .expect("clicking track outside thumb pages down");
    assert_eq!(paged, viewport.height());

    // 拖拽滑块走完完整行程恰好抵达最大偏移量。
    let travel = track.height() - thumb.height();
    let dragged = bar.offset_for_drag(0.0, thumb.center().y, thumb.center().y + travel);
    assert!((dragged - bar.max_offset()).abs() < 1e-3);
}

#[test]
fn overlay_scrollbar_enforces_win11_auto_hidden_and_always_expanded_contracts() {
    let viewport = Rect::new(0.0, 0.0, 300.0, 200.0);

    // 1. 不溢出 = 0 图元、0 命中区。
    let no_overflow = emit_scrollbar(600, &ScrollBarDesc::vertical(viewport, 200.0, 0.0));
    assert!(no_overflow.is_empty());

    // 2. Auto 溢出静止态 = 轨道胶囊透明（保持稳定槽位）+ 2px 半透明细指示条 + 2 个命中区。
    let idle = emit_scrollbar(600, &ScrollBarDesc::vertical(viewport, 800.0, 0.0));
    assert_eq!(idle.output.primitives.len(), 2);
    assert_eq!(idle.output.primitives[0].opacity(), 0.0);
    assert_eq!(
        idle.output.primitives[1].size[0],
        SCROLLBAR_THUMB_IDLE_WIDTH
    );
    assert!((idle.output.primitives[1].opacity() - SCROLLBAR_IDLE_OPACITY).abs() < 0.02);
    assert_eq!(idle.hits.len(), 2);

    // 3. Auto 溢出悬停 / 拖拽与 AlwaysExpanded = 6px 粗滑块 + 半透明轨道胶囊。
    let mut hovered = ScrollBarDesc::vertical(viewport, 800.0, 120.0);
    hovered.hovered = true;
    let mut always = ScrollBarDesc::vertical(viewport, 800.0, 120.0);
    always.visibility = ScrollBarVisibility::AlwaysExpanded;
    for desc in [hovered, always] {
        let out = emit_scrollbar(600, &desc);
        assert_eq!(out.output.primitives.len(), 2);
        assert_eq!(out.output.primitives[0].size[0], SCROLLBAR_WIDTH);
        assert!(
            (out.output.primitives[0].opacity() - SCROLLBAR_TRACK_CAPSULE_OPACITY).abs() < 0.02
        );
        assert_eq!(out.output.primitives[1].size[0], SCROLLBAR_THUMB_WIDTH);
        assert!(out.output.primitives[1].opacity() > 0.99);
        assert_eq!(out.hits.len(), 2);
    }

    // 4. Hidden 即使溢出且悬停也始终 0 图元、0 命中区。
    let mut hidden = ScrollBarDesc::vertical(viewport, 800.0, 120.0);
    hidden.visibility = ScrollBarVisibility::Hidden;
    hidden.hovered = true;
    assert!(emit_scrollbar(600, &hidden).is_empty());
}

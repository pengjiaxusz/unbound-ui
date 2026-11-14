//! 运行时持有布局世界：行、网格与自动换行段落必须由 `AppRuntime` 定位，
//! 而非手工硬编码矩形。

use unbound_abi::{Dim, RawNodeDesc};
use unbound_app::{AppEvent, AppRuntime, Application, EventOutcome, WidgetSpec};
use unbound_math::{Rect, Vec2};
use unbound_text::TextStyle;
use unbound_widgets::button::ButtonDesc;
use unbound_world::UiWorld;

const PROSE: &str = "The quick brown fox jumps over the lazy dog while the \
interface lays out every paragraph against the width the container offers.";

struct LayoutApp;

impl Application for LayoutApp {
    type State = ();

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        EventOutcome::default()
    }

    fn view(&mut self, _state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        view.column(10, |column| {
            column.row(20, |row| {
                row.widget(
                    21,
                    RawNodeDesc::container(),
                    WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, "Left")),
                );
                row.text(22, PROSE, TextStyle::sized(14.0));
                row.widget(
                    23,
                    RawNodeDesc::container(),
                    WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, "Right")),
                );
            });
            column.grid(30, &[Dim::fr(1.0), Dim::fr(2.0)], |grid| {
                grid.container(31, RawNodeDesc::container(), |cell| {
                    cell.widget(
                        310,
                        RawNodeDesc::container(),
                        WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, "A")),
                    );
                });
                grid.container(32, RawNodeDesc::container(), |cell| {
                    cell.widget(
                        320,
                        RawNodeDesc::container(),
                        WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, "B")),
                    );
                });
            });
        });
    }
}

fn find(world: &UiWorld, key: u64) -> Rect {
    fn walk(world: &UiWorld, id: unbound_world::NodeId, key: u64) -> Option<Rect> {
        if world.get(id).is_some_and(|node| node.desc.key == key) {
            return Some(world.get(id).expect("checked").rect);
        }
        world.children(id).find_map(|child| walk(world, child, key))
    }
    walk(world, world.root(), key).expect("node exists")
}

#[test]
fn rows_share_a_baseline_and_grid_tracks_follow_fractions() {
    let mut runtime = AppRuntime::new(LayoutApp, Vec2::new(600.0, 400.0));
    runtime.frame(0.0);

    let left = find(runtime.world(), 21);
    let prose = find(runtime.world(), 22);
    let right = find(runtime.world(), 23);
    assert!(
        (left.origin.y - prose.origin.y).abs() < 0.5,
        "row children share a baseline"
    );
    assert!((right.origin.y - left.origin.y).abs() < 0.5);
    assert!(left.width() > 0.0 && right.width() > 0.0);
    assert!(prose.width() > left.origin.x + left.width());

    let first = find(runtime.world(), 31);
    let second = find(runtime.world(), 32);
    assert!(
        (second.width() / first.width() - 2.0).abs() < 0.15,
        "1fr / 2fr grid tracks, got {} and {}",
        first.width(),
        second.width()
    );
    assert!(
        (first.origin.y - second.origin.y).abs() < 0.5,
        "same grid row"
    );
}

#[test]
fn wrapping_text_reflows_when_the_window_narrows() {
    let mut runtime = AppRuntime::new(LayoutApp, Vec2::new(600.0, 400.0));
    runtime.frame(0.0);
    let wide = find(runtime.world(), 22);

    runtime.resize(Vec2::new(240.0, 400.0));
    runtime.frame(1.0 / 60.0);
    let narrow = find(runtime.world(), 22);

    assert!(
        narrow.height() > wide.height(),
        "narrow text must wrap onto more lines: wide={}, narrow={}",
        wide.height(),
        narrow.height()
    );
    assert!(narrow.width() <= 240.0);
}

#[test]
fn layout_only_state_is_not_published_as_a_whole_scene() {
    let mut runtime = AppRuntime::new(LayoutApp, Vec2::new(600.0, 400.0));
    let first = runtime.frame(0.0);
    assert_eq!(first.diff, unbound_widgets::diff::SceneDiff::Whole);

    let report = runtime.frame(1.0 / 60.0);
    assert_eq!(report.diff, unbound_widgets::diff::SceneDiff::Identical);
}

struct BoundaryApp;

impl Application for BoundaryApp {
    type State = bool;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        false
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        if matches!(event, AppEvent::Click { .. }) {
            *state = !*state;
            return EventOutcome::new(unbound_app::DirtyFlags::LAYOUT);
        }
        EventOutcome::default()
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let boundary_desc = RawNodeDesc::container()
            .size(280.0, 180.0)
            .flags(unbound_abi::NodeFlags::CLIP);
        view.row(10, |row| {
            row.container(100, boundary_desc, |card| {
                let label = if *state { "Expanded" } else { "Ok" };
                card.widget(
                    101,
                    RawNodeDesc::container(),
                    WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, label)),
                );
            });
            row.container(200, boundary_desc, |card| {
                card.widget(
                    201,
                    RawNodeDesc::container(),
                    WidgetSpec::Button(ButtonDesc::new(Rect::ZERO, "Static")),
                );
            });
        });
    }
}

#[test]
fn incremental_frame_reuses_world_and_solves_only_dirty_boundary() {
    let mut runtime = AppRuntime::new(BoundaryApp, Vec2::new(640.0, 320.0));
    runtime.frame(0.0);
    let card_a_before = runtime.world().find_key(100).expect("card 100");
    let card_b_before = runtime.world().find_key(200).expect("card 200");

    // 点击卡片 100 的按钮内部，仅修改卡片 100。
    let btn_rect = find(runtime.world(), 101);
    runtime.pointer_move(btn_rect.center());
    runtime.pointer_down(btn_rect.center(), unbound_input::PointerButton::Left);
    runtime.pointer_up(btn_rect.center(), unbound_input::PointerButton::Left);
    runtime.frame(1.0 / 60.0);

    assert_eq!(runtime.world().find_key(100), Some(card_a_before));
    assert_eq!(runtime.world().find_key(200), Some(card_b_before));
    assert_eq!(
        runtime.last_layout_stats().boundaries_solved,
        1,
        "only the dirty relayout boundary should be re-solved"
    );
}

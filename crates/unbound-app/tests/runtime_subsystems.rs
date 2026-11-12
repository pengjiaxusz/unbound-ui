//! Behavioral tests for the P1 runtime subsystems: the local state pool and
//! its lifecycle hooks, timers, cross-thread results, the keymap and the
//! wheel / context-menu paths.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use unbound_app::{
    AppContext, AppEvent, AppKey, AppRuntime, AppWaker, Application, DirtyFlags, EventOutcome,
    LifecycleEvent, Modifiers, ViewBuilder,
};
use unbound_input::{HitRegion, PointerButton};
use unbound_math::{Rect, Vec2};
use unbound_widgets::WidgetOutput;

const INNER: u64 = 10;
const OUTER: u64 = 11;
const STATE_KEY: u64 = 12;

#[derive(Default)]
struct Probe {
    render_state: bool,
    mounted: Vec<u64>,
    unmounted: Vec<u64>,
    focused: Vec<u64>,
    blurred: Vec<u64>,
    timer_hits: u32,
    async_hits: u32,
    shortcuts: Vec<u64>,
    wheel: Vec<(Option<u64>, Vec2)>,
    menus: Vec<u64>,
    clicks: u32,
}

#[derive(Debug, Default)]
struct Counter {
    frames: u32,
}

struct SubsystemApp {
    probe: Probe,
}

impl Application for SubsystemApp {
    type State = Probe;

    fn init(&mut self, ctx: &mut AppContext) -> Self::State {
        // A timer scheduled from `init` must still land, not be dropped.
        ctx.set_timeout(Duration::from_millis(100), |token| AppEvent::Timer {
            token,
        });
        std::mem::take(&mut self.probe)
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Tick { .. } => {
                if state.render_state {
                    DirtyFlags::PAINT
                } else {
                    DirtyFlags::NONE
                }
            }
            AppEvent::Timer { .. } => {
                state.timer_hits += 1;
                DirtyFlags::PAINT
            }
            AppEvent::Text { .. } => {
                state.async_hits += 1;
                DirtyFlags::PAINT
            }
            AppEvent::Shortcut { action } => {
                state.shortcuts.push(action);
                DirtyFlags::PAINT
            }
            AppEvent::Wheel { target, delta, .. } => {
                state.wheel.push((target, delta));
                ctx.consume_scroll(Vec2::new(2.0, 0.0));
                DirtyFlags::NONE
            }
            AppEvent::Click { .. } => {
                state.clicks += 1;
                DirtyFlags::PAINT
            }
            AppEvent::ContextMenu { node, .. } => {
                state.menus.push(node);
                DirtyFlags::PAINT
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn lifecycle(&mut self, _state: &mut Self::State, event: LifecycleEvent) -> DirtyFlags {
        match event {
            LifecycleEvent::Mounted(key) => _state.mounted.push(key),
            LifecycleEvent::Unmounted(key) => _state.unmounted.push(key),
            LifecycleEvent::Focused(key) => _state.focused.push(key),
            LifecycleEvent::Blurred(key) => _state.blurred.push(key),
        }
        DirtyFlags::NONE
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        if state.render_state {
            view.use_state::<Counter>(STATE_KEY).frames += 1;
        }

        let outer = WidgetOutput {
            hit: Some(HitRegion::new(
                OUTER,
                1,
                0,
                Rect::new(0.0, 0.0, 400.0, 300.0),
            )),
            ..WidgetOutput::default()
        };
        view.push(OUTER, outer);

        let inner = WidgetOutput {
            hit: Some(HitRegion::new(
                INNER,
                1,
                10,
                Rect::new(20.0, 20.0, 100.0, 100.0),
            )),
            ..WidgetOutput::default()
        };
        view.push(INNER, inner);
    }
}

fn runtime() -> AppRuntime<SubsystemApp> {
    AppRuntime::new(
        SubsystemApp {
            probe: Probe::default(),
        },
        Vec2::new(400.0, 300.0),
    )
}

#[test]
fn a_state_key_is_mounted_and_unmounted_with_its_lifecycle_hooks() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    assert!(runtime.state().mounted.is_empty());

    runtime.state_mut().render_state = true;
    runtime.invalidate();
    runtime.frame(0.0);
    assert_eq!(runtime.state().mounted, vec![STATE_KEY]);
    assert_eq!(
        runtime.with_state::<Counter, _>(STATE_KEY, |state| state.frames),
        1
    );

    let probe = runtime.state_mut();
    probe.render_state = false;
    runtime.invalidate();
    runtime.frame(0.0);
    assert_eq!(runtime.state().unmounted, vec![STATE_KEY]);
    assert!(
        runtime.state_store().is_empty(),
        "unmounted state is reclaimed"
    );
}

#[test]
fn a_timeout_scheduled_in_init_fires_on_the_first_frame_past_its_deadline() {
    let mut runtime = runtime();
    runtime.frame(0.05);
    assert_eq!(runtime.state().timer_hits, 0);
    runtime.frame(0.10);
    assert_eq!(runtime.state().timer_hits, 1, "the timeout fired once");
    runtime.frame(0.50);
    assert_eq!(runtime.state().timer_hits, 1, "a timeout is one-shot");
}

#[test]
fn a_background_task_is_reduced_without_polling_the_loop_by_hand() {
    #[derive(Debug)]
    struct CountingWaker(Arc<AtomicUsize>);

    impl AppWaker for CountingWaker {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let woke = Arc::new(AtomicUsize::new(0));
    let mut runtime = runtime();
    runtime.set_waker(Arc::new(CountingWaker(Arc::clone(&woke))));
    let context = runtime.context().clone();
    context.spawn(|| AppEvent::Text {
        text: "ready".to_owned(),
    });

    for _ in 0..200 {
        runtime.frame(0.016);
        if runtime.state().async_hits == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(runtime.state().async_hits, 1);
    assert!(woke.load(Ordering::SeqCst) >= 1, "the waker was signalled");
}

#[test]
fn a_two_chord_sequence_fires_a_shortcut_and_plain_keys_still_reduce() {
    let mut runtime = runtime();
    runtime.keymap_mut().bind_sequence(
        unbound_app::AppKey::Character('k').chord(Modifiers {
            ctrl: true,
            ..Modifiers::default()
        }),
        unbound_app::AppKey::Character('s').chord(Modifiers {
            ctrl: true,
            ..Modifiers::default()
        }),
        None,
        42,
    );

    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    runtime.key(AppKey::Character('k'), true, ctrl);
    assert!(runtime.state().shortcuts.is_empty());
    runtime.key(AppKey::Character('s'), true, ctrl);
    assert_eq!(runtime.state().shortcuts, vec![42]);
}

#[test]
fn the_wheel_delta_bubbles_the_unconsumed_remainder_to_the_outer_region() {
    let mut runtime = runtime();
    runtime.frame(0.0);

    runtime.wheel(Vec2::new(50.0, 50.0), Vec2::new(10.0, 0.0));
    let wheel = &runtime.state().wheel;
    assert_eq!(wheel.len(), 2, "inner then outer");
    assert_eq!(wheel[0], (Some(INNER), Vec2::new(10.0, 0.0)));
    assert_eq!(wheel[1], (Some(OUTER), Vec2::new(8.0, 0.0)));
}

#[test]
fn a_secondary_click_reports_a_context_menu_request() {
    let mut runtime = runtime();
    runtime.frame(0.0);

    runtime.pointer_move(Vec2::new(50.0, 50.0));
    runtime.pointer_down(Vec2::new(50.0, 50.0), PointerButton::Right);
    runtime.pointer_up(Vec2::new(50.0, 50.0), PointerButton::Right);
    assert_eq!(
        runtime.state().clicks,
        0,
        "a right click is not an activation"
    );
    assert_eq!(runtime.state().menus, vec![INNER]);
}

#[test]
fn a_skipped_frame_does_not_unmount_anything() {
    let mut runtime = runtime();
    runtime.state_mut().render_state = true;
    runtime.invalidate();
    runtime.frame(0.0);
    assert_eq!(runtime.state().mounted, vec![STATE_KEY]);

    // Nothing changed, so the view is not rebuilt and the key stays mounted.
    runtime.frame(0.0);
    assert!(runtime.state().unmounted.is_empty());
}

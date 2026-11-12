//! The runtime must drive a real application with no window and no GPU.
//!
//! This is the acceptance test for the "one event loop" refactor: a counter
//! application is built, clicked and focused through the same [`AppRuntime`]
//! the windowed shell uses, and the reports are asserted at the scene level.

use unbound_app::{AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome, Modifiers};
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_widgets::button::emit_button;
use unbound_widgets::diff::SceneDiff;
use unbound_widgets::text_input::{TextEditState, TextInputDesc, TextInputVisual, TextInputWidget};
use unbound_widgets::{ButtonDesc, ButtonVariant, ControlState, WidgetDesc, WidgetOutput};

const BUTTON: u64 = 10;
const FIELD: u64 = 42;
const BACKGROUND: u64 = 9;
const TEXT_INPUT_SLOTS: usize = 64;

#[derive(Default)]
struct Counter;

struct CounterState {
    clicks: u32,
}

impl Application for Counter {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        CounterState { clicks: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == BUTTON => {
                state.clicks += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let mut desc = ButtonDesc::new(Rect::new(40.0, 40.0, 200.0, 44.0), "Count");
        desc.variant = if state.clicks % 2 == 0 {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        };
        let control = view.ctx.control_state(BUTTON, false, false);
        let output: WidgetOutput = emit_button(BUTTON, &desc, control, &mut []);
        view.push(BUTTON, output);
        if state.clicks == 0 {
            if let Some(request) = unbound_widgets::BackdropRequest::new(
                BUTTON,
                Rect::new(20.0, 20.0, 240.0, 80.0),
                12.0,
            ) {
                view.request_backdrop(request);
            }
        }
    }
}

fn runtime() -> AppRuntime<Counter> {
    AppRuntime::new(Counter, Vec2::new(800.0, 600.0))
}

/// A scene with 5,000 inert background primitives and a single text field
/// whose span is reserved, so typing cannot shift the background.
#[derive(Default)]
struct TypingScene;

struct TypingState {
    text: String,
}

impl Application for TypingScene {
    type State = TypingState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        TypingState {
            text: String::new(),
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Text { text } => {
                state.text.push_str(&text);
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let background = WidgetOutput {
            primitives: (0..5_000)
                .map(|index| GpuPrimitiveInstance {
                    local_pos: [(index % 800) as f32, (index / 800) as f32],
                    size: [1.0, 1.0],
                    ..GpuPrimitiveInstance::default()
                })
                .collect(),
            ..WidgetOutput::default()
        };
        view.push(BACKGROUND, background);

        let mut glyphs: Vec<GlyphQuad> = state
            .text
            .chars()
            .enumerate()
            .map(|(index, _)| GlyphQuad {
                rect: Rect::new(20.0 + index as f32 * 8.0, 20.0, 7.0, 16.0),
                uv: [0.0; 4],
                layer: 0,
                color: false,
                msdf: false,
                phase: 0,
            })
            .collect();
        let widget = TextInputWidget {
            node: FIELD,
            desc: TextInputDesc::new(Rect::new(10.0, 10.0, 400.0, 32.0)),
            edit: TextEditState::with_text(state.text.clone()),
            visual: TextInputVisual::default(),
        };
        let output = widget.emit(ControlState::default(), &mut glyphs);
        view.push_with_capacity(FIELD, output, TEXT_INPUT_SLOTS);
    }
}

#[test]
fn the_first_frame_is_a_whole_scene() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert_eq!(report.diff, SceneDiff::Whole);
    assert!(report.primitives > 0);
    assert!(report.bytes > 0);
    assert!(!runtime.scene().semantics.is_empty());
}

#[test]
fn hovering_repaints_only_the_button_span() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    let (start, end) = runtime.scene().node_spans[&BUTTON];

    runtime.pointer_move(Vec2::new(100.0, 60.0));
    let report = runtime.frame(1.0 / 60.0);
    let SceneDiff::Ranges(ranges) = &report.diff else {
        panic!("hover should produce ranges, got {:?}", report.diff);
    };
    assert!(!ranges.is_empty());
    for (from, to) in ranges {
        assert!(
            *from >= start && *to <= end,
            "{from}..{to} outside {start}..{end}"
        );
    }
    assert!(report.bytes < u32::MAX);
}

#[test]
fn a_click_reaches_reduce_and_rebuilds_the_button() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.pointer_move(Vec2::new(100.0, 60.0));
    runtime.pointer_down(Vec2::new(100.0, 60.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(100.0, 60.0), PointerButton::Left);
    assert_eq!(runtime.state().clicks, 1);

    let report = runtime.frame(1.0 / 60.0);
    assert!(matches!(
        report.diff,
        SceneDiff::Ranges(_) | SceneDiff::Whole
    ));
}

#[test]
fn clicking_outside_does_nothing() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.pointer_move(Vec2::new(700.0, 500.0));
    runtime.pointer_down(Vec2::new(700.0, 500.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(700.0, 500.0), PointerButton::Left);
    assert_eq!(runtime.state().clicks, 0);
}

#[test]
fn a_frame_with_no_dirty_flags_does_no_work() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    let report = runtime.frame(1.0 / 60.0);
    assert_eq!(report.diff, SceneDiff::Identical);
    assert_eq!(report.bytes, 0);
}

#[test]
fn tab_focuses_the_button_and_the_next_frame_repaints_the_ring() {
    let mut runtime = runtime();
    runtime.frame(0.0);

    let flags = runtime.key(AppKey::Tab, true, Modifiers::default());
    assert!(flags.merge(DirtyFlags::NONE) != DirtyFlags::NONE);
    assert_eq!(runtime.focus().focused(), Some(BUTTON));
    assert!(runtime.focus().focus_visible());

    let report = runtime.frame(1.0 / 60.0);
    assert!(matches!(
        report.diff,
        SceneDiff::Ranges(_) | SceneDiff::Whole
    ));
}

#[test]
fn backdrop_requests_reach_the_frame_report() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert_eq!(report.backdrops.len(), 1);
    assert_eq!(report.backdrops[0].blur, 12.0);

    runtime.pointer_move(Vec2::new(100.0, 60.0));
    runtime.pointer_down(Vec2::new(100.0, 60.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(100.0, 60.0), PointerButton::Left);
    let report = runtime.frame(1.0 / 60.0);
    assert!(
        report.backdrops.is_empty(),
        "the dialog closed, so the blur pass must not run"
    );
}

#[test]
fn resizing_forces_a_whole_scene() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.resize(Vec2::new(1024.0, 768.0));
    assert_eq!(runtime.context().viewport, Vec2::new(1024.0, 768.0));
    let report = runtime.frame(1.0 / 60.0);
    assert_eq!(report.diff, SceneDiff::Whole);
}

#[test]
fn typing_in_text_input_uploads_only_local_range() {
    let mut runtime = AppRuntime::new(TypingScene, Vec2::new(800.0, 600.0));
    let first = runtime.frame(0.0);
    assert_eq!(first.diff, SceneDiff::Whole);

    for index in 0..10 {
        runtime.text(if index % 2 == 0 { "a" } else { "b" });
        let report = runtime.frame(1.0 / 60.0);
        assert!(
            matches!(report.diff, SceneDiff::Ranges(_)),
            "frame {index} re-uploaded the whole scene: {:?}",
            report.diff
        );
        assert!(
            report.bytes <= 512,
            "frame {index} uploaded {} B, expected <= 512 B",
            report.bytes
        );
        assert!(
            report.aux_changed,
            "the glyph payload table must ride the auxiliary upload"
        );
    }
}

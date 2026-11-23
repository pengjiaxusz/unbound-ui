//! 确定性验收：同一动作序列跑两遍，状态与像素必须完全一致。
//!
//! 这是 Agent 回归的**前提**。若动作序列的结果依赖真实时间或随机数，
//! 那么「昨天过的测试今天挂」就无从区分是回归还是抖动，自动化也就失去意义。
//!
//! 框架侧的保证是：帧推进只认显式的 `dt`，运行时的时间轴由 `dt` 累加而得，
//! 绝不掺入真实时钟。

use unbound_agent::{AgentSession, OffscreenPixelSource, TreeSnapshot};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, ViewBuilder,
};
use unbound_gpu::device::{gpu_test_guard, test_device};
use unbound_math::Vec2;
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 9001;
const BUTTON_KEY: u64 = 104;
const TOGGLE_KEY: u64 = 105;
const STATUS_KEY: u64 = 106;

const BUTTON_TEXT: &str = "Reset counter";
const TOGGLE_TEXT: &str = "Auto refresh";

const VIEW_WIDTH: u32 = 640;
const VIEW_HEIGHT: u32 = 480;

/// 一个会因交互而改变状态的应用。
struct CounterApp;

struct CounterState {
    clicks: u32,
    toggled: bool,
}

impl Application for CounterApp {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CounterState {
            clicks: 0,
            toggled: false,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == BUTTON_KEY => {
                state.clicks += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            AppEvent::Click { node, .. } if node == TOGGLE_KEY => {
                state.toggled = !state.toggled;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            // 状态直接进画面：语义树与像素里都要能看出「点了几次」，
            // 否则「两次一致」就成了一句空话。
            col.text_primary(
                STATUS_KEY,
                format!("count {} / auto {}", state.clicks, state.toggled),
                14.0,
                400,
            );
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
            col.button(TOGGLE_KEY, ButtonDesc::label(TOGGLE_TEXT));
        });
    }
}

fn runtime() -> AppRuntime<CounterApp> {
    let mut runtime = AppRuntime::new(CounterApp, Vec2::new(VIEW_WIDTH as f32, VIEW_HEIGHT as f32));
    runtime.frame(1.0 / 60.0);
    runtime
}

/// 准备一个离屏像素源；无 GPU 设备时返回 `None`（测试跳过而非失败）。
fn offscreen_source() -> Option<OffscreenPixelSource> {
    let _guard = gpu_test_guard();
    let gpu = test_device()?;
    OffscreenPixelSource::with_gpu(gpu, VIEW_WIDTH, VIEW_HEIGHT, 1.0).ok()
}

/// 跑一串动作，返回结束时的语义树与像素。
fn run_sequence(
    runtime: &mut AppRuntime<CounterApp>,
    source: &mut OffscreenPixelSource,
) -> (TreeSnapshot, Vec<u8>) {
    let mut session = AgentSession::new(runtime);

    let button = session
        .find(accesskit::Role::Button, BUTTON_TEXT)
        .expect("the button must exist");
    session.click(button).expect("the click must succeed");

    let toggle = session
        .find(accesskit::Role::Button, TOGGLE_TEXT)
        .expect("the toggle must exist");
    session.toggle(toggle).expect("the toggle must succeed");

    // 固定时长推进：步长恒定，动画落点必然一致。
    session.advance_seconds(0.5);

    let tree = session.view().to_tree();
    let pixels = session
        .capture_offscreen(source)
        .expect("the capture must succeed")
        .pixels
        .to_vec();
    (tree, pixels)
}

/// 同一动作序列跑两遍：语义树必须逐字段相同。
#[test]
fn the_same_action_sequence_produces_the_same_semantic_tree() {
    let mut runtime_a = runtime();
    let mut runtime_b = runtime();

    let (tree_a, _) = {
        let Some(mut source) = offscreen_source() else {
            return;
        };
        run_sequence(&mut runtime_a, &mut source)
    };
    let (tree_b, _) = {
        let Some(mut source) = offscreen_source() else {
            return;
        };
        run_sequence(&mut runtime_b, &mut source)
    };

    assert!(
        !tree_a.nodes.is_empty(),
        "the semantics tree must not be empty"
    );
    assert_eq!(
        tree_a, tree_b,
        "the two runs must agree on the semantics tree"
    );
}

/// 同一动作序列跑两遍：像素必须逐字节相同。
#[test]
fn the_same_action_sequence_produces_the_same_pixels() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime_a = runtime();
    let (_, pixels_a) = run_sequence(&mut runtime_a, &mut source);
    drop(source);

    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime_b = runtime();
    let (_, pixels_b) = run_sequence(&mut runtime_b, &mut source);

    assert_eq!(pixels_a.len(), pixels_b.len());
    assert_eq!(pixels_a, pixels_b, "the two runs must agree on every pixel");
}

/// 帧步长可配置，且非法步长会被拒绝。
#[test]
fn the_frame_step_is_configurable_and_validated() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);

    assert!(
        (session.frame_dt() - 1.0 / 60.0).abs() < f32::EPSILON,
        "the default step must be 1/60 second"
    );

    session.set_frame_dt(1.0 / 30.0);
    assert!((session.frame_dt() - 1.0 / 30.0).abs() < f32::EPSILON);

    session.set_frame_dt(0.0);
    assert!(
        (session.frame_dt() - 1.0 / 30.0).abs() < f32::EPSILON,
        "a non-positive step must be rejected"
    );

    session.set_frame_dt(f32::NAN);
    assert!(
        (session.frame_dt() - 1.0 / 30.0).abs() < f32::EPSILON,
        "a non-finite step must be rejected"
    );
}

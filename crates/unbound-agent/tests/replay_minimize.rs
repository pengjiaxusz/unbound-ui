//! 回放与最小复现验收：录 20 步 → 注入失败 → 剪出最短的复现序列。
//!
//! 「界面上点了 20 下之后坏了」对任何人都没用；「点了 5 下就坏了」才有用。
//! 这一组测试要证明的正是后者：**剪枝后的序列仍然能复现问题，且明显更短**。

use std::any::Any;
use std::sync::Arc;

use accesskit::Role;
use unbound_agent::{AgentSession, RecordedStep};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, ViewBuilder,
};
use unbound_math::Vec2;
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 9001;
const BUTTON_KEY: u64 = 104;

const BUTTON_TEXT: &str = "Reset counter";

/// 点击多少次之后界面进入失败状态。
const FAILURE_THRESHOLD: u32 = 5;
/// 录制多少步。
const RECORDED_STEPS: usize = 20;

struct ClickApp;

#[derive(Clone)]
struct ClickState {
    clicks: u32,
}

impl Application for ClickApp {
    type State = ClickState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        ClickState { clicks: 0 }
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
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
        });
    }

    fn snapshot_state(&self, state: &Self::State) -> Option<Arc<dyn Any>> {
        Some(Arc::new(state.clone()))
    }

    fn restore_state(&mut self, state: &mut Self::State, snapshot: &dyn Any) {
        if let Some(snapshot) = snapshot.downcast_ref::<ClickState>() {
            *state = snapshot.clone();
        }
    }
}

fn runtime() -> AppRuntime<ClickApp> {
    let mut runtime = AppRuntime::new(ClickApp, Vec2::new(640.0, 480.0));
    runtime.frame(1.0 / 60.0);
    runtime
}

/// 录制 20 步点击，返回基线快照与录制好的步骤。
fn record(
    session: &mut AgentSession<'_, ClickApp>,
) -> (unbound_agent::SessionSnapshot, Vec<RecordedStep>) {
    let baseline = session.snapshot();
    let button = session
        .find(Role::Button, BUTTON_TEXT)
        .expect("the button must exist");
    for _ in 0..RECORDED_STEPS {
        session.click(button).expect("the click must succeed");
    }
    let steps = session.log().steps().to_vec();
    (baseline, steps)
}

/// 回放能把录好的动作重新执行一遍。
#[test]
fn a_recorded_sequence_can_be_replayed() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let (baseline, steps) = record(&mut session);
    assert_eq!(steps.len(), RECORDED_STEPS);
    assert_eq!(session.runtime().state().clicks, RECORDED_STEPS as u32);

    // 从基线重新回放，结果必须与录制时完全一致。
    session
        .restore(&baseline)
        .expect("the restore must succeed");
    assert_eq!(session.runtime().state().clicks, 0);

    let executed = session.replay(&steps).expect("the replay must succeed");
    assert_eq!(executed, RECORDED_STEPS);
    assert_eq!(session.runtime().state().clicks, RECORDED_STEPS as u32);
}

/// 回放期间不录制：否则日志会自己套自己地长。
#[test]
fn replaying_does_not_record_again() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let (baseline, steps) = record(&mut session);

    session
        .restore(&baseline)
        .expect("the restore must succeed");
    session.replay(&steps).expect("the replay must succeed");
    assert_eq!(
        session.log().len(),
        RECORDED_STEPS,
        "the log must not grow while replaying"
    );
}

/// 剪枝：20 步的序列缩到「刚刚好会坏」的长度，且仍然能复现失败。
#[test]
fn a_failure_is_minimized_to_its_shortest_reproduction() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let (baseline, steps) = record(&mut session);

    let minimal = session.minimize_failure(&baseline, &steps, |session| {
        session.runtime().state().clicks >= FAILURE_THRESHOLD
    });

    assert_eq!(
        minimal.len(),
        FAILURE_THRESHOLD as usize,
        "the minimal reproduction must be exactly as long as it takes to fail"
    );
    assert!(
        minimal.len() < steps.len(),
        "the minimal reproduction must be shorter than the original"
    );

    // 剪出来的序列必须仍然能复现失败。
    session
        .restore(&baseline)
        .expect("the restore must succeed");
    session
        .replay(&minimal.steps)
        .expect("the minimal reproduction must replay");
    assert!(
        session.runtime().state().clicks >= FAILURE_THRESHOLD,
        "the minimal reproduction must still trigger the failure"
    );

    // 复现脚本要能直接贴进 issue。
    assert!(
        minimal.script.contains("action"),
        "the reproduction script must describe the actions"
    );
    assert_eq!(
        minimal.script.lines().count(),
        minimal.len(),
        "the script must have one line per step"
    );
}

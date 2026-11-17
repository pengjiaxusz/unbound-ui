//! 快照与恢复验收：探索 → 回到原样。
//!
//! Agent 探索界面时免不了顺手点一点。快照与恢复要回答的是：
//! **「回到刚才那个状态再看看别的分支」到底行不行？**
//!
//! 验收标准不是「不报错」，而是**逐字段相同**：语义树相同，像素相同。

use std::any::Any;
use std::sync::Arc;

use accesskit::Role;
use unbound_agent::{AgentSession, OffscreenPixelSource};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, SemanticIntent,
    ViewBuilder,
};
use unbound_gpu::device::{gpu_test_guard, test_device};
use unbound_math::Vec2;
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 9001;
const BUTTON_KEY: u64 = 104;
const STATUS_KEY: u64 = 106;

const BUTTON_TEXT: &str = "Reset counter";
const STATUS_NAME: &str = "Click count";

const VIEW_WIDTH: u32 = 640;
const VIEW_HEIGHT: u32 = 480;

/// 支持状态快照的应用。
struct SnapshotApp;

#[derive(Clone)]
struct SnapshotState {
    clicks: u32,
}

impl Application for SnapshotApp {
    type State = SnapshotState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        SnapshotState { clicks: 0 }
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

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            // 语义节点必须挂在真实存在的控件上：先建控件，再标注语义。
            col.text_primary(STATUS_KEY, format!("count {}", state.clicks), 14.0, 400);
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
        });
        // 计数进语义树的取值：恢复后这个值必须一并回到原样。
        view.declare_semantics(
            STATUS_KEY,
            SemanticIntent::new(Role::Status, STATUS_NAME)
                .with_value(format!("count {}", state.clicks)),
        );
    }

    fn snapshot_state(&self, state: &Self::State) -> Option<Arc<dyn Any>> {
        Some(Arc::new(state.clone()))
    }

    fn restore_state(&mut self, state: &mut Self::State, snapshot: &dyn Any) {
        if let Some(snapshot) = snapshot.downcast_ref::<SnapshotState>() {
            *state = snapshot.clone();
        }
    }
}

/// 不支持状态快照的应用：沿用框架的默认实现。
struct PlainApp;

impl Application for PlainApp {
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
        });
    }
}

/// 准备一个离屏像素源；无 GPU 设备时返回 `None`（测试跳过而非失败）。
fn offscreen_source() -> Option<OffscreenPixelSource> {
    let _guard = gpu_test_guard();
    let gpu = test_device()?;
    OffscreenPixelSource::with_gpu(gpu, VIEW_WIDTH, VIEW_HEIGHT, 1.0).ok()
}

/// 快照 → 探索 → 恢复：语义树与像素都必须回到原样。
#[test]
fn restoring_brings_the_interface_back() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime = AppRuntime::new(
        SnapshotApp,
        Vec2::new(VIEW_WIDTH as f32, VIEW_HEIGHT as f32),
    );
    runtime.frame(1.0 / 60.0);
    let mut session = AgentSession::new(&mut runtime);

    let snapshot = session.snapshot();
    assert!(snapshot.is_restorable());
    assert!(snapshot.semantics_len() > 0);

    let tree_before = session.view().to_tree();
    let pixels_before = session
        .capture_offscreen(&mut source)
        .expect("the capture must succeed")
        .pixels
        .to_vec();

    // 探索：点一下按钮，界面必须真的变样，否则后面的对比毫无意义。
    let button = session
        .find(Role::Button, BUTTON_TEXT)
        .expect("the button must exist");
    session.click(button).expect("the click must succeed");
    let tree_after = session.view().to_tree();
    assert_ne!(
        tree_before, tree_after,
        "the click must change the semantics tree"
    );

    // 恢复。
    session
        .restore(&snapshot)
        .expect("the restore must succeed");

    let tree_restored = session.view().to_tree();
    assert_eq!(
        tree_before, tree_restored,
        "the restored semantics tree must match the original one"
    );

    let pixels_restored = session
        .capture_offscreen(&mut source)
        .expect("the capture must succeed")
        .pixels
        .to_vec();
    assert_eq!(
        pixels_before, pixels_restored,
        "the restored pixels must match the original ones"
    );
}

/// 应用不提供状态快照时，恢复必须明确失败，而不是悄悄恢复一半。
#[test]
fn restoring_without_state_support_fails_loudly() {
    let mut runtime = AppRuntime::new(PlainApp, Vec2::new(VIEW_WIDTH as f32, VIEW_HEIGHT as f32));
    runtime.frame(1.0 / 60.0);
    let mut session = AgentSession::new(&mut runtime);

    let snapshot = session.snapshot();
    assert!(!snapshot.is_restorable());
    let error = session
        .restore(&snapshot)
        .expect_err("the restore must fail");
    assert_eq!(
        error.describe(),
        "the application does not provide state snapshots"
    );
}

//! 跨线程 Agent 桥的验收测试。
//!
//! 这些测试刻意把请求放在**另一个线程**发起：桥的全部价值就在这里——
//! 界面状态不离开 UI 线程，而请求照样能驱动它。

use std::time::Duration;

use accesskit::Role;
use unbound_agent::{AgentBridge, AgentBridgeHandle, AgentReply, AgentRequest};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, ViewBuilder,
};
use unbound_math::Vec2;
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 7101;
const BUTTON_KEY: u64 = 7102;
const STATUS_KEY: u64 = 7103;

const BUTTON_TEXT: &str = "Reset counter";
const STATUS_NAME: &str = "Run status";

const VIEWPORT: Vec2 = Vec2::new(640.0, 480.0);

/// 一个会因点击而改变状态的应用。
struct CounterApp;

struct CounterState {
    clicks: u32,
}

impl Application for CounterApp {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CounterState { clicks: 0 }
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
        view.declare_semantics(
            STATUS_KEY,
            unbound_app::prelude::SemanticIntent::new(Role::Status, STATUS_NAME)
                .with_value(format!("count {}", state.clicks)),
        );
    }
}

/// 建桥，并返回发送端与接收句柄。
fn bridge() -> (AgentBridge, AgentBridgeHandle) {
    AgentBridge::new()
}

/// 在 UI 线程上持续排空请求，直到后台线程拿到回执。
///
/// 真实运行时，UI 线程每帧都会排空一次；这里只是把那个循环压缩到
/// 一个测试里。**不阻塞**地轮询，也因此不会掩盖「UI 线程被卡住」这类缺陷。
fn drain_until_replied<A: unbound_app::Application>(
    handle: &AgentBridgeHandle,
    runtime: &mut AppRuntime<A>,
    done: &std::sync::mpsc::Receiver<()>,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        handle.drain_builtin(runtime, None);
        if done.try_recv().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the UI thread never drained the pending request");
}

#[test]
fn a_tree_query_from_another_thread_returns_the_whole_tree() {
    let (bridge, handle) = bridge();
    let mut runtime = AppRuntime::new(CounterApp, VIEWPORT);
    runtime.frame(1.0 / 60.0);

    // 后台线程发起请求：界面状态始终留在 UI 线程。
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let reply = bridge.request_default(AgentRequest::QueryTree);
        let _ = done_tx.send(());
        reply
    });

    drain_until_replied(&handle, &mut runtime, &done_rx);

    let reply = worker.join().expect("the worker must not panic");
    match reply {
        AgentReply::Tree(tree) => {
            assert!(!tree.is_empty(), "the tree must not be empty");
            assert!(
                tree.nodes.iter().any(|node| node.label == BUTTON_TEXT),
                "the button must be present in the tree"
            );
        }
        other => panic!("expected a tree reply, got {other:?}"),
    }
}

#[test]
fn a_semantic_action_from_another_thread_actually_changes_the_state() {
    let (bridge, handle) = bridge();
    let mut runtime = AppRuntime::new(CounterApp, VIEWPORT);
    runtime.frame(1.0 / 60.0);

    // 先找到按钮的节点标识。
    let button = runtime
        .semantics()
        .iter()
        .find(|(_, node)| node.label == BUTTON_TEXT)
        .map(|(id, _)| id)
        .expect("the button must exist");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let reply = bridge.request_default(AgentRequest::Perform(
            unbound_agent::SemanticAction::Click { node: button },
        ));
        let _ = done_tx.send(());
        reply
    });

    drain_until_replied(&handle, &mut runtime, &done_rx);

    match worker.join().expect("the worker must not panic") {
        AgentReply::Done => {}
        other => panic!("expected a successful action, got {other:?}"),
    }

    // 状态必须真的变了：计数语义节点的值应当是 1。
    let status = runtime
        .semantics()
        .iter()
        .find(|(_, node)| node.label == STATUS_NAME)
        .map(|(_, node)| node.value.clone())
        .expect("the status node must exist");
    assert_eq!(status, "count 1", "the click must have been applied");
}

#[test]
fn a_failing_assertion_comes_back_as_a_structured_diagnosis() {
    let (bridge, handle) = bridge();
    let mut runtime = AppRuntime::new(CounterApp, VIEWPORT);
    runtime.frame(1.0 / 60.0);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let reply = bridge.request_default(AgentRequest::Assert(unbound_agent::Assertion::Text {
            node: 999_999,
            expected: "nothing".to_owned(),
        }));
        let _ = done_tx.send(());
        reply
    });

    drain_until_replied(&handle, &mut runtime, &done_rx);

    match worker.join().expect("the worker must not panic") {
        AgentReply::Failed(diagnosis) => {
            assert!(
                diagnosis.contains("999999"),
                "the diagnosis must name the missing node: {diagnosis}"
            );
        }
        other => panic!("expected a failed assertion, got {other:?}"),
    }
}

#[test]
fn a_request_to_a_dead_ui_thread_fails_fast_instead_of_hanging() {
    let (bridge, handle) = bridge();
    drop(handle);

    let started = std::time::Instant::now();
    let reply = bridge.request(AgentRequest::QueryTree, Duration::from_secs(5));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a dead UI thread must fail fast, not block for the whole timeout"
    );
    assert!(reply.is_failure());
}

#[test]
fn the_ui_thread_keeps_rendering_while_a_request_is_in_flight() {
    let (bridge, handle) = bridge();
    let mut runtime = AppRuntime::new(CounterApp, VIEWPORT);

    // 后台线程投递请求并等待回执。
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let reply = bridge.request_default(AgentRequest::QueryTree);
        let _ = done_tx.send(());
        reply
    });

    // 请求在途时，UI 线程必须还能照常推进帧——这正是
    //「Agent 工作时用户仍可正常使用电脑」的前提。
    for _ in 0..5 {
        runtime.frame(1.0 / 60.0);
        assert!(
            done_rx.try_recv().is_err(),
            "the UI thread must not have needed the request to be answered in order to render"
        );
    }

    drain_until_replied(&handle, &mut runtime, &done_rx);
    let reply = worker.join().expect("the worker must not panic");
    assert!(matches!(reply, AgentReply::Tree(_)), "got {reply:?}");
}

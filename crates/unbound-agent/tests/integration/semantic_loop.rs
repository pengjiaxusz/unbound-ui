//! Agent 闭环验收：语义树完整 → 按语义定位 → 执行语义动作 → 断言结果。
//!
//! 这组测试是「Agent 原生」的**验收标准**：如果框架做不到「不看截图、
//! 不知道坐标，就能理解界面并操作它」，这里就会红。

use accesskit::Role;
use unbound_agent::{
    A11yNode, AgentSession, Assertion, NodePredicate, NodeQuery, SemanticAction, Semantics,
    TreeSnapshot,
};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, SemanticIntent,
    ViewBuilder,
};
use unbound_math::{Rect, Vec2};
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 9001;
const TITLE_KEY: u64 = 101;
const READING_KEY: u64 = 102;
const COUNT_KEY: u64 = 103;
const BUTTON_KEY: u64 = 104;
const STATUS_KEY: u64 = 105;

const TITLE_TEXT: &str = "VRAM usage";
const READING_TEXT: &str = "12.4 GB";
const BUTTON_TEXT: &str = "Reset counter";
const STATUS_TEXT: &str = "Ready";
const STATUS_NAME: &str = "Run status";
const READING_NAME: &str = "VRAM reading";

struct DemoApp;

struct DemoState {
    count: u32,
}

impl Application for DemoApp {
    type State = DemoState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        DemoState { count: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == BUTTON_KEY => {
                state.count += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            col.text_primary(TITLE_KEY, TITLE_TEXT, 16.0, 600);
            col.text_primary(READING_KEY, READING_TEXT, 16.0, 400);
            col.text_primary(COUNT_KEY, format!("count {}", state.count), 14.0, 400);
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
            col.text_primary(STATUS_KEY, STATUS_TEXT, 14.0, 400);
        });
        // 自绘控件的语义标注：框架层不含任何业务知识，语义由应用注入。
        view.declare_semantics(
            STATUS_KEY,
            SemanticIntent::new(Role::Status, STATUS_NAME).with_value(STATUS_TEXT),
        );
        // 只读读数：名称与当前值分开暴露，Agent 无需解析文本即可取到数值。
        view.declare_semantics(
            READING_KEY,
            SemanticIntent::new(Role::Label, READING_NAME).with_value(READING_TEXT),
        );
    }
}

/// 一个故意拿**语义根键**当容器键的应用。
///
/// 这正是把整机拖死的场景：控件占用了语义根的键，层级重建时窗口就成了
/// 自己的子节点，任何树遍历都只会在环上打转。框架必须在源头把它挡下来。
struct CollidingKeyApp;

impl Application for CollidingKeyApp {
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
        view.column(unbound_app::ROOT, |col| {
            col.text_primary(TITLE_KEY, TITLE_TEXT, 16.0, 600);
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
        });
    }
}

fn runtime() -> AppRuntime<DemoApp> {
    let mut runtime = AppRuntime::new(DemoApp, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);
    runtime
}

/// 取语义树快照。
fn snapshot(runtime: &mut AppRuntime<DemoApp>) -> TreeSnapshot {
    AgentSession::new(runtime).view().to_tree()
}

/// 按条件查询节点。
fn query(runtime: &mut AppRuntime<DemoApp>, query: &NodeQuery) -> Vec<u64> {
    AgentSession::new(runtime).view().query(query)
}

/// 取节点的名称。
fn name(runtime: &mut AppRuntime<DemoApp>, id: u64) -> String {
    AgentSession::new(runtime)
        .view()
        .node(id)
        .map(|node| node.label.clone())
        .unwrap_or_default()
}

/// 取节点的语义路径。
fn path(runtime: &mut AppRuntime<DemoApp>, id: u64) -> String {
    AgentSession::new(runtime).view().path(id)
}

/// 枚举界面上的全部文本。
fn all_text(runtime: &mut AppRuntime<DemoApp>) -> Vec<String> {
    AgentSession::new(runtime)
        .view()
        .all_text()
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn read_only_text_must_reach_the_semantics_tree() {
    let mut runtime = runtime();
    let text = query(&mut runtime, &NodeQuery::any().name(TITLE_TEXT));
    assert_eq!(
        text.len(),
        1,
        "the title text must be reachable by semantic query"
    );
    assert_eq!(name(&mut runtime, text[0]), TITLE_TEXT);
}

#[test]
fn read_only_text_must_use_the_static_text_role() {
    let mut runtime = runtime();
    let text = query(&mut runtime, &NodeQuery::any().name(TITLE_TEXT));
    let tree = snapshot(&mut runtime);
    let node = tree.node(text[0]).expect("the text node must exist");
    assert_eq!(
        node.role, "Label",
        "read-only text must use the static text role"
    );
}

#[test]
fn numeric_content_must_be_readable() {
    let mut runtime = runtime();
    let hits = query(&mut runtime, &NodeQuery::any().value(READING_TEXT));
    assert_eq!(
        hits.len(),
        1,
        "read-only data must be searchable by its current value"
    );
    let tree = snapshot(&mut runtime);
    let node = tree.node(hits[0]).expect("the reading node must exist");
    assert_eq!(
        node.value, READING_TEXT,
        "the current value must be readable verbatim"
    );
    assert_eq!(
        node.label, READING_NAME,
        "the reading must expose its name and its value separately"
    );
}

#[test]
fn all_text_must_be_enumerable() {
    let mut runtime = runtime();
    let text = all_text(&mut runtime);
    assert!(
        text.iter().any(|item| item == TITLE_TEXT),
        "all text must contain the title, actual: {text:?}"
    );
    assert!(
        text.iter().any(|item| item == BUTTON_TEXT),
        "all text must contain the button label, actual: {text:?}"
    );
    assert!(
        text.iter().any(|item| item == READING_TEXT),
        "the current value of read-only data must be enumerable too, actual: {text:?}"
    );
}

#[test]
fn buttons_must_carry_a_readable_name() {
    let mut runtime = runtime();
    let button = query(
        &mut runtime,
        &NodeQuery::any().role(Role::Button).name(BUTTON_TEXT),
    );
    assert_eq!(
        button.len(),
        1,
        "the button must be locatable by role and name"
    );
    let tree = snapshot(&mut runtime);
    let node = tree.node(button[0]).expect("the button node must exist");
    assert!(node.focusable, "the button must be marked focusable");
}

#[test]
fn explicit_annotations_must_take_effect() {
    let mut runtime = runtime();
    let status = query(
        &mut runtime,
        &NodeQuery::any().role(Role::Status).name(STATUS_NAME),
    );
    assert_eq!(
        status.len(),
        1,
        "explicit annotations must reach the semantics tree"
    );
    let tree = snapshot(&mut runtime);
    let node = tree.node(status[0]).expect("the status node must exist");
    assert_eq!(
        node.value, STATUS_TEXT,
        "the annotated current value must be written"
    );
}

#[test]
fn the_semantics_tree_must_be_layered() {
    let mut runtime = runtime();
    let button = query(&mut runtime, &NodeQuery::any().role(Role::Button));
    let path_text = path(&mut runtime, button[0]);
    assert!(
        path_text.starts_with("Window"),
        "the semantic path must start at the window root, actual: {path_text}"
    );
    assert!(
        path_text.contains(&format!("Button \"{BUTTON_TEXT}\"")),
        "the semantic path must contain the node itself, actual: {path_text}"
    );
}

#[test]
fn semantic_actions_must_drive_the_interface() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let button = session
        .find(Role::Button, BUTTON_TEXT)
        .expect("the button must be findable by role and name");
    session
        .perform(&SemanticAction::Click { node: button })
        .expect("the click must succeed");
    // 点击之后，界面上的计数文本必须同步更新。
    let hits = session.query(&NodeQuery::any().name("count 1"));
    assert_eq!(
        hits.len(),
        1,
        "the semantics tree must reflect the new state after the click"
    );
}

#[test]
fn assertions_must_give_structured_diagnostics_on_both_outcomes() {
    let mut runtime = runtime();
    let session = AgentSession::new(&mut runtime);
    let passed = session.assert(&Assertion::Exists {
        query: NodeQuery::any().role(Role::Button).name(BUTTON_TEXT),
    });
    assert!(
        passed.is_ok(),
        "an existing element must pass the assertion"
    );

    let failed = session
        .assert(&Assertion::Exists {
            query: NodeQuery::any().role(Role::Button).name("no such button"),
        })
        .expect_err("a missing element must fail the assertion");
    assert_eq!(failed.kind, "exists");
    assert!(!failed.expected.is_empty());
    assert!(!failed.actual.is_empty());
}

#[test]
fn a_failed_state_assertion_must_suggest_the_next_action() {
    let mut runtime = runtime();
    let session = AgentSession::new(&mut runtime);
    let button = session
        .find(Role::Button, BUTTON_TEXT)
        .expect("the button must be findable by role and name");
    let failed = session
        .assert(&Assertion::State {
            node: button,
            predicates: vec![NodePredicate::Selected],
        })
        .expect_err("an unselected state must fail the assertion");
    let suggestion = failed.suggestion.expect("a failure must suggest an action");
    assert!(
        suggestion.contains("Click"),
        "the suggestion must state what to do next, actual: {suggestion}"
    );
}

#[test]
fn a_wait_condition_must_be_reachable() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    session
        .wait_for(
            &unbound_agent::WaitCondition::NodeExists {
                query: NodeQuery::any().role(Role::Button).name(BUTTON_TEXT),
            },
            std::time::Duration::from_millis(500),
        )
        .expect("the wait condition must be met");
}

#[test]
fn a_wait_timeout_must_report_what_was_observed() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let timeout = session
        .wait_for(
            &unbound_agent::WaitCondition::NodeExists {
                query: NodeQuery::any().name("text that never appears"),
            },
            std::time::Duration::from_millis(50),
        )
        .expect_err("an impossible condition must time out");
    assert_eq!(timeout.observed, 0);
    assert!(timeout.describe().contains("not met"));
}

#[test]
fn action_sequences_must_be_recorded_as_a_readable_script() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let button = session
        .find(Role::Button, BUTTON_TEXT)
        .expect("the button must be findable by role and name");
    session
        .perform(&SemanticAction::Click { node: button })
        .expect("the click must succeed");
    let script = session.log().to_script();
    assert!(
        script.contains("click"),
        "the recorded script must describe the intent of the action, actual: {script}"
    );
    assert!(
        !script.contains("coordinates"),
        "the recorded script must not carry coordinates, actual: {script}"
    );
}

#[test]
fn a_control_colliding_with_the_root_key_must_not_form_a_cycle() {
    let mut runtime = AppRuntime::new(CollidingKeyApp, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);

    let tree = AgentSession::new(&mut runtime).view().to_tree();
    let root = tree.root;
    let root_node = tree.node(root).expect("the semantic root must exist");
    assert!(
        !root_node.children.contains(&root),
        "the semantic root must not become its own child, actual children: {:?}",
        root_node.children
    );
    for node in &tree.nodes {
        assert!(
            !node.children.contains(&node.id),
            "node {} must not become its own child",
            node.id
        );
    }

    // 遍历必须有界：成环会让收集长度无限膨胀，最终把宿主进程拖死。
    let order = AgentSession::new(&mut runtime).view().tree_order();
    assert!(
        order.len() <= tree.nodes.len().max(1) * 2,
        "the traversal length must stay bounded, actual {}, node count {}",
        order.len(),
        tree.nodes.len()
    );
}

#[test]
fn a_key_collision_must_not_swallow_the_control_semantics() {
    let mut runtime = AppRuntime::new(CollidingKeyApp, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);
    let text = AgentSession::new(&mut runtime)
        .view()
        .query(&NodeQuery::any().name(TITLE_TEXT));
    assert_eq!(
        text.len(),
        1,
        "a control with a colliding key must be re-keyed, not swallowed"
    );
}

#[test]
fn the_semantics_tree_must_refuse_cycle_mounts() {
    let mut semantics = Semantics::new(1);
    semantics.upsert(
        1,
        A11yNode::new(Role::Window, "root".to_owned(), Rect::ZERO),
    );
    semantics.upsert(
        2,
        A11yNode::new(Role::Button, "button".to_owned(), Rect::ZERO),
    );

    // 自环必须被拒绝。
    semantics.set_children(1, &[1]);
    assert!(
        !semantics.children_of(1).contains(&1),
        "a parent must not become its own child"
    );

    // 祖先成环必须被拒绝：1 → 2 之后不能再把 1 挂回 2 之下。
    semantics.set_children(1, &[2]);
    semantics.set_children(2, &[1]);
    assert!(
        !semantics.children_of(2).contains(&1),
        "an ancestor must not be re-mounted under its own descendant"
    );

    // 子树收集长度必须有界。
    assert!(
        semantics.descendants(1).len() <= 2,
        "the subtree collection must match the actual node count"
    );
}

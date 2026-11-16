//! Agent 闭环验收：语义树完整 → 按语义定位 → 执行语义动作 → 断言结果。
//!
//! 这组测试是「Agent 原生」的**验收标准**：如果框架做不到「不看截图、
//! 不知道坐标，就能理解界面并操作它」，这里就会红。

use accesskit::Role;
use unbound_agent::{
    AgentSession, Assertion, A11yNode, NodePredicate, NodeQuery, Semantics, SemanticAction,
    TreeSnapshot,
};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, SemanticIntent,
    ViewBuilder,
};
use unbound_math::{Rect, Vec2};
use unbound_widgets::ButtonDesc;

const 布局键: u64 = 9001;
const 标题键: u64 = 101;
const 数值键: u64 = 102;
const 计数键: u64 = 103;
const 按钮键: u64 = 104;
const 状态键: u64 = 105;

struct 演示应用;

struct 演示状态 {
    计数: u32,
}

impl Application for 演示应用 {
    type State = 演示状态;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        演示状态 { 计数: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == 按钮键 => {
                state.计数 += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(布局键, |col| {
            col.text_primary(标题键, "显存占用", 16.0, 600);
            col.text_primary(数值键, "12.4 GB", 16.0, 400);
            col.text_primary(计数键, format!("计数 {}", state.计数), 14.0, 400);
            col.button(按钮键, ButtonDesc::label("重置计数"));
            col.text_primary(状态键, "就绪", 14.0, 400);
        });
        // 自绘控件的语义标注：框架层不含任何业务知识，语义由应用注入。
        view.declare_semantics(
            状态键,
            SemanticIntent::new(Role::Status, "运行状态").with_value("就绪"),
        );
        // 只读读数：名称与当前值分开暴露，Agent 无需解析文本即可取到数值。
        view.declare_semantics(
            数值键,
            SemanticIntent::new(Role::Label, "显存读数").with_value("12.4 GB"),
        );
    }
}

/// 一个故意拿**语义根键**当容器键的应用。
///
/// 这正是把整机拖死的场景：控件占用了语义根的键，层级重建时窗口就成了
/// 自己的子节点，任何树遍历都只会在环上打转。框架必须在源头把它挡下来。
struct 撞键应用;

impl Application for 撞键应用 {
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
            col.text_primary(标题键, "显存占用", 16.0, 600);
            col.button(按钮键, ButtonDesc::label("重置计数"));
        });
    }
}

fn runtime() -> AppRuntime<演示应用> {
    let mut runtime = AppRuntime::new(演示应用, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);
    runtime
}

/// 取语义树快照。
fn 快照(runtime: &mut AppRuntime<演示应用>) -> TreeSnapshot {
    AgentSession::new(runtime).view().to_tree()
}

/// 按条件查询节点。
fn 查询(runtime: &mut AppRuntime<演示应用>, query: &NodeQuery) -> Vec<u64> {
    AgentSession::new(runtime).view().query(query)
}

/// 取节点的名称。
fn 名称(runtime: &mut AppRuntime<演示应用>, id: u64) -> String {
    AgentSession::new(runtime)
        .view()
        .node(id)
        .map(|node| node.label.clone())
        .unwrap_or_default()
}

/// 取节点的语义路径。
fn 路径(runtime: &mut AppRuntime<演示应用>, id: u64) -> String {
    AgentSession::new(runtime).view().path(id)
}

/// 枚举界面上的全部文本。
fn 全部文本(runtime: &mut AppRuntime<演示应用>) -> Vec<String> {
    AgentSession::new(runtime)
        .view()
        .all_text()
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn 只读文本必须进入语义树() {
    let mut runtime = runtime();
    let 文本 = 查询(&mut runtime, &NodeQuery::any().name("显存占用"));
    assert_eq!(文本.len(), 1, "标题文本必须可被语义查询命中");
    assert_eq!(名称(&mut runtime, 文本[0]), "显存占用");
}

#[test]
fn 只读文本的角色必须是静态文本() {
    let mut runtime = runtime();
    let 文本 = 查询(&mut runtime, &NodeQuery::any().name("显存占用"));
    let 树 = 快照(&mut runtime);
    let 节点 = 树.node(文本[0]).expect("文本节点必须存在");
    assert_eq!(节点.role, "Label", "只读文本应使用静态文本角色");
}

#[test]
fn 数值内容必须可被读取() {
    let mut runtime = runtime();
    let 命中 = 查询(&mut runtime, &NodeQuery::any().value("12.4 GB"));
    assert_eq!(命中.len(), 1, "只读数据必须能按当前值检索");
    let 树 = 快照(&mut runtime);
    let 节点 = 树.node(命中[0]).expect("读数节点必须存在");
    assert_eq!(节点.value, "12.4 GB", "当前值必须原样可读");
    assert_eq!(节点.label, "显存读数", "读数的名称与数值必须分开暴露");
}

#[test]
fn 全部文本都应可枚举() {
    let mut runtime = runtime();
    let 文本 = 全部文本(&mut runtime);
    assert!(
        文本.iter().any(|text| text == "显存占用"),
        "全部文本必须包含标题，实际：{文本:?}"
    );
    assert!(
        文本.iter().any(|text| text == "重置计数"),
        "全部文本必须包含按钮标签，实际：{文本:?}"
    );
    assert!(
        文本.iter().any(|text| text == "12.4 GB"),
        "只读数据的当前值也必须可枚举，实际：{文本:?}"
    );
}

#[test]
fn 按钮必须带可读名称() {
    let mut runtime = runtime();
    let 按钮 = 查询(&mut runtime, &NodeQuery::any().role(Role::Button).name("重置计数"));
    assert_eq!(按钮.len(), 1, "按钮必须能按角色与名称定位");
    let 树 = 快照(&mut runtime);
    let 节点 = 树.node(按钮[0]).expect("按钮节点必须存在");
    assert!(节点.focusable, "按钮必须标记为可聚焦");
}

#[test]
fn 显式标注必须生效() {
    let mut runtime = runtime();
    let 状态 = 查询(&mut runtime, &NodeQuery::any().role(Role::Status).name("运行状态"));
    assert_eq!(状态.len(), 1, "显式语义标注必须写入语义树");
    let 树 = 快照(&mut runtime);
    let 节点 = 树.node(状态[0]).expect("状态节点");
    assert_eq!(节点.value, "就绪", "标注里的当前值必须写入");
}

#[test]
fn 语义树必须有层级() {
    let mut runtime = runtime();
    let 按钮 = 查询(&mut runtime, &NodeQuery::any().role(Role::Button));
    let 路径文本 = 路径(&mut runtime, 按钮[0]);
    assert!(
        路径文本.starts_with("Window"),
        "语义路径必须从窗口根节点起步，实际：{路径文本}"
    );
    assert!(
        路径文本.contains("Button \"重置计数\""),
        "语义路径必须包含节点自身，实际：{路径文本}"
    );
}

#[test]
fn 语义动作必须驱动界面() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let 按钮 = session
        .find(Role::Button, "重置计数")
        .expect("必须能按角色与名称找到按钮");
    session
        .perform(&SemanticAction::Click { node: 按钮 })
        .expect("点击必须成功");
    // 点击之后，界面上的计数文本必须同步更新。
    let 命中 = session.query(&NodeQuery::any().name("计数 1"));
    assert_eq!(命中.len(), 1, "点击后语义树必须反映新状态");
}

#[test]
fn 断言通过与失败都要给出结构化诊断() {
    let mut runtime = runtime();
    let session = AgentSession::new(&mut runtime);
    let 通过 = session.assert(&Assertion::Exists {
        query: NodeQuery::any().role(Role::Button).name("重置计数"),
    });
    assert!(通过.is_ok(), "存在的元素必须通过断言");

    let 失败 = session
        .assert(&Assertion::Exists {
            query: NodeQuery::any().role(Role::Button).name("不存在的按钮"),
        })
        .expect_err("不存在的元素必须失败");
    assert_eq!(失败.kind, "exists");
    assert!(!失败.expected.is_empty());
    assert!(!失败.actual.is_empty());
}

#[test]
fn 状态断言失败时要给出下一步动作() {
    let mut runtime = runtime();
    let session = AgentSession::new(&mut runtime);
    let 按钮 = session
        .find(Role::Button, "重置计数")
        .expect("必须能按角色与名称找到按钮");
    let 失败 = session
        .assert(&Assertion::State {
            node: 按钮,
            predicates: vec![NodePredicate::Selected],
        })
        .expect_err("未选中状态必须让断言失败");
    let 建议 = 失败.suggestion.expect("失败必须给出建议动作");
    assert!(
        建议.contains("Click"),
        "建议动作必须指明下一步怎么做，实际：{建议}"
    );
}

#[test]
fn 条件等待必须能达成() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    session
        .wait_for(
            &unbound_agent::WaitCondition::NodeExists {
                query: NodeQuery::any().role(Role::Button).name("重置计数"),
            },
            std::time::Duration::from_millis(500),
        )
        .expect("等待条件必须达成");
}

#[test]
fn 条件等待超时必须给出观察结果() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let 超时 = session
        .wait_for(
            &unbound_agent::WaitCondition::NodeExists {
                query: NodeQuery::any().name("永远不会出现的文本"),
            },
            std::time::Duration::from_millis(50),
        )
        .expect_err("不存在的条件必须超时");
    assert_eq!(超时.observed, 0);
    assert!(超时.describe().contains("未达成"));
}

#[test]
fn 动作序列必须被录制成可读脚本() {
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);
    let 按钮 = session
        .find(Role::Button, "重置计数")
        .expect("必须能按角色与名称找到按钮");
    session
        .perform(&SemanticAction::Click { node: 按钮 })
        .expect("点击必须成功");
    let 脚本 = session.log().to_script();
    assert!(
        脚本.contains("click"),
        "录制脚本必须描述动作意图，实际：{脚本}"
    );
    assert!(
        !脚本.contains("坐标"),
        "录制脚本不得包含坐标，实际：{脚本}"
    );
}

#[test]
fn 控件撞上语义根键时语义树不得成环() {
    let mut runtime = AppRuntime::new(撞键应用, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);

    let 树 = AgentSession::new(&mut runtime).view().to_tree();
    let 根 = 树.root;
    let 根节点 = 树.node(根).expect("语义根必须存在");
    assert!(
        !根节点.children.contains(&根),
        "语义根不得成为自己的子节点，实际子节点：{:?}",
        根节点.children
    );
    for 节点 in &树.nodes {
        assert!(
            !节点.children.contains(&节点.id),
            "节点 {} 不得成为自己的子节点",
            节点.id
        );
    }

    // 遍历必须有界：成环会让收集长度无限膨胀，最终把宿主进程拖死。
    let 顺序 = AgentSession::new(&mut runtime).view().tree_order();
    assert!(
        顺序.len() <= 树.nodes.len().max(1) * 2,
        "遍历长度必须有界，实际 {}，节点数 {}",
        顺序.len(),
        树.nodes.len()
    );
}

#[test]
fn 撞键不得吞掉控件的语义() {
    let mut runtime = AppRuntime::new(撞键应用, Vec2::new(800.0, 600.0));
    runtime.frame(1.0 / 60.0);
    let 文本 = AgentSession::new(&mut runtime)
        .view()
        .query(&NodeQuery::any().name("显存占用"));
    assert_eq!(文本.len(), 1, "撞键的控件必须被改派，而不是被吞掉");
}

#[test]
fn 语义树结构必须拒绝成环挂载() {
    let mut 语义 = Semantics::new(1);
    语义.upsert(1, A11yNode::new(Role::Window, "根".to_owned(), Rect::ZERO));
    语义.upsert(2, A11yNode::new(Role::Button, "按钮".to_owned(), Rect::ZERO));

    // 自环必须被拒绝。
    语义.set_children(1, &[1]);
    assert!(
        !语义.children_of(1).contains(&1),
        "父节点不得成为自己的子节点"
    );

    // 祖先成环必须被拒绝：1 → 2 之后不能再把 1 挂回 2 之下。
    语义.set_children(1, &[2]);
    语义.set_children(2, &[1]);
    assert!(
        !语义.children_of(2).contains(&1),
        "祖先不得重新挂到后代之下"
    );

    // 子树收集长度必须有界。
    assert!(
        语义.descendants(1).len() <= 2,
        "子树收集必须与实际节点数相符"
    );
}

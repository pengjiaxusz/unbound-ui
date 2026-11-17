//! 动作录制与回放。
//!
//! 录下来的不是「在某坐标点了某下」，而是**语义动作序列**。因此它可以在
//! 界面微调之后继续回放——坐标会漂，语义不会。

use crate::action::SemanticAction;
use crate::assert::Assertion;
use crate::wait::WaitCondition;

/// 录制的步骤。
#[derive(Clone, Debug, PartialEq)]
pub enum RecordedStep {
    /// 执行一个语义动作。
    Action(SemanticAction),
    /// 等待某个条件。
    Wait(WaitCondition),
    /// 验证某个断言。
    Assert(Assertion),
    /// 场景标记（如「打开某页面」）。
    Marker(String),
}

impl RecordedStep {
    /// 步骤的简短名称。
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Action(action) => action.name(),
            Self::Wait(_) => "wait",
            Self::Assert(assertion) => assertion.name(),
            Self::Marker(_) => "marker",
        }
    }
}

/// 动作序列。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionLog {
    steps: Vec<RecordedStep>,
}

impl ActionLog {
    /// 空序列。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一步。
    pub fn push(&mut self, step: RecordedStep) {
        self.steps.push(step);
    }

    /// 全部步骤。
    #[must_use]
    pub fn steps(&self) -> &[RecordedStep] {
        &self.steps
    }

    /// 步骤数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.steps.clear();
    }

    /// 追加标记。
    pub fn mark(&mut self, marker: impl Into<String>) {
        self.steps.push(RecordedStep::Marker(marker.into()));
    }

    /// 生成人类可读的脚本。
    ///
    /// 这份文本可以直接贴进issue 或交给另一个人复现——它描述的是**意图**，
    /// 不是鼠标轨迹。
    #[must_use]
    pub fn to_script(&self) -> String {
        Self::script_of(&self.steps)
    }

    /// 生成任意一段步骤的可读脚本。
    ///
    /// 最小复现剪枝产出的子序列也要能直接贴进 issue，因此脚本生成不能
    /// 只认「已经录好的那一份」。
    #[must_use]
    pub fn script_of(steps: &[RecordedStep]) -> String {
        let mut out = String::new();
        for (index, step) in steps.iter().enumerate() {
            out.push_str(&format!("{index:>3}. {}", describe_step(step)));
            out.push('\n');
        }
        out
    }
}

/// 步骤的文本描述。
fn describe_step(step: &RecordedStep) -> String {
    match step {
        RecordedStep::Action(action) => {
            format!("action {} -> node {}", action.name(), action.target())
        }
        RecordedStep::Wait(condition) => format!("wait {}", describe_condition(condition)),
        RecordedStep::Assert(assertion) => format!("assert {}", assertion.name()),
        RecordedStep::Marker(marker) => format!("marker \"{marker}\""),
    }
}

/// 等待条件的文本描述。
fn describe_condition(condition: &WaitCondition) -> String {
    match condition {
        WaitCondition::NodeExists { .. } => "node appears".to_owned(),
        WaitCondition::NodeGone { .. } => "node disappears".to_owned(),
        WaitCondition::NodeState { node, predicates } => {
            let names: Vec<&str> = predicates.iter().map(|p| p.name()).collect();
            format!("node {node} satisfies [{}]", names.join(", "))
        }
        WaitCondition::Settled => "interface settled".to_owned(),
        WaitCondition::TextVisible { text } => format!("text \"{text}\" appears"),
    }
}

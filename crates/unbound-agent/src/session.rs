//! 会话：把语义视图、合成输入、断言与录制串成一条闭环。

use std::time::{Duration, Instant};

use accesskit::Role;
use unbound_a11y::A11yAction;
use unbound_app::{AppRuntime, Application, FrameReport};
use unbound_input::PointerButton;
use unbound_math::{Color, Vec2};
use unbound_widgets::diff::SceneDiff;

use crate::action::SemanticAction;
use crate::assert::{Assertion, AssertionFailure, BoundsExpect, PixelProbe};
use crate::log::{ActionLog, RecordedStep};
use crate::query::{NodePredicate, NodeQuery};
use crate::view::AgentView;
use crate::wait::{WaitCondition, WaitTimeout};

/// 动作执行失败。
///
/// 失败信息一律是**结构化**的：不仅说「没找到」，还要把候选与下一步动作
/// 一并给出，Agent 不必再猜。
#[derive(Clone, Debug, PartialEq)]
pub enum ActionError {
    /// 节点在语义树中不存在。
    NodeMissing {
        /// 目标节点。
        node: u64,
    },
    /// 节点没有有效矩形（尚未布局或已被卸载）。
    NoBounds {
        /// 目标节点。
        node: u64,
    },
    /// 没有匹配的元素。
    NoMatch {
        /// 期望角色。
        role: Option<Role>,
        /// 期望名称。
        name: String,
        /// 实际候选的语义路径。
        candidates: Vec<String>,
    },
    /// 匹配到多个元素，调用方必须显式消歧。
    Ambiguous {
        /// 期望名称。
        name: String,
        /// 候选的语义路径。
        candidates: Vec<String>,
    },
    /// 下拉框里没有该选项。
    OptionMissing {
        /// 下拉框节点。
        node: u64,
        /// 期望的选项文本。
        option: String,
        /// 实际可用的选项。
        available: Vec<String>,
    },
    /// 运行时拒绝了该动作。
    Rejected {
        /// 动作名。
        action: String,
        /// 拒绝原因。
        reason: String,
    },
}

impl ActionError {
    /// 结构化的错误描述。
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NodeMissing { node } => format!("节点 {node} 不在语义树中"),
            Self::NoBounds { node } => format!("节点 {node} 没有有效矩形"),
            Self::NoMatch {
                role,
                name,
                candidates,
            } => {
                let role_text = role.map_or_else(String::new, |role| format!("{:?}", role));
                format!(
                    "未找到角色为 {role_text}、名称为「{name}」的元素；当前候选：{}",
                    describe_candidates(candidates)
                )
            }
            Self::Ambiguous { name, candidates } => format!(
                "名称为「{name}」的元素有多个，无法自动消歧：{}",
                describe_candidates(candidates)
            ),
            Self::OptionMissing {
                node,
                option,
                available,
            } => format!(
                "节点 {node} 下没有选项「{option}」；可用选项：{}",
                describe_candidates(available)
            ),
            Self::Rejected { action, reason } => format!("动作 {action} 被拒绝：{reason}"),
        }
    }
}

/// 候选列表的文本描述。
fn describe_candidates(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "（空）".to_owned()
    } else {
        candidates.join("、")
    }
}

/// Agent 会话。
///
/// 它**不拥有**运行时，只借用；会话期间对界面的全部操作都走框架自身的
/// 事件管线，不注入任何操作系统级输入，因此**用户可以同时正常使用电脑**。
pub struct AgentSession<'a, A: Application> {
    runtime: &'a mut AppRuntime<A>,
    log: ActionLog,
    probe: Option<&'a dyn PixelProbe>,
    record: bool,
}

impl<'a, A: Application> AgentSession<'a, A> {
    /// 建立会话。
    #[must_use]
    pub fn new(runtime: &'a mut AppRuntime<A>) -> Self {
        Self {
            runtime,
            log: ActionLog::new(),
            probe: None,
            record: true,
        }
    }

    /// 建立会话并接入像素探针（供像素级断言使用）。
    #[must_use]
    pub fn with_probe(runtime: &'a mut AppRuntime<A>, probe: &'a dyn PixelProbe) -> Self {
        Self {
            runtime,
            log: ActionLog::new(),
            probe: Some(probe),
            record: true,
        }
    }

    /// 是否记录动作序列。
    pub fn set_record(&mut self, record: bool) {
        self.record = record;
    }

    /// 已录制的动作序列。
    #[must_use]
    pub fn log(&self) -> &ActionLog {
        &self.log
    }

    /// 底层运行时。
    #[must_use]
    pub fn runtime(&self) -> &AppRuntime<A> {
        self.runtime
    }

    /// 底层运行时的可变引用。
    pub fn runtime_mut(&mut self) -> &mut AppRuntime<A> {
        self.runtime
    }

    /// 当前界面视图。
    #[must_use]
    pub fn view(&self) -> AgentView<'_> {
        AgentView::new(
            self.runtime.semantics(),
            self.runtime.router().regions(),
            self.runtime.focus().focused().unwrap_or(0),
            self.runtime.context().viewport,
        )
    }

    /// 推进一帧。
    ///
    /// 帧长固定为 1/60 秒：足够让动画与异步结果落地，又不至于让等待条件
    /// 因时基不同而表现不一致。
    pub fn advance(&mut self) -> FrameReport {
        self.runtime.frame(1.0 / 60.0)
    }

    /// 推进若干帧。
    pub fn advance_frames(&mut self, frames: usize) {
        for _ in 0..frames {
            self.advance();
        }
    }

    /// 等待界面稳定。
    ///
    /// 判据是**帧差异为零**（`SceneDiff::Identical`），而不是固定时长——
    /// 后者是所有自动化脚本里最常见的脆弱点。
    ///
    /// 返回是否在给定帧数内达成稳定。
    pub fn settle(&mut self, max_frames: usize) -> bool {
        let mut stable = 0usize;
        for _ in 0..max_frames {
            let report = self.advance();
            if report.diff == SceneDiff::Identical {
                stable += 1;
                if stable >= 2 {
                    return true;
                }
            } else {
                stable = 0;
            }
        }
        false
    }

    /// 按条件查找节点。
    #[must_use]
    pub fn query(&self, query: &NodeQuery) -> Vec<u64> {
        self.view().query(query)
    }

    /// 查找唯一匹配的元素。
    ///
    /// 命中零个或多个都返回错误——歧义必须由调用方显式消解。
    pub fn find_unique(&self, query: &NodeQuery) -> Result<u64, ActionError> {
        let view = self.view();
        let hits = view.query(query);
        match hits.len() {
            0 => Err(ActionError::NoMatch {
                role: query.role,
                name: query.name.clone().unwrap_or_default(),
                candidates: Vec::new(),
            }),
            1 => Ok(hits[0]),
            _ => Err(ActionError::Ambiguous {
                name: query.name.clone().unwrap_or_default(),
                candidates: hits
                    .iter()
                    .map(|id| view.path(*id))
                    .collect::<Vec<String>>(),
            }),
        }
    }

    /// 按角色与名称查找唯一元素。
    pub fn find(&self, role: Role, name: &str) -> Result<u64, ActionError> {
        let query = NodeQuery::any().role(role).name(name);
        self.find_unique(&query)
    }

    /// 执行一个语义动作。
    pub fn perform(&mut self, action: &SemanticAction) -> Result<(), ActionError> {
        let result = match action {
            SemanticAction::Click { node } => self.click(*node),
            SemanticAction::Focus { node } => self.focus(*node),
            SemanticAction::Toggle { node } => self.toggle(*node),
            SemanticAction::SetText { node, text } => self.set_text(*node, text),
            SemanticAction::SelectOption { node, option } => self.select_option(*node, option),
            SemanticAction::SetValue { node, value } => self.set_value(*node, *value),
            SemanticAction::Expand { node, expanded } => self.set_expanded(*node, *expanded),
            SemanticAction::ScrollIntoView { node } => self.scroll_into_view(*node),
            SemanticAction::Scroll { node, delta } => self.scroll(*node, *delta),
        };
        if result.is_ok() && self.record {
            self.log.push(RecordedStep::Action(action.clone()));
        }
        result
    }

    /// 点击节点。
    pub fn click(&mut self, node: u64) -> Result<(), ActionError> {
        let pos = self.center_of(node)?;
        self.click_at(pos);
        self.advance();
        Ok(())
    }

    /// 点击指定角色与名称的元素。
    pub fn click_named(&mut self, role: Role, name: &str) -> Result<u64, ActionError> {
        let node = self.find(role, name)?;
        self.click(node)?;
        Ok(node)
    }

    /// 在坐标处注入一次完整点击。
    ///
    /// 这是**唯一**接触坐标的地方，且坐标由节点矩形算得。
    fn click_at(&mut self, pos: Vec2) {
        self.runtime.pointer_move(pos);
        self.runtime.pointer_down(pos, PointerButton::Left);
        self.runtime.pointer_up(pos, PointerButton::Left);
    }

    /// 让节点获得焦点。
    pub fn focus(&mut self, node: u64) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.focus_node(Some(node));
        self.advance();
        Ok(())
    }

    /// 切换开关 / 复选框 / 药丸按钮的选中态。
    pub fn toggle(&mut self, node: u64) -> Result<(), ActionError> {
        self.click(node)
    }

    /// 设置文本输入框的内容。
    pub fn set_text(&mut self, node: u64, text: &str) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.focus_node(Some(node));
        self.runtime.text(text);
        self.advance();
        Ok(())
    }

    /// 选择下拉框的某一项。
    ///
    /// 优先直接点击已在语义树中的选项；选项尚未出现时，先展开下拉框、等选项
    /// 浮现，再选中它。对 Agent 而言仍是**一步**。
    pub fn select_option(&mut self, node: u64, option: &str) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        if let Some(option_node) = self.find_option(node, option) {
            self.click(option_node)?;
            return Ok(());
        }
        // 选项尚未出现：展开下拉框后重试。
        self.click(node)?;
        self.settle(12);
        if let Some(option_node) = self.find_option(node, option) {
            self.click(option_node)?;
            return Ok(());
        }
        Err(ActionError::OptionMissing {
            node,
            option: option.to_owned(),
            available: self.available_options(node),
        })
    }

    /// 查找选项节点。
    fn find_option(&self, combobox: u64, option: &str) -> Option<u64> {
        let view = self.view();
        for role in crate::query::OPTION_ROLES {
            let query = NodeQuery::any().role(role).name(option);
            // 先在下拉框子树内找；浮层挂在别处时再全局找。
            if let Some(id) = view.query(&query.clone().within(combobox)).first() {
                return Some(*id);
            }
            if let Some(id) = view.query(&query).first() {
                return Some(*id);
            }
        }
        None
    }

    /// 列出下拉框当前可用的选项。
    fn available_options(&self, combobox: u64) -> Vec<String> {
        let view = self.view();
        let mut names: Vec<String> = Vec::new();
        for role in crate::query::OPTION_ROLES {
            for id in view.query(&NodeQuery::any().role(role).within(combobox)) {
                if let Some(node) = view.node(id) {
                    names.push(node.label.clone());
                }
            }
        }
        if names.is_empty() {
            for role in crate::query::OPTION_ROLES {
                for id in view.query(&NodeQuery::any().role(role)) {
                    if let Some(node) = view.node(id) {
                        names.push(node.label.clone());
                    }
                }
            }
        }
        names
    }

    /// 设置滑块 / 数值输入的值。
    pub fn set_value(&mut self, node: u64, value: f32) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.a11y(node, A11yAction::SetValue, Some(value.to_string()));
        self.advance();
        Ok(())
    }

    /// 展开或收起可展开节点。
    pub fn set_expanded(&mut self, node: u64, expanded: bool) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        let action = if expanded {
            A11yAction::Expand
        } else {
            A11yAction::Collapse
        };
        self.runtime.a11y(node, action, None);
        self.advance();
        Ok(())
    }

    /// 滚动到节点可见。
    pub fn scroll_into_view(&mut self, node: u64) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.a11y(node, A11yAction::ScrollIntoView, None);
        self.advance();
        Ok(())
    }

    /// 滚动指定节点。
    pub fn scroll(&mut self, node: u64, delta: Vec2) -> Result<(), ActionError> {
        let pos = self.center_of(node)?;
        self.runtime.wheel(pos, delta);
        self.advance();
        Ok(())
    }

    /// 节点中心点。
    fn center_of(&self, node: u64) -> Result<Vec2, ActionError> {
        let view = self.view();
        match view.center(node) {
            Some(pos) => Ok(pos),
            None => {
                if self.runtime.semantics().get(node).is_none() {
                    Err(ActionError::NodeMissing { node })
                } else {
                    Err(ActionError::NoBounds { node })
                }
            }
        }
    }

    /// 等待某个条件达成。
    pub fn wait_for(
        &mut self,
        condition: &WaitCondition,
        timeout: Duration,
    ) -> Result<(), WaitTimeout> {
        let start = Instant::now();
        let mut observed = 0usize;
        loop {
            if self.check_condition(condition, &mut observed) {
                if self.record {
                    self.log.push(RecordedStep::Wait(condition.clone()));
                }
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(WaitTimeout {
                    condition: condition.clone(),
                    waited_ms: start.elapsed().as_millis() as u64,
                    observed,
                });
            }
            self.advance();
        }
    }

    /// 检查等待条件是否达成。
    fn check_condition(&self, condition: &WaitCondition, observed: &mut usize) -> bool {
        let view = self.view();
        match condition {
            WaitCondition::NodeExists { query } => {
                let hits = view.query(query);
                *observed = hits.len();
                !hits.is_empty()
            }
            WaitCondition::NodeGone { query } => {
                let hits = view.query(query);
                *observed = hits.len();
                hits.is_empty()
            }
            WaitCondition::NodeState { node, predicates } => {
                *observed = usize::from(view.node(*node).is_some());
                let Some(target) = view.node(*node) else {
                    return false;
                };
                predicates
                    .iter()
                    .all(|predicate| predicate.matches(target, *node == view.focus()))
            }
            WaitCondition::Settled => false,
            WaitCondition::TextVisible { text } => {
                let hits = view.query(&NodeQuery::any().name_contains(text.clone()));
                *observed = hits.len();
                !hits.is_empty()
            }
        }
    }

    /// 执行断言。
    pub fn assert(&self, assertion: &Assertion) -> Result<(), AssertionFailure> {
        let view = self.view();
        match assertion {
            Assertion::Exists { query } => {
                let hits = view.query(query);
                if hits.is_empty() {
                    Err(self.failure(assertion, None, "至少一个匹配节点", "零个", None))
                } else {
                    Ok(())
                }
            }
            Assertion::Absent { query } => {
                let hits = view.query(query);
                if hits.is_empty() {
                    Ok(())
                } else {
                    Err(self.failure(
                        assertion,
                        hits.first().copied(),
                        "零个匹配节点",
                        &format!("{} 个", hits.len()),
                        None,
                    ))
                }
            }
            Assertion::State { node, predicates } => {
                let Some(target) = view.node(*node) else {
                    return Err(self.failure(assertion, Some(*node), "节点存在", "节点缺失", None));
                };
                let failed = predicates
                    .iter()
                    .find(|predicate| !predicate.matches(target, *node == view.focus()));
                match failed {
                    Some(predicate) => Err(self.failure(
                        assertion,
                        Some(*node),
                        &format!("满足 {}", predicate.name()),
                        "未满足",
                        suggestion_for(target.role),
                    )),
                    None => Ok(()),
                }
            }
            Assertion::Text { node, expected } => {
                let Some(target) = view.node(*node) else {
                    return Err(self.failure(assertion, Some(*node), "节点存在", "节点缺失", None));
                };
                if target.label == *expected {
                    Ok(())
                } else {
                    Err(self.failure(
                        assertion,
                        Some(*node),
                        expected,
                        &target.label,
                        suggestion_for(target.role),
                    ))
                }
            }
            Assertion::Bounds { node, expect } => {
                let Some(rect) = view.bounds(*node) else {
                    return Err(self.failure(assertion, Some(*node), "节点存在", "节点缺失", None));
                };
                if let Some((low, high)) = expect.width {
                    if rect.width() < low || rect.width() > high {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("宽度在 [{low}, {high}]"),
                            &format!("宽度 {}", rect.width()),
                            None,
                        ));
                    }
                }
                if let Some((low, high)) = expect.height {
                    if rect.height() < low || rect.height() > high {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("高度在 [{low}, {high}]"),
                            &format!("高度 {}", rect.height()),
                            None,
                        ));
                    }
                }
                if let Some(point) = expect.contains {
                    if !rect.contains_point(point) {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("矩形包含 {point:?}"),
                            &format!("矩形 {rect:?}"),
                            None,
                        ));
                    }
                }
                Ok(())
            }
            Assertion::ColorAt {
                pos,
                expect,
                tolerance,
            } => {
                let Some(probe) = self.probe else {
                    return Err(self.failure(
                        assertion,
                        None,
                        "像素探针已接入",
                        "未接入像素探针",
                        Some("为会话接入 PixelProbe 后方可做像素级断言".to_owned()),
                    ));
                };
                let Some(actual) = probe.color_at(*pos) else {
                    return Err(self.failure(
                        assertion,
                        None,
                        &format!("{pos:?} 处可采样"),
                        "该位置不可采样",
                        None,
                    ));
                };
                if colors_close(*expect, actual, *tolerance) {
                    Ok(())
                } else {
                    Err(self.failure(
                        assertion,
                        None,
                        &format!("{expect:?}"),
                        &format!("{actual:?}"),
                        Some(format!("容差 {tolerance}，可放宽后重试")),
                    ))
                }
            }
        }
    }

    /// 构造结构化失败信息。
    fn failure(
        &self,
        assertion: &Assertion,
        node: Option<u64>,
        expected: &str,
        actual: &str,
        suggestion: Option<String>,
    ) -> AssertionFailure {
        let view = self.view();
        AssertionFailure {
            kind: assertion.name(),
            node,
            node_path: node.map(|id| view.path(id)),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
            suggestion,
        }
    }
}

/// 按角色给出下一步动作建议。
fn suggestion_for(role: Role) -> Option<String> {
    match role {
        Role::Switch | Role::CheckBox => Some("该节点可通过 Toggle 动作切换状态".to_owned()),
        Role::ComboBox => Some("该节点可通过 SelectOption 动作直接选取选项".to_owned()),
        Role::TextInput => Some("该节点可通过 SetText 动作写入内容".to_owned()),
        Role::Button | Role::MenuItem => Some("该节点可通过 Click 动作触发".to_owned()),
        _ => None,
    }
}

/// 判断两个颜色在给定容差内是否一致。
fn colors_close(expect: Color, actual: Color, tolerance: u8) -> bool {
    let tolerance = f32::from(tolerance) / 255.0;
    let diff = |a: f32, b: f32| (a - b).abs() <= tolerance;
    diff(expect.r, actual.r) && diff(expect.g, actual.g) && diff(expect.b, actual.b)
}

/// 便捷构造：只关心「存在且满足谓词」的断言。
#[must_use]
pub fn exists(role: Role, name: &str) -> Assertion {
    Assertion::Exists {
        query: NodeQuery::any().role(role).name(name),
    }
}

/// 便捷构造：状态断言。
#[must_use]
pub fn state(node: u64, predicates: Vec<NodePredicate>) -> Assertion {
    Assertion::State { node, predicates }
}

/// 便捷构造：矩形约束。
#[must_use]
pub fn bounds(node: u64, expect: BoundsExpect) -> Assertion {
    Assertion::Bounds { node, expect }
}

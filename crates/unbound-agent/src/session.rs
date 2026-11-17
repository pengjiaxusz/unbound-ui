//! 会话：把语义视图、合成输入、断言与录制串成一条闭环。

use std::time::{Duration, Instant};

use accesskit::Role;
use unbound_a11y::A11yAction;
use unbound_app::{AppRuntime, Application, FrameReport};
use unbound_input::PointerButton;
use unbound_math::{Color, Vec2};
use unbound_widgets::diff::SceneDiff;

use crate::action::SemanticAction;
use crate::assert::{Assertion, AssertionFailure, BoundsExpect};
use crate::log::{ActionLog, RecordedStep};
use crate::pixel::{CapturedFrame, OffscreenPixelSource, PixelSource};
use crate::query::{NodePredicate, NodeQuery};
use crate::replay::MinimalReproduction;
use crate::snapshot::{SemanticsEntry, SessionSnapshot, SnapshotError};
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
            Self::NodeMissing { node } => format!("node {node} is not in the semantics tree"),
            Self::NoBounds { node } => format!("node {node} has no valid rectangle"),
            Self::NoMatch {
                role,
                name,
                candidates,
            } => {
                let role_text = role.map_or_else(String::new, |role| format!("{:?}", role));
                format!(
                    "no element with role {role_text} and name \"{name}\"; current candidates: {}",
                    describe_candidates(candidates)
                )
            }
            Self::Ambiguous { name, candidates } => format!(
                "multiple elements named \"{name}\" cannot be disambiguated automatically: {}",
                describe_candidates(candidates)
            ),
            Self::OptionMissing {
                node,
                option,
                available,
            } => format!(
                "node {node} has no option \"{option}\"; available options: {}",
                describe_candidates(available)
            ),
            Self::Rejected { action, reason } => format!("action {action} was rejected: {reason}"),
        }
    }
}

/// 候选列表的文本描述。
fn describe_candidates(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "(none)".to_owned()
    } else {
        candidates.join(", ")
    }
}

/// 默认帧步长：每秒 60 帧。
const DEFAULT_FRAME_DT: f32 = 1.0 / 60.0;

/// 回放时单个等待条件的超时上限。
///
/// 回放不该无限期地等：条件达不成就是达不成，等再久也没用。
const REPLAY_WAIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Agent 会话。
///
/// 它**不拥有**运行时，只借用；会话期间对界面的全部操作都走框架自身的
/// 事件管线，不注入任何操作系统级输入，因此**用户可以同时正常使用电脑**。
pub struct AgentSession<'a, A: Application> {
    runtime: &'a mut AppRuntime<A>,
    log: ActionLog,
    probe: Option<&'a mut dyn PixelSource>,
    /// 最近一次采集的画面。
    ///
    /// 离屏采集的结果缓存在这里，像素断言从它取值：断言问的是
    /// 「**上一次采集到的**画面是什么颜色」，而不是「此刻画面是什么颜色」。
    frame: Option<CapturedFrame>,
    /// 帧步长（秒）。
    ///
    /// 固定步长是确定性的前提：一旦掺入真实时间，同一串动作跑两遍就会
    /// 得到不同结果，回归也就无从谈起。
    dt: f32,
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
            frame: None,
            dt: DEFAULT_FRAME_DT,
            record: true,
        }
    }

    /// 建立会话并接入像素源（供像素级断言使用）。
    #[must_use]
    pub fn with_probe(runtime: &'a mut AppRuntime<A>, probe: &'a mut dyn PixelSource) -> Self {
        Self {
            runtime,
            log: ActionLog::new(),
            probe: Some(probe),
            frame: None,
            dt: DEFAULT_FRAME_DT,
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
    /// 帧长默认固定为 1/60 秒：足够让动画与异步结果落地，又不至于让等待条件
    /// 因时基不同而表现不一致。步长可由 [`Self::set_frame_dt`] 调整。
    pub fn advance(&mut self) -> FrameReport {
        self.runtime.frame(self.dt)
    }

    /// 设置帧步长（秒）。
    ///
    /// 非正值与非有限值会被忽略：那样的步长会让动画与计时器永远不推进，
    /// 等待条件也就永远等不到。
    pub fn set_frame_dt(&mut self, dt: f32) {
        if dt > 0.0 && dt.is_finite() {
            self.dt = dt;
        }
    }

    /// 当前帧步长（秒）。
    #[must_use]
    pub fn frame_dt(&self) -> f32 {
        self.dt
    }

    /// 以固定步长推进指定时长。
    pub fn advance_seconds(&mut self, seconds: f32) {
        let steps = (seconds / self.dt).round().max(0.0) as usize;
        self.advance_frames(steps);
    }

    /// 采集会话快照。
    ///
    /// 快照含三部分：应用自己序列化的状态、框架采集的语义树，以及焦点与
    /// 指针位置。应用不提供状态快照时，快照仍然可用——只是无法用来
    /// **恢复**（见 [`Self::restore`]）。
    pub fn snapshot(&self) -> SessionSnapshot {
        let app_state = self
            .runtime
            .application()
            .snapshot_state(self.runtime.state());
        let view = self.view();
        let order = view.tree_order();
        let mut semantics = Vec::with_capacity(order.len());
        for id in order {
            let Some(node) = view.node(id) else {
                continue;
            };
            semantics.push(SemanticsEntry {
                id,
                node: node.clone(),
                children: view.semantics().children_of(id).to_vec(),
            });
        }
        SessionSnapshot {
            app_state,
            focus: self.runtime.focus().focused(),
            semantics,
            pointer: self.runtime.context().pointer,
        }
    }

    /// 恢复会话快照。
    ///
    /// 顺序是**状态 → 焦点 → 指针 → 重建**：焦点节点可能随状态一起消失，
    /// 指针的悬停事件也应当作用在已恢复的状态上。最后推进一帧，
    /// 让语义树与像素跟随状态一起回到原样。
    ///
    /// 快照可反复恢复：回放与最小复现剪枝正是靠这一点。
    pub fn restore(&mut self, snapshot: &SessionSnapshot) -> Result<(), SnapshotError> {
        let Some(app_state) = snapshot.app_state.as_ref() else {
            return Err(SnapshotError::StateUnavailable);
        };
        {
            let (app, state) = self.runtime.application_and_state_mut();
            app.restore_state(state, app_state.as_ref());
        }
        self.runtime.focus_node(snapshot.focus);
        self.runtime.pointer_move(snapshot.pointer);
        self.advance();
        Ok(())
    }

    /// 回放一段已录制的动作序列。
    ///
    /// 回放期间**不再录制**：否则日志会自己套自己地长。返回实际执行的
    /// 步数；遇到失败即停，并把失败原因作为 [`ActionError::Rejected`] 抛出。
    pub fn replay(&mut self, steps: &[RecordedStep]) -> Result<usize, ActionError> {
        let was_recording = self.record;
        self.record = false;
        let mut executed = 0usize;
        let outcome = (|| -> Result<(), ActionError> {
            for step in steps {
                match step {
                    RecordedStep::Action(action) => self.perform(action)?,
                    RecordedStep::Wait(condition) => {
                        self.wait_for(condition, REPLAY_WAIT_TIMEOUT)
                            .map_err(|timeout| ActionError::Rejected {
                                action: condition.name().to_owned(),
                                reason: timeout.describe(),
                            })?;
                    }
                    RecordedStep::Assert(assertion) => {
                        self.assert(assertion)
                            .map_err(|failure| ActionError::Rejected {
                                action: assertion.name().to_owned(),
                                reason: failure.describe(),
                            })?;
                    }
                    // 标记只是给人看的注记，回放时跳过。
                    RecordedStep::Marker(_) => {}
                }
                executed += 1;
            }
            Ok(())
        })();
        self.record = was_recording;
        outcome.map(|()| executed)
    }

    /// 从基线出发回放，并剪出最小失败复现。
    ///
    /// `fails` 判定当前会话**是否仍处于失败状态**：它在每次候选回放之后
    /// 被调用一次。剪枝的每一步判定都会先恢复到 `baseline`——否则结果只对
    /// 那一次有效，剪出来的序列换个起点就复现不了。
    ///
    /// 返回最小失败子序列与它的可读脚本。
    pub fn minimize_failure<F>(
        &mut self,
        baseline: &SessionSnapshot,
        steps: &[RecordedStep],
        mut fails: F,
    ) -> MinimalReproduction
    where
        F: FnMut(&AgentSession<'_, A>) -> bool,
    {
        let mut probe = |candidate: &[RecordedStep]| -> bool {
            if self.restore(baseline).is_err() {
                return false;
            }
            // 回放本身出错也算「仍能复现问题」——出错正是要抓的东西。
            if self.replay(candidate).is_err() {
                return true;
            }
            fails(&*self)
        };
        let minimal = crate::replay::minimize_failing_steps(steps, &mut probe);
        MinimalReproduction {
            script: crate::replay::script_of(&minimal),
            steps: minimal,
        }
    }

    /// 推进若干帧。
    pub fn advance_frames(&mut self, frames: usize) {
        for _ in 0..frames {
            self.advance();
        }
    }

    /// 采集当前画面。
    ///
    /// 使用会话接入的像素源；未接入时返回 `None`。
    pub fn capture(&mut self) -> Option<&CapturedFrame> {
        let probe = self.probe.as_mut()?;
        let frame = probe.capture()?;
        self.frame = Some(frame.clone());
        self.frame.as_ref()
    }

    /// 用离屏渲染采集当前画面。
    ///
    /// 场景、主题与字形图集全部由运行时提供：调用方不必关心渲染细节，
    /// 也不必自备像素源。画面被渲染到独立目标再回读，
    /// **不占屏幕、不抢鼠标**。
    ///
    /// 采集前会先推进一帧，让本轮的状态变化进入场景。
    pub fn capture_offscreen<'s>(
        &mut self,
        source: &'s mut OffscreenPixelSource,
    ) -> Option<&'s CapturedFrame> {
        let report = self.advance();
        let ctx = self.runtime.context().clone();
        source.sync(&ctx, &report.atlas_uploads).ok()?;
        source.render(self.runtime.scene()).ok()?;
        let frame = source.capture()?;
        self.frame = Some(frame.clone());
        Some(frame)
    }

    /// 采样一个逻辑坐标处的像素颜色。
    ///
    /// 优先取接入的像素源；像素源没有可用画面时退回最近一次采集的结果。
    fn sample_pixel(&self, pos: Vec2) -> Option<Color> {
        if let Some(probe) = self.probe.as_ref() {
            if let Some(color) = probe.color_at(pos) {
                return Some(color);
            }
        }
        self.frame.as_ref()?.color_at(pos)
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
    ///
    /// 录制由各个动作方法自己负责：直接调用 [`Self::click`] 与经由本方法
    /// 执行，录下来的东西必须一模一样。
    pub fn perform(&mut self, action: &SemanticAction) -> Result<(), ActionError> {
        match action {
            SemanticAction::Click { node } => self.click(*node),
            SemanticAction::Focus { node } => self.focus(*node),
            SemanticAction::Toggle { node } => self.toggle(*node),
            SemanticAction::SetText { node, text } => self.set_text(*node, text),
            SemanticAction::SelectOption { node, option } => self.select_option(*node, option),
            SemanticAction::SetValue { node, value } => self.set_value(*node, *value),
            SemanticAction::Expand { node, expanded } => self.set_expanded(*node, *expanded),
            SemanticAction::ScrollIntoView { node } => self.scroll_into_view(*node),
            SemanticAction::Scroll { node, delta } => self.scroll(*node, *delta),
        }
    }

    /// 记录一个动作。
    fn record_action(&mut self, action: SemanticAction) {
        if self.record {
            self.log.push(RecordedStep::Action(action));
        }
    }

    /// 点击节点。
    pub fn click(&mut self, node: u64) -> Result<(), ActionError> {
        self.click_internal(node)?;
        self.record_action(SemanticAction::Click { node });
        Ok(())
    }

    /// 执行点击本身，不记录。
    fn click_internal(&mut self, node: u64) -> Result<(), ActionError> {
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
        self.record_action(SemanticAction::Focus { node });
        Ok(())
    }

    /// 切换开关 / 复选框 / 药丸按钮的选中态。
    pub fn toggle(&mut self, node: u64) -> Result<(), ActionError> {
        self.click_internal(node)?;
        self.record_action(SemanticAction::Toggle { node });
        Ok(())
    }

    /// 设置文本输入框的内容。
    pub fn set_text(&mut self, node: u64, text: &str) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.focus_node(Some(node));
        self.runtime.text(text);
        self.advance();
        self.record_action(SemanticAction::SetText {
            node,
            text: text.to_owned(),
        });
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
            self.click_internal(option_node)?;
        } else {
            // 选项尚未出现：展开下拉框后重试。
            self.click_internal(node)?;
            self.settle(12);
            if let Some(option_node) = self.find_option(node, option) {
                self.click_internal(option_node)?;
            } else {
                return Err(ActionError::OptionMissing {
                    node,
                    option: option.to_owned(),
                    available: self.available_options(node),
                });
            }
        }
        self.record_action(SemanticAction::SelectOption {
            node,
            option: option.to_owned(),
        });
        Ok(())
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
        self.runtime
            .a11y(node, A11yAction::SetValue, Some(value.to_string()));
        self.advance();
        self.record_action(SemanticAction::SetValue { node, value });
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
        self.record_action(SemanticAction::Expand { node, expanded });
        Ok(())
    }

    /// 滚动到节点可见。
    pub fn scroll_into_view(&mut self, node: u64) -> Result<(), ActionError> {
        if self.runtime.semantics().get(node).is_none() {
            return Err(ActionError::NodeMissing { node });
        }
        self.runtime.a11y(node, A11yAction::ScrollIntoView, None);
        self.advance();
        self.record_action(SemanticAction::ScrollIntoView { node });
        Ok(())
    }

    /// 滚动指定节点。
    pub fn scroll(&mut self, node: u64, delta: Vec2) -> Result<(), ActionError> {
        let pos = self.center_of(node)?;
        self.runtime.wheel(pos, delta);
        self.advance();
        self.record_action(SemanticAction::Scroll { node, delta });
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
    ///
    /// 失败类型装箱：诊断信息里带着完整的等待条件（含查询条件），
    /// 直接按值返回会让 `Result` 的错误分支背上上百字节的栈拷贝。
    /// 装箱后调用方依然能直接 `timeout.observed` 这样访问字段。
    pub fn wait_for(
        &mut self,
        condition: &WaitCondition,
        timeout: Duration,
    ) -> Result<(), Box<WaitTimeout>> {
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
                return Err(Box::new(WaitTimeout {
                    condition: condition.clone(),
                    waited_ms: start.elapsed().as_millis() as u64,
                    observed,
                }));
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
    ///
    /// 失败类型装箱，理由同 [`Self::wait_for`]：诊断信息是给人和 Agent 读的，
    /// 不值得为它在成功路径上留一份几百字节的栈空间。
    pub fn assert(&self, assertion: &Assertion) -> Result<(), Box<AssertionFailure>> {
        let view = self.view();
        match assertion {
            Assertion::Exists { query } => {
                let hits = view.query(query);
                if hits.is_empty() {
                    Err(self.failure(assertion, None, "at least one matching node", "none", None))
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
                        "no matching node",
                        &format!("{} matching node(s)", hits.len()),
                        None,
                    ))
                }
            }
            Assertion::State { node, predicates } => {
                let Some(target) = view.node(*node) else {
                    return Err(self.failure(
                        assertion,
                        Some(*node),
                        "node exists",
                        "node missing",
                        None,
                    ));
                };
                let failed = predicates
                    .iter()
                    .find(|predicate| !predicate.matches(target, *node == view.focus()));
                match failed {
                    Some(predicate) => Err(self.failure(
                        assertion,
                        Some(*node),
                        &format!("satisfies {}", predicate.name()),
                        "not satisfied",
                        suggestion_for(target.role),
                    )),
                    None => Ok(()),
                }
            }
            Assertion::Text { node, expected } => {
                let Some(target) = view.node(*node) else {
                    return Err(self.failure(
                        assertion,
                        Some(*node),
                        "node exists",
                        "node missing",
                        None,
                    ));
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
                    return Err(self.failure(
                        assertion,
                        Some(*node),
                        "node exists",
                        "node missing",
                        None,
                    ));
                };
                if let Some((low, high)) = expect.width {
                    if rect.width() < low || rect.width() > high {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("width within [{low}, {high}]"),
                            &format!("width {}", rect.width()),
                            None,
                        ));
                    }
                }
                if let Some((low, high)) = expect.height {
                    if rect.height() < low || rect.height() > high {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("height within [{low}, {high}]"),
                            &format!("height {}", rect.height()),
                            None,
                        ));
                    }
                }
                if let Some(point) = expect.contains {
                    if !rect.contains_point(point) {
                        return Err(self.failure(
                            assertion,
                            Some(*node),
                            &format!("rect contains {point:?}"),
                            &format!("rect {rect:?}"),
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
                let Some(actual) = self.sample_pixel(*pos) else {
                    return Err(self.failure(
                        assertion,
                        None,
                        "a pixel sample is available",
                        "no pixel sample is available",
                        Some(
                            "attach a PixelSource to the session and capture a frame before asserting on pixels"
                                .to_owned(),
                        ),
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
                        Some(format!("tolerance is {tolerance}; widen it and retry")),
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
    ) -> Box<AssertionFailure> {
        let view = self.view();
        Box::new(AssertionFailure {
            kind: assertion.name(),
            node,
            node_path: node.map(|id| view.path(id)),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
            suggestion,
        })
    }
}

/// 按角色给出下一步动作建议。
fn suggestion_for(role: Role) -> Option<String> {
    match role {
        Role::Switch | Role::CheckBox => {
            Some("this node can be toggled with the Toggle action".to_owned())
        }
        Role::ComboBox => {
            Some("this node can pick an option directly with the SelectOption action".to_owned())
        }
        Role::TextInput => Some("this node can be filled in with the SetText action".to_owned()),
        Role::Button | Role::MenuItem => {
            Some("this node can be triggered with the Click action".to_owned())
        }
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

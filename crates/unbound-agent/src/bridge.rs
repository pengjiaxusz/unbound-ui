//! 跨线程 Agent 桥：让后台线程安全地驱动界面，并取回结构化结果。
//!
//! 界面的状态住在 UI 线程的 [`unbound_app::AppRuntime`] 里；Agent 却通常
//! 从 HTTP、命名管道或测试进程发来指令。若让它们直接碰运行时，就得把
//! `Send`/`Sync` 的约束一路传染到整个渲染栈——得不偿失。
//!
//! 本模块提供另一条路：**请求排队，UI 线程自己取**。
//!
//! * 后台线程把一条 [`AgentRequest`] 投进 [`AgentBridge`]，然后等回执；
//! * UI 线程在每帧开头调用 [`AgentBridge::drain`]，逐条执行并回传结果；
//! * 界面状态**从未离开 UI 线程**，跨线程的只有纯值数据。
//!
//! 这样做的直接好处是：合成输入不抢真实鼠标，不影响用户操作电脑；
//! 而 Agent 拿到的仍是**完整颗粒度**的语义信息，不是截图。

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use unbound_app::AppRuntime;
use unbound_math::Vec2;

use crate::action::SemanticAction;
use crate::assert::Assertion;
use crate::pixel::PixelSource;
use crate::session::{ActionError, AgentSession};
use crate::view::TreeSnapshot;

/// 一次 Agent 请求。
///
/// 这些类型都是**纯值**：它们跨线程时不需要任何引擎内部类型的引用。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRequest {
    /// 读取当前界面的完整语义树。
    ///
    /// 一次返回整棵树，而不是逐节点查询——语义树有数百节点，逐条问就是
    /// 数百次往返，那不叫好用。
    QueryTree,
    /// 执行一个语义动作。
    Perform(SemanticAction),
    /// 执行一条断言。
    Assert(Assertion),
    /// 读取指定位置的像素颜色。
    ProbeColor(Vec2),
}

/// 一次 Agent 请求的结果。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentReply {
    /// 语义树快照。
    Tree(TreeSnapshot),
    /// 动作执行成功。
    Done,
    /// 断言通过。
    Passed,
    /// 断言失败（携带结构化诊断）。
    Failed(String),
    /// 采样到的像素颜色。
    Color(Option<[u8; 4]>),
    /// 请求失败。
    Error(String),
}

impl AgentReply {
    /// 是否是失败结果。
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Error(_))
    }
}

/// 一条待处理的请求及其回执通道。
struct Pending {
    /// 请求内容。
    request: AgentRequest,
    /// 回执发送端。
    reply: Sender<AgentReply>,
}

/// 跨线程 Agent 桥。
///
/// 它分成两半，因为两端的能力本就不同：
///
/// * [`AgentBridge`] 是**发送端**，可随意克隆并交给后台线程；
/// * [`AgentBridgeHandle`] 是**接收端**，只留在 UI 线程。
///
/// 这样「谁能投递请求」与「谁能执行请求」在类型上就分开了——后台线程
/// 拿得到句柄也没用，它根本排不空队列。
#[derive(Clone)]
pub struct AgentBridge {
    /// 请求投递端。
    inbox: Sender<Pending>,
}

/// UI 线程侧的接收句柄。
pub struct AgentBridgeHandle {
    /// 待处理请求的接收端。
    pending: Receiver<Pending>,
}

impl AgentBridge {
    /// 建立桥，返回发送端与 UI 线程侧的接收句柄。
    #[must_use]
    pub fn new() -> (Self, AgentBridgeHandle) {
        let (inbox, pending) = channel();
        (Self { inbox }, AgentBridgeHandle { pending })
    }

    /// 投递一条请求并等待结果。
    ///
    /// 超时返回 [`AgentReply::Error`]：UI 线程可能正忙，也可能已经退出。
    /// 等待本身**不阻塞 UI 线程**——回执是在 UI 线程排空请求时发出的。
    pub fn request(&self, request: AgentRequest, timeout: Duration) -> AgentReply {
        let (reply, wait) = channel();
        if self.inbox.send(Pending { request, reply }).is_err() {
            return AgentReply::Error("the UI thread is no longer accepting requests".to_owned());
        }
        wait.recv_timeout(timeout).unwrap_or_else(|_| {
            AgentReply::Error(format!("the request timed out after {timeout:?}"))
        })
    }

    /// 投递一条请求并等待结果（使用默认超时）。
    pub fn request_default(&self, request: AgentRequest) -> AgentReply {
        self.request(request, DEFAULT_TIMEOUT)
    }
}

impl AgentBridgeHandle {
    /// 排空待处理请求并逐条执行。
    ///
    /// **只应由 UI 线程调用**：它在运行时上做输入注入与断言求值。
    pub fn drain<A: unbound_app::Application>(
        &self,
        runtime: &mut AppRuntime<A>,
        mut handle: impl FnMut(&mut AgentSession<'_, A>, AgentRequest) -> AgentReply,
    ) {
        while let Ok(item) = self.pending.try_recv() {
            let Pending { request, reply } = item;
            let mut session = AgentSession::new(runtime);
            let outcome = handle(&mut session, request);
            let _ = reply.send(outcome);
        }
    }

    /// 排空待处理请求，用框架内置的语义执行。
    ///
    /// 这是最常用的路径：查询树、执行动作、执行断言都由框架完成，
    /// 调用方不必自己写任何求值逻辑。
    ///
    /// 传入像素源即可同时支持像素级断言与采样；不传则像素相关请求
    /// 直接回「无法采样」，而不是让整条链路失败。
    pub fn drain_builtin<A: unbound_app::Application>(
        &self,
        runtime: &mut AppRuntime<A>,
        mut probe: Option<&mut dyn PixelSource>,
    ) {
        self.drain(runtime, |session, request| match request {
            AgentRequest::QueryTree => AgentReply::Tree(session.view().to_tree()),
            AgentRequest::Perform(action) => match session.perform(&action) {
                Ok(()) => AgentReply::Done,
                Err(error) => AgentReply::Error(describe_action_error(&error)),
            },
            AgentRequest::Assert(assertion) => match session.assert(&assertion) {
                Ok(()) => AgentReply::Passed,
                Err(failure) => AgentReply::Failed(failure.describe()),
            },
            AgentRequest::ProbeColor(pos) => {
                let color = probe
                    .as_deref_mut()
                    .and_then(|source| source.color_at(pos))
                    .or_else(|| session.probe_color(pos));
                AgentReply::Color(color.map(to_srgb8))
            }
        });
    }
}

/// 默认请求超时。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// 把动作错误转成一句可读的诊断。
fn describe_action_error(error: &ActionError) -> String {
    error.describe()
}

/// 把线性颜色转成 sRGB 字节。
///
/// 框架内部一律用线性浮点，颜色对不上时没人能一眼看出差在哪；
/// Agent 拿到的应当是**显示出来的**颜色，也就是 sRGB 字节。
#[must_use]
pub fn srgb8(color: unbound_math::Color) -> [u8; 4] {
    let [r, g, b, a] = color.to_array();
    [
        (unbound_math::color::linear_to_srgb(r).clamp(0.0, 1.0) * 255.0).round() as u8,
        (unbound_math::color::linear_to_srgb(g).clamp(0.0, 1.0) * 255.0).round() as u8,
        (unbound_math::color::linear_to_srgb(b).clamp(0.0, 1.0) * 255.0).round() as u8,
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// 把线性颜色转成 sRGB 字节（内部别名）。
fn to_srgb8(color: unbound_math::Color) -> [u8; 4] {
    srgb8(color)
}

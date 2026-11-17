//! Agent 原生的界面视图：感知、寻址、操作、验证、复现。
//!
//! 这个 crate **不创造第二套语义**。它看到的就是
//! [`unbound_a11y::Semantics`] 那棵树——同一棵被屏幕阅读器、被 UI Automation
//! 消费的树。差别只在于**怎么用**它：
//!
//! * 屏幕阅读器把它念给人听；
//! * UI Automation 把它交给桌面自动化栈；
//! * 这里把它交给 Agent —— 按角色与名称定位、按语义动作操作、按断言验证。
//!
//! 因此本 crate 里不会出现任何业务词汇（「设置」「缩放」之类）：框架层只知道
//! 角色、名称、状态与几何。业务语义由应用在构建视图时通过
//! [`unbound_app::ViewBuilder::declare_semantics`] 注入。
//!
//! 三条不可让步的约定：
//!
//! * **禁止坐标契约**：对外的动作接口只接受节点标识，坐标是内部实现细节
//!   （由节点矩形算出中心点后注入合成事件）。
//! * **禁止操作系统级输入注入**：只走框架自身的事件管线，不碰系统光标，
//!   因此 Agent 操作时用户仍可正常使用电脑。
//! * **禁止第二套语义**：Agent 视图与无障碍视图恒等。

pub mod action;
pub mod assert;
pub mod log;
pub mod query;
pub mod session;
pub mod view;
pub mod wait;

pub use action::SemanticAction;
pub use assert::{Assertion, AssertionFailure, BoundsExpect};
pub use log::{ActionLog, RecordedStep};
pub use query::{NodePredicate, NodeQuery, SpatialRelation};
pub use session::{ActionError, AgentSession};
pub use view::{AgentView, NodeSnapshot, TreeSnapshot};
pub use wait::{WaitCondition, WaitTimeout};

pub use accesskit::Role;
pub use unbound_a11y::{A11yAction, A11yNode, Semantics};

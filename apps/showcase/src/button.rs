//! 第一个可交互控件：扁平按钮。
//!
//! 控件本身已经迁移到引擎层的 [`unbound_widgets::button`]：那里有描述符、
//! 视觉状态与纯函数发射器，还有五种植变（Primary / Secondary / Subtle /
//! Icon / OnHero）。本模块只保留展台需要的再导出与一处便利函数，公开 API
//! 与迁移前保持一致。
//!
//! 交互瞬态（hover / pressed）不落任何 `bool` 字段，统一从
//! [`Router`] 的捕获状态派生；因此"按下后拖出边界再松开"这种经典场景不会
//! 出现两套状态彼此失步。组件身份来自 `unbound-world` 的 [`NodeId`]。

pub use unbound_widgets::button::{
    emit_button, emit_button_with_tokens, ink_bounds, insert_button, node_u64, ButtonDesc,
    ButtonTokens, ButtonVariant, ButtonWidget, BUTTON_FONT_SIZE, BUTTON_LAYER, BUTTON_RADIUS,
};
pub use unbound_widgets::{ControlState, PointerPhase};

use unbound_input::Router;
use unbound_world::NodeId;

/// 一帧的按钮视觉状态。
///
/// `focused` 与 `phase` 正交：通过 Tab 聚焦的按钮在鼠标悬停时必须同时呈现
/// hover 背景与外部焦点环，所以它们不能塞进同一个互斥枚举。
pub type ButtonVisualState = ControlState;

/// 从指针路由器与焦点槽位派生按钮的视觉状态。
///
/// 按下后指针被 [`Router`] 捕获，`pressed()` 会一直指向按钮，因此拖出
/// 边界仍显示 pressed，与原生按钮一致；松开或被取消后自动回落。
#[must_use]
pub fn resolve_state(
    node: NodeId,
    router: &Router,
    focused: Option<NodeId>,
    disabled: bool,
) -> ButtonVisualState {
    ButtonVisualState::resolve(
        node_u64(node),
        router,
        focused.map(node_u64),
        disabled,
        false,
    )
}

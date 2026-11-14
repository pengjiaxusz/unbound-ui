//! 将 UBMF guest 托管为常规 [`Application`]。
//!
//! guest 在 C-ABI 后方持有自身状态；host 持有 guest 每次构建视图的节点 arena。
//! 本适配器即二者间的桥梁：向 guest 请求树结构，将每个节点翻译为运行时认识的
//! 声明式控件，并在这些控件被点击时通过 [`GuestHost::reduce`] 回传绑定的动作。
//!
//!
//!
//! 翻译过程刻意保持机械化。节点的 [`NodeTag`] 决定控件类型：带有绑定动作的
//! `Text` 叶节点变为按钮（guest 正是借此表达“该行可点击”），其他 `Text` 叶节点
//! 变为段落，带子节点的节点则变为继承 guest 自身布局描述符的容器。
//! `Scroll`、`Portal` 与 `Canvas` 在 host 尚未提供专用控件前保持其容器形态。
//!
//!

use rustc_hash::FxHashMap;

use unbound_abi::action::RawAction;
use unbound_abi::desc::{NodeTag, RawNodeDesc, RawNodeId};
use unbound_host::{GuestHost, NodeArena};
use unbound_math::Rect;
use unbound_text::TextStyle;
use unbound_widgets::{ButtonDesc, ButtonVariant};

use crate::{AppContext, AppEvent, Application, DirtyFlags, EventOutcome, ViewBuilder, WidgetSpec};

/// 围绕已挂载 guest 的应用封装。
pub struct GuestAppAdapter {
    host: GuestHost,
    /// 每个控件键所绑定的动作，每次视图重建时同步更新。
    actions: FxHashMap<u64, RawAction>,
}

impl GuestAppAdapter {
    /// 封装已挂载的 host。
    ///
    /// host 可以为空；此时 [`Application::view`] 会绘制简短诊断信息而非失败。
    ///
    #[must_use]
    pub fn new(host: GuestHost) -> Self {
        Self {
            host,
            actions: FxHashMap::default(),
        }
    }

    /// 获取已挂载的 host。
    #[must_use]
    pub fn host(&self) -> &GuestHost {
        &self.host
    }

    /// 可变访问，用于挂载或热重载虚表。
    pub fn host_mut(&mut self) -> &mut GuestHost {
        &mut self.host
    }

    /// 获取某个控件键上一次视图构建时绑定的动作（如有）。
    #[must_use]
    pub fn action_for(&self, key: u64) -> Option<&RawAction> {
        self.actions.get(&key)
    }
}

impl Application for GuestAppAdapter {
    /// guest 持有业务状态；host 侧状态为单位类型 `()`。
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let AppEvent::Click { node, .. } = event else {
            return EventOutcome::default();
        };
        let Some(action) = self.actions.get(&node).copied() else {
            return EventOutcome::default();
        };
        // 拒绝某动作的 guest 保留旧视图；下一帧原样重绘即可。
        //
        if self.host.reduce(&action).is_err() {
            return EventOutcome::default();
        }
        EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::A11Y)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        // guest 基于其（可能刚被 reduce 更新的）状态重建 arena。
        let built = self.host.build().is_ok();
        self.actions.clear();
        let arena = self.host.arena();
        if !built || arena.is_empty() {
            view.text(1, "guest not mounted", TextStyle::sized(16.0));
            return;
        }
        if let Some(root) = arena
            .finished_root()
            .or_else(|| arena.roots().first().copied())
        {
            emit_node(arena, root, view, &mut self.actions);
        }
    }
}

/// 翻译单个 arena 节点，并递归翻译其子节点。
fn emit_node(
    arena: &NodeArena,
    id: RawNodeId,
    view: &mut ViewBuilder<'_>,
    actions: &mut FxHashMap<u64, RawAction>,
) {
    let Some(node) = arena.get(id) else {
        return;
    };
    // ID 从 1 开始，因此 0 留空表示“无节点”。
    let key = u64::from(node.id.0);
    if let Some(action) = node.action {
        actions.insert(key, action);
    }
    let desc: RawNodeDesc = node.desc;
    let tag = NodeTag::from_u8(desc.tag);
    if tag == NodeTag::Text && node.children.is_empty() {
        if node.action.is_some() {
            let mut button = ButtonDesc::new(Rect::new(0.0, 0.0, 320.0, 28.0), &node.text);
            button.variant = ButtonVariant::Secondary;
            view.widget(key, desc, WidgetSpec::Button(button));
        } else {
            let size = if desc.font_size > 0.0 {
                desc.font_size
            } else {
                14.0
            };
            view.text(key, node.text.clone(), TextStyle::sized(size));
        }
        return;
    }
    // 容器原样保留 guest 的方向、间距与尺寸约束：底层布局引擎与 host UI 一致。
    //
    let children = node.children.clone();
    view.container(key, desc, |inner| {
        for child in children {
            emit_node(arena, child, inner, actions);
        }
    });
}

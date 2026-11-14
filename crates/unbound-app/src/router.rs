//! 类型安全的视图路由栈与面包屑同步（S5.4）。

use unbound_widgets::breadcrumb::Crumb;

/// 当前路由过渡动画的方向。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TransitionDirection {
    /// 当前无过渡动画播放。
    #[default]
    None,
    /// 推入或向前导航。
    Forward,
    /// 弹出或向后导航。
    Backward,
}

/// [`RouterStack`] 上的单个路由条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteEntry {
    /// 稳定的路由标识。
    pub id: u64,
    /// 路由名称 / 路径段（例如 `"settings"` 或 `"users/:id"`）。
    pub name: String,
    /// 面包屑中展示的可读标签。
    pub label: String,
    /// 键值对路由参数。
    pub params: Vec<(String, String)>,
}

impl RouteEntry {
    /// 创建新路由条目。
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            label: label.into(),
            params: Vec::new(),
        }
    }

    /// 附加一个路由参数。
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    /// 按键查找路由参数。
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// 基于历史记录的视图导航栈，支持前进/后退、过渡进度与自动生成 [`BreadcrumbItem`]。
///
#[derive(Clone, Debug)]
pub struct RouterStack {
    back_stack: Vec<RouteEntry>,
    forward_stack: Vec<RouteEntry>,
    transition_direction: TransitionDirection,
    transition_progress: f32,
    transition_duration: f32,
}

impl RouterStack {
    /// 使用初始根路由创建路由器。
    #[must_use]
    pub fn new(root: RouteEntry) -> Self {
        Self {
            back_stack: vec![root],
            forward_stack: Vec::new(),
            transition_direction: TransitionDirection::None,
            transition_progress: 1.0,
            transition_duration: 0.22,
        }
    }

    /// 当前激活路由（`back_stack` 栈顶）。
    #[must_use]
    pub fn current(&self) -> &RouteEntry {
        self.back_stack
            .last()
            .expect("RouterStack always contains at least the root route")
    }

    /// 从根路由到当前路由的完整激活面包屑路径。
    #[must_use]
    pub fn history(&self) -> &[RouteEntry] {
        &self.back_stack
    }

    /// 可供 [`Self::forward`] 前进的路由列表。
    #[must_use]
    pub fn forward_history(&self) -> &[RouteEntry] {
        &self.forward_stack
    }

    /// 推入新路由并清空前进栈。
    pub fn push(&mut self, entry: RouteEntry) {
        self.back_stack.push(entry);
        self.forward_stack.clear();
        self.transition_direction = TransitionDirection::Forward;
        self.transition_progress = 0.0;
    }

    /// 替换当前栈顶路由而不增加历史记录。
    pub fn replace(&mut self, entry: RouteEntry) {
        if let Some(last) = self.back_stack.last_mut() {
            *last = entry;
        } else {
            self.back_stack.push(entry);
        }
        self.transition_direction = TransitionDirection::Forward;
        self.transition_progress = 0.0;
    }

    /// 将当前路由弹出至前进栈（永不弹出根路由）。
    pub fn pop(&mut self) -> bool {
        if self.back_stack.len() <= 1 {
            return false;
        }
        if let Some(popped) = self.back_stack.pop() {
            self.forward_stack.push(popped);
            self.transition_direction = TransitionDirection::Backward;
            self.transition_progress = 0.0;
            true
        } else {
            false
        }
    }

    /// 向前导航至最近弹出的路由。
    pub fn forward(&mut self) -> bool {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(next);
            self.transition_direction = TransitionDirection::Forward;
            self.transition_progress = 0.0;
            true
        } else {
            false
        }
    }

    /// 回退至历史路径中的指定路由索引（例如点击面包屑）。
    pub fn pop_to_index(&mut self, index: usize) -> bool {
        if index >= self.back_stack.len().saturating_sub(1) {
            return false;
        }
        while self.back_stack.len() - 1 > index {
            if let Some(popped) = self.back_stack.pop() {
                self.forward_stack.push(popped);
            }
        }
        self.transition_direction = TransitionDirection::Backward;
        self.transition_progress = 0.0;
        true
    }

    /// [`Self::pop`] 是否可后退。
    #[must_use]
    pub fn can_pop(&self) -> bool {
        self.back_stack.len() > 1
    }

    /// [`Self::forward`] 是否可前进。
    #[must_use]
    pub fn can_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    /// 当前过渡方向及归一化 `[0.0, 1.0]` 进度。
    #[must_use]
    pub fn transition(&self) -> (TransitionDirection, f32) {
        (self.transition_direction, self.transition_progress)
    }

    /// 按 `dt` 秒推进路由过渡动画。
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.transition_direction == TransitionDirection::None {
            return false;
        }
        let dur = self.transition_duration.max(0.001);
        self.transition_progress = (self.transition_progress + dt.max(0.0) / dur).min(1.0);
        if self.transition_progress >= 1.0 {
            self.transition_direction = TransitionDirection::None;
        }
        true
    }

    /// 生成与当前路由栈同步的 [`Crumb`] 列表。
    #[must_use]
    pub fn breadcrumb_items(&self) -> Vec<Crumb> {
        self.back_stack
            .iter()
            .map(|entry| Crumb::new(entry.id, entry.label.clone()))
            .collect()
    }
}

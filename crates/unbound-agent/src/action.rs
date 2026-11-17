//! 语义动作：Agent 用「意图」而不是「坐标」来操作界面。

use unbound_math::Vec2;

/// 语义动作。
///
/// 动作只接受节点标识。坐标由框架从节点矩形算出中心点后注入合成事件，
/// 因此**对外不存在坐标契约**——Agent 不需要（也不应当）知道元素在屏幕上的位置。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticAction {
    /// 点击节点。
    Click {
        /// 目标节点。
        node: u64,
    },
    /// 让节点获得焦点。
    Focus {
        /// 目标节点。
        node: u64,
    },
    /// 切换开关 / 复选框 / 药丸按钮的选中态。
    Toggle {
        /// 目标节点。
        node: u64,
    },
    /// 设置文本输入框的内容。
    SetText {
        /// 目标节点。
        node: u64,
        /// 新内容。
        text: String,
    },
    /// 选择下拉框的某一项。
    ///
    /// 这一步对 Agent 而言是**一步到位**的：框架会先看选项是否已经出现在
    /// 语义树里，若没有则展开下拉框、等选项浮现、再选中它。Agent 不必
    /// 自己编排「展开 → 点选 → 等关闭」三步。
    SelectOption {
        /// 下拉框节点。
        node: u64,
        /// 选项文本。
        option: String,
    },
    /// 设置滑块 / 数值输入的值。
    SetValue {
        /// 目标节点。
        node: u64,
        /// 新值。
        value: f32,
    },
    /// 展开或收起可展开节点。
    Expand {
        /// 目标节点。
        node: u64,
        /// 目标状态。
        expanded: bool,
    },
    /// 滚动到节点可见。
    ScrollIntoView {
        /// 目标节点。
        node: u64,
    },
    /// 滚动指定节点。
    Scroll {
        /// 目标节点。
        node: u64,
        /// 滚动量。
        delta: Vec2,
    },
}

impl SemanticAction {
    /// 动作作用的目标节点。
    #[must_use]
    pub fn target(&self) -> u64 {
        match self {
            Self::Click { node }
            | Self::Focus { node }
            | Self::Toggle { node }
            | Self::SetText { node, .. }
            | Self::SelectOption { node, .. }
            | Self::SetValue { node, .. }
            | Self::Expand { node, .. }
            | Self::ScrollIntoView { node }
            | Self::Scroll { node, .. } => *node,
        }
    }

    /// 动作的简短名称，用于录制日志与诊断输出。
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Click { .. } => "click",
            Self::Focus { .. } => "focus",
            Self::Toggle { .. } => "toggle",
            Self::SetText { .. } => "set-text",
            Self::SelectOption { .. } => "select-option",
            Self::SetValue { .. } => "set-value",
            Self::Expand { .. } => "expand",
            Self::ScrollIntoView { .. } => "scroll-into-view",
            Self::Scroll { .. } => "scroll",
        }
    }
}

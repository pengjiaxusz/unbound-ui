//! 快照与恢复：把界面带回探索之前的样子。
//!
//! Agent 探索界面时会顺手点一点、翻一翻。等它想「回到刚才那个状态再看看
//! 别的分支」时，就需要一个能回到原样的快照。
//!
//! 快照记的是**两样**东西，缺一不可：
//!
//! * **应用状态**——由应用自己序列化（见 [`unbound_app::Application::snapshot_state`]）。
//!   框架不关心状态的形状：它可能是计数器、页码，也可能是整个文档。
//! * **界面现状**——语义树、焦点与指针位置，由框架采集。语义树是应用状态的
//!   派生物，恢复状态后重建一帧它自然跟上；但把它一并记下，才能**验证**
//!   恢复是否真的到位。

use std::any::Any;
use std::sync::Arc;

use unbound_a11y::A11yNode;
use unbound_math::Vec2;

/// 语义树的一个条目。
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticsEntry {
    /// 节点标识。
    pub id: u64,
    /// 节点数据。
    pub node: A11yNode,
    /// 子节点标识。
    pub children: Vec<u64>,
}

/// 会话快照。
///
/// 快照是**可重复使用**的：应用状态那一份用 `Arc` 包裹，同一个基线可以
/// 恢复很多遍。回放与最小复现剪枝正需要这一点——每个候选子序列都要从
/// 同一个基线重新跑一遍。
#[derive(Clone)]
pub struct SessionSnapshot {
    /// 应用状态快照，由应用自己解释。
    pub app_state: Option<Arc<dyn Any>>,
    /// 快照时的焦点节点。
    pub focus: Option<u64>,
    /// 快照时的语义树（按视觉顺序）。
    pub semantics: Vec<SemanticsEntry>,
    /// 快照时的指针位置（逻辑像素）。
    pub pointer: Vec2,
}

impl SessionSnapshot {
    /// 快照是否包含可恢复的应用状态。
    ///
    /// 为 `false` 时恢复会失败：界面能被重建，但应用内部的状态回不去。
    #[must_use]
    pub fn is_restorable(&self) -> bool {
        self.app_state.is_some()
    }

    /// 快照的语义树条目数。
    #[must_use]
    pub fn semantics_len(&self) -> usize {
        self.semantics.len()
    }
}

impl std::fmt::Debug for SessionSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionSnapshot")
            .field("restorable", &self.is_restorable())
            .field("focus", &self.focus)
            .field("semantics", &self.semantics.len())
            .field("pointer", &self.pointer)
            .finish()
    }
}

/// 快照恢复失败。
#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotError {
    /// 应用不提供状态快照，因此无法恢复。
    StateUnavailable,
    /// 应用拒绝了这次恢复。
    Rejected {
        /// 拒绝原因。
        reason: String,
    },
}

impl SnapshotError {
    /// 结构化的错误描述。
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::StateUnavailable => "the application does not provide state snapshots".to_owned(),
            Self::Rejected { reason } => format!("the application rejected the restore: {reason}"),
        }
    }
}

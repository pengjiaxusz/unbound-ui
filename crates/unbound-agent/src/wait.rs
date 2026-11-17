//! 条件等待：让 Agent 不必猜「要等多久」。

use crate::query::NodeQuery;

/// 等待条件。
#[derive(Clone, Debug, PartialEq)]
pub enum WaitCondition {
    /// 节点出现。
    NodeExists {
        /// 查询条件。
        query: NodeQuery,
    },
    /// 节点消失。
    NodeGone {
        /// 查询条件。
        query: NodeQuery,
    },
    /// 节点状态满足。
    NodeState {
        /// 目标节点。
        node: u64,
        /// 状态谓词（全部满足才算达成）。
        predicates: Vec<crate::query::NodePredicate>,
    },
    /// 界面稳定（连续若干帧无变化）。
    Settled,
    /// 文本出现。
    TextVisible {
        /// 文本片段。
        text: String,
    },
}

/// 等待超时。
///
/// 失败时携带**已经观察到的状态**，让 Agent 能立刻判断「差在哪里」，
/// 而不是只得到一句「超时了」。
#[derive(Clone, Debug, PartialEq)]
pub struct WaitTimeout {
    /// 超时的条件。
    pub condition: WaitCondition,
    /// 已等待的时长（毫秒）。
    pub waited_ms: u64,
    /// 等待期间观察到的相关节点数量。
    pub observed: usize,
}

impl WaitTimeout {
    /// 结构化的错误描述。
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "wait condition not met within {} ms; observed {} related nodes meanwhile",
            self.waited_ms, self.observed
        )
    }
}

//! 断言：Agent 怎么确认自己做对了。
//!
//! 断言失败时返回**结构化**的诊断，而不是一句「测试挂了」——它同时给出
//! 期望值、实际值、节点路径，以及**下一步该做什么**。

use unbound_math::Vec2;

use crate::query::{NodePredicate, NodeQuery};

/// 矩形约束。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundsExpect {
    /// 允许的宽度区间。
    pub width: Option<(f32, f32)>,
    /// 允许的高度区间。
    pub height: Option<(f32, f32)>,
    /// 必须包含的点（逻辑像素）。
    pub contains: Option<Vec2>,
}

/// 断言。
#[derive(Clone, Debug, PartialEq)]
pub enum Assertion {
    /// 存在匹配的节点。
    Exists {
        /// 查询条件。
        query: NodeQuery,
    },
    /// 不存在匹配的节点。
    Absent {
        /// 查询条件。
        query: NodeQuery,
    },
    /// 节点状态符合预期。
    State {
        /// 目标节点。
        node: u64,
        /// 状态谓词（全部满足才算通过）。
        predicates: Vec<NodePredicate>,
    },
    /// 节点名称等于预期。
    Text {
        /// 目标节点。
        node: u64,
        /// 预期名称。
        expected: String,
    },
    /// 节点矩形符合约束。
    Bounds {
        /// 目标节点。
        node: u64,
        /// 约束。
        expect: BoundsExpect,
    },
    /// 指定位置的像素颜色符合预期。
    ///
    /// 采样点是**逻辑像素**，与语义树的矩形同一坐标系。
    ColorAt {
        /// 采样位置（逻辑像素）。
        pos: Vec2,
        /// 预期颜色。
        expect: unbound_math::Color,
        /// 每通道容差（`0`–`255`）。
        tolerance: u8,
    },
}

impl Assertion {
    /// 断言的简短名称。
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Exists { .. } => "exists",
            Self::Absent { .. } => "absent",
            Self::State { .. } => "state",
            Self::Text { .. } => "text",
            Self::Bounds { .. } => "bounds",
            Self::ColorAt { .. } => "color-at",
        }
    }
}

/// 断言失败的结构化诊断。
#[derive(Clone, Debug, PartialEq)]
pub struct AssertionFailure {
    /// 失败的断言种类。
    pub kind: &'static str,
    /// 相关节点标识。
    pub node: Option<u64>,
    /// 相关节点的语义路径。
    pub node_path: Option<String>,
    /// 期望值。
    pub expected: String,
    /// 实际值。
    pub actual: String,
    /// 下一步建议。
    pub suggestion: Option<String>,
}

impl AssertionFailure {
    /// 结构化的错误描述。
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = format!("assertion `{}` failed", self.kind);
        if let Some(path) = &self.node_path {
            out.push_str(&format!("; node {path}"));
        }
        out.push_str(&format!(
            "; expected {}, actual {}",
            self.expected, self.actual
        ));
        if let Some(suggestion) = &self.suggestion {
            out.push_str(&format!("; {suggestion}"));
        }
        out
    }
}

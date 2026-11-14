//! 作为*数据*的可序列化动作（Action）。
//!
//! `docs` §4.1：视图节点不得持有 `Box<dyn Fn>` 闭包。每次交互都绑定一个
//! 强类型且实现 `Copy` 的动作记录，使撤销、重放、网络同步和 Agent 驱动
//! 的模糊测试（fuzzing）变得简单直接。

use bytemuck::{Pod, Zeroable};

/// 语义动作类型。保留为 `u32` 判别值，使不同构建版本的 guest 与 host
/// 仍可通过编号路由动作。
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ActionKind {
    #[default]
    None = 0,
    /// 激活控件（点击、Enter、Space、无障碍调用）。
    Press = 1,
    PointerDown = 2,
    PointerUp = 3,
    PointerMove = 4,
    PointerCancel = 5,
    /// 滚轮 / 触控板；payload = 转为 `f32` 位模式的 (dx, dy)。
    Scroll = 6,
    /// 文本编辑：从动作 arena 插入字符串。
    TextInsert = 7,
    TextBackspace = 8,
    TextDelete = 9,
    /// 画布手势。
    CanvasPan = 10,
    CanvasZoom = 11,
    CanvasSelect = 12,
    CanvasMoveSelection = 13,
    CanvasDelete = 14,
    CanvasDuplicate = 15,
    CanvasGroup = 16,
    CanvasUngroup = 17,
    /// 撤销/重做作用域控制（`docs` §5.3）。
    BeginTransaction = 18,
    UpdateTransaction = 19,
    CommitTransaction = 20,
    CancelTransaction = 21,
    Undo = 22,
    Redo = 23,
    /// 主题与视口。
    SetTheme = 24,
    Resize = 25,
    Focus = 26,
    Blur = 27,
    /// 系统文件拖拽进入窗口（`docs` §6.1）。
    DragEnter = 28,
    /// 系统文件拖拽在放置目标上移动。
    DragOver = 29,
    /// 系统文件拖拽释放。
    Drop = 30,
    /// 开始拖拽客户端自绘边框窗口。
    WindowDrag = 31,
    /// 最小化窗口。
    WindowMinimize = 32,
    /// 切换窗口最大化状态。
    WindowToggleMaximize = 33,
    /// 关闭窗口。
    WindowClose = 34,
    /// 请求 host 应用新的缩放因子；payload 0 = `f32` 位模式。
    SetZoom = 35,
    /// 应用自定义动作的逃生通道。
    Custom = 0xFFFF,
}

impl ActionKind {
    #[must_use]
    pub const fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Press,
            2 => Self::PointerDown,
            3 => Self::PointerUp,
            4 => Self::PointerMove,
            5 => Self::PointerCancel,
            6 => Self::Scroll,
            7 => Self::TextInsert,
            8 => Self::TextBackspace,
            9 => Self::TextDelete,
            10 => Self::CanvasPan,
            11 => Self::CanvasZoom,
            12 => Self::CanvasSelect,
            13 => Self::CanvasMoveSelection,
            14 => Self::CanvasDelete,
            15 => Self::CanvasDuplicate,
            16 => Self::CanvasGroup,
            17 => Self::CanvasUngroup,
            18 => Self::BeginTransaction,
            19 => Self::UpdateTransaction,
            20 => Self::CommitTransaction,
            21 => Self::CancelTransaction,
            22 => Self::Undo,
            23 => Self::Redo,
            24 => Self::SetTheme,
            28 => Self::DragEnter,
            29 => Self::DragOver,
            30 => Self::Drop,
            25 => Self::Resize,
            26 => Self::Focus,
            27 => Self::Blur,
            31 => Self::WindowDrag,
            32 => Self::WindowMinimize,
            33 => Self::WindowToggleMaximize,
            34 => Self::WindowClose,
            35 => Self::SetZoom,
            0xFFFF => Self::Custom,
            _ => Self::None,
        }
    }

    /// 会修改文档状态的动作，因此属于撤销栈；
    /// 瞬态指针事件明确不入栈。
    #[must_use]
    pub const fn is_transactional(self) -> bool {
        matches!(
            self,
            Self::CanvasMoveSelection
                | Self::CanvasDelete
                | Self::CanvasDuplicate
                | Self::CanvasGroup
                | Self::CanvasUngroup
                | Self::TextInsert
                | Self::TextBackspace
                | Self::TextDelete
        )
    }

    /// 即使在文本合并时间窗口内到达也绝不能合并的动作。
    ///
    #[must_use]
    pub const fn is_boundary(self) -> bool {
        matches!(
            self,
            Self::CommitTransaction
                | Self::CancelTransaction
                | Self::Undo
                | Self::Redo
                | Self::CanvasDelete
        )
    }
}

/// 40 字节的 POD 动作记录（4B 类型 + 4B 目标 + 24B 负载 + 8B 时间）。
///
/// `payload` 有意设计为无类型：reducer 会根据 [`ActionKind`] 解释其含义。
/// 浮点数以位模式存储，使该记录可实现 `Eq` + `Hash`（用于事件合并与动作日志）。
///
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct RawAction {
    pub kind: u32,
    /// 目标节点，全局动作则为 `NO_NODE`。
    pub target: u32,
    /// `u64` 负载槽位：索引、token id、打包坐标对等。
    pub payload: [u64; 3],
    /// 单调递增的输入时间戳（微秒，用于文本输入合并）。
    pub timestamp_us: u64,
}

impl RawAction {
    pub const NONE: Self = Self {
        kind: 0,
        target: 0,
        payload: [0; 3],
        timestamp_us: 0,
    };

    #[must_use]
    pub const fn new(kind: ActionKind, target: u32) -> Self {
        Self {
            kind: kind as u32,
            target,
            payload: [0; 3],
            timestamp_us: 0,
        }
    }

    #[must_use]
    pub const fn with_payload(mut self, index: usize, value: u64) -> Self {
        if index < 3 {
            self.payload[index] = value;
        }
        self
    }

    #[must_use]
    pub const fn with_time(mut self, us: u64) -> Self {
        self.timestamp_us = us;
        self
    }

    #[inline]
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        ActionKind::from_u32(self.kind)
    }

    #[inline]
    #[must_use]
    pub const fn f32_at(&self, index: usize) -> f32 {
        if index < 3 {
            f32::from_bits(self.payload[index] as u32)
        } else {
            0.0
        }
    }

    #[inline]
    #[must_use]
    pub const fn f32_pair(&self, a: usize, b: usize) -> (f32, f32) {
        (self.f32_at(a), self.f32_at(b))
    }

    /// 将两个 `f32` 坐标打包进单个负载槽位（可精确往返还原）。
    #[must_use]
    pub const fn pack_vec2(x: f32, y: f32) -> u64 {
        (x.to_bits() as u64) | ((y.to_bits() as u64) << 32)
    }

    #[must_use]
    pub const fn unpack_vec2(v: u64) -> (f32, f32) {
        (f32::from_bits(v as u32), f32::from_bits((v >> 32) as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_round_trips_kind_and_payload() {
        let a = RawAction::new(ActionKind::CanvasZoom, 7)
            .with_payload(0, RawAction::pack_vec2(12.5, -3.25))
            .with_time(1234);
        assert_eq!(a.kind(), ActionKind::CanvasZoom);
        assert_eq!(a.target, 7);
        assert_eq!(a.timestamp_us, 1234);
        let (x, y) = RawAction::unpack_vec2(a.payload[0]);
        assert_eq!((x, y), (12.5, -3.25));
    }

    #[test]
    fn float_payload_is_exact_through_bits() {
        let v = 0.1f32 + 0.2f32;
        let a = RawAction::new(ActionKind::PointerMove, 1).with_payload(1, v.to_bits() as u64);
        assert_eq!(a.f32_at(1), v);
    }

    #[test]
    fn action_size_is_40_bytes() {
        assert_eq!(core::mem::size_of::<RawAction>(), 40);
        assert_eq!(core::mem::align_of::<RawAction>(), 8);
    }

    #[test]
    fn window_actions_round_trip() {
        for kind in [
            ActionKind::WindowDrag,
            ActionKind::WindowMinimize,
            ActionKind::WindowToggleMaximize,
            ActionKind::WindowClose,
            ActionKind::SetZoom,
        ] {
            assert_eq!(ActionKind::from_u32(kind as u32), kind);
            assert!(!kind.is_transactional());
        }
    }

    #[test]
    fn unknown_kind_maps_to_none() {
        assert_eq!(ActionKind::from_u32(999), ActionKind::None);
        let a = RawAction {
            kind: 999,
            ..Default::default()
        };
        assert_eq!(a.kind(), ActionKind::None);
    }

    #[test]
    fn transactional_classification() {
        assert!(ActionKind::CanvasMoveSelection.is_transactional());
        assert!(ActionKind::TextInsert.is_transactional());
        assert!(!ActionKind::PointerMove.is_transactional());
        assert!(!ActionKind::Undo.is_transactional());
        assert!(ActionKind::Undo.is_boundary());
        assert!(!ActionKind::TextInsert.is_boundary());
    }

    #[test]
    fn out_of_range_payload_is_ignored() {
        let a = RawAction::new(ActionKind::Press, 0).with_payload(9, 5);
        assert_eq!(a.payload, [0; 3]);
        assert_eq!(a.f32_at(9), 0.0);
    }
}

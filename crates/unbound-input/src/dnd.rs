//! OS 原生拖拽穿透（`docs` §6.1）。
//!
//! 当用户从资源管理器将文件拖入窗口时，拖拽由操作系统接管。
//! 引擎的任务是将该事件流转换为与内部拖拽手势**相同**类型的瞬态
//! `DragOver` 动作，从而复用现有的双层拾取测试高亮放置目标，
//! 使 reducer 无需关心文件究竟来自何处。
//!
//!
//! 文件路径无法塞入 24 字节的动作负载中，因此它们存放在一个小型
//! arena 中，动作仅携带索引。该 arena 按设计为瞬态：拖拽离开窗口时
//! 即被清空，从而避免 10 万个文件的拖放操作撑大撤销栈。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::PathBuf;

use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::RawNodeId;
use unbound_math::Vec2;

/// 操作系统正在携带的拖拽内容。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragPayload {
    pub paths: Vec<PathBuf>,
    /// 来自浏览器或编辑器的纯文本拖拽（不带文件路径）。
    pub text: Option<String>,
}

impl DragPayload {
    /// 文件拖拽载荷。
    #[must_use]
    pub fn files(paths: Vec<PathBuf>) -> Self {
        Self { paths, text: None }
    }

    /// 纯文本拖拽载荷。
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            paths: Vec::new(),
            text: Some(text.into()),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.text.is_none()
    }

    /// 文件数量，供事件日志记录使用。
    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }
}

/// 活跃拖拽载荷的 arena 存储区。
#[derive(Clone, Debug, Default)]
pub struct PayloadArena {
    entries: Vec<Option<DragPayload>>,
}

impl PayloadArena {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 存入载荷并返回其 ID。
    pub fn insert(&mut self, payload: DragPayload) -> u64 {
        if let Some((index, slot)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(payload);
            return index as u64 + 1;
        }
        self.entries.push(Some(payload));
        self.entries.len() as u64
    }

    /// 获取 `id` 对应的载荷。
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&DragPayload> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get(index)?.as_ref()
    }

    /// 释放载荷。只要仍有其他载荷存活，ID 就绝不会复用，
    /// 但底层槽位会被回收，因此长时间拖拽会话不会导致内存持续增长。
    pub fn remove(&mut self, id: u64) -> Option<DragPayload> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get_mut(index)?.take()
    }

    /// 清空所有载荷（拖拽已离开窗口）。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 当前存活的载荷数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|slot| slot.is_some()).count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 单帧 OS 拖拽状态。
#[derive(Clone, Debug)]
pub struct DragOver {
    pub position: Vec2,
    /// 若窗口接受该拖拽，则为指针下方的命中测试目标。
    pub target: Option<RawNodeId>,
    pub payload_id: u64,
}

impl DragOver {
    /// 构建供 reducer 与浮层消费的瞬态动作。
    ///
    /// 该动作**非**事务性的：它绝不会进入撤销栈
    /// （`docs` §5.3 将临时状态排除在历史记录之外）。
    #[must_use]
    pub fn to_action(&self, kind: ActionKind) -> RawAction {
        let mut action = RawAction::new(
            kind,
            self.target.map_or(RawNodeId::ROOT.0, |target| target.0),
        );
        action.payload[0] = self.payload_id;
        // 将坐标打包为两个 `f32` 位模式，使 40 字节记录保持
        // `Eq + Hash`（动作日志所需）。
        action.payload[1] = u64::from(self.position.x.to_bits());
        action.payload[2] = u64::from(self.position.y.to_bits());
        action
    }

    /// 解码此前由 [`Self::to_action`] 打包的坐标。
    #[must_use]
    pub fn position_from(action: &RawAction) -> Vec2 {
        Vec2::new(
            f32::from_bits(action.payload[1] as u32),
            f32::from_bits(action.payload[2] as u32),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_round_trip_through_the_arena() {
        let mut arena = PayloadArena::new();
        let first = arena.insert(DragPayload::files(vec![PathBuf::from("a.png")]));
        let second = arena.insert(DragPayload::text("hello"));
        assert_eq!(arena.get(first).unwrap().paths.len(), 1);
        assert_eq!(arena.get(second).unwrap().text.as_deref(), Some("hello"));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn removed_slots_are_recycled() {
        let mut arena = PayloadArena::new();
        let first = arena.insert(DragPayload::text("a"));
        arena.insert(DragPayload::text("b"));
        arena.remove(first);
        let reused = arena.insert(DragPayload::text("c"));
        assert_eq!(reused, first, "the freed slot must be reused");
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn an_unknown_id_is_none() {
        let arena = PayloadArena::new();
        assert!(arena.get(0).is_none());
        assert!(arena.get(99).is_none());
    }

    #[test]
    fn the_action_carries_target_id_and_position() {
        let drag = DragOver {
            position: Vec2::new(12.5, -3.25),
            target: Some(RawNodeId(7)),
            payload_id: 42,
        };
        let action = drag.to_action(ActionKind::DragOver);
        assert_eq!(action.kind(), ActionKind::DragOver);
        assert_eq!(action.target, 7);
        assert_eq!(action.payload[0], 42);
        assert_eq!(DragOver::position_from(&action), drag.position);
    }

    #[test]
    fn a_drag_outside_any_target_targets_the_root() {
        let drag = DragOver {
            position: Vec2::ZERO,
            target: None,
            payload_id: 1,
        };
        let action = drag.to_action(ActionKind::Drop);
        assert_eq!(action.kind(), ActionKind::Drop);
        assert_eq!(action.target, RawNodeId::ROOT.0);
    }

    #[test]
    fn clearing_drops_everything() {
        let mut arena = PayloadArena::new();
        arena.insert(DragPayload::text("x"));
        arena.clear();
        assert!(arena.is_empty());
    }

    #[test]
    fn an_empty_payload_is_recognised() {
        assert!(DragPayload::default().is_empty());
        assert!(!DragPayload::text("x").is_empty());
        assert!(!DragPayload::files(vec![PathBuf::from("a")]).is_empty());
        assert_eq!(DragPayload::files(vec![PathBuf::from("a")]).len(), 1);
    }
}

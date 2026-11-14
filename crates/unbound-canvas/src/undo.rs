//! 基于写时复制快照的事务型撤销/重做（`docs` §5.3）。
//!
//! 文档定义的形态为：指针按下时 `BeginTransaction`，拖拽期间持续发送低开销
//! `UpdateTransaction`，指针抬起时 `CommitTransaction`，最终只产生**一个**撤销步骤。
//! 本模块即实现该机制，且采用*补丁日志（patch log）*而非全量状态快照：
//!
//!
//! * 在 1 000 000 节点的画布上拖拽三个节点只记录三个矩形，而非一百万个——
//!   每步内存增量仅数十字节，这正是文档“单步 ≤ 数 KB”要求的本质；
//!
//!
//! * 该日志同时也是归约器（reducer）直接应用的对象，因此撤销/重做绝不会与
//!   产生它的状态发生漂移。
//!
//! 文本编辑较为特殊：连续按键必须合并为单个撤销步骤，但遇到词边界或停顿 500 ms
//! 必须开启新步骤。[`UndoStack::begin`] 会通过检查当前仍打开的事务自动决策，
//! 调用方无需关心。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use unbound_math::Rect;

/// 两次文本编辑允许合并的最大时间间隔（秒）。
pub const TEXT_COALESCE_WINDOW: f64 = 0.5;

/// 开启事务的交互类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransactionKind {
    /// 绝不应合并的操作（如按钮点击、菜单命令）。
    Discrete,
    /// 拖拽或缩放；同一实体的更新会原地覆盖。
    Drag,
    /// 键盘输入；在 [`TEXT_COALESCE_WINDOW`] 内合并，除非跨越词边界。
    ///
    Text,
}

/// 单条可逆变更记录。
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// 单个实体的几何变更。
    Geometry { id: u32, before: Rect, after: Rect },
    /// 文本缓冲区编辑。
    Text { before: String, after: String },
    /// 单个实体的创建或销毁。
    ///
    /// 实体的包围盒随编辑一并保存：撤销删除操作必须将实体放回*原位*，
    /// 而非仅仅重新创建一个空实体。
    Exist {
        id: u32,
        /// 创建时为 true，销毁时为 false。
        existed: bool,
        /// 编辑发生时实体的包围盒。
        bounds: Rect,
    },
}

impl Edit {
    /// 返回反向的同一条编辑记录。
    #[must_use]
    pub fn inverted(&self) -> Self {
        match self {
            Self::Geometry { id, before, after } => Self::Geometry {
                id: *id,
                before: *after,
                after: *before,
            },
            Self::Text { before, after } => Self::Text {
                before: after.clone(),
                after: before.clone(),
            },
            Self::Exist {
                id,
                existed,
                bounds,
            } => Self::Exist {
                id: *id,
                existed: !existed,
                bounds: *bounds,
            },
        }
    }

    /// 载荷字节数，供 KPI 断言使用。
    #[must_use]
    pub fn bytes(&self) -> usize {
        core::mem::size_of::<Self>()
            + match self {
                Self::Text { before, after } => before.len() + after.len(),
                _ => 0,
            }
    }

    /// 若此编辑涉及特定实体，则返回其句柄。
    #[must_use]
    pub fn entity(&self) -> Option<u32> {
        match self {
            Self::Geometry { id, .. } | Self::Exist { id, .. } => Some(*id),
            Self::Text { .. } => None,
        }
    }
}

/// 一组已提交（或进行中）的编辑事务。
#[derive(Clone, Debug)]
pub struct Transaction {
    label: &'static str,
    kind: TransactionKind,
    edits: Vec<Edit>,
    /// 记录每个实体对应几何编辑在 `edits` 中的索引，使拖拽能原地覆盖而非追加数千条记录。
    ///
    geometry_slot: FxHashMap<u32, usize>,
    started: f64,
    last_edit: f64,
}

impl Transaction {
    fn new(label: &'static str, kind: TransactionKind, now: f64) -> Self {
        Self {
            label,
            kind,
            edits: Vec::new(),
            geometry_slot: FxHashMap::default(),
            started: now,
            last_edit: now,
        }
    }

    /// 可读标签，例如 `"drag 3 items"`。
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// 开启该事务的交互类型。
    #[must_use]
    pub fn kind(&self) -> TransactionKind {
        self.kind
    }

    /// 按应用顺序排列的编辑列表。
    #[must_use]
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// 当未发生任何有效变更时返回 true（该事务不应记入历史）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// 事务开启的逻辑时间戳。
    #[must_use]
    pub fn started(&self) -> f64 {
        self.started
    }

    /// 最近一次记录编辑的逻辑时间戳。
    #[must_use]
    pub fn last_edit(&self) -> f64 {
        self.last_edit
    }

    /// 载荷字节数，供 KPI 断言使用。
    #[must_use]
    pub fn bytes(&self) -> usize {
        core::mem::size_of::<Self>() + self.edits.iter().map(Edit::bytes).sum::<usize>()
    }

    /// 若最近一次记录的文本以词边界结尾则返回 true，
    /// 表示下一次按键必须开启新的撤销步骤。
    fn ends_on_boundary(&self) -> bool {
        match self.edits.last() {
            Some(Edit::Text { after, .. }) => after.chars().last().is_some_and(is_word_boundary),
            _ => false,
        }
    }

    /// 按从新到旧的顺序迭代反向编辑：即撤销时必须应用的序列。
    pub fn inverted(&self) -> impl Iterator<Item = Edit> + '_ {
        self.edits.iter().rev().map(Edit::inverted)
    }
}

/// 撤销/重做历史栈。
#[derive(Debug)]
pub struct UndoStack {
    past: Vec<Transaction>,
    future: Vec<Transaction>,
    active: Option<Transaction>,
    limit: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(256)
    }
}

impl UndoStack {
    /// 创建最多保留 `limit` 个已提交步骤的撤销栈。
    ///
    /// # Panic 说明
    /// 当 `limit` 为 0 时 panic。
    #[must_use]
    pub fn new(limit: usize) -> Self {
        assert!(limit > 0, "history limit must be positive");
        Self {
            past: Vec::new(),
            future: Vec::new(),
            active: None,
            limit,
        }
    }

    /// 可撤销的步骤数量。
    #[must_use]
    pub fn depth(&self) -> usize {
        self.past.len()
    }

    /// 可重做的步骤数量。
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.future.len()
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// 当存在未提交的打开事务时返回 true。
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// 返回当前打开的事务（若有）。
    #[must_use]
    pub fn active(&self) -> Option<&Transaction> {
        self.active.as_ref()
    }

    /// 开启一个事务。
    ///
    /// 若当前已打开文本事务、新事务也是文本且时间间隔在 [`TEXT_COALESCE_WINDOW`] 内，
    /// 则*保留*原事务——这就是合并规则。其他任何情况都会先提交上一个事务，
    /// 从而确保被快捷键打断的拖拽仍能进入历史记录。
    ///
    pub fn begin(&mut self, kind: TransactionKind, label: &'static str, now: f64) {
        let mergeable = self.active.as_ref().is_some_and(|active| {
            active.kind == TransactionKind::Text
                && kind == TransactionKind::Text
                && now - active.last_edit <= TEXT_COALESCE_WINDOW
                && !active.ends_on_boundary()
        });
        if mergeable {
            return;
        }
        self.commit();
        self.active = Some(Transaction::new(label, kind, now));
    }

    /// 记录几何变更；若未发生移动则返回 `false`。
    ///
    /// 拖拽期间对同一实体的重复调用会覆盖已记录的 `after` 矩形并保留原始 `before`，
    /// 因此 120 Hz 拖拽持续一秒也只产生单条记录。
    ///
    pub fn record_geometry(&mut self, id: u32, before: Rect, after: Rect, now: f64) -> bool {
        if before == after {
            return false;
        }
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        if let Some(slot) = active.geometry_slot.get(&id) {
            if let Edit::Geometry { after: stored, .. } = &mut active.edits[*slot] {
                *stored = after;
            }
            return true;
        }
        let slot = active.edits.len();
        active.edits.push(Edit::Geometry { id, before, after });
        active.geometry_slot.insert(id, slot);
        true
    }

    /// 记录文本变更；当文档规则允许时与上一次按键合并。
    ///
    /// 合并要求新缓冲区是上一缓冲区的扩展（在刚输入内容的末尾继续输入），
    /// 且追加字符为普通词字符。空格、标点与换行会结束当前合并段，
    /// 这使得“撤销”能按词回退。
    ///
    ///
    pub fn record_text(&mut self, before: &str, after: &str, now: f64) -> bool {
        if before == after {
            return false;
        }
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        if active.kind != TransactionKind::Text {
            active.edits.push(Edit::Text {
                before: before.to_owned(),
                after: after.to_owned(),
            });
            return true;
        }
        if let Some(Edit::Text {
            after: previous, ..
        }) = active.edits.last_mut()
        {
            let extends = after.starts_with(previous.as_str()) && after.len() > previous.len();
            let appended_is_word = after[previous.len()..]
                .chars()
                .next()
                .is_some_and(|c| !is_word_boundary(c));
            if extends && appended_is_word {
                *previous = after.to_owned();
                return true;
            }
        }
        active.edits.push(Edit::Text {
            before: before.to_owned(),
            after: after.to_owned(),
        });
        true
    }

    /// 记录实体的出现或消失。
    ///
    /// `bounds` 为编辑时实体的几何信息。它随编辑一并保存，
    /// 以便撤销删除时将实体放回原位，而非重建空实体。
    ///
    pub fn record_exist(&mut self, id: u32, existed: bool, bounds: Rect, now: f64) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        active.edits.push(Edit::Exist {
            id,
            existed,
            bounds,
        });
        true
    }

    /// 提交当前打开的事务。空事务会被直接丢弃，因此无移动的点击不会污染历史栈。
    ///
    pub fn commit(&mut self) -> Option<&Transaction> {
        let active = self.active.take()?;
        if active.is_empty() {
            return None;
        }
        self.past.push(active);
        self.future.clear();
        if self.past.len() > self.limit {
            let excess = self.past.len() - self.limit;
            self.past.drain(0..excess);
        }
        self.past.last()
    }

    /// 放弃当前打开的事务（例如拖拽中按下 Escape）。
    ///
    /// 返回已记录编辑的反向序列，供调用方将状态恢复原样。若直接丢弃而不回滚，
    /// 会导致日志与状态失步：本模块的契约是“日志*即*归约器所应用的状态”，
    /// 因此中止的事务必须被回滚，而不能被遗忘。
    ///
    ///
    pub fn abort(&mut self) -> Vec<Edit> {
        let Some(active) = self.active.take() else {
            return Vec::new();
        };
        active.edits.iter().rev().map(Edit::inverted).collect()
    }

    /// 弹出最新已提交事务并交给调用方，调用方应应用 [`Transaction::inverted`]。
    ///
    pub fn undo(&mut self) -> Option<&Transaction> {
        if self.active.is_some() {
            self.commit();
        }
        let entry = self.past.pop()?;
        self.future.push(entry);
        self.future.last()
    }

    /// 弹出最新已撤销事务并交给调用方，调用方应按序应用 [`Transaction::edits`]。
    ///
    pub fn redo(&mut self) -> Option<&Transaction> {
        let entry = self.future.pop()?;
        self.past.push(entry);
        self.past.last()
    }

    /// 历史栈持有的总载荷字节数，供 KPI 断言使用。
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.past.iter().map(Transaction::bytes).sum::<usize>()
            + self.future.iter().map(Transaction::bytes).sum::<usize>()
            + self.active.as_ref().map_or(0, Transaction::bytes)
    }

    /// [`Self::undo`] 将要回退的步骤标签。
    #[must_use]
    pub fn undo_label(&self) -> Option<&'static str> {
        self.past.last().map(Transaction::label)
    }

    /// [`Self::redo`] 将要重做的步骤标签。
    #[must_use]
    pub fn redo_label(&self) -> Option<&'static str> {
        self.future.last().map(Transaction::label)
    }
}

/// 终止文本合并段的字符集合。
///
/// 涵盖 ASCII 空白与标点，以及中文输入法（IME）提交的 CJK 标点——
/// 对这些语言而言，“词”边界往往是全角字符而非空格。
///
#[must_use]
pub fn is_word_boundary(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation() || is_cjk_punctuation(c)
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}'   // CJK 符号与标点
        | '\u{FF00}'..='\u{FF65}' // 全角形式
        | '\u{2018}'..='\u{201F}' // 中文弯引号
        | '\u{2026}' | '\u{2014}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_drag_of_many_frames_is_one_undo_step() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        let start = rect(0.0, 0.0, 10.0, 10.0);
        for frame in 1..=120u32 {
            let now = rect(f32::from(frame as u16), 0.0, 10.0, 10.0);
            stack.record_geometry(7, start, now, f64::from(frame) / 120.0);
        }
        stack.commit();
        assert_eq!(stack.depth(), 1);
        let txn = &stack.past[0];
        assert_eq!(txn.edits().len(), 1, "one entity, one entry");
        match &txn.edits()[0] {
            Edit::Geometry { before, after, .. } => {
                assert_eq!(*before, start);
                assert_eq!(*after, rect(120.0, 0.0, 10.0, 10.0));
            }
            other => panic!("unexpected edit {other:?}"),
        }
    }

    #[test]
    fn a_click_without_movement_records_nothing() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        assert!(!stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(0.0, 0.0, 1.0, 1.0), 0.1));
        assert!(stack.commit().is_none());
        assert_eq!(stack.depth(), 0);
        assert!(!stack.can_undo());
    }

    #[test]
    fn abort_discards_the_open_transaction() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(5.0, 5.0, 1.0, 1.0), 0.1);
        stack.abort();
        assert!(!stack.is_open());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "move a", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(10.0, 0.0, 1.0, 1.0), 0.1);
        stack.commit();
        stack.begin(TransactionKind::Drag, "move b", 0.2);
        stack.record_geometry(2, rect(0.0, 0.0, 1.0, 1.0), rect(20.0, 0.0, 1.0, 1.0), 0.3);
        stack.commit();

        let undo = stack.undo().unwrap();
        assert_eq!(undo.label(), "move b");
        let inverse: Vec<_> = undo.inverted().collect();
        assert_eq!(
            inverse[0],
            Edit::Geometry {
                id: 2,
                before: rect(20.0, 0.0, 1.0, 1.0),
                after: rect(0.0, 0.0, 1.0, 1.0)
            }
        );
        assert_eq!(stack.redo_depth(), 1);

        stack.undo();
        assert!(!stack.can_undo());
        assert_eq!(stack.redo_depth(), 2);

        let redo = stack.redo().unwrap();
        assert_eq!(redo.label(), "move a");
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.redo_depth(), 1);
    }

    #[test]
    fn a_new_commit_clears_the_redo_stack() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Discrete, "a", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(1.0, 0.0, 1.0, 1.0), 0.1);
        stack.commit();
        stack.undo();
        assert_eq!(stack.redo_depth(), 1);
        stack.begin(TransactionKind::Discrete, "b", 0.2);
        stack.record_geometry(1, rect(1.0, 0.0, 1.0, 1.0), rect(2.0, 0.0, 1.0, 1.0), 0.3);
        stack.commit();
        assert_eq!(stack.redo_depth(), 0, "branching must drop the redo tail");
    }

    #[test]
    fn typing_merges_into_one_step() {
        let mut stack = UndoStack::new(16);
        let mut now = 0.0;
        let mut text = String::new();
        for c in "hello".chars() {
            stack.begin(TransactionKind::Text, "type", now);
            let before = text.clone();
            text.push(c);
            assert!(stack.record_text(&before, &text, now));
            now += 0.1;
        }
        stack.commit();
        assert_eq!(stack.depth(), 1, "five keystrokes must be one step");
        match &stack.past[0].edits()[0] {
            Edit::Text { before, after } => {
                assert_eq!(before, "");
                assert_eq!(after, "hello");
            }
            other => panic!("unexpected edit {other:?}"),
        }
    }

    #[test]
    fn a_space_starts_a_new_undo_step() {
        let mut stack = UndoStack::new(16);
        let mut now = 0.0;
        let mut text = String::new();
        for c in "ab cd".chars() {
            stack.begin(TransactionKind::Text, "type", now);
            let before = text.clone();
            text.push(c);
            stack.record_text(&before, &text, now);
            now += 0.1;
        }
        stack.commit();
        // "ab " 为一步，"cd" 为另一步。
        assert_eq!(stack.depth(), 2, "{:?}", stack.past);
    }

    #[test]
    fn a_long_pause_starts_a_new_undo_step() {
        let mut stack = UndoStack::new(16);
        let mut text = String::new();
        stack.begin(TransactionKind::Text, "type", 0.0);
        let before = text.clone();
        text.push('a');
        stack.record_text(&before, &text, 0.0);
        stack.commit();

        stack.begin(TransactionKind::Text, "type", 10.0);
        let before = text.clone();
        text.push('b');
        stack.record_text(&before, &text, 10.0);
        stack.commit();
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn a_non_extension_starts_a_new_edit_within_the_transaction() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Text, "type", 0.0);
        stack.record_text("ab", "abc", 0.1);
        // 用户移动了光标并在别处输入：不属于末尾扩展。
        stack.record_text("abc", "Xabc", 0.2);
        stack.commit();
        assert_eq!(stack.past[0].edits().len(), 2);
    }

    #[test]
    fn history_respects_its_limit() {
        let mut stack = UndoStack::new(3);
        for i in 0..10u32 {
            stack.begin(TransactionKind::Discrete, "step", f64::from(i));
            stack.record_geometry(
                i,
                rect(0.0, 0.0, 1.0, 1.0),
                rect(1.0, 0.0, 1.0, 1.0),
                f64::from(i),
            );
            stack.commit();
        }
        assert_eq!(stack.depth(), 3);
    }

    #[test]
    fn a_drag_on_a_million_node_canvas_costs_bytes_not_megabytes() {
        let mut stack = UndoStack::new(128);
        // 百万节点中仅选中了三个。
        stack.begin(TransactionKind::Drag, "drag 3", 0.0);
        for frame in 0..600u32 {
            for id in [11u32, 22, 33] {
                let x = frame as f32;
                stack.record_geometry(
                    id,
                    rect(0.0, id as f32, 10.0, 10.0),
                    rect(x, id as f32, 10.0, 10.0),
                    f64::from(frame) / 120.0,
                );
            }
        }
        stack.commit();
        assert_eq!(stack.past[0].edits().len(), 3);
        assert!(
            stack.bytes() < 1024,
            "history grew to {} bytes",
            stack.bytes()
        );
    }

    #[test]
    fn opening_a_second_transaction_commits_the_first() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(3.0, 0.0, 1.0, 1.0), 0.1);
        stack.begin(TransactionKind::Discrete, "delete", 0.2);
        assert_eq!(stack.depth(), 1, "the interrupted drag must be preserved");
        assert!(stack.is_open());
    }

    #[test]
    fn undo_commits_an_open_transaction_first() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(3.0, 0.0, 1.0, 1.0), 0.1);
        let undone = stack.undo().unwrap();
        assert_eq!(undone.label(), "drag");
        assert!(!stack.is_open());
    }

    #[test]
    fn existence_edits_invert_cleanly() {
        let bounds = rect(1.0, 2.0, 3.0, 4.0);
        let edit = Edit::Exist {
            id: 4,
            existed: true,
            bounds,
        };
        assert_eq!(
            edit.inverted(),
            Edit::Exist {
                id: 4,
                existed: false,
                bounds,
            }
        );
        assert_eq!(edit.entity(), Some(4));
    }

    /// 撤销删除必须将实体放回*原位*。
    ///
    /// 此前的编辑记录仅携带 ID 与布尔值，导致归约器只能重建一个空实体：
    /// 其几何位置、层级顺序与内容全部丢失。
    ///
    #[test]
    fn an_undone_deletion_restores_the_bounds() {
        let mut stack = UndoStack::new(16);
        let bounds = rect(10.0, 20.0, 30.0, 40.0);
        stack.begin(TransactionKind::Discrete, "delete", 0.0);
        stack.record_exist(7, false, bounds, 0.1);
        stack.commit();
        let transaction = stack.undo().expect("a step to undo");
        let edits: Vec<Edit> = transaction.inverted().collect();
        assert_eq!(
            edits,
            vec![Edit::Exist {
                id: 7,
                existed: true,
                bounds,
            }],
            "undoing a deletion must restore the entity's geometry"
        );
    }

    /// 中止事务必须返回用于回滚它的编辑记录。
    ///
    /// 日志即归约器应用的状态，若直接丢弃而不回滚会导致状态与日志失步。
    ///
    #[test]
    fn aborting_returns_the_edits_that_revert_it() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(5.0, 5.0, 1.0, 1.0), 0.1);
        let reverted = stack.abort();
        assert_eq!(
            reverted,
            vec![Edit::Geometry {
                id: 1,
                before: rect(5.0, 5.0, 1.0, 1.0),
                after: rect(0.0, 0.0, 1.0, 1.0),
            }],
            "aborting must revert the edits, not forget them"
        );
        assert!(!stack.is_open());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn word_boundaries_cover_ascii_and_cjk() {
        assert!(is_word_boundary(' '));
        assert!(is_word_boundary('\n'));
        assert!(is_word_boundary(','));
        assert!(is_word_boundary('，'));
        assert!(is_word_boundary('。'));
        assert!(!is_word_boundary('a'));
        assert!(!is_word_boundary('中'));
    }
}

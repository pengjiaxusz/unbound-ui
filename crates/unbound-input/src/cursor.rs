//! 确定性光标状态栈（`docs` §6.2）。
//!
//! 光标解析是一个**优先级**问题，而非“后来者覆盖”问题：
//! 被滚动条滑块捕获的指针即使划过文本输入框，也必须保持 `grabbing` 状态；
//! 缩放手柄也必须优先于其后方的画布工具。文档规定了如下优先级顺序：
//!
//!
//! ```text
//! 指针捕获 > 浮层 > 缩放手柄 > 画布工具 > 文本 > 默认
//! ```
//!
//! 每个**来源（source）**最多贡献一条记录，因此每帧重复推入同一来源
//! 不会使栈无限增长；同一来源内部的平局由插入序号打破——
//! 这使得最终解析出的光标成为当前帧输入的纯函数。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_canvas::Handle;

/// 操作系统指针应呈现的外观形态。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Default,
    /// 悬停在可点击元素上时的手型光标。
    Pointer,
    /// 悬停在可选中文本上时的工字光标。
    Text,
    /// 张开的手掌：表示内容可平移。
    Grab,
    /// 握紧的手掌：表示正在平移内容。
    Grabbing,
    /// 十字准星，例如旋转控制柄。
    Crosshair,
    /// 四向移动光标。
    Move,
    ResizeNorthSouth,
    ResizeEastWest,
    ResizeNorthWestSouthEast,
    ResizeNorthEastSouthWest,
    NotAllowed,
    /// 平台定义的光标 ID。
    Custom(u16),
}

impl CursorKind {
    /// 将变换手柄映射到对应的光标类型。
    #[must_use]
    pub fn for_handle(handle: Handle) -> Self {
        match handle {
            Handle::North | Handle::South => Self::ResizeNorthSouth,
            Handle::East | Handle::West => Self::ResizeEastWest,
            Handle::NorthWest | Handle::SouthEast => Self::ResizeNorthWestSouthEast,
            Handle::NorthEast | Handle::SouthWest => Self::ResizeNorthEastSouthWest,
            Handle::Rotate => Self::Crosshair,
        }
    }

    /// 该光标所竞争的栈槽位。
    #[must_use]
    pub fn source(self) -> CursorSource {
        match self {
            Self::ResizeNorthSouth
            | Self::ResizeEastWest
            | Self::ResizeNorthWestSouthEast
            | Self::ResizeNorthEastSouthWest => CursorSource::Handle,
            Self::Text => CursorSource::Text,
            Self::Grab | Self::Grabbing | Self::Crosshair | Self::Move => CursorSource::CanvasTool,
            _ => CursorSource::Default,
        }
    }
}

/// 请求光标的来源方，其数值**即**为优先级。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CursorSource {
    Default = 0,
    Text = 1,
    CanvasTool = 2,
    Handle = 3,
    Overlay = 4,
    Capture = 5,
}

#[derive(Copy, Clone, Debug)]
struct Entry {
    source: CursorSource,
    kind: CursorKind,
    seq: u64,
}

/// 单帧内按优先级排序的光标请求集合。
#[derive(Clone, Debug, Default)]
pub struct CursorStack {
    entries: Vec<Entry>,
    seq: u64,
}

impl CursorStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始新一帧：丢弃上一帧的所有请求。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 为 `source` 请求光标，替换其此前的任意请求。
    pub fn push(&mut self, source: CursorSource, kind: CursorKind) {
        self.seq += 1;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.source == source) {
            entry.kind = kind;
            entry.seq = self.seq;
            return;
        }
        self.entries.push(Entry {
            source,
            kind,
            seq: self.seq,
        });
    }

    /// 移除来自 `source` 的光标请求。
    pub fn pop(&mut self, source: CursorSource) {
        self.entries.retain(|e| e.source != source);
    }

    /// 最高优先级的光标请求，若无请求则返回 [`CursorKind::Default`]。
    #[must_use]
    pub fn resolve(&self) -> CursorKind {
        self.entries
            .iter()
            .max_by_key(|e| (e.source, e.seq))
            .map_or(CursorKind::Default, |e| e.kind)
    }

    /// 当前正在请求光标的来源数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_beats_everything() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Text, CursorKind::Text);
        stack.push(CursorSource::Handle, CursorKind::ResizeEastWest);
        stack.push(CursorSource::Overlay, CursorKind::Pointer);
        stack.push(CursorSource::Capture, CursorKind::Grabbing);
        assert_eq!(stack.resolve(), CursorKind::Grabbing);
    }

    #[test]
    fn releasing_the_capture_falls_back_in_priority_order() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Text, CursorKind::Text);
        stack.push(CursorSource::Handle, CursorKind::ResizeNorthSouth);
        stack.push(CursorSource::Capture, CursorKind::Grabbing);
        stack.pop(CursorSource::Capture);
        assert_eq!(stack.resolve(), CursorKind::ResizeNorthSouth);
        stack.pop(CursorSource::Handle);
        assert_eq!(stack.resolve(), CursorKind::Text);
    }

    #[test]
    fn one_slot_per_source() {
        let mut stack = CursorStack::new();
        for _ in 0..10 {
            stack.push(CursorSource::CanvasTool, CursorKind::Grab);
        }
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn clears_between_frames() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Handle, CursorKind::ResizeEastWest);
        stack.clear();
        assert_eq!(stack.resolve(), CursorKind::Default);
        assert!(stack.is_empty());
    }

    #[test]
    fn handles_map_to_the_right_resize_cursors() {
        assert_eq!(
            CursorKind::for_handle(Handle::North),
            CursorKind::ResizeNorthSouth
        );
        assert_eq!(
            CursorKind::for_handle(Handle::West),
            CursorKind::ResizeEastWest
        );
        assert_eq!(
            CursorKind::for_handle(Handle::NorthWest),
            CursorKind::ResizeNorthWestSouthEast
        );
        assert_eq!(
            CursorKind::for_handle(Handle::SouthWest),
            CursorKind::ResizeNorthEastSouthWest
        );
    }
}

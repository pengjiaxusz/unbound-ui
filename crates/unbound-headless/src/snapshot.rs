//! 零拷贝时间旅行快照（`docs` §7.2）。
//!
//! 当一帧失败时，核心诉求是能够*重新检视它*：重放精确的几何结构、
//! 单步调试，或与后续帧进行 diff 比对。`rkyv` 使这一过程极为廉价——
//! 归档字节可直接原地读取，无需反序列化过程，也无需堆分配。
//!
//!
//! 此处有意并存两种格式：
//!
//! * [`TimeTravel`] —— `rkyv` 归档。可原地读取，或恢复为活跃 [`Frame`]。
//!   这是调试器保存的格式。
//! * [`capture`] / [`restore`] —— `postcard` 紧凑 serde 格式。
//!   适合跨网络传输帧或持久化存储数千帧。
//!
//! DTO 层的存在是因为 [`crate::frame::NodeSnapshot`] 携带 `&'static str`
//! 源码位置。序列化字符串并在加载时将其泄漏回静态区，对每次排查仅恢复
//! 一次的调试产物完全合理，同时让热路径免于 `String` 分配抖动。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use serde::{Deserialize, Serialize};
use unbound_math::Rect;

use crate::frame::{Frame, NodeKind, NodeSnapshot, SourceLoc};

/// 帧的稳定传输格式。
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct FrameSnapshot {
    pub viewport: [f32; 4],
    pub nodes: Vec<SnapNode>,
}

/// 单个节点的稳定传输格式。字段顺序是格式契约的一部分。
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct SnapNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub path: String,
    pub rect: [f32; 4],
    pub clip: Option<[f32; 4]>,
    /// `0` 容器，`1` 文本，`2` 可交互，`3` portal。
    pub kind: u8,
    pub ink: [f32; 4],
    pub ellipsis: bool,
    pub occluded: bool,
    pub min_size: [f32; 2],
    pub overlays_siblings: bool,
    pub scrollable: bool,
    /// 文本前景色与背景色，`r,g,b,r,g,b`。
    pub text_colors: Option<[u8; 6]>,
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.origin.x, rect.origin.y, rect.size.x, rect.size.y]
}

fn rect_from_array(array: [f32; 4]) -> Rect {
    Rect::new(array[0], array[1], array[2], array[3])
}

impl FrameSnapshot {
    /// 捕获一帧。
    #[must_use]
    pub fn capture(frame: &Frame) -> Self {
        Self {
            viewport: rect_to_array(frame.viewport),
            nodes: frame
                .nodes
                .iter()
                .map(|node| {
                    let (kind, ink, ellipsis, occluded, min_size) = match &node.kind {
                        NodeKind::Container => (0u8, Rect::ZERO, false, false, [0.0, 0.0]),
                        NodeKind::Text { ink, ellipsis } => {
                            (1u8, *ink, *ellipsis, false, [0.0, 0.0])
                        }
                        NodeKind::Interactive { min_size, occluded } => {
                            (2u8, Rect::ZERO, false, *occluded, [min_size.x, min_size.y])
                        }
                        NodeKind::Portal => (3u8, Rect::ZERO, false, false, [0.0, 0.0]),
                    };
                    SnapNode {
                        id: node.id,
                        parent: node.parent,
                        children: node.children.clone(),
                        file: node.loc.file.to_owned(),
                        line: node.loc.line,
                        column: node.loc.column,
                        path: node.path.clone(),
                        rect: rect_to_array(node.rect),
                        clip: node.clip.map(rect_to_array),
                        kind,
                        ink: rect_to_array(ink),
                        ellipsis,
                        occluded,
                        min_size,
                        overlays_siblings: node.overlays_siblings,
                        scrollable: node.scrollable,
                        text_colors: node.text_colors.map(|colors| {
                            [
                                colors.foreground[0],
                                colors.foreground[1],
                                colors.foreground[2],
                                colors.background[0],
                                colors.background[1],
                                colors.background[2],
                            ]
                        }),
                    }
                })
                .collect(),
        }
    }

    /// 重建一帧。源码位置会在进程生命周期内驻留（intern）。
    #[must_use]
    pub fn restore(&self) -> Frame {
        Frame::from_nodes(
            rect_from_array(self.viewport),
            self.nodes
                .iter()
                .map(|node| {
                    let kind = match node.kind {
                        1 => NodeKind::Text {
                            ink: rect_from_array(node.ink),
                            ellipsis: node.ellipsis,
                        },
                        2 => NodeKind::Interactive {
                            min_size: unbound_math::Vec2::new(node.min_size[0], node.min_size[1]),
                            occluded: node.occluded,
                        },
                        3 => NodeKind::Portal,
                        _ => NodeKind::Container,
                    };
                    NodeSnapshot {
                        id: node.id,
                        parent: node.parent,
                        children: node.children.clone(),
                        loc: SourceLoc {
                            file: intern(&node.file),
                            line: node.line,
                            column: node.column,
                        },
                        path: node.path.clone(),
                        rect: rect_from_array(node.rect),
                        clip: node.clip.map(rect_from_array),
                        kind,
                        overlays_siblings: node.overlays_siblings,
                        scrollable: node.scrollable,
                        text_colors: node.text_colors.map(|colors| crate::frame::TextColors {
                            foreground: [colors[0], colors[1], colors[2]],
                            background: [colors[3], colors[4], colors[5]],
                        }),
                    }
                })
                .collect(),
        )
    }
}

/// 将文件路径驻留到进程生命周期。
fn intern(file: &str) -> &'static str {
    // 每次排查至多恢复几次快照，而 `Location` 需要 `&'static str`，
    // 因此在此处泄漏内存是正确的权衡。
    Box::leak(file.to_owned().into_boxed_str())
}

/// 将帧序列化为字节。
///
/// # Errors
/// 当帧超出声明的大小限制（表明 `Frame` 已损坏）时，返回底层 postcard 错误。
///
pub fn capture(frame: &Frame) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(&FrameSnapshot::capture(frame))
}

/// 从字节反序列化帧。
///
/// # Errors
/// 输入截断或损坏时返回底层 postcard 错误。
pub fn restore(bytes: &[u8]) -> Result<Frame, postcard::Error> {
    let snapshot: FrameSnapshot = postcard::from_bytes(bytes)?;
    Ok(snapshot.restore())
}

/// 零拷贝时间旅行快照。
///
/// 字节本身即归档；每次读取都是直接指向缓冲区内的指针跳转，而非拷贝。
/// 调试器可在内存中保留数千份此类快照并依然瞬时单步回溯。
///
#[derive(Clone, Debug)]
pub struct TimeTravel {
    bytes: rkyv::util::AlignedVec,
}

impl TimeTravel {
    /// 归档一帧。
    ///
    /// # Errors
    /// 帧无法序列化时返回归档错误。
    pub fn capture(frame: &Frame) -> Result<Self, rkyv::rancor::Error> {
        let snapshot = FrameSnapshot::capture(frame);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&snapshot)?;
        Ok(Self { bytes })
    }

    /// 原地读取已归档的帧。
    ///
    /// # Errors
    /// 缓冲区损坏或由不同版本格式写入时返回校验错误。
    ///
    pub fn access(&self) -> Result<&rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error> {
        rkyv::access::<rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error>(&self.bytes)
    }

    /// 归档字节切片。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 快照占用的字节数。
    #[must_use]
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// 从归档恢复为活跃帧。
    ///
    /// # Errors
    /// 缓冲区损坏时返回归档错误。
    pub fn restore(&self) -> Result<Frame, rkyv::rancor::Error> {
        let archived = self.access()?;
        let restored = rkyv::deserialize::<FrameSnapshot, rkyv::rancor::Error>(archived)?;
        Ok(restored.restore())
    }
}

/// 打开由外部写入的归档快照。
///
/// # Errors
/// 缓冲区损坏时返回校验错误。
pub fn open(bytes: &[u8]) -> Result<TimeTravel, rkyv::rancor::Error> {
    // 接管所有权前先校验：损坏的缓冲区绝不可当作正常帧进入调试器其余部分。
    //
    rkyv::access::<rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error>(bytes)?;
    let mut owned = rkyv::util::AlignedVec::new();
    owned.extend_from_slice(bytes);
    Ok(TimeTravel { bytes: owned })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameBuilder;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn sample() -> Frame {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(10.0, 10.0, 80.0, 30.0),
            "Button",
        );
        b.close();
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 12.5, 8.0),
                ellipsis: true,
            },
            rect(0.0, 0.0, 12.5, 8.0),
            "Label",
        );
        b.close();
        b.open(NodeKind::Portal, rect(5.0, 5.0, 100.0, 50.0), "Menu");
        b.close();
        b.close();
        b.build()
    }

    #[test]
    fn a_frame_round_trips_through_postcard() {
        let frame = sample();
        let bytes = capture(&frame).unwrap();
        let restored = restore(&bytes).unwrap();
        assert_eq!(restored.viewport, frame.viewport);
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        for (a, b) in restored.nodes.iter().zip(&frame.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.parent, b.parent);
            assert_eq!(a.children, b.children);
            assert_eq!(a.rect, b.rect);
            assert_eq!(a.clip, b.clip);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.path, b.path);
            assert_eq!(a.loc.line, b.loc.line);
        }
    }

    #[test]
    fn a_time_travel_snapshot_round_trips_through_rkyv() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let restored = snapshot.restore().unwrap();
        assert_eq!(restored.viewport, frame.viewport);
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        for (a, b) in restored.nodes.iter().zip(&frame.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.rect, b.rect);
            assert_eq!(a.path, b.path);
        }
    }

    #[test]
    fn the_archive_can_be_read_in_place() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let archived = snapshot.access().unwrap();
        // 直接从缓冲区原地读取归档的视口。
        assert_eq!(archived.viewport[0], 0.0);
        assert_eq!(archived.viewport[2], 800.0);
        assert_eq!(archived.nodes.len(), frame.nodes.len());
        // 源码路径作为指向归档内的字符串切片保留。
        assert!(archived.nodes[0].path.as_str().contains("Root"));
    }

    #[test]
    fn a_snapshot_survives_a_trip_through_a_buffer() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let reopened = open(snapshot.bytes()).unwrap();
        let restored = reopened.restore().unwrap();
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        assert_eq!(restored.nodes[0].path, frame.nodes[0].path);
    }

    #[test]
    fn a_corrupt_buffer_is_rejected_instead_of_read() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let mut bytes = snapshot.bytes().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(open(&bytes).is_err());
    }

    #[test]
    fn the_archive_is_compact() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        // 10 KB 几何数据不得膨胀成数 MB。
        assert!(
            snapshot.size() < 64 * 1024,
            "archive is {} bytes",
            snapshot.size()
        );
    }

    /// 快照用于跨边界传输，其 ID 完全由发送方决定。
    /// ID 0 在稠密索引中没有槽位：旧代码从零减一并发生 panic，而非恢复帧。
    ///
    #[test]
    fn a_snapshot_carrying_a_zero_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = 0;
        let bytes = postcard::to_allocvec(&snapshot).unwrap();
        let restored = restore(&bytes).expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(0).is_some());
    }

    /// `u64::MAX` 曾导致按 ID 本身大小分配索引，向分配器请求 `usize::MAX` 个条目。
    ///
    #[test]
    fn a_snapshot_carrying_an_absurd_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = u64::MAX;
        let bytes = postcard::to_allocvec(&snapshot).unwrap();
        let restored = restore(&bytes).expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(u64::MAX).is_some());
    }

    /// 经由 rkyv 路径同样如此：`open` 会校验*归档结构*，但内部的 ID 仍由发送方控制。
    ///
    #[test]
    fn an_archive_carrying_a_zero_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = 0;
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&snapshot).unwrap();
        let reopened = open(&bytes).expect("the archive itself is well formed");
        let restored = reopened.restore().expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(0).is_some());
    }
}

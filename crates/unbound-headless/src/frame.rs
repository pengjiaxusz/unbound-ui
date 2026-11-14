//! 无头验证器的几何模型（`docs` §7.2）。
//!
//! [`Frame`] 是单个无头步骤的*产物*：reducer 运行且 Taffy 求解完成后的节点树。
//! 它精确携带五项布局检查器所需的全部数据，且不包含任何依赖 GPU、
//! 窗口句柄或字体文件的内容。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use core::panic::Location;
use unbound_math::{Rect, Vec2};

/// 节点构建位置。通过 `#[track_caller]` 捕获，使检查器能直接指向
/// 创建该节点的源码行，而非仅报告节点 ID。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct SourceLoc {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

impl SourceLoc {
    /// 捕获调用方源码位置。
    #[must_use]
    #[track_caller]
    pub fn caller() -> Self {
        Self::from_location(Location::caller())
    }

    #[must_use]
    pub fn from_location(location: &'static Location<'static>) -> Self {
        Self {
            file: location.file(),
            line: location.line(),
            column: location.column(),
        }
    }
}

impl core::fmt::Display for SourceLoc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// 从检查器视角看到的节点类型。
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// 普通布局容器。
    Container,
    /// 文本：携带测量后的墨迹包围盒（ink box），用于检测裁剪。
    Text {
        /// 整形后文本在父坐标空间中的真实墨迹边界。
        ink: Rect,
        /// 当组件声明 `text-overflow: ellipsis` 时为 `true`，此时裁剪是有意为之。
        ///
        ellipsis: bool,
    },
    /// 可点击元素：携带最小目标尺寸以及中心点是否被上层兄弟节点遮挡。
    ///
    Interactive {
        /// 可接受的最小裁剪后尺寸；按文档默认为 `24 × 24`。
        min_size: Vec2,
        /// 当节点中心被完全遮挡时由命中测试器置位。
        occluded: bool,
    },
    /// Portal 覆盖层：必须保持在物理视口内。
    Portal,
}

impl NodeKind {
    /// 文档默认的最小可交互目标尺寸。
    pub const DEFAULT_MIN_SIZE: Vec2 = Vec2 { x: 24.0, y: 24.0 };

    #[must_use]
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::Interactive { .. })
    }

    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    #[must_use]
    pub fn is_portal(&self) -> bool {
        matches!(self, Self::Portal)
    }
}

/// 文本节点绘制时使用的颜色，供对比度检查器使用。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextColors {
    /// 不透明前景色（`sRGB`）。
    pub foreground: [u8; 3],
    /// 不透明背景色（`sRGB`）。
    pub background: [u8; 3],
}

/// 已验证帧中的单个节点。
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSnapshot {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub loc: SourceLoc,
    /// 语义路径，例如 `Root/Toolbar/Button "Save"`。
    pub path: String,
    /// 布局后的边框盒（视口空间）。
    pub rect: Rect,
    /// 有效裁剪区域（已与所有祖先裁剪区域求交），如有。
    pub clip: Option<Rect>,
    pub kind: NodeKind,
    /// 绝对定位的覆盖层允许与其兄弟节点重叠。
    pub overlays_siblings: bool,
    /// 位于滚动容器内：此处的文本裁剪属于预期行为。
    pub scrollable: bool,
    /// 当组件声明时记录文本前景色与背景色；对比度检查器仅检查此处声明的颜色。
    ///
    pub text_colors: Option<TextColors>,
}

impl NodeSnapshot {
    /// 应用有效裁剪后的矩形区域。
    #[must_use]
    pub fn visible_rect(&self) -> Rect {
        match self.clip {
            Some(clip) => self.rect.intersect(&clip),
            None => self.rect,
        }
    }

    /// 当节点自身或任意祖先为可滚动容器时返回 true。
    #[must_use]
    pub fn in_scroll_container(&self) -> bool {
        self.scrollable
    }
}

/// 已求解完成的帧，可直接交由检查器检查。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub viewport: Rect,
    pub nodes: Vec<NodeSnapshot>,
    /// `index[id - 1]` 为节点 `id` 在 `nodes` 中的位置。
    ///
    /// ID 稠密且由构建器分配，因此此处普通 `Vec` 远快于哈希表：
    /// `get` 仅需两次内存加载，这在检查器对大型帧调用数百万次时至关重要。
    ///
    index: Vec<usize>,
    /// 当帧的 ID *非*稠密时的回退查找表。
    ///
    /// 从网络反序列化的帧可能携带任意 ID：ID 0 在 `index` 中无槽位，
    // 超大 ID 会要求按 ID 大小分配内存，重复 ID 则会让两个节点别名到同一槽位。
    // 此类帧改用按 `(id, position)` 排序的列表——虽稍慢，但既不会
    // 被诱导 panic，也不会分配数 GB 内存。
    //
    sparse: Option<Vec<(u64, usize)>>,
}

impl Frame {
    #[must_use]
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            nodes: Vec::new(),
            index: Vec::new(),
            sparse: None,
        }
    }

    /// 基于现有节点列表构建帧，并重建 ID 索引。
    #[must_use]
    pub fn from_nodes(viewport: Rect, nodes: Vec<NodeSnapshot>) -> Self {
        let mut frame = Self {
            viewport,
            nodes,
            index: Vec::new(),
            sparse: None,
        };
        frame.rebuild_index();
        frame
    }

    fn rebuild_index(&mut self) {
        self.index = Vec::new();
        self.sparse = None;
        let count = self.nodes.len();
        if count == 0 {
            return;
        }
        // `index[id - 1]` 仅能容纳 `1..=count` 的 ID。其余情况要么没有槽位，
        // 要么需要按 ID 大小分配内存，因此这些帧走有序回退路径。
        //
        let mut index = vec![usize::MAX; count];
        let mut dense = true;
        for (position, node) in self.nodes.iter().enumerate() {
            let Some(slot) = usize::try_from(node.id)
                .ok()
                .and_then(|id| id.checked_sub(1))
            else {
                dense = false;
                break;
            };
            if slot >= count || index[slot] != usize::MAX {
                // 超出范围，或存在重复 ID 导致两节点别名冲突。
                dense = false;
                break;
            }
            index[slot] = position;
        }
        if dense {
            self.index = index;
        } else {
            let sorted = self.sorted_positions();
            self.sparse = Some(sorted);
        }
    }

    /// 按 ID 排序的 `(id, position)` 对列表，用于 ID 非稠密的帧。
    fn sorted_positions(&self) -> Vec<(u64, usize)> {
        let mut sorted: Vec<(u64, usize)> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(position, node)| (node.id, position))
            .collect();
        sorted.sort_unstable_by_key(|(id, _)| *id);
        sorted
    }

    fn position_of(&self, id: u64) -> Option<usize> {
        if let Some(sparse) = &self.sparse {
            return sparse
                .binary_search_by_key(&id, |(node_id, _)| *node_id)
                .ok()
                .map(|at| sparse[at].1);
        }
        let slot = usize::try_from(id).ok()?.checked_sub(1)?;
        let position = *self.index.get(slot)?;
        if position == usize::MAX || position >= self.nodes.len() {
            None
        } else {
            Some(position)
        }
    }

    #[must_use]
    pub fn get(&self, id: u64) -> Option<&NodeSnapshot> {
        self.position_of(id).map(|position| &self.nodes[position])
    }

    /// `id` 的父节点链，根节点在前。
    #[must_use]
    pub fn chain(&self, id: u64) -> Vec<u64> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            chain.push(node);
            current = self.get(node).and_then(|n| n.parent);
        }
        chain.reverse();
        chain
    }
}

/// 基于栈的帧构建器。
///
/// 每次 `push` 均记录调用源码行，使诊断信息直接指向组件代码而非数字 ID。
///
#[derive(Debug)]
pub struct FrameBuilder {
    frame: Frame,
    /// 当前打开的节点栈，附带其是否建立滚动容器的标志。
    stack: Vec<(u64, bool)>,
    next_id: u64,
    labels: Vec<String>,
    scroll_depth: u32,
}

impl FrameBuilder {
    #[must_use]
    pub fn new(viewport: Rect) -> Self {
        Self {
            frame: Frame::new(viewport),
            stack: Vec::new(),
            next_id: 1,
            labels: Vec::new(),
            scroll_depth: 0,
        }
    }

    /// 作为当前节点的子节点打开一个新节点。
    #[track_caller]
    pub fn open(&mut self, kind: NodeKind, rect: Rect, label: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let parent = self.stack.last().map(|(id, _)| *id);
        let path = match parent.and_then(|p| self.labels.get(p as usize - 1)) {
            Some(parent_path) if !label.is_empty() => format!("{parent_path}/{label}"),
            Some(parent_path) => parent_path.clone(),
            None => label.to_owned(),
        };
        if let Some(parent_id) = parent {
            if let Some(node) = self.frame.get_mut(parent_id) {
                node.children.push(id);
            }
        }
        // 继承父节点的裁剪区域；若父节点裁剪溢出，则与父节点自身边界求交。
        //
        let clip = match parent.and_then(|p| self.frame.get(p)) {
            Some(parent_node) => parent_node.clip,
            None => None,
        };
        let scrollable = self.scroll_depth > 0;
        self.frame.index.resize(id as usize, usize::MAX);
        self.frame.index[id as usize - 1] = self.frame.nodes.len();
        self.frame.nodes.push(NodeSnapshot {
            id,
            parent,
            children: Vec::new(),
            loc: SourceLoc::caller(),
            path: path.clone(),
            rect,
            clip,
            kind,
            overlays_siblings: false,
            scrollable,
            text_colors: None,
        });
        self.labels.push(path);
        self.stack.push((id, false));
        id
    }

    /// 关闭当前节点。
    pub fn close(&mut self) {
        if let Some((_, guard)) = self.stack.pop() {
            if guard {
                self.scroll_depth = self.scroll_depth.saturating_sub(1);
            }
        }
    }

    /// 将当前节点标记为绝对定位的覆盖层。
    pub fn set_overlays_siblings(&mut self, value: bool) {
        if let Some((id, _)) = self.stack.last().copied() {
            if let Some(node) = self.frame.get_mut(id) {
                node.overlays_siblings = value;
            }
        }
    }

    /// 将当前节点标记为滚动容器：其后代内容被裁剪不视为布局缺陷。
    ///
    pub fn set_scrollable(&mut self, value: bool) {
        if let Some(entry) = self.stack.last_mut() {
            if value && !entry.1 {
                entry.1 = true;
                self.scroll_depth += 1;
                if let Some(node) = self.frame.get_mut(entry.0) {
                    node.scrollable = true;
                }
            }
        }
    }

    /// 将当前节点（及其后代）裁剪至 `rect`。
    pub fn set_clip(&mut self, rect: Rect) {
        if let Some(id) = self.stack.last().map(|(id, _)| *id) {
            let inherited = self.frame.get(id).and_then(|n| n.clip);
            let clip = match inherited {
                Some(existing) => existing.intersect(&rect),
                None => rect,
            };
            if let Some(node) = self.frame.get_mut(id) {
                node.clip = Some(clip);
            }
        }
    }

    /// 声明文本节点绘制时使用的不透明前景色/背景色。
    pub fn set_text_colors(&mut self, foreground: [u8; 3], background: [u8; 3]) {
        if let Some((id, _)) = self.stack.last().copied() {
            if let Some(node) = self.frame.get_mut(id) {
                if node.kind.is_text() {
                    node.text_colors = Some(TextColors {
                        foreground,
                        background,
                    });
                }
            }
        }
    }

    /// 当前打开的节点。
    #[must_use]
    pub fn current(&self) -> Option<u64> {
        self.stack.last().map(|(id, _)| *id)
    }

    /// 完成构建并返回帧。
    #[must_use]
    pub fn build(self) -> Frame {
        self.frame
    }
}

impl Frame {
    fn get_mut(&mut self, id: u64) -> Option<&mut NodeSnapshot> {
        let position = self.position_of(id)?;
        self.nodes.get_mut(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn the_builder_nests_and_names_nodes() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        b.open(NodeKind::Container, rect(0.0, 0.0, 400.0, 40.0), "Toolbar");
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(10.0, 5.0, 80.0, 30.0),
            "Button \"Save\"",
        );
        b.close();
        b.close();
        b.close();
        let frame = b.build();
        assert_eq!(frame.nodes.len(), 3);
        assert_eq!(frame.nodes[2].path, "Root/Toolbar/Button \"Save\"");
        assert_eq!(frame.chain(3), vec![1, 2, 3]);
        assert_eq!(frame.nodes[0].children, vec![2]);
    }

    #[test]
    fn every_push_records_the_calling_line() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        let frame = b.build();
        assert!(frame.nodes[0].loc.file.ends_with("frame.rs"));
        assert!(frame.nodes[0].loc.line > 0);
        assert!(frame.nodes[0].loc.column > 0);
    }

    #[test]
    fn clips_are_inherited_and_intersected() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.set_clip(rect(0.0, 0.0, 50.0, 50.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Inner");
        b.set_clip(rect(25.0, 25.0, 100.0, 100.0));
        let id = b.current().unwrap();
        b.close();
        b.close();
        let frame = b.build();
        let inner = frame.get(id).unwrap();
        assert_eq!(inner.clip, Some(rect(25.0, 25.0, 25.0, 25.0)));
    }

    #[test]
    fn the_visible_rect_is_the_clipped_rect() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.set_clip(rect(0.0, 0.0, 30.0, 30.0));
        let id = b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Big");
        b.close();
        b.close();
        let frame = b.build();
        let node = frame.get(id).unwrap();
        assert_eq!(node.visible_rect(), rect(0.0, 0.0, 30.0, 30.0));
    }

    /// 从网络恢复的帧可能携带任意 ID，而稠密索引没有 ID 0 的槽位——
    /// 旧代码从零减一并在索引时发生 panic。
    ///
    #[test]
    fn a_node_with_id_zero_is_still_addressable() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.close();
        let built = b.build();
        let mut nodes = built.nodes;
        nodes[0].id = 0;
        let frame = Frame::from_nodes(built.viewport, nodes);
        assert!(
            frame.get(0).is_some(),
            "a node with id 0 must still be found"
        );
    }

    /// 超大 ID 曾导致按 ID 本身大小分配索引：`u64::MAX` 会请求分配
    /// `usize::MAX` 个条目并直接中止进程。
    #[test]
    fn an_absurd_id_does_not_demand_an_absurd_allocation() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.close();
        let built = b.build();
        let mut nodes = built.nodes;
        nodes[0].id = u64::MAX;
        let frame = Frame::from_nodes(built.viewport, nodes);
        assert!(frame.get(u64::MAX).is_some());
    }

    /// 共享同一 ID 的两个节点无法同时存在于稠密索引中；
    /// 帧仍需正确解析它们而不使彼此产生别名覆盖。
    #[test]
    fn duplicate_ids_do_not_alias_and_do_not_panic() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.close();
        let built = b.build();
        let mut nodes = built.nodes;
        nodes[0].id = 1;
        let frame = Frame::from_nodes(built.viewport, nodes);
        assert!(frame.get(1).is_some());
    }

    /// 构建器生成的帧必须保留稠密快路径——检查器会对这些帧调用
    /// 数百万次 `get`。
    #[test]
    fn a_builder_frame_uses_the_dense_index() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        let id = b.open(NodeKind::Container, rect(0.0, 0.0, 10.0, 10.0), "Child");
        b.close();
        b.close();
        let frame = b.build();
        assert!(frame.sparse.is_none(), "a builder frame must stay dense");
        assert!(frame.get(id).is_some());
    }
}

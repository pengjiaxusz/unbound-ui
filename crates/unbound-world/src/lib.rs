//! DOD（面向数据设计）UI 世界树：扁平实体、侵入式兄弟链表、脏状态分级。
//!
//! `docs` §4.1 禁止指针树与 `Rc<RefCell<..>>` 图。此处所有数据均存放在
//! 由带代际（generational）[`NodeId`] 索引的扁平数组中：
//!
//! ```text
//! nodes:        [ N0 | N1 | N2 | N3 | ... ]   SlotMap<u32, NodeData>
//! first_child:  [ 1  | 2  | -  | -  | ... ]   intrusive, no Vec<Vec<T>>
//! next_sibling: [ -  | 3  | -  | -  | ... ]
//! ```
//!
//! 由此带来的关键特性：
//!
//! * 添加/移除子节点只需四次整数写入，绝不分配内存；
//! * 遍历子节点是在连续数组上的索引跳转；
//! * 已移除节点的槽位复用时会递增代际号，因此过期句柄会被检测到，
//!   而不会静默寻址到另一个节点。
//!
//! 脏状态按架构定义的三个层级拆分：
//! 仅重绘（paint-only：透明度、颜色、变换）、布局（RelayoutBoundary 内的尺寸/位置）
//! 与拓扑（树结构或视口变化）。只有最后一种才会强制触发全量重算。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::{FxHashMap, FxHashSet};
use slotmap::{new_key_type, Key, SlotMap};
use unbound_abi::{bitflags_like, NodeFlags, NodeTag, RawAction, RawNodeDesc};
use unbound_math::{project_rounded_radii, Rect, Vec2};

pub mod virtualize;

pub use virtualize::{TreeRow, VirtualGrid, VirtualList, VirtualTree, WindowUpdate};

new_key_type! {
    /// 节点的稳定代际句柄。
    pub struct NodeId;
}

impl NodeId {
    /// 当句柄为哨兵值“无节点”时返回 true。
    #[inline]
    #[must_use]
    pub fn is_none(self) -> bool {
        Key::is_null(&self)
    }
}

/// “无节点”哨兵值。定义为函数而非 `const`，因为 `slotmap` 1.x 中
/// `Key::null()` 尚非 `const fn`。
#[inline]
#[must_use]
pub fn no_node() -> NodeId {
    NodeId::null()
}

/// 应用一次变更的开销级别。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirtyLevel {
    /// 无任何变化。
    #[default]
    Clean = 0,
    /// 仅颜色 / 透明度 / 变换变化：无需重新布局，也无需重新上传几何数据。
    Paint = 1,
    /// `RelayoutBoundary` 内部的几何尺寸发生变更。
    Layout = 2,
    /// 树拓扑或视口尺寸发生变更：所有状态均需重新验证。
    Topology = 3,
}

impl DirtyLevel {
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if other as u8 > self as u8 {
            other
        } else {
            self
        }
    }
}

bitflags_like! {
    /// 每节点状态位标记，打包在单个 `u32` 中，使节点标记与几何信息
    /// 共同装入同一个缓存行。
    pub struct NodeState: u32 {
        /// 固定宽高且溢出不可见：子节点无法改变父节点尺寸，因此布局脏传播止步于此。
        ///
        const RELAYOUT_BOUNDARY = 1 << 0;
        /// 节点裁剪其子节点。
        const CLIPS_CHILDREN    = 1 << 1;
        /// 节点为可滚动容器。
        const SCROLLABLE        = 1 << 2;
        /// 节点当前正在执行动画（绕过像素网格吸附）。
        const ANIMATING         = 1 << 3;
        /// 节点连同其子树一并隐藏。
        const HIDDEN            = 1 << 4;
        /// 节点通过 `Portal` 逃逸出父节点的裁剪区域。
        const PORTAL            = 1 << 5;
        /// 至少已完成过一次布局计算。
        const LAID_OUT          = 1 << 6;
        /// 节点属于某个虚拟化窗口。
        const VIRTUALIZED       = 1 << 7;
        /// 节点绑定了交互动作。
        const INTERACTIVE       = 1 << 8;
    }
}

/// 引擎需要了解的单个节点的全部数据。
#[derive(Clone, Debug)]
pub struct NodeData {
    /// 宿主/声明式构建器发出的节点描述符。
    pub desc: RawNodeDesc,
    pub parent: NodeId,
    pub first_child: NodeId,
    pub last_child: NodeId,
    pub next_sibling: NodeId,
    pub prev_sibling: NodeId,
    /// 距根节点的深度，用于命中测试与无障碍访问。
    pub depth: u16,
    pub state: NodeState,
    /// 位于父节点坐标空间内的最终布局矩形（已含偏移）。
    pub rect: Rect,
    /// 将 `rect` 平移到 UI 层绝对世界空间的矩形。
    pub world_rect: Rect,
    /// 世界空间中的折叠硬裁剪矩形；`None` 表示“无裁剪”。
    pub clip: Option<Rect>,
    /// 最多两个圆角父级裁剪槽位，最内层优先。
    pub rounded_clips: [Option<RoundedClip>; 2],
    /// 预测量固有尺寸（文本、图像）。布局阶段从不调用文本引擎，
    /// 从而将文本塑形移出布局关键路径。
    pub measure: Option<Vec2>,
    /// `SCROLLABLE` 容器当前的滚动偏移量（逻辑像素）。
    ///
    /// 该偏移量会从子节点的世界位置中减去，使内容移动而容器自身保持原地。
    ///
    pub scroll_offset: Vec2,
    /// 容器内所有子节点范围的并集。
    ///
    /// 用于限制滚动范围：容器绝不能滚动超出其自身内容。
    ///
    pub content_size: Vec2,
    /// 生成此节点视图的状态单调代际号。代际号未变的子节点会被视图构建器跳过。
    ///
    pub state_generation: u64,
    /// 由 `#[track_caller]` 捕获的源码位置，用于内存检查器报告。
    pub source: &'static core::panic::Location<'static>,
    /// 激活时触发的绑定动作（若有）。
    pub action: Option<RawAction>,
}

impl NodeData {
    #[must_use]
    pub fn new(desc: RawNodeDesc, source: &'static core::panic::Location<'static>) -> Self {
        let interactive = desc.has(NodeFlags::INTERACTIVE);
        let mut state = NodeState::EMPTY;
        if interactive {
            state |= NodeState::INTERACTIVE;
        }
        Self {
            desc,
            parent: no_node(),
            first_child: no_node(),
            last_child: no_node(),
            next_sibling: no_node(),
            prev_sibling: no_node(),
            depth: 0,
            state,
            rect: Rect::ZERO,
            world_rect: Rect::ZERO,
            clip: None,
            rounded_clips: [None, None],
            measure: None,
            scroll_offset: Vec2::ZERO,
            content_size: Vec2::ZERO,
            state_generation: 0,
            source,
            action: None,
        }
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.state.contains(NodeState::HIDDEN) && self.desc.opacity > 0.001
    }

    #[must_use]
    pub fn is_relayout_boundary(&self) -> bool {
        self.state.contains(NodeState::RELAYOUT_BOUNDARY)
    }

    #[must_use]
    pub fn tag(&self) -> NodeTag {
        self.desc.kind()
    }
}

/// 从祖先节点继承的圆角裁剪。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoundedClip {
    pub rect: Rect,
    pub radii: [f32; 4],
}

/// 零内存分配的扁平化子节点迭代器。
#[derive(Copy, Clone, Debug)]
pub struct ChildIter<'a> {
    world: &'a UiWorld,
    next: NodeId,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }
        let current = self.next;
        self.next = self.world.nodes[current].next_sibling;
        Some(current)
    }
}

/// 祖先节点迭代器，用于裁剪链折叠与命中测试。
#[derive(Copy, Clone, Debug)]
pub struct AncestorIter<'a> {
    world: &'a UiWorld,
    next: NodeId,
}

impl<'a> Iterator for AncestorIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }
        let current = self.next;
        let parent = self.world.nodes[current].parent;
        self.next = parent;
        Some(current)
    }
}

/// 单次重建的统计信息，供调试覆盖层与测试使用。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub nodes: u32,
    pub paint_dirty: u32,
    pub layout_dirty: u32,
    pub topology_dirty: bool,
    pub relayout_boundaries: u32,
    pub nodes_visited: u32,
}

impl WorldStats {
    /// 当上一帧完全无需工作时返回 true。
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.paint_dirty == 0 && self.layout_dirty == 0 && !self.topology_dirty
    }
}

/// UI 世界树。
pub struct UiWorld {
    nodes: SlotMap<NodeId, NodeData>,
    texts: FxHashMap<NodeId, String>,
    paint_dirty: FxHashSet<NodeId>,
    layout_dirty: FxHashSet<NodeId>,
    topology_dirty: bool,
    root: NodeId,
    /// 用于收集重布局边界的临时缓冲区，每帧复用。
    boundary_scratch: Vec<NodeId>,
    /// 上一次脏传播期间访问过的节点数（用于诊断）。
    visited: u32,
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for UiWorld {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiWorld")
            .field("nodes", &self.nodes.len())
            .field("paint_dirty", &self.paint_dirty.len())
            .field("layout_dirty", &self.layout_dirty.len())
            .field("topology_dirty", &self.topology_dirty)
            .finish_non_exhaustive()
    }
}

impl UiWorld {
    /// 创建仅包含单个根节点的世界树。
    #[must_use]
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let mut root_data = NodeData::new(
            RawNodeDesc::container()
                .size(0.0, 0.0)
                .flags(NodeFlags::CLIP),
            core::panic::Location::caller(),
        );
        root_data.state |= NodeState::RELAYOUT_BOUNDARY | NodeState::CLIPS_CHILDREN;
        let root = nodes.insert(root_data);
        Self {
            nodes,
            texts: FxHashMap::default(),
            paint_dirty: FxHashSet::default(),
            layout_dirty: FxHashSet::default(),
            topology_dirty: true,
            root,
            boundary_scratch: Vec::new(),
            visited: 0,
        }
    }

    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&NodeData> {
        self.nodes.get(id)
    }

    #[must_use]
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut NodeData> {
        self.nodes.get_mut(id)
    }

    /// # Panic 说明
    /// 当 `id` 不是活跃节点时 panic；可使用 [`Self::contains`] 预先检查。
    #[must_use]
    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id]
    }

    /// 将新节点作为 `parent` 的最后一个子节点插入。
    ///
    /// `source` 应为 `core::panic::Location::caller()`——得益于 `#[track_caller]`，
    /// 视图构建器会自动传入它，使检查器报告能精准指向构造源码位置。
    ///
    pub fn insert(
        &mut self,
        parent: NodeId,
        desc: RawNodeDesc,
        source: &'static core::panic::Location<'static>,
    ) -> NodeId {
        let depth = if parent.is_null() {
            0
        } else {
            self.nodes.get(parent).map_or(0, |p| p.depth + 1)
        };
        let mut data = NodeData::new(desc, source);
        data.parent = parent;
        data.depth = depth;
        // 边界启发式规则（`docs` §4.2）：固定尺寸且溢出不可见的容器
        // 不会被子节点改变尺寸，因此布局脏标记止步于此。
        let fixed = matches!(desc.width.kind(), unbound_abi::DimUnit::Px)
            && matches!(desc.height.kind(), unbound_abi::DimUnit::Px);
        if fixed && desc.is_clipping() {
            data.state |= NodeState::RELAYOUT_BOUNDARY;
        }
        if desc.is_clipping() {
            data.state |= NodeState::CLIPS_CHILDREN;
        }
        if desc.has(NodeFlags::SCROLL_Y) || desc.has(NodeFlags::SCROLL_X) {
            data.state |= NodeState::SCROLLABLE;
        }
        if desc.kind() == NodeTag::Portal {
            data.state |= NodeState::PORTAL;
        }
        if desc.has(NodeFlags::HIDDEN) {
            data.state |= NodeState::HIDDEN;
        }
        if desc.has(NodeFlags::ANIMATED) {
            data.state |= NodeState::ANIMATING;
        }

        let id = self.nodes.insert(data);
        if !parent.is_null() {
            self.link_last(parent, id);
        } else {
            self.root = id;
        }
        if let Some(p) = self.nodes.get_mut(parent) {
            p.state.remove(NodeState::LAID_OUT);
        }
        self.topology_dirty = true;
        id
    }

    fn link_last(&mut self, parent: NodeId, child: NodeId) {
        let last = self.nodes[parent].last_child;
        {
            let p = &mut self.nodes[parent];
            p.last_child = child;
            if p.first_child.is_null() {
                p.first_child = child;
            }
        }
        {
            let c = &mut self.nodes[child];
            c.parent = parent;
            c.prev_sibling = last;
            c.next_sibling = no_node();
        }
        if !last.is_null() {
            self.nodes[last].next_sibling = child;
        }
    }

    fn unlink(&mut self, id: NodeId) {
        let (parent, prev, next) = {
            let n = &self.nodes[id];
            (n.parent, n.prev_sibling, n.next_sibling)
        };
        if !prev.is_null() {
            self.nodes[prev].next_sibling = next;
        } else if !parent.is_null() {
            self.nodes[parent].first_child = next;
        }
        if !next.is_null() {
            self.nodes[next].prev_sibling = prev;
        } else if !parent.is_null() {
            self.nodes[parent].last_child = prev;
        }
        {
            let n = &mut self.nodes[id];
            n.prev_sibling = no_node();
            n.next_sibling = no_node();
            n.parent = no_node();
        }
    }

    /// 移除节点及其整棵子树。
    pub fn remove(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || id == self.root {
            return;
        }
        self.unlink(id);
        // 无递归深度优先移除（百万节点文档中深层树否则会栈溢出）。
        //
        let mut stack = vec![id];
        let mut doomed = Vec::new();
        while let Some(current) = stack.pop() {
            doomed.push(current);
            let mut child = self.nodes[current].first_child;
            while !child.is_null() {
                stack.push(child);
                child = self.nodes[child].next_sibling;
            }
        }
        for n in doomed {
            self.texts.remove(&n);
            self.paint_dirty.remove(&n);
            self.layout_dirty.remove(&n);
            self.nodes.remove(n);
        }
        self.topology_dirty = true;
    }

    /// 将节点从其父节点摘除但不销毁（用于 Portal 领养或虚拟化槽位复用）。
    ///
    pub fn detach(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }
        self.unlink(id);
        self.topology_dirty = true;
    }

    /// 将 `id` 移动到 `new_parent` 下作为最后一个子节点。
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId) {
        if id == new_parent || !self.nodes.contains_key(id) || !self.nodes.contains_key(new_parent)
        {
            return;
        }
        // 拒绝成环：从新父节点向上回溯绝不能遇到 `id`。
        if self.ancestors(new_parent).any(|a| a == id) {
            return;
        }
        self.unlink(id);
        self.link_last(new_parent, id);
        self.refresh_depth(id);
        self.topology_dirty = true;
    }

    fn refresh_depth(&mut self, id: NodeId) {
        let depth = self.nodes[id].parent;
        let base = if depth.is_null() {
            0
        } else {
            self.nodes[depth].depth + 1
        };
        let mut stack = vec![(id, base)];
        while let Some((current, d)) = stack.pop() {
            self.nodes[current].depth = d;
            let mut child = self.nodes[current].first_child;
            while !child.is_null() {
                stack.push((child, d + 1));
                child = self.nodes[child].next_sibling;
            }
        }
    }

    /// 零分配地迭代节点的直接子节点。
    #[must_use]
    pub fn children(&self, id: NodeId) -> ChildIter<'_> {
        ChildIter {
            world: self,
            next: if self.nodes.contains_key(id) {
                self.nodes[id].first_child
            } else {
                no_node()
            },
        }
    }

    /// 依次迭代 `id` 及其向上直至根节点的全部祖先。
    #[must_use]
    pub fn ancestors(&self, id: NodeId) -> AncestorIter<'_> {
        AncestorIter {
            world: self,
            next: id,
        }
    }

    /// 深度优先前序遍历。
    ///
    /// 兄弟节点按插入顺序（从左到右）输出，且整棵子树会在下一个兄弟节点之前输出。
    ///
    pub fn walk(&self, root: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = vec![root];
        core::iter::from_fn(move || {
            let id = stack.pop()?;
            if !self.nodes.contains_key(id) {
                return Some(id);
            }
            let mut kids = Vec::new();
            let mut child = self.nodes[id].first_child;
            while !child.is_null() {
                kids.push(child);
                child = self.nodes[child].next_sibling;
            }
            // 栈为后进先出，因此必须逆序压入子节点才能按从左到右的顺序弹出。
            //
            for kid in kids.into_iter().rev() {
                stack.push(kid);
            }
            Some(id)
        })
    }

    /// 直接子节点数量。
    #[must_use]
    pub fn child_count(&self, id: NodeId) -> usize {
        self.children(id).count()
    }

    /// 按稳定语义键查找节点。
    ///
    /// 语义键是 diff/记忆化句柄，因此实践中映射是一对一的；
    /// 若构建器误复用了同一个键，则返回首个匹配项。
    #[must_use]
    pub fn find_key(&self, key: u64) -> Option<NodeId> {
        if key == 0 {
            return None;
        }
        self.walk(self.root())
            .find(|id| self.node(*id).desc.key == key)
    }

    /// 当节点布局描述符发生变更时替换它，并标记布局为脏。
    pub fn set_desc(&mut self, id: NodeId, desc: RawNodeDesc) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.desc == desc {
            return;
        }
        let was_boundary = node.is_relayout_boundary();
        node.desc = desc;
        let is_boundary = node.is_relayout_boundary();
        if was_boundary != is_boundary {
            self.mark_topology_dirty();
        } else {
            self.mark_layout_dirty(id);
        }
    }

    /// 为节点附加文本（替换先前的文本）。
    pub fn set_text(&mut self, id: NodeId, text: impl Into<String>) {
        let text = text.into();
        if self.texts.get(&id) == Some(&text) {
            return;
        }
        self.texts.insert(id, text);
        self.mark_layout_dirty(id);
    }

    #[must_use]
    pub fn text(&self, id: NodeId) -> Option<&str> {
        self.texts.get(&id).map(String::as_str)
    }

    /// 为节点绑定交互动作。
    pub fn bind_action(&mut self, id: NodeId, action: RawAction) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.action = Some(action);
            n.state |= NodeState::INTERACTIVE;
        }
        self.mark_paint_dirty(id);
    }

    /// 标记节点为仅重绘脏并立即停止：祖先节点不关心颜色变更，因此无需向上传播。
    ///
    pub fn mark_paint_dirty(&mut self, id: NodeId) {
        if self.nodes.contains_key(id) {
            self.paint_dirty.insert(id);
        }
    }

    /// 标记节点为布局脏，并向上传播**直至最近的重布局边界**（`docs` §4.2）。
    /// 这是增量开销模型的核心：固定尺寸卡片内的文本变更只需重算一张卡片，而非整棵树。
    ///
    pub fn mark_layout_dirty(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        self.layout_dirty.insert(id);
        // 边界自身即为其停止点。
        if node.is_relayout_boundary() {
            return;
        }
        let mut current = node.parent;
        self.visited = 0;
        while !current.is_null() {
            self.visited += 1;
            let Some(parent) = self.nodes.get(current) else {
                break;
            };
            self.layout_dirty.insert(current);
            if parent.is_relayout_boundary() {
                break;
            }
            current = parent.parent;
        }
        // 子节点变大可能会改变祖先的测量尺寸，但仅限于边界之内；
        // 越过边界后尺寸按定义是固定的。
    }

    /// 强制全量重新布局（用于视口调整或根节点样式变更）。
    pub fn mark_topology_dirty(&mut self) {
        self.topology_dirty = true;
    }

    #[must_use]
    pub fn topology_dirty(&self) -> bool {
        self.topology_dirty
    }

    /// 消费并清空重绘脏节点集合（此后由绘制 Pass 接管）。
    pub fn take_paint_dirty(&mut self) -> FxHashSet<NodeId> {
        core::mem::take(&mut self.paint_dirty)
    }

    /// 需要布局求解的脏节点，按祖先优先排序。
    #[must_use]
    pub fn layout_dirty_sorted(&mut self) -> Vec<NodeId> {
        let mut list: Vec<NodeId> = self.layout_dirty.drain().collect();
        list.sort_by_key(|id| self.nodes.get(*id).map_or(0, |n| n.depth));
        list
    }

    /// 按深度排序的布局脏节点集合只读视图（不排空）。
    #[must_use]
    pub fn layout_dirty_snapshot(&self) -> Vec<NodeId> {
        let mut list: Vec<NodeId> = self.layout_dirty.iter().copied().collect();
        list.sort_by_key(|id| self.nodes.get(*id).map_or(0, |n| n.depth));
        list
    }

    /// 收集待处理布局脏节点所触及的重布局边界。
    ///
    /// 这些子树彼此独立，因此布局引擎可以并行求解它们。
    ///
    #[must_use]
    pub fn dirty_boundaries(&mut self) -> Vec<NodeId> {
        self.boundary_scratch.clear();
        let dirty: Vec<NodeId> = self.layout_dirty.drain().collect();
        let mut boundaries = Vec::new();
        for id in dirty {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.is_relayout_boundary() {
                boundaries.push(id);
                continue;
            }
            // 查找最近的边界祖先；若不存在则交由根节点处理。
            //
            let mut current = node.parent;
            let mut found = no_node();
            while !current.is_null() {
                let Some(p) = self.nodes.get(current) else {
                    break;
                };
                if p.is_relayout_boundary() {
                    found = current;
                    break;
                }
                current = p.parent;
            }
            boundaries.push(if found.is_null() { self.root } else { found });
        }
        // 按深度排序使祖先先于后代求解，随后去重。`dedup` 仅移除*相邻*的重复元素，
        // 因此排序必须把相同边界聚在一起：仅按深度排序可能会让两个同深度边界之间
        // 夹杂第三个边界，导致重复项保留——进而让布局引擎对同一边界求解两次。
        // 加入 id 作为次级排序键使顺序全序化，从而保证重复边界必然相邻。
        //
        //
        //
        boundaries.sort_by_key(|id| (self.nodes.get(*id).map_or(0, |n| n.depth), *id));
        boundaries.dedup();
        boundaries
    }

    /// 统计重布局边界数量（用于诊断与测试）。
    #[must_use]
    pub fn relayout_boundary_count(&self) -> usize {
        self.nodes
            .values()
            .filter(|n| n.is_relayout_boundary())
            .count()
    }

    /// 上一次脏传播的统计信息。
    #[must_use]
    pub fn stats(&self) -> WorldStats {
        WorldStats {
            nodes: self.nodes.len() as u32,
            paint_dirty: self.paint_dirty.len() as u32,
            layout_dirty: self.layout_dirty.len() as u32,
            topology_dirty: self.topology_dirty,
            relayout_boundaries: self.relayout_boundary_count() as u32,
            nodes_visited: self.visited,
        }
    }

    /// 清除所有脏标记（完整帧结束后调用）。
    pub fn clear_dirty(&mut self) {
        self.paint_dirty.clear();
        self.layout_dirty.clear();
        self.topology_dirty = false;
        self.visited = 0;
    }

    /// 将计算得到的布局写回世界树，并折叠裁剪链。
    ///
    /// 裁剪折叠**在 CPU 上一次性完成**，因此着色器只需处理单个 AABB
    /// 外加两个圆角槽位（`docs` §3.2）。
    pub fn apply_rects(&mut self, rects: &FxHashMap<NodeId, Rect>) {
        for (id, rect) in rects {
            if let Some(node) = self.nodes.get_mut(*id) {
                node.rect = *rect;
                node.state |= NodeState::LAID_OUT;
            }
        }
        let root = self.root;
        self.fold_clips(root);
    }

    /// 应用节点子集的矩形结果，并从 `start` 开始重新折叠裁剪链。
    ///
    /// 增量求解只触及少数节点；若为刷新 `world_rect`/`clip` 而遍历整棵树，
    /// 会让 9 个节点的重布局付出与全量布局相同的代价。
    ///
    pub fn apply_rects_from(
        &mut self,
        rects: &FxHashMap<NodeId, Rect>,
        changed: &[NodeId],
        start: NodeId,
    ) {
        for id in changed {
            if let Some(rect) = rects.get(id) {
                if let Some(node) = self.nodes.get_mut(*id) {
                    node.rect = *rect;
                    node.state |= NodeState::LAID_OUT;
                }
            }
        }
        self.fold_clips(start);
    }

    fn fold_clips(&mut self, start: NodeId) {
        // 增量重折叠从树的*中间*开始，因此 `start` 从祖先继承的硬裁剪与圆角槽位
        // 无法在局部推导——必须从其父节点获取（父节点已存有向下传递给子节点的完整裁剪）。
        //
        //
        //
        // 若从空裁剪开始，子树会遗忘其祖先：属于滚动容器或圆角面板内部的内容
        // 在增量重布局后会逃逸出去，因为折叠过程只看到了边界自身的包围盒。
        //
        //
        let (inherited_clip, inherited_rounded) = match self.nodes.get(start) {
            Some(node) if !node.parent.is_null() => match self.nodes.get(node.parent) {
                Some(parent) => (parent.clip, parent.rounded_clips),
                None => (None, [None; 2]),
            },
            _ => (None, [None; 2]),
        };
        let mut stack = vec![(start, inherited_clip, inherited_rounded)];
        while let Some((id, inherited_clip, inherited_rounded)) = stack.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let world = if node.parent.is_null() {
                node.rect
            } else {
                let (parent_origin, parent_scroll) = self
                    .nodes
                    .get(node.parent)
                    .map_or((Rect::ZERO, Vec2::ZERO), |p| {
                        (p.world_rect, p.scroll_offset)
                    });
                // 父节点的滚动偏移移动的是内容而非容器本身：子节点随滚动而移动。
                //
                node.rect.translate(parent_origin.origin - parent_scroll)
            };
            let mut clip = inherited_clip;
            let mut rounded = inherited_rounded;

            // Portal 会逃逸所有祖先裁剪（`docs` §6.3）：它绘制在浮层中，
            // 因此继承父节点的裁剪矩形是错误的。
            if node.state.contains(NodeState::PORTAL) {
                clip = None;
                rounded = [None; 2];
            }

            if node.state.contains(NodeState::CLIPS_CHILDREN)
                && !node.state.contains(NodeState::PORTAL)
            {
                let own = world;
                clip = Some(match clip {
                    Some(c) => c.intersect(&own),
                    None => own,
                });
                // 着色器最多支持测试两个圆角槽位。若压入第三个嵌套容器会静默丢弃最外层圆弧，
                // 因此将被丢弃的圆角半径折叠进保留槽位中：矩形求交会把外层圆弧转化为
                // 保留容器角上的较小等效半径。
                //
                //
                if node.desc.radius.iter().any(|r| *r > 0.0) {
                    let own = RoundedClip {
                        rect: world,
                        radii: node.desc.radius,
                    };
                    rounded = match (rounded[0], rounded[1]) {
                        (Some(mut kept), Some(dropped)) => {
                            kept.radii = project_rounded_radii(
                                kept.rect,
                                kept.radii,
                                dropped.rect,
                                dropped.radii,
                            );
                            [Some(own), Some(kept)]
                        }
                        (Some(kept), None) => [Some(own), Some(kept)],
                        (None, _) => [Some(own), None],
                    };
                }
            }

            // 内容尺寸为所有子节点范围的并集：它限制了滚动范围，
            // 因此必须在钳制滚动偏移前完成测量。
            //
            let mut child = node.first_child;
            let mut kids = Vec::new();
            let mut content = Vec2::ZERO;
            while !child.is_null() {
                kids.push(child);
                if let Some(kid) = self.nodes.get(child) {
                    content = content.max(kid.rect.max());
                }
                child = self.nodes[child].next_sibling;
            }

            let node = self.nodes.get_mut(id).expect("checked above");
            node.world_rect = world;
            node.clip = clip;
            node.rounded_clips = rounded;
            node.content_size = content;

            // 逆序压栈，使弹出顺序保持从左到右。
            for k in kids.into_iter().rev() {
                stack.push((k, clip, rounded));
            }
        }
    }

    /// 设置可滚动容器的滚动偏移量。
    ///
    /// 偏移量会被钳制在容器内容范围内，确保滚动绝不会在末尾留出空白。
    /// 当偏移量发生实际变化时返回 `true`。
    pub fn set_scroll(&mut self, id: NodeId, offset: Vec2) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if !node.state.contains(NodeState::SCROLLABLE) {
            return false;
        }
        let max_x = (node.content_size.x - node.rect.size.x).max(0.0);
        let max_y = (node.content_size.y - node.rect.size.y).max(0.0);
        let clamped = Vec2::new(offset.x.clamp(0.0, max_x), offset.y.clamp(0.0, max_y));
        if (node.scroll_offset - clamped).length_squared() < 1e-4 {
            return false;
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        node.scroll_offset = clamped;
        self.fold_clips(id);
        self.mark_paint_dirty(id);
        true
    }

    /// 按增量 `delta` 逻辑像素滚动容器（`delta.y > 0` 表示向下滚动）。
    pub fn scroll_by(&mut self, id: NodeId, delta: Vec2) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        let target = node.scroll_offset + delta;
        self.set_scroll(id, target)
    }

    /// 朴素全量重布局需访问的节点总数——测试中用作增量路径的对比基线。
    ///
    #[must_use]
    pub fn full_layout_cost(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::{Dim, Edges};

    fn loc() -> &'static core::panic::Location<'static> {
        core::panic::Location::caller()
    }

    fn world() -> (UiWorld, NodeId) {
        let w = UiWorld::new();
        let root = w.root();
        (w, root)
    }

    #[test]
    fn children_are_linked_in_insertion_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, b, c]);
        assert_eq!(w.node(a).next_sibling, b);
        assert_eq!(w.node(b).prev_sibling, a);
        assert_eq!(w.node(c).prev_sibling, b);
        assert_eq!(w.node(root).first_child, a);
        assert_eq!(w.node(root).last_child, c);
        assert_eq!(w.node(a).depth, 1);
        assert_eq!(w.node(c).depth, 1);
    }

    #[test]
    fn removing_a_middle_child_repairs_the_links() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        w.remove(b);
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, c]);
        assert_eq!(w.node(a).next_sibling, c);
        assert_eq!(w.node(c).prev_sibling, a);
        assert_eq!(w.len(), 3, "root + a + c");
        // 过期句柄必须被拒绝，绝不能被静默复用。
        assert!(!w.contains(b));
    }

    #[test]
    fn removing_a_subtree_frees_every_descendant() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let a2 = w.insert(a, RawNodeDesc::container(), loc());
        let a21 = w.insert(a2, RawNodeDesc::container(), loc());
        assert_eq!(w.len(), 5);
        w.remove(a);
        assert_eq!(w.len(), 1);
        for id in [a, a1, a2, a21] {
            assert!(!w.contains(id));
        }
    }

    /// 可滚动容器移动其内容，且滚动范围受内容边界限制。
    /// 若未应用偏移量，子节点会留在原地：容器声明了可滚动，但内容纹丝不动。
    ///
    ///
    ///
    #[test]
    fn scrolling_a_container_translates_its_children_and_clamps() {
        let (mut w, root) = world();
        let container = w.insert(root, RawNodeDesc::container(), loc());
        let child = w.insert(container, RawNodeDesc::container(), loc());
        if let Some(node) = w.get_mut(container) {
            node.state |= NodeState::SCROLLABLE;
            node.rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        }
        if let Some(node) = w.get_mut(child) {
            node.rect = Rect::new(0.0, 0.0, 80.0, 400.0);
        }
        w.apply_rects_from(&FxHashMap::default(), &[container, child], root);

        let before = w.get(child).expect("child").world_rect;
        assert_eq!(before.origin.y, 0.0);

        // 100 高的容器内有 400 高的内容：可用滚动范围为 300 px。
        //
        assert_eq!(w.get(container).expect("container").content_size.y, 400.0);
        assert!(w.set_scroll(container, Vec2::new(0.0, 120.0)));
        let after = w.get(child).expect("child").world_rect;
        assert_eq!(after.origin.y, -120.0, "the content follows the scroll");

        // 滚动偏移被钳制在内容范围内。
        assert!(w.set_scroll(container, Vec2::new(0.0, 9999.0)));
        assert_eq!(w.get(container).expect("container").scroll_offset.y, 300.0);
        let clamped = w.get(child).expect("child").world_rect;
        assert_eq!(clamped.origin.y, -300.0);

        // 不可滚动的容器不会发生滚动。
        assert!(!w.set_scroll(child, Vec2::new(0.0, 10.0)));
    }

    /// `scroll_by` 是 `set_scroll` 的相对位移形式。
    #[test]
    fn scrolling_by_a_delta_moves_the_content() {
        let (mut w, root) = world();
        let container = w.insert(root, RawNodeDesc::container(), loc());
        let child = w.insert(container, RawNodeDesc::container(), loc());
        if let Some(node) = w.get_mut(container) {
            node.state |= NodeState::SCROLLABLE;
            node.rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        }
        if let Some(node) = w.get_mut(child) {
            node.rect = Rect::new(0.0, 0.0, 80.0, 400.0);
        }
        w.apply_rects_from(&FxHashMap::default(), &[container, child], root);
        assert!(w.scroll_by(container, Vec2::new(0.0, 50.0)));
        assert_eq!(w.get(container).expect("container").scroll_offset.y, 50.0);
        assert_eq!(w.get(child).expect("child").world_rect.origin.y, -50.0);
    }

    #[test]
    fn generations_are_bumped_on_slot_reuse() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        w.remove(a);
        let b = w.insert(root, RawNodeDesc::container(), loc());
        assert_ne!(a, b, "a reused slot must get a new generation");
        assert!(!w.contains(a));
        assert!(w.contains(b));
    }

    #[test]
    fn reparenting_preserves_order_and_updates_depth() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        assert_eq!(w.node(a1).depth, 2);
        w.reparent(a1, b);
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(w.children(b).collect::<Vec<_>>(), vec![a1]);
        assert_eq!(w.node(a1).parent, b);
        assert_eq!(w.node(a1).depth, 2);
    }

    #[test]
    fn reparenting_refuses_to_create_a_cycle() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(a, RawNodeDesc::container(), loc());
        w.reparent(a, b);
        assert_eq!(w.node(a).parent, root, "cycle must be refused");
        assert_eq!(w.node(b).parent, a);
    }

    #[test]
    fn a_fixed_clipping_container_becomes_a_relayout_boundary() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(320.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        assert!(w.node(card).is_relayout_boundary());
        let free = w.insert(root, RawNodeDesc::container(), loc());
        assert!(!w.node(free).is_relayout_boundary());
    }

    #[test]
    fn layout_dirt_stops_at_the_nearest_boundary() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(320.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(card, RawNodeDesc::container(), loc());
        let label = w.insert(inner, RawNodeDesc::text(), loc());
        w.clear_dirty();

        w.mark_layout_dirty(label);
        let dirty = w.layout_dirty_snapshot();
        assert!(dirty.contains(&label));
        assert!(dirty.contains(&inner));
        assert!(dirty.contains(&card));
        assert!(!dirty.contains(&root), "root must not be dirtied");
        assert!(w.stats().nodes_visited <= 4);

        // `dirty_boundaries` 会排空集合，因此需在快照之后执行。
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![card]);
        assert!(w.layout_dirty_snapshot().is_empty());
    }

    #[test]
    fn paint_dirt_never_propagates() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(card, RawNodeDesc::text(), loc());
        w.clear_dirty();
        w.mark_paint_dirty(label);
        let stats = w.stats();
        assert_eq!(stats.paint_dirty, 1);
        assert_eq!(stats.layout_dirty, 0);
        assert!(!stats.topology_dirty);
    }

    #[test]
    fn clips_fold_through_the_ancestor_chain() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(200.0, 200.0)
                .radius(12.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 400.0, 400.0));
        rects.insert(inner, Rect::new(20.0, 20.0, 200.0, 200.0));
        rects.insert(leaf, Rect::new(5.0, 5.0, 50.0, 50.0));
        w.apply_rects(&rects);

        let leaf_node = w.node(leaf);
        // 世界坐标：world = root.origin(0) + outer.origin(10) + inner.origin(20) + leaf(5)
        assert_eq!(leaf_node.world_rect, Rect::new(35.0, 35.0, 50.0, 50.0));
        // 裁剪区域为两个祖先裁剪在世界空间中的交集。
        let clip = leaf_node.clip.expect("clipped");
        assert_eq!(clip, Rect::new(30.0, 30.0, 200.0, 200.0));
        // 最内层的圆角容器位于槽位 0。
        let rounded = leaf_node.rounded_clips[0].expect("rounded clip");
        assert_eq!(rounded.radii, [12.0; 4]);
        assert_eq!(rounded.rect, Rect::new(30.0, 30.0, 200.0, 200.0));
    }

    #[test]
    fn a_third_nested_rounded_clip_is_folded_into_the_second_slot() {
        let (mut w, root) = world();
        let build = |w: &mut UiWorld, parent, size: f32, radius: f32| {
            w.insert(
                parent,
                RawNodeDesc::container()
                    .size(size, size)
                    .radius(radius)
                    .flags(NodeFlags::CLIP),
                loc(),
            )
        };
        let outer = build(&mut w, root, 400.0, 40.0);
        let middle = build(&mut w, outer, 380.0, 16.0);
        let inner = build(&mut w, middle, 360.0, 8.0);
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(0.0, 0.0, 400.0, 400.0));
        rects.insert(middle, Rect::new(10.0, 10.0, 380.0, 380.0));
        rects.insert(inner, Rect::new(20.0, 20.0, 360.0, 360.0));
        rects.insert(leaf, Rect::new(0.0, 0.0, 10.0, 10.0));
        w.apply_rects(&rects);

        let slots = w.node(leaf).rounded_clips;
        assert_eq!(slots[0].expect("inner").radii, [8.0; 4]);
        let folded = slots[1].expect("middle slot kept");
        assert_eq!(folded.rect, Rect::new(10.0, 10.0, 380.0, 380.0));
        // 被丢弃的 40px 外层圆弧投影到保留的中间容器上变为 40 - 10 = 30px，
        // 因此外层圆角依然可见。
        assert_eq!(folded.radii, [30.0; 4]);
    }

    #[test]
    fn portals_escape_ancestor_clips() {
        let (mut w, root) = world();
        let panel = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let portal = w.insert(panel, RawNodeDesc::container().tag(NodeTag::Portal), loc());
        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(panel, Rect::new(0.0, 0.0, 100.0, 100.0));
        rects.insert(portal, Rect::new(500.0, 500.0, 80.0, 80.0));
        w.apply_rects(&rects);
        assert!(
            w.node(portal).clip.is_none(),
            "a portal must not be clipped"
        );
    }

    #[test]
    fn text_is_stored_once_and_dedups() {
        let (mut w, root) = world();
        let label = w.insert(root, RawNodeDesc::text(), loc());
        w.set_text(label, "hello");
        w.clear_dirty();
        w.set_text(label, "hello");
        let stats = w.stats();
        // 第二次设置不得产生脏标记：对字符串执行比较并交换（compare-and-set）。
        assert_eq!(stats.layout_dirty, 0);
        assert_eq!(w.text(label), Some("hello"));
    }

    #[test]
    fn walk_visits_every_node_once_in_pre_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(visited.len(), 4);
        assert_eq!(visited[0], root);
        assert!(visited.contains(&a1));
        assert!(visited.contains(&b));
    }

    /// 兄弟节点必须按插入顺序输出。
    ///
    /// 遍历过程将子节点压入栈中，而栈是后进先出的——若不逆序压入，
    /// 每组兄弟节点都会按**从右到左**的顺序输出。函数注释曾承诺从左到右，
    /// 且代码库中另外两次树遍历（此处的 `fold_clips` 与 `unbound-layout` 中的 `preorder`）
    /// 均执行了逆序。按树遍历进行绘制或命中测试的调用方会看到所有节点的子节点顺序颠倒。
    ///
    ///
    ///
    #[test]
    fn walk_yields_siblings_in_insertion_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(
            visited,
            vec![root, a, b, c],
            "siblings must be visited left-to-right"
        );
    }

    /// 更深层树也需保持同样顺序保证：整棵子树必须在下一个兄弟节点之前输出。
    ///
    #[test]
    fn walk_yields_a_whole_subtree_before_the_next_sibling() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let a2 = w.insert(a, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(visited, vec![root, a, a1, a2, b]);
    }

    /// 增量求解不得丢失边界上方的裁剪链。
    ///
    /// `apply_rects_from` 仅重折叠边界子树，但该子树的*有效*裁剪是所有裁剪祖先的交集。
    /// 从空继承裁剪开始折叠会导致子树遗忘祖先：本应位于滚动容器或圆角面板内的内容
    /// 在增量重布局后会逃逸出来。
    ///
    ///
    ///
    #[test]
    fn an_incremental_refold_keeps_the_ancestor_clip_chain() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        // 内层容器比外层容器*更宽*，因此“被祖先裁剪”与“被自身裁剪”的区别清晰可见。
        //
        //
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 100.0, 100.0));
        rects.insert(inner, Rect::new(0.0, 0.0, 400.0, 400.0));
        rects.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects(&rects);

        // 全量折叠将叶子节点裁剪到外层容器。
        let full = w.node(leaf).clip.expect("clipped");
        assert_eq!(full, Rect::new(10.0, 10.0, 100.0, 100.0));

        // 现在仅重折叠内层子树，模拟增量布局求解的路径。
        //
        let mut changed = FxHashMap::default();
        changed.insert(inner, Rect::new(0.0, 0.0, 400.0, 400.0));
        changed.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects_from(&changed, &[inner, leaf], inner);

        let incremental = w.node(leaf).clip.expect("still clipped");
        assert_eq!(
            incremental, full,
            "the incremental refold dropped the ancestor clip chain"
        );
    }

    /// 圆角槽位同样适用此保证：增量重折叠不得遗忘边界上方的圆角祖先。
    ///
    #[test]
    fn an_incremental_refold_keeps_the_ancestor_rounded_clips() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .radius(12.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(200.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 400.0, 400.0));
        rects.insert(inner, Rect::new(0.0, 0.0, 200.0, 200.0));
        rects.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects(&rects);
        assert_eq!(
            w.node(leaf).rounded_clips[0].expect("rounded").radii,
            [12.0; 4]
        );

        let mut changed = FxHashMap::default();
        changed.insert(inner, Rect::new(0.0, 0.0, 200.0, 200.0));
        changed.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects_from(&changed, &[inner, leaf], inner);

        assert_eq!(
            w.node(leaf).rounded_clips[0].expect("rounded").radii,
            [12.0; 4],
            "the incremental refold dropped the ancestor rounded clip"
        );
    }

    /// 边界列表绝不能两次列出同一个边界。
    ///
    /// 此前边界按深度排序后直接 `dedup`——但 `dedup` 仅移除*相邻*的重复项。
    /// 若两个同深度边界之间夹了第三个边界，重复项就会保留下来，
    /// 导致布局引擎对同一边界求解两次。
    ///
    #[test]
    fn dirty_boundaries_never_repeat_a_boundary() {
        let (mut w, root) = world();
        // 两个相同深度的边界。
        let a = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let b = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let a_leaf = w.insert(a, RawNodeDesc::text(), loc());
        let a_leaf2 = w.insert(a, RawNodeDesc::text(), loc());
        let b_leaf = w.insert(b, RawNodeDesc::text(), loc());

        // `a` 与 `b` 内部均有脏节点。`a` 被记录了两次——对应其内部每个脏节点一次。
        //
        w.mark_layout_dirty(a_leaf);
        w.mark_layout_dirty(a_leaf2);
        w.mark_layout_dirty(b_leaf);

        let boundaries = w.dirty_boundaries();
        let mut unique = boundaries.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            boundaries.len(),
            unique.len(),
            "the same boundary was listed twice: {boundaries:?}"
        );
        assert!(boundaries.contains(&a));
        assert!(boundaries.contains(&b));
    }

    #[test]
    fn dirty_levels_are_ordered() {
        assert!(DirtyLevel::Topology > DirtyLevel::Layout);
        assert!(DirtyLevel::Layout > DirtyLevel::Paint);
        assert_eq!(
            DirtyLevel::Clean.max(DirtyLevel::Layout),
            DirtyLevel::Layout
        );
        assert_eq!(
            DirtyLevel::Topology.max(DirtyLevel::Paint),
            DirtyLevel::Topology
        );
    }

    #[test]
    fn edge_values_do_not_leak_into_the_tree() {
        let (mut w, root) = world();
        let a = w.insert(
            root,
            RawNodeDesc::container()
                .padding(Edges::all(4.0))
                .width(Dim::percent(50.0)),
            loc(),
        );
        assert_eq!(w.children(root).count(), 1);
        assert!(w.contains(a));
        assert_eq!(w.node(a).desc.padding.left, 4.0);
    }
}

//! 基于扁平 SoA 数组与侵入式链表的松散四叉树（LooseQuadtree，`docs` §5.1）。
//!
//!
//! 为何不用固定尺寸的空间哈希：画布同时混有 8 px 图标与 40 000 px 分组框，
//! 任何固定网格尺寸都会退化为每格一个桶或一个巨型桶。为何不用指针四叉树：
//! `Vec<Box<Node>>` 每次访问子节点都会缓存未命中，且每次分裂都要分配内存。
//!
//!
//! 本实现的特点：
//!
//! * **线性（linear）** — 节点通过 `(depth, morton_index)` 寻址，
//!   完全没有子节点指针，只有指向各层级数组的 `u32` 索引；
//! * **松散（loose）** — 每个单元格向外膨胀 2 倍，因此尺寸不超过标准单元格的元素
//!   总能完整落入单个节点内而无需分裂。深度为 `floor(log2(world / extent))`——
//!   即等价于文档中 `leading_zeros` 的 O(1) 决策，无需逐层下探；
//! * **侵入式（intrusive）** — 元素挂接在存储在平行 `Vec<u32>` 中的每节点链表上，
//!   因此移动元素只需少数几次整数写入，绝不分配内存。
//!
//!
//!
//! 另有两个辅助结构用于保持查询低开销：
//!
//! * `level_occupied` 仅记录实际持有元素的节点，因此查询开销为
//!   `O(非空节点数)` 而非 `O(4^MAX_DEPTH)`——在深度 8 下稠密扫描每次查询需访问 87 381 个节点；
//!
//! * `level_slot` 将节点索引映射回其在该列表中的位置，
//!   使插入首个元素与移除最后一个元素均为 O(1)。
//!
//! 四叉树覆盖以原点为中心的固定世界正方形（`world_size`）。
//! 任何无法被根节点松散区域查询覆盖的元素都会计入 [`LooseQuadtree::overflowed`]，
//! 而非被静默丢弃。

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};

/// 最大细分深度。
///
/// *线性*四叉树为每个节点存储一个链表头，因此深度 `d` 占用 `4^d` 个字。
/// 深度 8 是稠密层级数组仍能控制在数百 KiB（约 87 000 个 `u32`）内的最大值，
/// 它在 4096 px 世界中已能提供 16 px 单元格，在 10⁶ px 世界中提供 3.9 kpx 单元格。
/// 继续加深只会用数 MiB 的稀疏表换取精确测试本就已提供的剪枝收益。
///
pub const MAX_DEPTH: u8 = 8;

/// “无元素” / “无槽位”的哨兵值。
pub const NO_ELEMENT: u32 = u32::MAX;

/// 四叉树中的单个元素。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// 调用方的实体索引（画布条目 ID）。
    pub entity: u32,
    /// 用于放置节点及查询时精确相交测试的包围盒。
    pub aabb: Rect,
    /// 用于在精确测试前排序查询结果的图层键。
    pub layer: u16,
    /// 同一节点链表中的下一个元素。
    pub next: u32,
    /// 同一节点链表中的上一个元素。
    ///
    /// 采用双向链表以使从中间移除元素为 O(1)。单向链表需要遍历查找前驱，
    /// 而朴素版本（用被移除元素的后继覆盖链表头）会静默丢弃其后的所有元素。
    ///
    ///
    pub prev: u32,
    /// 该元素当前挂载的节点。
    pub node: u32,
}

/// 单个非空节点：包含节点索引及其松散包围盒。
#[derive(Copy, Clone, Debug, PartialEq)]
struct Occupied {
    index: u32,
    bounds: Rect,
}

/// 查询统计信息，供 KPI 测试使用。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// 参与查询松散包围盒测试的节点数。
    pub nodes_visited: u32,
    /// 作为候选提供的元素数（均未通过粗筛以外的筛选）。
    pub elements_tested: u32,
    /// 通过精确相交测试的存活元素数。
    pub elements_returned: u32,
}

impl QueryStats {
    /// 通过精确测试的元素占候选元素的比例。
    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        if self.elements_tested == 0 {
            0.0
        } else {
            self.elements_returned as f32 / self.elements_tested as f32
        }
    }
}

/// 松散线性四叉树。
pub struct LooseQuadtree {
    /// 四叉树覆盖的世界正方形边长（以原点为中心）。
    world_size: f32,
    /// `level_heads[d]` 包含 `4^d` 个表项：每个节点的链表头元素。
    level_heads: Vec<Vec<u32>>,
    /// 每层的非空节点稠密列表，每个节点在插入时预计算好松散包围盒。
    ///
    ///
    /// 缓存包围盒将查询转化为对连续矩形的线性扫描，而非为每个节点重新解码 Morton 位：
    /// 在百万实体画布上，这是 4 500 次随机 8 步跳转与单次 200 KiB 顺序扫描的区别。
    ///
    ///
    level_occupied: Vec<Vec<Occupied>>,
    /// `level_slot[d][node]` 为 `node` 在 `level_occupied[d]` 中的位置，
    /// 节点为空时为 [`NO_ELEMENT`]。
    level_slot: Vec<Vec<u32>>,
    /// 元素存储数组，按元素句柄索引。
    elements: Vec<Element>,
    free_elements: Vec<u32>,
    /// 每个实体对应的元素句柄，用于 O(1) 移除。
    entity_of: FxHashMap<u32, u32>,
    stats: QueryStats,
    overflowed: u64,
}

impl core::fmt::Debug for LooseQuadtree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LooseQuadtree")
            .field("world_size", &self.world_size)
            .field("elements", &self.entity_of.len())
            .field("levels", &self.level_heads.len())
            .field("occupied_nodes", &self.occupied_nodes())
            .field("overflowed", &self.overflowed)
            .finish_non_exhaustive()
    }
}

impl LooseQuadtree {
    /// 创建以原点为中心、覆盖 `world_size × world_size` 区域的四叉树。
    ///
    /// # Panic 说明
    /// 当 `world_size <= 0` 时 panic。
    #[must_use]
    pub fn new(world_size: f32) -> Self {
        assert!(world_size > 0.0, "world size must be positive");
        let mut tree = Self {
            world_size,
            level_heads: Vec::with_capacity(MAX_DEPTH as usize + 1),
            level_occupied: Vec::with_capacity(MAX_DEPTH as usize + 1),
            level_slot: Vec::with_capacity(MAX_DEPTH as usize + 1),
            elements: Vec::new(),
            free_elements: Vec::new(),
            entity_of: FxHashMap::default(),
            stats: QueryStats::default(),
            overflowed: 0,
        };
        // 所有层级均预先分配：查询会遍历全部层级，若延迟扩容层级会在帧中改变循环上界。
        //
        tree.ensure_level(MAX_DEPTH);
        tree
    }

    /// 活跃实体数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entity_of.len()
    }

    /// 当树中无任何实体时返回 true。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entity_of.is_empty()
    }

    /// 无法放置的元素数量（其包围盒超出根节点松散区域）。显式暴露而非静默丢弃。
    ///
    #[must_use]
    pub fn overflowed(&self) -> u64 {
        self.overflowed
    }

    /// 上一次 [`Self::query`] / [`Self::query_exact`] 调用的统计信息。
    #[must_use]
    pub fn stats(&self) -> QueryStats {
        self.stats
    }

    /// 所有层级中的非空节点总数。
    #[must_use]
    pub fn occupied_nodes(&self) -> usize {
        self.level_occupied.iter().map(Vec::len).sum()
    }

    /// 层级存储占用的字节数，供 KPI 测试套件上报。
    ///
    /// 三个层级数组的元素类型不同：链表头与槽位表各存一个 `u32`，
    /// 但非空节点记录包含一个 `u32` 索引和一个 `Rect`。若全部按 4 字节统计会少算五倍。
    ///
    ///
    #[must_use]
    pub fn level_bytes(&self) -> usize {
        let heads: usize = self.level_heads.iter().map(|l| l.len() * 4).sum();
        let occ: usize = self
            .level_occupied
            .iter()
            .map(|l| l.len() * core::mem::size_of::<Occupied>())
            .sum();
        let slots: usize = self.level_slot.iter().map(|l| l.len() * 4).sum();
        heads + occ + slots
    }

    fn ensure_level(&mut self, depth: u8) {
        while self.level_heads.len() <= depth as usize {
            let d = self.level_heads.len() as u8;
            let count = 1usize << (2 * d);
            self.level_heads.push(vec![NO_ELEMENT; count]);
            self.level_occupied.push(Vec::new());
            self.level_slot.push(vec![NO_ELEMENT; count]);
        }
    }

    /// 深度 `depth` 下*标准*（非松散）单元格的半边长。
    fn cell_half(&self, depth: u8) -> f32 {
        self.world_size * 0.5 / (1u32 << depth) as f32
    }

    /// 仍能完整容纳该元素的最深层级。
    ///
    /// 深度 `d` 的松散单元格宽度为 `2 · world / 2^d`，即在标准单元格四周各向外延伸半格。
    /// 因此中心位于标准单元格内任意位置的元素，只要满足 `extent ≤ world / 2^d` 即可完整放入，
    /// 即 `d ≤ log2(world / extent)`：对应文档中的 `leading_zeros` 计算。
    /// 下方循环是该表达式的精确浮点形式，最多执行 `MAX_DEPTH + 1` 次。
    ///
    ///
    fn depth_for(&self, aabb: &Rect) -> Option<u8> {
        if !aabb.is_finite() || !aabb.origin.is_finite() {
            return None;
        }
        let extent = aabb.size.x.max(aabb.size.y);
        if extent >= self.world_size {
            // 大于整个世界：只有根节点的松散区域可能容纳它，`insert` 会在接收前再次检查包含关系。
            //
            return Some(0);
        }
        for d in (0..=MAX_DEPTH).rev() {
            if self.cell_half(d) * 2.0 >= extent {
                return Some(d);
            }
        }
        Some(0)
    }

    /// 深度 `depth` 下包含 `point` 的类 Morton 单元格索引。
    ///
    /// 四进制顺序（非 Morton 位交错）：每次追加两位，象限 `qy * 2 + qx` 置于低位。
    /// 这使索引保持为纯四进制数，供 `node_bounds` 逐位回放。
    ///
    fn node_index(&self, point: Vec2, depth: u8) -> u32 {
        let mut index = 0u32;
        let mut half = self.world_size * 0.5;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for _ in 0..depth {
            let qx = u32::from(point.x >= cx);
            let qy = u32::from(point.y >= cy);
            index = index * 4 + qy * 2 + qx;
            half *= 0.5;
            if qx == 1 {
                cx += half;
            } else {
                cx -= half;
            }
            if qy == 1 {
                cy += half;
            } else {
                cy -= half;
            }
        }
        index
    }

    /// 计算 `(depth, index)` 处*松散*单元格的包围盒。
    ///
    /// 松散区域在每个方向上是标准单元格的两倍（文档中的 `k = 0.5`），
    /// 正是这一裕量使跨单元格边界的元素在查询相邻单元格时仍能被找到。
    ///
    fn node_bounds(&self, index: u32, depth: u8) -> Rect {
        let half = self.cell_half(depth);
        // 从最高位开始回放象限数字，与 `node_index` 将当前象限追加到低位的顺序镜像对应。
        //
        let mut h = self.world_size * 0.5;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for d in 0..depth {
            let shift = 2 * (depth - 1 - d);
            let bit = (index >> shift) & 0b11;
            let qx = bit & 1;
            let qy = (bit >> 1) & 1;
            h *= 0.5;
            cx += if qx == 1 { h } else { -h };
            cy += if qy == 1 { h } else { -h };
        }
        Rect::new(cx - half * 2.0, cy - half * 2.0, half * 4.0, half * 4.0)
    }

    /// 插入或移动实体。
    ///
    /// 插入已存在的实体会直接移动它，使更新逻辑无需编写“是否存在”分支。
    ///
    pub fn insert(&mut self, entity: u32, aabb: Rect, layer: u16) {
        self.remove(entity);
        let Some(depth) = self.depth_for(&aabb) else {
            self.overflowed += 1;
            return;
        };
        let index = self.node_index(aabb.center(), depth);
        let node_bounds = self.node_bounds(index, depth);
        if !node_bounds.touches(&aabb) {
            // 超出根节点松散区域（或高层级覆盖范围）：任何查询都无法触及，因此上报溢出而非错误放置。
            //
            self.overflowed += 1;
            return;
        }
        let handle = match self.free_elements.pop() {
            Some(h) => {
                let e = &mut self.elements[h as usize];
                e.entity = entity;
                e.aabb = aabb;
                e.layer = layer;
                h
            }
            None => {
                self.elements.push(Element {
                    entity,
                    aabb,
                    layer,
                    next: NO_ELEMENT,
                    prev: NO_ELEMENT,
                    node: 0,
                });
                (self.elements.len() - 1) as u32
            }
        };
        self.attach(depth, index, handle);
        self.entity_of.insert(entity, handle);
    }

    /// 将 `handle` 压入节点链表；若节点原为空则将其注册到非空列表。
    fn attach(&mut self, depth: u8, index: u32, handle: u32) {
        let head_slot = &mut self.level_heads[depth as usize][index as usize];
        let head = *head_slot;
        {
            let e = &mut self.elements[handle as usize];
            e.node = Self::pack_node(depth, index);
            e.next = head;
            e.prev = NO_ELEMENT;
        }
        if head == NO_ELEMENT {
            // 在获取链表可变借用前先计算包围盒。
            let bounds = self.node_bounds(index, depth);
            let slot = self.level_occupied[depth as usize].len() as u32;
            self.level_occupied[depth as usize].push(Occupied { index, bounds });
            self.level_slot[depth as usize][index as usize] = slot;
        } else {
            // 原链表头现在有了前驱节点。
            self.elements[head as usize].prev = handle;
        }
        self.level_heads[depth as usize][index as usize] = handle;
    }

    /// 将 `handle` 从其所属节点链表中摘除；若节点变空则注销。
    fn detach(&mut self, handle: u32) {
        let (depth, index) = Self::unpack_node(self.elements[handle as usize].node);
        let next = self.elements[handle as usize].next;
        let prev = self.elements[handle as usize].prev;

        if prev == NO_ELEMENT {
            // 该元素曾是链表头：链表现在从其后继开始。
            self.level_heads[depth as usize][index as usize] = next;
        } else {
            // 该元素位于链表中间：闭合链条缺口，确保其后的元素仍可达。
            //
            self.elements[prev as usize].next = next;
        }
        if next != NO_ELEMENT {
            self.elements[next as usize].prev = prev;
        }

        // 当被移除元素是唯一成员时，节点变为空。
        if prev == NO_ELEMENT && next == NO_ELEMENT {
            let level = depth as usize;
            let slot = self.level_slot[level][index as usize];
            debug_assert!(slot != NO_ELEMENT, "occupied list out of sync");
            let last = self.level_occupied[level].pop().expect("occupied node");
            if last.index != index {
                self.level_occupied[level][slot as usize] = last;
                self.level_slot[level][last.index as usize] = slot;
            }
            self.level_slot[level][index as usize] = NO_ELEMENT;
        }
    }

    fn pack_node(depth: u8, index: u32) -> u32 {
        // 8 位深度 + 24 位索引：足以覆盖深度 12 的稠密索引，
        // 更深层级也可打包进同一 24 位空间（因为 4^12 == 2^24）。
        //
        ((depth as u32) << 24) | (index & 0x00ff_ffff)
    }

    fn unpack_node(packed: u32) -> (u8, u32) {
        ((packed >> 24) as u8, packed & 0x00ff_ffff)
    }

    /// 移除实体。若实体不存在则返回 `false`。
    pub fn remove(&mut self, entity: u32) -> bool {
        let Some(handle) = self.entity_of.remove(&entity) else {
            return false;
        };
        self.detach(handle);
        {
            let e = &mut self.elements[handle as usize];
            e.next = NO_ELEMENT;
            e.prev = NO_ELEMENT;
            e.aabb = Rect::ZERO;
        }
        self.free_elements.push(handle);
        true
    }

    /// 移动实体并保留其原有图层键。
    pub fn update(&mut self, entity: u32, aabb: Rect) {
        let layer = self.layer_of(entity);
        self.insert(entity, aabb, layer);
    }

    /// 获取 `entity` 记录的图层键；不存在时返回 `0`。
    #[must_use]
    pub fn layer_of(&self, entity: u32) -> u16 {
        self.entity_of
            .get(&entity)
            .map(|h| self.elements[*h as usize].layer)
            .unwrap_or(0)
    }

    /// 获取 `entity` 记录的包围盒。
    #[must_use]
    pub fn bounds_of(&self, entity: u32) -> Option<Rect> {
        self.entity_of
            .get(&entity)
            .map(|h| self.elements[*h as usize].aabb)
    }

    /// 收集所有所属节点松散区域与 `query` 相交的实体。
    ///
    /// 返回列表为*候选*几何体：结果是保守的，必须再经过精确相交测试，
    /// 即 [`Self::query_exact`] 所做的过滤。
    ///
    pub fn query(&mut self, query: Rect, out: &mut Vec<u32>) {
        out.clear();
        self.stats = QueryStats::default();
        for depth in 0..self.level_heads.len() as u8 {
            let occupied = self.level_occupied[depth as usize].len();
            for slot in 0..occupied {
                let node = self.level_occupied[depth as usize][slot];
                if !node.bounds.intersects(&query) {
                    continue;
                }
                self.stats.nodes_visited += 1;
                let mut current = self.level_heads[depth as usize][node.index as usize];
                while current != NO_ELEMENT {
                    self.stats.elements_tested += 1;
                    out.push(self.elements[current as usize].entity);
                    current = self.elements[current as usize].next;
                }
            }
        }
    }

    /// 执行带精确 AABB 测试的查询，并按 `(layer, entity)` 排序。
    ///
    /// 排序保证了拾取的确定性：结果绝不依赖插入顺序或节点访问顺序。
    ///
    pub fn query_exact(&mut self, query: Rect, out: &mut Vec<u32>) {
        let mut candidates = Vec::new();
        self.query(query, &mut candidates);
        out.clear();
        for entity in candidates {
            let Some(bounds) = self.bounds_of(entity) else {
                continue;
            };
            // 精确测试采用闭区间（inclusive）：边缘恰好落在实体边缘上的框选框仍会选中该实体，
            // 与 `Rect::contains_point` 将边界点视为内部的行为一致。
            //
            if bounds.touches(&query) {
                out.push(entity);
            }
        }
        out.sort_unstable_by_key(|e| (self.layer_of(*e), *e));
        out.dedup();
        self.stats.elements_returned = out.len() as u32;
    }

    /// 当除 `ignore` 外的任意实体与 `query` 相交时返回 true。
    pub fn any_intersecting(&mut self, query: Rect, ignore: Option<u32>) -> bool {
        let mut hits = Vec::new();
        self.query_exact(query, &mut hits);
        hits.iter().any(|e| Some(*e) != ignore)
    }

    /// 清空四叉树但保留已分配的层级数组。
    pub fn clear(&mut self) {
        for heads in &mut self.level_heads {
            heads.fill(NO_ELEMENT);
        }
        for occupied in &mut self.level_occupied {
            occupied.clear();
        }
        for slots in &mut self.level_slot {
            slots.fill(NO_ELEMENT);
        }
        self.elements.clear();
        self.free_elements.clear();
        self.entity_of.clear();
        self.overflowed = 0;
        self.stats = QueryStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_single_element_is_found_by_its_own_bounds() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(7, rect(100.0, 100.0, 50.0, 50.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(90.0, 90.0, 100.0, 100.0), &mut out);
        assert_eq!(out, vec![7]);
    }

    #[test]
    fn a_far_element_is_not_returned() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(-3000.0, -3000.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(90.0, 90.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![1]);
    }

    #[test]
    fn removal_is_exact_and_idempotent() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(5, rect(0.0, 0.0, 10.0, 10.0), 0);
        assert!(t.remove(5));
        assert!(!t.remove(5), "second removal must be a no-op");
        assert!(t.is_empty());
        let mut out = Vec::new();
        t.query_exact(rect(-10.0, -10.0, 50.0, 50.0), &mut out);
        assert!(out.is_empty());
        assert_eq!(t.occupied_nodes(), 0, "emptied node must be unregistered");
    }

    #[test]
    fn moving_an_element_updates_its_placement() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(3, rect(0.0, 0.0, 10.0, 10.0), 0);
        t.update(3, rect(1000.0, 1000.0, 10.0, 10.0));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 20.0, 20.0), &mut out);
        assert!(out.is_empty(), "old position must be empty");
        t.query_exact(rect(990.0, 990.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![3]);
    }

    #[test]
    fn elements_of_many_scales_coexist() {
        let mut t = LooseQuadtree::new(1_000_000.0);
        // 一个紧邻巨大分组框的微小图标。
        t.insert(1, rect(10.0, 10.0, 4.0, 4.0), 0);
        t.insert(2, rect(-5000.0, -5000.0, 100_000.0, 100_000.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 100.0, 100.0), &mut out);
        assert_eq!(out, vec![1, 2], "both must be found");
    }

    #[test]
    fn every_element_is_returned_exactly_once() {
        let mut t = LooseQuadtree::new(16384.0);
        for i in 0..500u32 {
            let x = (i % 25) as f32 * 100.0 - 1200.0;
            let y = (i / 25) as f32 * 100.0 - 1200.0;
            t.insert(i, rect(x, y, 40.0, 40.0), 0);
        }
        let mut out = Vec::new();
        t.query_exact(rect(-5000.0, -5000.0, 10_000.0, 10_000.0), &mut out);
        out.sort_unstable();
        out.dedup();
        assert_eq!(out.len(), 500);
    }

    #[test]
    fn query_prunes_nodes_far_away() {
        let mut t = LooseQuadtree::new(65536.0);
        for i in 0..2000u32 {
            let x = ((i * 37) % 4000) as f32;
            let y = ((i * 71) % 4000) as f32;
            t.insert(i, rect(x, y, 8.0, 8.0), 0);
        }
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 200.0, 200.0), &mut out);
        let stats = t.stats();
        assert!(
            stats.elements_tested < 500,
            "culling failed: tested {} of 2000",
            stats.elements_tested
        );
        assert!(
            stats.nodes_visited < 200,
            "pruning failed: visited {} nodes",
            stats.nodes_visited
        );
        assert!(out.len() < 2000);
    }

    #[test]
    fn elements_outside_the_world_are_counted_not_lost() {
        let mut t = LooseQuadtree::new(1000.0);
        t.insert(1, rect(5000.0, 5000.0, 10.0, 10.0), 0);
        assert_eq!(t.overflowed(), 1);
        assert!(t.is_empty());
    }

    #[test]
    fn non_finite_input_is_reported_not_inserted() {
        let mut t = LooseQuadtree::new(1000.0);
        t.insert(1, rect(f32::NAN, 0.0, 10.0, 10.0), 0);
        t.insert(2, rect(0.0, 0.0, f32::INFINITY, 10.0), 0);
        assert_eq!(t.overflowed(), 2);
        assert!(t.is_empty());
    }

    #[test]
    fn handles_are_recycled_after_removal() {
        let mut t = LooseQuadtree::new(4096.0);
        for i in 0..100u32 {
            t.insert(i, rect(i as f32, 0.0, 4.0, 4.0), 0);
        }
        let capacity = t.elements.capacity();
        for i in 0..100u32 {
            t.remove(i);
        }
        for i in 0..100u32 {
            t.insert(i, rect(i as f32, 0.0, 4.0, 4.0), 0);
        }
        assert_eq!(t.elements.capacity(), capacity, "storage must be reused");
    }

    #[test]
    fn node_bounds_round_trip_for_every_depth() {
        let t = LooseQuadtree::new(1024.0);
        for depth in 0..=6u8 {
            let count = 1usize << (2 * depth);
            for index in [0usize, count / 2, count - 1] {
                let bounds = t.node_bounds(index as u32, depth);
                let centre = bounds.center();
                // 记录的中心点必须映射回同一个节点索引。
                assert_eq!(
                    t.node_index(centre, depth) as usize,
                    index,
                    "depth {depth} index {index} -> {bounds:?}"
                );
            }
        }
    }

    #[test]
    fn loose_cells_overlap_their_neighbours() {
        let t = LooseQuadtree::new(1024.0);
        let depth = 2u8;
        let a = t.node_bounds(0, depth);
        let b = t.node_bounds(1, depth);
        // 相邻单元格在每侧必须恰好重叠半个标准单元格；该重叠正是 `k = 0.5` 的意义所在。
        //
        let overlap = a.intersect(&b);
        assert!(overlap.size.x > 0.0 && overlap.size.y > 0.0);
    }

    #[test]
    fn placement_uses_the_deepest_fitting_level() {
        let t = LooseQuadtree::new(65536.0);
        // 65 536 px 世界中的 8 px 图标应落在最深层级。
        assert_eq!(t.depth_for(&rect(0.0, 0.0, 8.0, 8.0)), Some(MAX_DEPTH));
        // 100 000 px 的边框无法放入根节点以下的任何层级。
        assert_eq!(t.depth_for(&rect(0.0, 0.0, 100_000.0, 100_000.0)), Some(0));
    }

    #[test]
    fn a_hundred_thousand_inserts_stay_fast_and_correct() {
        let mut t = LooseQuadtree::new(1_000_000.0);
        for i in 0..100_000u32 {
            // 扩宽位宽算术：直接相乘会溢出 `u32`。
            let x = ((u64::from(i) * 7919) % 100_000) as f32 - 50_000.0;
            let y = ((u64::from(i) * 104_729) % 100_000) as f32 - 50_000.0;
            t.insert(i, rect(x, y, 16.0, 16.0), 0);
        }
        assert_eq!(t.len(), 100_000);
        let mut out = Vec::new();
        t.query_exact(rect(-500.0, -500.0, 1000.0, 1000.0), &mut out);
        // 10 万个元素中只有极小部分会落入 1000x1000 方框内。
        assert!(out.len() < 2000, "returned {}", out.len());
        assert!(!out.is_empty());
        assert!(
            t.stats().elements_returned < t.stats().elements_tested,
            "broad phase must reject something"
        );
    }

    #[test]
    fn clear_resets_everything() {
        let mut t = LooseQuadtree::new(2048.0);
        t.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.overflowed(), 0);
        assert_eq!(t.occupied_nodes(), 0);
        t.insert(2, rect(0.0, 0.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(-10.0, -10.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![2]);
    }

    #[test]
    fn layer_orders_results() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(1, rect(0.0, 0.0, 10.0, 10.0), 5);
        t.insert(2, rect(5.0, 5.0, 10.0, 10.0), 1);
        t.insert(3, rect(2.0, 2.0, 10.0, 10.0), 3);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 20.0, 20.0), &mut out);
        assert_eq!(out, vec![2, 3, 1]);
    }

    /// 移除元素不得连带丢失其同节点邻居。
    ///
    /// 节点链表采用头插法，因此被移除的元素通常*不是*链表头。
    /// 旧版 `detach` 用被移除元素的后继覆盖链表头，导致所有在其之前插入的元素被静默丢弃。
    ///
    ///
    #[test]
    fn removing_one_entity_keeps_the_others() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        t.insert(3, rect(120.0, 100.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![1, 2, 3]);

        // 移除最旧的元素：它位于链表尾部。
        assert!(t.remove(1));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![2, 3], "entities 2 and 3 must survive");

        // 接着移除链表头。
        assert!(t.remove(3));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![2]);
    }

    /// 移动实体（即移除后重新插入）不得干扰共享同一四叉树节点的其他实体。
    ///
    #[test]
    fn moving_an_entity_keeps_its_neighbours() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        t.insert(3, rect(120.0, 100.0, 10.0, 10.0), 0);
        // 移动最旧的实体：`update` 会将其移除并重新插入。
        t.update(1, rect(130.0, 100.0, 10.0, 10.0));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out.len(), 3, "all three entities must survive the move");
        assert!(out.contains(&1) && out.contains(&2) && out.contains(&3));
    }

    /// 移除节点的最后一个元素时必须注销该节点。
    #[test]
    fn emptying_a_node_unregisters_it() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        let occupied = t.occupied_nodes();
        assert!(occupied > 0);
        t.remove(2);
        t.remove(1);
        assert_eq!(t.occupied_nodes(), 0, "the node must be gone");
        assert!(t.is_empty());
    }
}

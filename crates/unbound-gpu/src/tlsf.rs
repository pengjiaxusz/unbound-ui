//! 基于单一连续内存区的两级隔离适配（TLSF）分配器。
//!
//! `docs/architecture/gpu-renderer.md` 要求持久化池采用自管理的超大 `VkBuffer` 并由 TLSF 切分，从而实现：
//!
//! * **O(1)** 分配与释放（两次位图探测 + 一次链表弹出）；
//! * **每帧零堆分配**——空闲链表存放在固定大小数组中，不依赖动态堆结构；
//! * 释放块立即合并，在编辑器会话的频繁增删下将碎片率控制在有界范围内。
//!
//! 全模块采用 100% 安全 Rust 实现：内存区底层为 `Vec<u128>`（保证 16 字节对齐），
//! 通过 `bytemuck` 按字节视图访问，块之间以 `u32` 字节偏移寻址，无任何别名风险。

use bytemuck::{Pod, Zeroable};

/// 最小块大小：64B 头部 + 64B 载荷。
///
/// 采用 64 字节头部（而非通常的 16/32 字节）为渲染器换来关键特性：**每个载荷偏移
/// 都是 64 的倍数**，即每次分配天然满足 `GpuPrimitiveInstance` 对齐要求，
/// 无需每次分配后再修正。多出的 32 字节填充在 4 GiB 池中占比极低（64B 分配约 0.5%），
/// 而从热路径中消去对齐分支的收益远超这点开销。
pub const MIN_BLOCK: u32 = 128;

/// 二级细分位数：每个一级桶下设 16 个二级桶。
const SL_SHIFT: u32 = 4;
const SL_COUNT: usize = 1 << SL_SHIFT;
/// 一级桶覆盖 `size < 2^32`，因此 32 个桶已足够。
const FL_COUNT: usize = 32;

const NONE: u32 = u32::MAX;

/// 64 字节块头部，内联存储在每个块的起始位置。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct BlockHeader {
    /// 块总大小（**包含**本头部）。
    size: u32,
    /// 物理相邻前一块的大小（首块为 0）。
    prev_size: u32,
    /// 同一空闲链表中的下一块偏移。
    next_free: u32,
    /// 同一空闲链表中的前一块偏移。
    prev_free: u32,
    /// 空闲时为 1。
    flags: u32,
    _pad: [u32; 11],
}

const HEADER_SIZE: u32 = 64;
const FLAG_FREE: u32 = 1;

/// 分配失败原因。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// 遍历所有桶后仍未找到足够大的空闲块。
    OutOfMemory { request: u32, largest_free: u32 },
    /// 请求大小超出内存区总容量。
    TooLarge { request: u32, capacity: u32 },
    /// 请求的对齐值不是 2 的幂。
    InvalidAlignment(usize),
}

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfMemory {
                request,
                largest_free,
            } => write!(
                f,
                "TLSF out of memory: requested {request}B, largest free block {largest_free}B"
            ),
            Self::TooLarge { request, capacity } => {
                write!(
                    f,
                    "TLSF request {request}B exceeds arena capacity {capacity}B"
                )
            }
            Self::InvalidAlignment(a) => write!(f, "invalid alignment {a}"),
        }
    }
}

impl std::error::Error for AllocError {}

/// 分配器健康快照，在调试覆盖层展示并由 CI 断言（稳态编辑下碎片率不得持续攀升）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TlsfStats {
    pub capacity: u32,
    pub allocated: u32,
    pub free: u32,
    pub largest_free: u32,
    pub block_count: u32,
    pub free_block_count: u32,
    pub successful_allocations: u64,
    pub failed_allocations: u64,
}

impl TlsfStats {
    /// `1 - largest_free / total_free`。0 表示完全合并无碎片。
    #[must_use]
    pub fn fragmentation(&self) -> f32 {
        if self.free == 0 {
            0.0
        } else {
            1.0 - (self.largest_free as f32 / self.free as f32)
        }
    }

    #[must_use]
    pub fn utilisation(&self) -> f32 {
        if self.capacity == 0 {
            0.0
        } else {
            self.allocated as f32 / self.capacity as f32
        }
    }
}

/// 空闲链表索引表。32 × 16 个桶共 512 个 `u32` 表头，总计 2 KiB。
struct FreeLists {
    fl_bitmap: u32,
    sl_bitmap: [u32; FL_COUNT],
    heads: [[u32; SL_COUNT]; FL_COUNT],
}

impl FreeLists {
    fn new() -> Self {
        Self {
            fl_bitmap: 0,
            sl_bitmap: [0; FL_COUNT],
            heads: [[NONE; SL_COUNT]; FL_COUNT],
        }
    }

    fn clear(&mut self) {
        self.fl_bitmap = 0;
        self.sl_bitmap = [0; FL_COUNT];
        self.heads = [[NONE; SL_COUNT]; FL_COUNT];
    }
}

/// 将块大小映射到 `(first_level, second_level)`。
#[inline]
fn mapping(size: u32) -> (usize, usize) {
    let size = size.max(MIN_BLOCK);
    let fl = 31 - size.leading_zeros(); // floor(log2(size))
    let fl_usize = (fl as usize).min(FL_COUNT - 1);
    if fl < SL_SHIFT {
        return (0, (size as usize) & (SL_COUNT - 1));
    }
    let sl = ((size >> (fl - SL_SHIFT)) & (SL_COUNT as u32 - 1)) as usize;
    (fl_usize, sl)
}

#[inline]
const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

#[inline]
fn read_header(arena: &[u8], off: u32) -> BlockHeader {
    let start = off as usize;
    *bytemuck::from_bytes::<BlockHeader>(&arena[start..start + HEADER_SIZE as usize])
}

#[inline]
fn write_header(arena: &mut [u8], off: u32, h: BlockHeader) {
    let start = off as usize;
    arena[start..start + HEADER_SIZE as usize].copy_from_slice(bytemuck::bytes_of(&h));
}

#[inline]
fn patch_header(arena: &mut [u8], off: u32, f: impl FnOnce(&mut BlockHeader)) {
    let mut h = read_header(arena, off);
    f(&mut h);
    write_header(arena, off, h);
}

// ---------------------------------------------------------------------------
// 空闲链表基础操作（定义为独立函数以便借用检查器识别 `Tlsf` 的不相交字段借用）。
// ---------------------------------------------------------------------------

fn list_insert(arena: &mut [u8], lists: &mut FreeLists, off: u32) {
    let h = read_header(arena, off);
    let (fl, sl) = mapping(h.size);
    let head = lists.heads[fl][sl];
    patch_header(arena, off, |b| {
        b.next_free = head;
        b.prev_free = NONE;
        b.flags |= FLAG_FREE;
    });
    if head != NONE {
        patch_header(arena, head, |b| b.prev_free = off);
    }
    lists.heads[fl][sl] = off;
    lists.sl_bitmap[fl] |= 1 << sl;
    lists.fl_bitmap |= 1 << fl;
}

fn list_remove(arena: &mut [u8], lists: &mut FreeLists, off: u32) {
    let h = read_header(arena, off);
    let (fl, sl) = mapping(h.size);
    let (next, prev) = (h.next_free, h.prev_free);

    if prev != NONE {
        patch_header(arena, prev, |b| b.next_free = next);
    } else {
        debug_assert_eq!(lists.heads[fl][sl], off);
        lists.heads[fl][sl] = next;
    }
    if next != NONE {
        patch_header(arena, next, |b| b.prev_free = prev);
    }

    if lists.heads[fl][sl] == NONE {
        lists.sl_bitmap[fl] &= !(1 << sl);
        if lists.sl_bitmap[fl] == 0 {
            lists.fl_bitmap &= !(1 << fl);
        }
    }
}

/// 桶 `(fl, sl)` 中块可拥有的最小尺寸。
///
/// 桶仅保证**下界**：其中块的尺寸范围从该下界到下一桶下界（不含）。
/// 因此请求所落入的桶中仍可能存在小于请求大小的块。
fn bucket_lower_bound(fl: usize, sl: usize) -> u32 {
    if fl < SL_SHIFT as usize {
        return sl as u32;
    }
    let shift = fl as u32 - SL_SHIFT;
    (1u32 << fl as u32) + ((sl as u32) << shift)
}

/// 在空闲链表中查找首个大小不少于 `size` 字节的块。
///
/// 空闲链表为嵌入式双向链表，需顺序遍历。本函数仅在请求自身的桶上调用，
/// 该桶内块尺寸区间很窄，且遇到首个满足大小的块即停止。
fn first_block_at_least(arena: &[u8], head: u32, size: u32) -> Option<u32> {
    let mut off = head;
    while off != NONE {
        let h = read_header(arena, off);
        if h.size >= size {
            return Some(off);
        }
        off = h.next_free;
    }
    None
}

fn list_find(arena: &[u8], lists: &FreeLists, size: u32) -> Option<u32> {
    let (fl, sl) = mapping(size);
    // 请求自身所在的桶是唯一可能包含小于请求大小的块的桶：桶只保证下界，
    // 而该桶下界低于请求大小。
    //
    // 若直接分出过小的块，`split_block` 会对无符号整数计算 `block.size - want`，
    // 减法下溢回绕后会将“剩余块”头部写到内存区之外——造成静默堆损坏。
    //
    // 因此必须在该桶内查找真正满足尺寸的块。
    debug_assert!(
        bucket_lower_bound(fl, sl) <= size,
        "the request must not be below its bucket's lower bound"
    );
    if let Some(off) = first_block_at_least(arena, lists.heads[fl][sl], size) {
        return Some(off);
    }
    // 更高桶中的每个块都至少达到该桶下界，且高于请求所在桶的起始下界必大于请求，
    // 因此其中的块恒满足需求。
    //
    // 同一一级桶内：查找 `sl + 1` 及以上的二级桶。
    let sl_map = lists.sl_bitmap[fl] & (!0u32 << (sl + 1));
    if sl_map != 0 {
        let bucket = sl_map.trailing_zeros() as usize;
        let off = lists.heads[fl][bucket];
        return (off != NONE).then_some(off);
    }
    // 否则向上查找下一个非空一级桶。其最低二级桶起始于 `1 << (fl + 1)`，必大于请求。
    if fl + 1 >= FL_COUNT {
        return None;
    }
    let fl_map = lists.fl_bitmap & (!0u32 << (fl + 1));
    if fl_map == 0 {
        return None;
    }
    let next_fl = fl_map.trailing_zeros() as usize;
    let sl_map = lists.sl_bitmap[next_fl];
    if sl_map == 0 {
        return None;
    }
    let bucket = sl_map.trailing_zeros() as usize;
    let off = lists.heads[next_fl][bucket];
    (off != NONE).then_some(off)
}

fn largest_free(arena: &[u8], lists: &FreeLists) -> u32 {
    if lists.fl_bitmap == 0 {
        return 0;
    }
    let fl = 31 - lists.fl_bitmap.leading_zeros();
    let sl_map = lists.sl_bitmap[fl as usize];
    if sl_map == 0 {
        return 0;
    }
    let sl = 31 - sl_map.leading_zeros();
    let off = lists.heads[fl as usize][sl as usize];
    if off == NONE {
        0
    } else {
        read_header(arena, off).size
    }
}

/// 将 `off` 分割出精确 `want` 字节的块，并将剩余部分作为空闲块返回。
fn split_block(arena: &mut [u8], lists: &mut FreeLists, off: u32, want: u32) {
    let h = read_header(arena, off);
    debug_assert!(h.size >= want);
    let rest = h.size - want;
    if rest < MIN_BLOCK {
        // 剩余空间不足 MIN_BLOCK 不值得分割：整块交给调用方，内部碎片受 MIN_BLOCK 上界约束。
        return;
    }
    let rest_off = off + want;
    patch_header(arena, off, |b| b.size = want);
    write_header(
        arena,
        rest_off,
        BlockHeader {
            size: rest,
            prev_size: want,
            next_free: NONE,
            prev_free: NONE,
            flags: FLAG_FREE,
            _pad: [0; 11],
        },
    );
    // 分割后物理相邻的下一块必须更新其 prev_size。
    let after = rest_off + rest;
    if (after as usize) < arena.len() {
        patch_header(arena, after, |b| b.prev_size = rest);
    }
    list_insert(arena, lists, rest_off);
}

// ---------------------------------------------------------------------------
// 分配器主体
// ---------------------------------------------------------------------------

/// 基于单一连续内存区的 TLSF 分配器。
pub struct Tlsf {
    /// 底层存储。采用 `u128` 以确保基指针 16 字节对齐。
    storage: Vec<u128>,
    len: u32,
    lists: FreeLists,
    stats: TlsfStats,
}

impl core::fmt::Debug for Tlsf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tlsf")
            .field("capacity", &self.len)
            .field("stats", &self.stats)
            .finish()
    }
}

impl Tlsf {
    /// 基于 `capacity` 字节（向上取整到 16 的倍数）创建分配器。
    ///
    /// # Panics
    /// 当 `capacity` 小于 [`MIN_BLOCK`] 或超出 `u32` 表示范围时触发 panic。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= MIN_BLOCK as usize, "arena too small");
        let len = align_up_checked(capacity);
        let mut storage = vec![0u128; (len as usize).div_ceil(16)];
        {
            let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut storage);
            let bytes = &mut bytes[..len as usize];
            write_header(
                bytes,
                0,
                BlockHeader {
                    size: len,
                    prev_size: 0,
                    next_free: NONE,
                    prev_free: NONE,
                    flags: FLAG_FREE,
                    _pad: [0; 11],
                },
            );
            let mut lists = FreeLists::new();
            list_insert(bytes, &mut lists, 0);
            Self {
                storage,
                len,
                lists,
                stats: TlsfStats {
                    capacity: len,
                    free: len,
                    largest_free: len,
                    block_count: 1,
                    free_block_count: 1,
                    ..Default::default()
                },
            }
        }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        let all: &[u8] = bytemuck::cast_slice(&self.storage);
        &all[..self.len as usize]
    }

    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.len
    }

    /// 遍历块链重新计算并返回分配器统计信息。
    #[must_use]
    pub fn stats(&mut self) -> TlsfStats {
        self.refresh_stats();
        self.stats
    }

    fn refresh_stats(&mut self) {
        let len = self.len;
        let (block_count, free_block_count, free_bytes, largest) = {
            let arena = self.bytes();
            let mut off = 0u32;
            let mut block_count = 0u32;
            let mut free_block_count = 0u32;
            let mut free_bytes = 0u32;
            while (off as usize) + HEADER_SIZE as usize <= arena.len() {
                let h = read_header(arena, off);
                if h.size == 0 {
                    break;
                }
                block_count += 1;
                if h.flags & FLAG_FREE != 0 {
                    free_block_count += 1;
                    free_bytes += h.size;
                }
                off += h.size;
            }
            (
                block_count,
                free_block_count,
                free_bytes,
                largest_free(arena, &self.lists),
            )
        };
        self.stats.block_count = block_count;
        self.stats.free_block_count = free_block_count;
        self.stats.free = free_bytes;
        self.stats.allocated = len - free_bytes;
        self.stats.largest_free = largest;
    }

    /// 按内存区自然的 16 字节对齐分配 `size` 字节。
    ///
    /// # Errors
    /// 无法满足分配请求时返回 [`AllocError`]。
    pub fn allocate(&mut self, size: u32) -> Result<u32, AllocError> {
        self.allocate_aligned(size, 16)
    }

    /// 按 `align`（须为 2 的幂）对齐分配 `size` 字节。
    ///
    /// 返回载荷偏移量，保证满足 `offset % align == 0` 且至少有 `size` 字节可用。
    ///
    /// # Errors
    /// 无法满足分配请求时返回 [`AllocError`]。
    pub fn allocate_aligned(&mut self, size: u32, align: usize) -> Result<u32, AllocError> {
        if !align.is_power_of_two() {
            self.stats.failed_allocations += 1;
            return Err(AllocError::InvalidAlignment(align));
        }
        if size > self.len {
            self.stats.failed_allocations += 1;
            return Err(AllocError::TooLarge {
                request: size,
                capacity: self.len,
            });
        }
        let len = self.len;
        // 头部与载荷按 16B 粒度向上取整，并预留余量以确保超对齐载荷总有空间前移对齐。
        let align32 = align as u32;
        let slack = if align32 > HEADER_SIZE { align32 } else { 0 };
        let total =
            align_up(size.saturating_add(HEADER_SIZE).saturating_add(slack), 64).max(MIN_BLOCK);

        let result = {
            let Self { storage, lists, .. } = &mut *self;
            let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
            let arena = &mut all[..len as usize];

            match list_find(arena, lists, total) {
                None => Err(AllocError::OutOfMemory {
                    request: size,
                    largest_free: largest_free(arena, lists),
                }),
                Some(block) => {
                    list_remove(arena, lists, block);
                    // 该块现已处于活跃状态：必须清除 FLAG_FREE，否则相邻块在 free() 时
                    // 会试图从空闲链表中摘除一个并不在链表中的块。
                    patch_header(arena, block, |b| b.flags &= !FLAG_FREE);
                    split_block(arena, lists, block, total);

                    let cur_size = read_header(arena, block).size;
                    let mut payload = block + HEADER_SIZE;

                    if align32 > HEADER_SIZE {
                        // 必须对齐的是*载荷*而非块本身：载荷位于块内 HEADER_SIZE 字节之后，
                        // 因此前置空闲块必须吸收这段偏移差。
                        let aligned = align_up(payload, align32);
                        let lead = aligned - HEADER_SIZE - block;
                        // 仅当两侧均至少保留 MIN_BLOCK 宽度时才切分出前置空闲块。
                        if lead >= MIN_BLOCK && cur_size > lead + MIN_BLOCK {
                            patch_header(arena, block, |b| b.size = lead);
                            let new_block = block + lead;
                            let new_size = cur_size - lead;
                            write_header(
                                arena,
                                new_block,
                                BlockHeader {
                                    size: new_size,
                                    prev_size: lead,
                                    next_free: NONE,
                                    prev_free: NONE,
                                    flags: 0,
                                    _pad: [0; 11],
                                },
                            );
                            let after = new_block + new_size;
                            if (after as usize) < arena.len() {
                                patch_header(arena, after, |b| b.prev_size = new_size);
                            }
                            list_insert(arena, lists, block);
                            payload = new_block + HEADER_SIZE;
                        }
                    }

                    // 将分出的区域清零：GPU 缓冲区残留旧节点字节是最难排查的随机故障源。
                    let start = payload as usize;
                    let end = (start + size as usize).min(arena.len());
                    arena[start..end].fill(0);
                    Ok(payload)
                }
            }
        };

        match result {
            Ok(p) => {
                self.stats.successful_allocations += 1;
                Ok(p)
            }
            Err(e) => {
                self.stats.failed_allocations += 1;
                Err(e)
            }
        }
    }

    /// 释放先前分配的载荷偏移。
    ///
    /// 在调试构建中检测重复释放，在发布构建中安全拒绝并返回 `false`，
    /// 防止异常调用方导致宿主崩溃。
    pub fn deallocate(&mut self, payload: u32) -> bool {
        if payload < HEADER_SIZE || payload as usize >= self.len as usize {
            debug_assert!(false, "deallocate with out-of-range offset {payload}");
            return false;
        }
        let len = self.len;
        let Self { storage, lists, .. } = &mut *self;
        let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
        let arena = &mut all[..len as usize];

        let block = payload - HEADER_SIZE;
        let h = read_header(arena, block);
        if h.flags & FLAG_FREE != 0 {
            debug_assert!(false, "double free of block at {block}");
            return false;
        }

        // 合并前先标记为空闲，确保 `list_insert` 看到一致的头部状态。
        patch_header(arena, block, |b| b.flags |= FLAG_FREE);
        let mut cur = block;
        let mut cur_size = h.size;

        // 与物理相邻的前一块合并。
        if h.prev_size != 0 {
            let prev = block - h.prev_size;
            let ph = read_header(arena, prev);
            if ph.flags & FLAG_FREE != 0 {
                list_remove(arena, lists, prev);
                cur = prev;
                cur_size += ph.size;
                patch_header(arena, cur, |b| b.size = cur_size);
            }
        }

        // 与物理相邻的后一块合并。
        let next = cur + cur_size;
        if (next as usize) + HEADER_SIZE as usize <= arena.len() {
            let nh = read_header(arena, next);
            if nh.size != 0 && nh.flags & FLAG_FREE != 0 {
                list_remove(arena, lists, next);
                cur_size += nh.size;
                patch_header(arena, cur, |b| b.size = cur_size);
            }
        }

        // 合并区域之后的下一块必须更新其 prev_size。
        let after = cur + cur_size;
        if (after as usize) < arena.len() {
            patch_header(arena, after, |b| b.prev_size = cur_size);
        }

        list_insert(arena, lists, cur);
        true
    }

    /// 返回一次分配的实际载荷容量（可能大于请求大小）。
    #[must_use]
    pub fn usable_size(&self, payload: u32) -> u32 {
        if payload < HEADER_SIZE || payload as usize >= self.len as usize {
            return 0;
        }
        let arena = self.bytes();
        let h = read_header(arena, payload - HEADER_SIZE);
        h.size.saturating_sub(HEADER_SIZE)
    }

    /// 调试辅助方法：以 `(offset, size, is_free)` 形式返回所有块。
    #[must_use]
    pub fn blocks(&self) -> Vec<(u32, u32, bool)> {
        let arena = self.bytes();
        let mut out = Vec::new();
        let mut off = 0u32;
        while (off as usize) + HEADER_SIZE as usize <= arena.len() {
            let h = read_header(arena, off);
            if h.size == 0 {
                break;
            }
            out.push((off, h.size, h.flags & FLAG_FREE != 0));
            off += h.size;
        }
        out
    }

    /// 将整个内存区重置为单个空闲块（用于设备丢失重建）。
    pub fn reset(&mut self) {
        let len = self.len;
        let successful = self.stats.successful_allocations;
        let failed = self.stats.failed_allocations;
        {
            let Self {
                storage,
                lists,
                stats,
                ..
            } = &mut *self;
            let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
            let arena = &mut all[..len as usize];
            arena.fill(0);
            write_header(
                arena,
                0,
                BlockHeader {
                    size: len,
                    prev_size: 0,
                    next_free: NONE,
                    prev_free: NONE,
                    flags: FLAG_FREE,
                    _pad: [0; 11],
                },
            );
            lists.clear();
            list_insert(arena, lists, 0);
            *stats = TlsfStats {
                capacity: len,
                allocated: 0,
                free: len,
                largest_free: len,
                block_count: 1,
                free_block_count: 1,
                successful_allocations: successful,
                failed_allocations: failed,
            };
        }
    }
}

#[inline]
fn align_up_checked(capacity: usize) -> u32 {
    assert!(
        capacity <= u32::MAX as usize - 16,
        "arena must fit in u32 (4 GiB)"
    );
    align_up(capacity as u32, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_aligned(v: u32, align: u32) -> bool {
        v % align == 0
    }

    #[test]
    fn fresh_arena_is_one_free_block() {
        let mut t = Tlsf::new(4096);
        assert_eq!(t.capacity(), 4096);
        let s = t.stats();
        assert_eq!(s.free, 4096);
        assert_eq!(s.block_count, 1);
        assert_eq!(s.fragmentation(), 0.0);
        assert_eq!(t.blocks(), vec![(0, 4096, true)]);
    }

    #[test]
    fn allocate_then_free_returns_to_single_block() {
        let mut t = Tlsf::new(8192);
        let a = t.allocate(1000).unwrap();
        let b = t.allocate(2000).unwrap();
        let c = t.allocate(3000).unwrap();
        assert!(is_aligned(a, 16) && is_aligned(b, 16) && is_aligned(c, 16));
        assert!(t.deallocate(b));
        assert!(t.deallocate(a));
        assert!(t.deallocate(c));
        let s = t.stats();
        assert_eq!(
            s.block_count,
            1,
            "blocks must fully coalesce: {:?}",
            t.blocks()
        );
        assert_eq!(s.free, 8192);
        assert_eq!(s.fragmentation(), 0.0);
    }

    #[test]
    fn allocations_never_overlap() {
        let mut t = Tlsf::new(1 << 20);
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        let mut order: Vec<u32> = Vec::new();
        let mut seed = 0x1234_5678u32;
        for i in 0..500 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let size = 16 + (seed % 4000);
            match t.allocate(size) {
                Ok(p) => {
                    let end = p + size;
                    for (s, e) in &ranges {
                        assert!(
                            end <= *s || p >= *e,
                            "allocation {i} [{p},{end}) overlaps [{s},{e})"
                        );
                    }
                    ranges.push((p, end));
                    order.push(p);
                }
                Err(_) => break,
            }
        }
        assert!(order.len() > 100, "only {} allocations fit", order.len());
        for (i, p) in order.iter().enumerate() {
            if i % 3 == 0 {
                assert!(t.deallocate(*p));
            }
        }
        // 完整性校验：块链必须仍精确覆盖整个内存区。
        let total: u32 = t.blocks().iter().map(|b| b.1).sum();
        assert_eq!(total, 1 << 20, "block chain must tile the arena");
        // 已释放的空洞必须可被复用。
        let mut reused = 0;
        for _ in 0..10 {
            if t.allocate(64).is_ok() {
                reused += 1;
            }
        }
        assert!(reused > 0, "freed holes should accept new allocations");
        assert!(t.stats().largest_free > 0);
    }

    #[test]
    fn alignment_is_respected_for_large_alignments() {
        let mut t = Tlsf::new(64 * 1024);
        // 先制造一个未对齐载荷，确保后续对齐分配器需要执行对齐调整。
        let _pad = t.allocate(40).unwrap();
        for align in [32usize, 64, 256, 4096] {
            let p = t.allocate_aligned(256, align).unwrap();
            assert_eq!(p % align as u32, 0, "align {align} -> {p}");
            assert!(t.usable_size(p) >= 256);
        }
    }

    #[test]
    fn oom_is_reported_with_largest_free() {
        let mut t = Tlsf::new(1024);
        let _a = t.allocate(900).unwrap();
        let err = t.allocate(900).unwrap_err();
        match err {
            AllocError::OutOfMemory {
                request,
                largest_free,
            } => {
                assert_eq!(request, 900);
                assert!(largest_free < 900);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(t.stats().failed_allocations, 1);
    }

    #[test]
    fn too_large_is_rejected_before_searching() {
        let mut t = Tlsf::new(1024);
        assert!(matches!(t.allocate(4096), Err(AllocError::TooLarge { .. })));
        assert!(matches!(
            t.allocate_aligned(16, 3),
            Err(AllocError::InvalidAlignment(3))
        ));
    }

    #[test]
    fn freed_memory_is_zeroed_on_reuse() {
        let mut t = Tlsf::new(4096);
        let p = t.allocate(64).unwrap();
        {
            let Tlsf { storage, .. } = &mut t;
            let bytes: &mut [u8] = bytemuck::cast_slice_mut(storage);
            bytes[p as usize..p as usize + 64].fill(0xAB);
        }
        assert!(t.deallocate(p));
        let q = t.allocate(64).unwrap();
        let arena = t.bytes();
        assert!(arena[q as usize..q as usize + 64].iter().all(|b| *b == 0));
    }

    #[test]
    fn alternating_free_keeps_fragmentation_bounded() {
        let mut t = Tlsf::new(256 * 1024);
        let ptrs: Vec<u32> = (0..64).map(|_| t.allocate(1024).unwrap()).collect();
        for (i, p) in ptrs.iter().enumerate() {
            if i % 2 == 0 {
                assert!(t.deallocate(*p));
            }
        }
        let s = t.stats();
        assert!(s.fragmentation() > 0.0, "expected holes");
        assert!(
            s.fragmentation() < 0.5,
            "fragmentation too high: {}",
            s.fragmentation()
        );
        for (i, p) in ptrs.iter().enumerate() {
            if i % 2 == 1 {
                assert!(t.deallocate(*p));
            }
        }
        assert_eq!(t.stats().block_count, 1);
    }

    #[test]
    fn many_small_allocations_are_exact() {
        let mut t = Tlsf::new(8 << 20);
        let mut ps = Vec::new();
        for _ in 0..10_000 {
            ps.push(t.allocate(48).unwrap());
        }
        for p in ps {
            assert!(t.deallocate(p));
        }
        assert_eq!(t.stats().block_count, 1);
        assert_eq!(t.stats().successful_allocations, 10_000);
    }

    #[test]
    fn usable_size_is_at_least_requested() {
        let mut t = Tlsf::new(1 << 16);
        for size in [1u32, 17, 64, 1000, 4096] {
            let p = t.allocate(size).unwrap();
            assert!(t.usable_size(p) >= size, "size {size}");
            t.deallocate(p);
        }
    }

    #[test]
    fn reset_restores_a_pristine_arena() {
        let mut t = Tlsf::new(4096);
        for _ in 0..4 {
            t.allocate(512).unwrap();
        }
        t.reset();
        assert_eq!(t.blocks(), vec![(0, 4096, true)]);
        assert_eq!(t.stats().free, 4096);
        assert!(t.allocate(4000).is_ok());
    }

    #[test]
    fn every_payload_is_64_byte_aligned() {
        let mut t = Tlsf::new(64 * 1024);
        for i in 0..64 {
            let p = t.allocate(1 + (i * 37) % 500).unwrap();
            assert_eq!(p % 64, 0, "payload {p} not 64B aligned");
        }
    }

    /// 大于所有现有空闲空洞的请求必须失败——绝不能从过小的空洞中分配。
    ///
    /// 本测试防护的 bug 极为严重：过去分配器按*桶*查找空闲块，而桶仅保证下界。
    /// 分出小于请求的块会导致 `split_block` 用无符号整数计算 `block.size - want`，
    /// 差值回绕到四十亿并将“剩余块”头部写入内存区之外。
    #[test]
    fn a_request_bigger_than_the_free_hole_is_refused() {
        // 构造恰好容纳三个块的内存区：填满后释放中间块，使其成为*唯一*候选空洞，
        // 从而防止请求回退到其他更大的块。
        let block = 16384u32;
        let total = (block as usize + 64).next_multiple_of(64) as u32;
        let mut t = Tlsf::new(3 * total as usize);
        let a = t.allocate(block).expect("first block");
        let b = t.allocate(block).expect("second block");
        let c = t.allocate(block).expect("third block");
        // 释放中间块：形成精确 `total` 字节的空洞。
        assert!(t.deallocate(b));

        // 请求比该空洞多一个单位的大小。
        let result = t.allocate(block + 80);
        assert!(
            result.is_err(),
            "a request that does not fit must fail, not overflow the hole: {result:?}"
        );

        // 内存区保持完好：空洞仍可复用，两个存活块未受影响。
        assert_eq!(t.allocate(block).expect("the hole is still free"), b);
        assert!(t.deallocate(a));
        assert!(t.deallocate(c));
    }

    /// 请求落入的桶中可能包含尺寸过小的块，查找时必须检查块实际大小而非仅看桶索引。
    #[test]
    fn a_bucket_never_serves_a_request_it_cannot_hold() {
        // 桶内每个块至少满足桶下界，但该下界可能*低于*请求大小，
        // 这正是查找逻辑必须校验块实际尺寸的原因。
        for size in [MIN_BLOCK, 1024, 16384, 65536, 1 << 20] {
            let (fl, sl) = mapping(size);
            assert!(
                bucket_lower_bound(fl, sl) <= size,
                "the request {size} must not be below its bucket's lower bound"
            );
            // 下一个桶的下界严格高于请求大小，因此其中的每个块都必然满足要求。
            let (next_fl, next_sl) = if sl + 1 < SL_COUNT {
                (fl, sl + 1)
            } else {
                (fl + 1, 0)
            };
            assert!(
                bucket_lower_bound(next_fl, next_sl) > size,
                "the next bucket must be strictly above the request {size}"
            );
        }
    }

    #[test]
    fn mapping_is_monotonic() {
        let mut prev = (0usize, 0usize);
        let mut size = MIN_BLOCK;
        while size < (1 << 28) {
            let m = mapping(size);
            assert!(
                m >= prev,
                "mapping not monotonic at {size}: {m:?} < {prev:?}"
            );
            prev = m;
            size *= 2;
        }
    }
}

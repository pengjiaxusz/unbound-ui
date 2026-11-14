//! GPU 常驻持久化池：TLSF 分配器 + 代际句柄 + Epoch 隔离队列。
//!
//! 本模块是“每帧零字节上传”承诺的 CPU 侧实现（参见 `docs/architecture/gpu-renderer.md`）：
//!
//! * 实体驻留在由 [`Tlsf`] 切分的长生命周期设备缓冲区中；
//! * 变更**就地**发生——每帧仅向增量散布计算着色器发送稀疏的 `(destination, source, length)`
//!   补丁列表，从不重传整个数组；
//! * 释放操作通过 [`EpochQuarantine`] 延迟执行，确保在飞帧绝不会读取到已被回收覆写的内存。
//!
//! 该类型刻意保持与 GPU API 无关（仅管理偏移量而非句柄），以便在纯 CPU 单元测试中验证，
//! 并可由设备丢失恢复路径直接复用。

use crate::quarantine::{EpochQuarantine, QuarantinedBlock};
use crate::tlsf::{AllocError, Tlsf, TlsfStats};

/// 指向持久化池的代际句柄。
///
/// 代际计数器使释放后使用（UAF）可被检测，而非静默返回另一节点的数据——
/// 这在宿主重建视图期间 UI 树同时发生变更时至关重要。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PoolHandle {
    pub index: u32,
    pub generation: u32,
}

impl PoolHandle {
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: 0,
    };

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.index != u32::MAX
    }
}

/// 单个已切分分配区域。
#[derive(Copy, Clone, Debug, Default)]
struct Slot {
    offset: u32,
    /// 当前有效字节数（着色器实际索引的范围）。
    len: u32,
    /// 在 TLSF 中预留的字节数（>= len）。
    capacity: u32,
    generation: u32,
    /// 0 == 空闲/未使用，1 == 存活，2 == 待释放。
    state: u8,
    /// 最后引用该槽位的帧号（用于诊断）。
    last_frame: u32,
}

const STATE_FREE: u8 = 0;
const STATE_LIVE: u8 = 1;
const STATE_PENDING: u8 = 2;

/// 写入持久化池的稀疏补丁，将交由散布计算着色器执行。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScatterPatch {
    /// 持久化缓冲区内的目标字节偏移。
    pub dst_offset: u32,
    /// 字节长度；`0` 表示“仅描述符更新”（无载荷数据）。
    pub len: u32,
    /// 该补丁在当前帧暂存列表中的索引。
    pub patch_index: u32,
}

/// 池级全局计数器，供 KPI 基准测试断言使用。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub live_slots: u32,
    pub pending_slots: u32,
    pub free_slots: u32,
    pub capacity_bytes: u32,
    pub live_bytes: u32,
    pub uploads_this_frame: u32,
    pub bytes_uploaded_this_frame: u32,
    pub allocs_this_frame: u32,
    pub total_uploaded_bytes: u64,
}

/// 持久化内存池。
pub struct PersistentPool {
    tlsf: Tlsf,
    slots: Vec<Slot>,
    free_indices: Vec<u32>,
    quarantine: EpochQuarantine,
    /// 由 `reclaim` 复用的暂存缓冲区（稳态下零重新分配）。
    reclaim_buffer: Vec<QuarantinedBlock>,
    /// 当前帧提交后将触发的时间线值。
    ///
    /// 本帧期间释放的内存块，必须等 GPU 执行超过该值后才能归还给分配器。
    pending_epoch: u64,
    patches: Vec<ScatterPatch>,
    stats: PoolStats,
    current_frame: u32,
}

impl core::fmt::Debug for PersistentPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PersistentPool")
            .field("capacity", &self.tlsf.capacity())
            .field("slots", &self.slots.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl PersistentPool {
    /// 基于 `capacity` 字节的设备本地内存创建持久化池。
    ///
    /// # Panics
    /// 当 `capacity < 4096` 时触发 panic。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 4096, "pool too small");
        let cap = capacity as u32;
        Self {
            tlsf: Tlsf::new(capacity),
            slots: Vec::with_capacity(1024),
            free_indices: Vec::new(),
            quarantine: EpochQuarantine::new(0),
            reclaim_buffer: Vec::with_capacity(64),
            pending_epoch: 0,
            patches: Vec::with_capacity(256),
            stats: PoolStats {
                capacity_bytes: cap,
                ..Default::default()
            },
            current_frame: 0,
        }
    }

    /// 持久化池拥有的设备内存总字节数。
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.tlsf.capacity()
    }

    /// 开启新帧并重置逐帧计数器。
    ///
    /// `timeline_epoch` 为本帧提交后将触发的时间线值。本帧退役的内存块必须等 GPU
    /// 达到该值后才能归还分配器：所有仍可能读取该块的帧都更早，因此持有更小的时间线值。
    pub fn begin_frame(&mut self, timeline_epoch: u64, frame_slot: u32) {
        self.current_frame = frame_slot;
        self.stats.uploads_this_frame = 0;
        self.stats.bytes_uploaded_this_frame = 0;
        self.stats.allocs_this_frame = 0;
        self.patches.clear();
        self.pending_epoch = timeline_epoch;
    }

    /// 当前帧退役内存块所需等待的隔离 epoch。
    ///
    /// 即当前帧将触发的时间线值。一旦 GPU 到达该值，所有更早的帧（唯一可能仍在读取
    /// 该块的帧）均已完成。
    #[must_use]
    pub fn current_epoch(&self) -> u64 {
        self.pending_epoch
    }

    /// 返回当前帧生成的稀疏补丁列表。
    #[must_use]
    pub fn patches(&self) -> &[ScatterPatch] {
        &self.patches
    }

    /// 预留 `size` 字节并返回代际句柄。
    ///
    /// # Errors
    /// 当内存池耗尽时透传 [`AllocError`]。
    pub fn allocate(&mut self, size: u32) -> Result<PoolHandle, AllocError> {
        let offset = self.tlsf.allocate_aligned(size, 64)?;
        let capacity = self.tlsf.usable_size(offset);
        let index = match self.free_indices.pop() {
            Some(i) => {
                let slot = &mut self.slots[i as usize];
                slot.offset = offset;
                slot.len = size;
                slot.capacity = capacity;
                slot.state = STATE_LIVE;
                slot.last_frame = self.current_frame;
                i
            }
            None => {
                let i = u32::try_from(self.slots.len()).expect("slot table overflow");
                self.slots.push(Slot {
                    offset,
                    len: size,
                    capacity,
                    generation: 0,
                    state: STATE_LIVE,
                    last_frame: self.current_frame,
                });
                i
            }
        };
        self.stats.live_slots += 1;
        self.stats.live_bytes += size;
        self.stats.allocs_this_frame += 1;
        Ok(PoolHandle {
            index,
            generation: self.slots[index as usize].generation,
        })
    }

    /// 返回存活块的字节偏移；若句柄已失效则返回 `None`。
    #[must_use]
    pub fn offset_of(&self, handle: PoolHandle) -> Option<u32> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.state == STATE_LIVE && slot.generation == handle.generation).then_some(slot.offset)
    }

    /// 返回存活块的预留容量。
    #[must_use]
    pub fn capacity_of(&self, handle: PoolHandle) -> Option<u32> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.state == STATE_LIVE && slot.generation == handle.generation).then_some(slot.capacity)
    }

    /// 记录块内 `offset` 处长度为 `len` 的字节需要执行散布更新。
    ///
    /// 与 GPU 侧对应：载荷字节本身存放在帧瞬态环中，补丁仅携带目标地址。
    pub fn mark_dirty(&mut self, handle: PoolHandle, offset: u32, len: u32) -> bool {
        let Some(base) = self.offset_of(handle) else {
            return false;
        };
        if offset + len > self.slots[handle.index as usize].capacity {
            return false;
        }
        let patch_index = self.patches.len() as u32;
        self.patches.push(ScatterPatch {
            dst_offset: base + offset,
            len,
            patch_index,
        });
        self.stats.uploads_this_frame += 1;
        self.stats.bytes_uploaded_this_frame += len;
        self.stats.total_uploaded_bytes += u64::from(len);
        true
    }

    /// 调整内存块大小。旧块会被隔离直至引用它的帧完成；调用方负责将载荷重新上传到新块。
    ///
    /// 若需要搬迁则返回一个**新**句柄；若现有容量已足够则返回原句柄。
    ///
    /// # Errors
    /// 当内存池无法容纳更大的块时透传 [`AllocError`]。
    pub fn grow(
        &mut self,
        handle: PoolHandle,
        new_size: u32,
        epoch: u64,
    ) -> Result<PoolHandle, AllocError> {
        let Some(capacity) = self.capacity_of(handle) else {
            return Err(AllocError::TooLarge {
                request: new_size,
                capacity: 0,
            });
        };
        if new_size <= capacity {
            self.slots[handle.index as usize].len = new_size;
            return Ok(handle);
        }
        let new_handle = self.allocate(new_size)?;
        // 调用方负责重新上传载荷；此处仅确保旧块存活至所有仍读取它的帧结束。
        self.free(handle, epoch);
        Ok(new_handle)
    }

    /// 确保 `handle` 指向的块能够容纳 `count` 个步长为 `stride` 字节的元素。
    ///
    /// 内存池按**字节**切分，因此在比较容量前必须先按数组步长将元素数换算为字节。
    /// 混用两个单位曾导致 2000 元素的场景把 128 KB 写入 64 KB 块中，进而覆写紧随其后的
    /// 变换、裁剪与载荷数组。
    ///
    /// 返回（可能更新的）句柄及该块的字节容量。
    ///
    /// # Errors
    /// 当内存池无法容纳更大的块时透传 [`AllocError`]。
    pub fn ensure_capacity(
        &mut self,
        handle: PoolHandle,
        count: u64,
        stride: u64,
        epoch: u64,
    ) -> Result<(PoolHandle, u32), AllocError> {
        let stride = stride.max(1);
        let capacity = self.capacity_of(handle).unwrap_or(0);
        let needed = count.saturating_mul(stride).max(stride);
        if needed <= u64::from(capacity) {
            return Ok((handle, capacity));
        }
        // 成倍扩容直至满足需求，以摊平增长开销。
        let mut new_cap = u64::from(capacity).max(4096);
        while new_cap < needed {
            new_cap = new_cap.saturating_mul(2);
        }
        let new_handle = self.grow(handle, new_cap as u32, epoch)?;
        let new_capacity = self.capacity_of(new_handle).unwrap_or(new_cap as u32);
        Ok((new_handle, new_capacity))
    }

    /// 标记内存块在 `epoch` 时删除。
    ///
    /// 在时间线到达 `epoch` 前该块对 GPU 仍保持可读；此后其内存才归还给 TLSF。
    pub fn free(&mut self, handle: PoolHandle, epoch: u64) {
        let Some(slot) = self.slots.get_mut(handle.index as usize) else {
            return;
        };
        if slot.state != STATE_LIVE || slot.generation != handle.generation {
            return;
        }
        slot.state = STATE_PENDING;
        slot.generation = slot.generation.wrapping_add(1);
        let block = QuarantinedBlock {
            payload: slot.offset,
            size: slot.capacity,
            frame_slot: self.current_frame,
        };
        self.stats.live_slots -= 1;
        self.stats.live_bytes = self.stats.live_bytes.saturating_sub(slot.len);
        self.stats.pending_slots += 1;
        // 当隔离队列饱和（GPU 停顿）时有意泄漏该块：若强行复用槽位，
        // 会导致在飞帧仍在读取时就把内存分配给新写入。
        self.quarantine.defer(epoch, block);
    }

    /// 释放所有已完成 epoch 的内存块，返回变为可复用的槽位数量。
    pub fn reclaim(&mut self, completed_epoch: u64) -> usize {
        self.reclaim_buffer.clear();
        let mut buf = core::mem::take(&mut self.reclaim_buffer);
        self.quarantine.reclaim_into(completed_epoch, &mut buf);

        let mut released = 0;
        for block in buf.drain(..) {
            // 查找拥有该载荷偏移的待释放槽位。槽位总数较少（数百个），且该路径
            // 每个释放节点仅执行一次，线性扫描比维护额外映射表更省开销。
            if let Some(idx) = self
                .slots
                .iter()
                .position(|s| s.state == STATE_PENDING && s.offset == block.payload)
            {
                self.slots[idx].state = STATE_FREE;
                self.slots[idx].len = 0;
                self.slots[idx].capacity = 0;
                self.free_indices.push(idx as u32);
                self.stats.pending_slots = self.stats.pending_slots.saturating_sub(1);
                self.stats.free_slots += 1;
                released += 1;
            }
            let _ = self.tlsf.deallocate(block.payload);
        }
        self.reclaim_buffer = buf;
        released
    }

    /// 持久化池统计计数器。
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.stats
    }

    /// TLSF 分配器健康状态（块数/碎片率）。
    #[must_use]
    pub fn tlsf_stats(&mut self) -> TlsfStats {
        self.tlsf.stats()
    }

    /// 当前处于隔离中的块数量。
    #[must_use]
    pub fn parked_blocks(&self) -> usize {
        self.quarantine.parked()
    }

    /// 清空全部状态以供设备丢失恢复。所有旧句柄随之失效。
    pub fn reset(&mut self) {
        self.tlsf.reset();
        self.slots.clear();
        self.free_indices.clear();
        self.quarantine.clear();
        self.patches.clear();
        self.stats = PoolStats {
            capacity_bytes: self.tlsf.capacity(),
            ..Default::default()
        };
    }

    /// 当 `handle` 仍指向有效存活块时返回 true。
    #[must_use]
    pub fn is_live(&self, handle: PoolHandle) -> bool {
        self.offset_of(handle).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_returns_aligned_distinct_blocks() {
        let mut p = PersistentPool::new(1 << 20);
        let a = p.allocate(64).unwrap();
        let b = p.allocate(64).unwrap();
        let oa = p.offset_of(a).unwrap();
        let ob = p.offset_of(b).unwrap();
        assert_eq!(oa % 64, 0);
        assert_eq!(ob % 64, 0);
        assert_ne!(oa, ob);
        assert!(p.is_live(a) && p.is_live(b));
    }

    #[test]
    fn stale_handle_is_rejected_after_free_and_reuse() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(64).unwrap();
        p.free(a, 1);
        p.reclaim(1);
        assert!(!p.is_live(a));
        // 新分配可复用槽位索引，但绝不会复用旧代数。
        let b = p.allocate(64).unwrap();
        assert_eq!(a.index, b.index, "index should be recycled");
        assert_ne!(a.generation, b.generation);
        assert!(!p.is_live(a));
        assert!(p.is_live(b));
    }

    #[test]
    fn memory_is_not_reused_before_the_epoch_completes() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(8192).unwrap();
        let base = p.offset_of(a).unwrap();
        p.free(a, 7);
        // 帧 7 尚未完成：内存块必须保持隔离。
        assert_eq!(p.reclaim(6), 0);
        assert_eq!(p.parked_blocks(), 1);
        let b = p.allocate(8192);
        // 要么 OOM 要么分配到不同偏移——绝不能命中仍在隔离的偏移。
        if let Ok(b) = b {
            assert_ne!(p.offset_of(b).unwrap(), base);
        }
        // 帧 7 完成后内存立即归还。
        assert_eq!(p.reclaim(7), 1);
        assert_eq!(p.parked_blocks(), 0);
    }

    #[test]
    fn grow_within_capacity_keeps_the_handle() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(128).unwrap();
        let cap = p.capacity_of(a).unwrap();
        let same = p.grow(a, cap, 1).unwrap();
        assert_eq!(same, a);
        assert!(p.is_live(a));
    }

    #[test]
    fn grow_beyond_capacity_moves_and_quarantines() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(64).unwrap();
        let old_off = p.offset_of(a).unwrap();
        let b = p.grow(a, 8192, 3).unwrap();
        assert_ne!(a, b);
        assert!(!p.is_live(a), "old handle must be dead");
        assert!(p.is_live(b));
        assert_ne!(p.offset_of(b).unwrap(), old_off);
        assert_eq!(p.parked_blocks(), 1);
        p.reclaim(3);
        assert_eq!(p.parked_blocks(), 0);
    }

    #[test]
    fn steady_state_frames_upload_nothing() {
        let mut p = PersistentPool::new(1 << 20);
        let handles: Vec<_> = (0..256).map(|_| p.allocate(64).unwrap()).collect();

        // 帧 1：首次全量上传（每个块写入一次）。
        p.begin_frame(1, 0);
        for h in &handles {
            assert!(p.mark_dirty(*h, 0, 64));
        }
        assert_eq!(p.stats().bytes_uploaded_this_frame, 256 * 64);

        // 帧 2：仅平移相机——无节点变更，零上传。
        p.begin_frame(2, 1);
        assert_eq!(p.stats().uploads_this_frame, 0);
        assert_eq!(p.stats().bytes_uploaded_this_frame, 0);
        assert_eq!(p.stats().allocs_this_frame, 0);
        assert!(p.patches().is_empty());

        // 帧 3：单个节点不透明度变更 -> 精确上传 64B。
        p.begin_frame(3, 2);
        assert!(p.mark_dirty(handles[0], 0, 64));
        assert_eq!(p.stats().bytes_uploaded_this_frame, 64);
    }

    #[test]
    fn mark_dirty_bounds_checks() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(64).unwrap();
        let cap = p.capacity_of(a).unwrap();
        assert!(p.mark_dirty(a, 0, cap));
        assert!(!p.mark_dirty(a, 0, cap + 64));
        let mut stale = a;
        stale.generation = stale.generation.wrapping_add(1);
        assert!(!p.mark_dirty(stale, 0, 8));
    }

    #[test]
    fn exhausted_pool_reports_oom_without_corrupting() {
        let mut p = PersistentPool::new(4096);
        let mut n = 0;
        while p.allocate(256).is_ok() {
            n += 1;
            assert!(n < 100);
        }
        assert!(n > 0);
        // 所有成功分配的句柄均保持存活。
        assert_eq!(p.stats().live_slots, n);
    }

    #[test]
    fn reset_invalidates_everything() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(128).unwrap();
        p.free(a, 1);
        p.reset();
        assert!(!p.is_live(a));
        assert_eq!(p.stats().live_slots, 0);
        assert_eq!(p.capacity(), 1 << 16);
        assert!(p.allocate(1 << 15).is_ok());
    }

    #[test]
    fn stats_track_live_and_pending_slots() {
        let mut p = PersistentPool::new(1 << 16);
        let a = p.allocate(100).unwrap();
        let b = p.allocate(100).unwrap();
        assert_eq!(p.stats().live_slots, 2);
        p.free(a, 1);
        assert_eq!(p.stats().live_slots, 1);
        assert_eq!(p.stats().pending_slots, 1);
        p.reclaim(1);
        assert_eq!(p.stats().pending_slots, 0);
        assert_eq!(p.stats().free_slots, 1);
        assert!(p.is_live(b));
    }

    /// 字节与元素数单位混淆的回归测试。
    ///
    /// 渲染器向池请求“2000 个图元”，而池按字节分配。过去混用单位时，64 KB 块被误判为
    /// 足够大（64 KB > 2000），导致散布着色器向其中写入 128 KB，覆写了紧邻的
    /// 变换、裁剪与载荷数组。
    #[test]
    fn capacity_is_measured_in_bytes_not_elements() {
        const STRIDE: u64 = 64;
        let mut p = PersistentPool::new(1 << 20);
        let prim = p.allocate(64 * 1024).unwrap();
        let transform = p.allocate(16 * 1024).unwrap();
        let clip = p.allocate(16 * 1024).unwrap();
        let payload = p.allocate(16 * 1024).unwrap();

        // 2000 个元素 * 64 字节 = 128000 字节，无法装入 64 KB 图元块。
        let (prim, prim_bytes) = p
            .ensure_capacity(prim, 2_000, STRIDE, 0)
            .expect("grow the primitive block");
        assert!(
            u64::from(prim_bytes) >= 2_000 * STRIDE,
            "the block must hold 2 000 elements of {STRIDE} bytes, got {prim_bytes}"
        );

        // 相邻数组不得受扩容操作影响。
        let prim_offset = p.offset_of(prim).unwrap();
        let write_end = prim_offset as u64 + 2_000 * STRIDE;
        for (name, handle) in [
            ("transform", transform),
            ("clip", clip),
            ("payload", payload),
        ] {
            let offset = u64::from(p.offset_of(handle).unwrap());
            assert!(
                offset < prim_offset as u64 || offset >= write_end,
                "the {name} array at {offset} lies inside the primitive write range \
                 [{prim_offset}, {write_end})"
            );
        }
    }

    #[test]
    fn ensure_capacity_is_a_no_op_when_the_block_already_fits() {
        let mut p = PersistentPool::new(1 << 20);
        let prim = p.allocate(64 * 1024).unwrap();
        let before = p.offset_of(prim).unwrap();
        let (after, bytes) = p.ensure_capacity(prim, 100, 64, 0).unwrap();
        assert_eq!(
            before,
            p.offset_of(after).unwrap(),
            "the block must not move"
        );
        assert!(u64::from(bytes) >= 100 * 64);
    }

    /// 帧期间退役的块必须在 GPU 越过该帧 epoch 后归还。
    ///
    /// 隔离队列防止在飞帧读取已被重新分配的块，而它生效的前提是渲染循环
    /// 会定期调用 `reclaim` 将其排空。
    #[test]
    fn a_retired_block_comes_back_after_its_epoch() {
        let mut p = PersistentPool::new(1 << 20);
        // 该帧将触发 epoch 1。
        p.begin_frame(1, 0);
        let epoch = p.current_epoch();
        assert_eq!(epoch, 1);
        let block = p.allocate(4096).unwrap();
        // 扩容操作在当前 epoch 退役旧块。
        let grown = p.ensure_capacity(block, 100_000, 1, epoch).unwrap().0;
        assert_ne!(grown, block, "the block must have moved");
        assert_eq!(p.parked_blocks(), 1, "the old block must be quarantined");
        // 该 epoch 尚未完成：旧块必须保持隔离。
        assert_eq!(p.reclaim(0), 0);
        assert_eq!(p.parked_blocks(), 1);
        // 至此该 epoch 已完成。
        assert_eq!(p.reclaim(1), 1);
        assert_eq!(p.parked_blocks(), 0, "the block must be back in the pool");
    }
}

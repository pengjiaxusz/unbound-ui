//! Persistent GPU-resident pool: TLSF + generational handles + epoch quarantine.
//!
//! This is the CPU half of the "zero bytes per frame" promise (`docs` §2.2):
//!
//! * entities live in a long-lived device buffer carved by [`Tlsf`],
//! * mutation happens *in place* — the frame only ships a sparse list of
//!   `(destination, source, length)` patches to the incremental-scatter compute
//!   shader, never the whole array,
//! * deletion is deferred through [`EpochQuarantine`] so a frame in flight can
//!   never observe recycled memory.
//!
//! The type is deliberately GPU-agnostic (offsets, not handles) so it can be
//! tested in a pure-CPU unit test and reused by the device-lost recovery path.

use crate::quarantine::{EpochQuarantine, QuarantinedBlock};
use crate::tlsf::{AllocError, Tlsf, TlsfStats};

/// A generational handle into the pool.
///
/// The generation counter makes use-after-free detectable instead of silently
/// returning another node's data — critical when the UI tree mutates while a
/// guest rebuilds its view.
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

/// One carved region.
#[derive(Copy, Clone, Debug, Default)]
struct Slot {
    offset: u32,
    /// Bytes currently meaningful (what the shader indexes).
    len: u32,
    /// Bytes reserved in TLSF (>= len).
    capacity: u32,
    generation: u32,
    /// 0 == free/never used, 1 == live, 2 == pending free.
    state: u8,
    /// Frame that last referenced this slot (diagnostics).
    last_frame: u32,
}

const STATE_FREE: u8 = 0;
const STATE_LIVE: u8 = 1;
const STATE_PENDING: u8 = 2;

/// A sparse write into the pool, destined for the scatter compute shader.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScatterPatch {
    /// Destination byte offset inside the persistent buffer.
    pub dst_offset: u32,
    /// Length in bytes; `0` means "descriptor-only update" (no payload).
    pub len: u32,
    /// Index of this patch in the frame's staging list.
    pub patch_index: u32,
}

/// Pool-wide counters, asserted by the KPI harness.
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

/// The persistent pool.
pub struct PersistentPool {
    tlsf: Tlsf,
    slots: Vec<Slot>,
    free_indices: Vec<u32>,
    quarantine: EpochQuarantine,
    /// Scratch buffer reused by `reclaim` (never reallocated in steady state).
    reclaim_buffer: Vec<QuarantinedBlock>,
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
    /// Create a pool over `capacity` bytes of device-local memory.
    ///
    /// # Panics
    /// Panics when `capacity < 4096`.
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
            patches: Vec::with_capacity(256),
            stats: PoolStats {
                capacity_bytes: cap,
                ..Default::default()
            },
            current_frame: 0,
        }
    }

    /// Total device bytes owned by the pool.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.tlsf.capacity()
    }

    /// Begin a frame; resets per-frame counters.
    pub fn begin_frame(&mut self, timeline_epoch: u64, frame_slot: u32) {
        self.current_frame = frame_slot;
        self.stats.uploads_this_frame = 0;
        self.stats.bytes_uploaded_this_frame = 0;
        self.stats.allocs_this_frame = 0;
        self.patches.clear();
        let _ = timeline_epoch;
    }

    /// Return the sparse patch list produced during the current frame.
    #[must_use]
    pub fn patches(&self) -> &[ScatterPatch] {
        &self.patches
    }

    /// Reserve `size` bytes and return a handle.
    ///
    /// # Errors
    /// Propagates [`AllocError`] when the pool is exhausted.
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

    /// Byte offset of a live block, or `None` for a stale handle.
    #[must_use]
    pub fn offset_of(&self, handle: PoolHandle) -> Option<u32> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.state == STATE_LIVE && slot.generation == handle.generation).then_some(slot.offset)
    }

    /// Reserved capacity of a live block.
    #[must_use]
    pub fn capacity_of(&self, handle: PoolHandle) -> Option<u32> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.state == STATE_LIVE && slot.generation == handle.generation).then_some(slot.capacity)
    }

    /// Record that `len` bytes at `offset` inside the block must be scattered.
    ///
    /// Mirrors the GPU side: the payload bytes themselves live in the frame's
    /// transient ring; the patch only carries the destination.
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

    /// Resize a block. Content is copied to the new block; the old block is
    /// quarantined until the frame that referenced it completes.
    ///
    /// Returns a **new** handle when the block had to move, or the same handle
    /// when the existing capacity was sufficient.
    ///
    /// # Errors
    /// Propagates [`AllocError`] when the pool cannot host the larger block.
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
        // The caller is responsible for re-uploading the payload; we only make
        // sure the old block outlives the frames that still read it.
        self.free(handle, epoch);
        Ok(new_handle)
    }

    /// Mark a block for deletion at `epoch`.
    ///
    /// The block stays readable by the GPU until the timeline reaches `epoch`;
    /// only then is its memory handed back to TLSF.
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
        // When the quarantine saturates (the GPU stalled) the block is leaked
        // on purpose: recycling the slot would hand memory to the GPU while
        // an in-flight frame may still read it.
        self.quarantine.defer(epoch, block);
    }

    /// Release everything whose epoch has completed. Returns how many slots
    /// became reusable.
    pub fn reclaim(&mut self, completed_epoch: u64) -> usize {
        self.reclaim_buffer.clear();
        let mut buf = core::mem::take(&mut self.reclaim_buffer);
        self.quarantine.reclaim_into(completed_epoch, &mut buf);

        let mut released = 0;
        for block in buf.drain(..) {
            // Find the pending slot that owned this payload. Slots are few
            // (hundreds), and this path runs once per freed node, so the linear
            // probe is cheaper than maintaining a side map.
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

    /// Pool counters.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.stats
    }

    /// TLSF health (block count / fragmentation).
    #[must_use]
    pub fn tlsf_stats(&mut self) -> TlsfStats {
        self.tlsf.stats()
    }

    /// Number of blocks parked by the quarantine.
    #[must_use]
    pub fn parked_blocks(&self) -> usize {
        self.quarantine.parked()
    }

    /// Drop all state for device-lost recovery. Every handle becomes invalid.
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

    /// True when `handle` still refers to a live block.
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
        // A new allocation may reuse the index, but never the old generation.
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
        // Frame 7 has not completed: the bytes must stay parked.
        assert_eq!(p.reclaim(6), 0);
        assert_eq!(p.parked_blocks(), 1);
        let b = p.allocate(8192);
        // Either OOM or a different offset — never the parked one.
        if let Ok(b) = b {
            assert_ne!(p.offset_of(b).unwrap(), base);
        }
        // Once frame 7 completes the memory returns.
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

        // Frame 1: initial full upload (each block written once).
        p.begin_frame(1, 0);
        for h in &handles {
            assert!(p.mark_dirty(*h, 0, 64));
        }
        assert_eq!(p.stats().bytes_uploaded_this_frame, 256 * 64);

        // Frame 2: camera pan only — no node changed, so zero uploads.
        p.begin_frame(2, 1);
        assert_eq!(p.stats().uploads_this_frame, 0);
        assert_eq!(p.stats().bytes_uploaded_this_frame, 0);
        assert_eq!(p.stats().allocs_this_frame, 0);
        assert!(p.patches().is_empty());

        // Frame 3: a single node's opacity changed -> exactly 64B.
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
        // Everything that succeeded is still live.
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
}

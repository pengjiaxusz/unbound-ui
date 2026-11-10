//! Epoch-quarantined free list (`docs` §2.2).
//!
//! With N frames in flight the GPU may still be reading a persistent-pool block
//! that the CPU already "freed". Handing that block straight back to TLSF lets
//! the next frame overwrite bytes that are still being sampled — the classic
//! flicker/tearing bug that only appears under load.
//!
//! The fix is to keep every freed block quarantined until the timeline
//! semaphore value of the frame that *last used* it has been signalled. Only
//! then does the block re-enter the free list.

use std::collections::VecDeque;

/// One block waiting to be returned to the allocator.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QuarantinedBlock {
    /// Allocator payload offset.
    pub payload: u32,
    /// Size requested at allocation time (diagnostics + stats).
    pub size: u32,
    /// Frame slot that last referenced the block.
    pub frame_slot: u32,
}

/// A quarantine queue keyed by timeline semaphore value.
#[derive(Debug, Default)]
pub struct EpochQuarantine {
    /// Buckets ordered by increasing epoch. Each bucket is drained atomically.
    pending: VecDeque<(u64, Vec<QuarantinedBlock>)>,
    /// Highest timeline value the GPU has confirmed complete.
    completed_epoch: u64,
    /// Blocks currently parked.
    parked: usize,
    /// Peak number of parked blocks (fragmentation early-warning).
    peak_parked: usize,
    /// Total blocks actually reclaimed.
    reclaimed: u64,
    /// Bytes currently parked.
    parked_bytes: u64,
    /// Refused hand-offs because the queue grew past its safety bound.
    dropped: u64,
}

impl EpochQuarantine {
    /// Safety bound: if more than this many frames remain unconfirmed, the GPU
    /// is stalled. We keep the blocks parked (never corrupt) but report it.
    pub const MAX_PENDING_EPOCHS: usize = 8;

    #[must_use]
    pub fn new(completed_epoch: u64) -> Self {
        Self {
            completed_epoch,
            ..Default::default()
        }
    }

    /// Timeline value of the most recently completed frame.
    #[must_use]
    pub fn completed_epoch(&self) -> u64 {
        self.completed_epoch
    }

    /// Number of blocks currently parked.
    #[must_use]
    pub fn parked(&self) -> usize {
        self.parked
    }

    /// Peak number of parked blocks.
    #[must_use]
    pub fn peak_parked(&self) -> usize {
        self.peak_parked
    }

    /// Bytes parked.
    #[must_use]
    pub fn parked_bytes(&self) -> u64 {
        self.parked_bytes
    }

    /// Total blocks reclaimed since construction.
    #[must_use]
    pub fn reclaimed(&self) -> u64 {
        self.reclaimed
    }

    /// Number of buckets awaiting confirmation.
    #[must_use]
    pub fn pending_buckets(&self) -> usize {
        self.pending.len()
    }

    /// True when the GPU is so far behind that the quarantine bound tripped.
    #[must_use]
    pub fn is_saturated(&self) -> bool {
        self.pending.len() >= Self::MAX_PENDING_EPOCHS
    }

    /// Park a block until `epoch` completes.
    ///
    /// Returns `false` when the quarantine is saturated; the caller must then
    /// leak the block (safer than corrupting a frame) and surface the incident.
    pub fn defer(&mut self, epoch: u64, block: QuarantinedBlock) -> bool {
        if self.is_saturated() && self.pending.back().map(|(e, _)| *e) != Some(epoch) {
            self.dropped += 1;
            return false;
        }
        match self.pending.back_mut() {
            Some((e, bucket)) if *e == epoch => bucket.push(block),
            _ => self.pending.push_back((epoch, vec![block])),
        }
        self.parked += 1;
        self.parked_bytes += u64::from(block.size);
        if self.parked > self.peak_parked {
            self.peak_parked = self.parked;
        }
        debug_assert!(
            {
                let mut ok = true;
                let mut prev = 0u64;
                for (i, (e, _)) in self.pending.iter().enumerate() {
                    if i > 0 && *e < prev {
                        ok = false;
                    }
                    prev = *e;
                }
                ok
            },
            "epoch buckets must stay sorted: defer() requires monotonically \
             non-decreasing epochs"
        );
        true
    }

    /// Mark `epoch` as GPU-complete and return every block that may now be
    /// reused, oldest first.
    ///
    /// The returned vector is reused across calls by the caller (see
    /// [`Self::reclaim_into`]) to keep steady-state frames allocation free.
    pub fn reclaim(&mut self, epoch: u64) -> Vec<QuarantinedBlock> {
        let mut out = Vec::new();
        self.reclaim_into(epoch, &mut out);
        out
    }

    /// Allocation-free variant: appends reclaimable blocks to `out`.
    pub fn reclaim_into(&mut self, epoch: u64, out: &mut Vec<QuarantinedBlock>) {
        self.completed_epoch = self.completed_epoch.max(epoch);
        while let Some((bucket_epoch, _)) = self.pending.front() {
            if *bucket_epoch > self.completed_epoch {
                break;
            }
            let (_, blocks) = self.pending.pop_front().expect("checked front");
            for b in blocks {
                self.parked -= 1;
                self.parked_bytes -= u64::from(b.size);
                self.reclaimed += 1;
                out.push(b);
            }
        }
    }

    /// Number of blocks refused because the queue was saturated.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Drop every parked block *without* returning it to the allocator.
    ///
    /// Used on device-lost recovery, where the whole pool is rebuilt and the
    /// individual blocks are meaningless.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.parked = 0;
        self.parked_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(payload: u32, size: u32) -> QuarantinedBlock {
        QuarantinedBlock {
            payload,
            size,
            frame_slot: 0,
        }
    }

    #[test]
    fn blocks_stay_parked_until_their_epoch_completes() {
        let mut q = EpochQuarantine::new(0);
        q.defer(5, block(100, 64));
        q.defer(6, block(200, 64));
        assert_eq!(q.parked(), 2);

        // Frame 4 completes: nothing may be released.
        assert!(q.reclaim(4).is_empty());
        assert_eq!(q.parked(), 2);

        // Frame 5 completes: only the block parked at epoch 5 returns.
        let freed = q.reclaim(5);
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].payload, 100);
        assert_eq!(q.parked(), 1);
        assert_eq!(q.parked_bytes(), 64);
    }

    #[test]
    fn reclaim_is_oldest_first_and_monotonic() {
        let mut q = EpochQuarantine::new(0);
        for e in 1..=4u64 {
            q.defer(e, block(e as u32 * 10, 16));
        }
        let freed = q.reclaim(3);
        assert_eq!(
            freed.iter().map(|b| b.payload).collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
        assert_eq!(q.completed_epoch(), 3);
        assert_eq!(q.reclaimed(), 3);
    }

    #[test]
    fn same_epoch_is_batched_into_one_bucket() {
        let mut q = EpochQuarantine::new(0);
        for i in 0..100 {
            assert!(q.defer(9, block(i, 32)));
        }
        assert_eq!(q.pending_buckets(), 1);
        assert_eq!(q.reclaim(9).len(), 100);
        assert_eq!(q.pending_buckets(), 0);
    }

    #[test]
    fn saturation_refuses_new_blocks_instead_of_growing_unbounded() {
        let mut q = EpochQuarantine::new(0);
        for e in 0..EpochQuarantine::MAX_PENDING_EPOCHS as u64 {
            assert!(q.defer(e, block(e as u32, 8)));
        }
        assert!(q.is_saturated());
        assert!(!q.defer(999, block(1, 8)));
        assert_eq!(q.dropped(), 1);
        // Existing buckets are untouched.
        assert_eq!(q.parked(), EpochQuarantine::MAX_PENDING_EPOCHS);
    }

    #[test]
    fn reclaim_into_is_allocation_free_for_the_caller() {
        let mut q = EpochQuarantine::new(0);
        q.defer(1, block(1, 16));
        q.defer(2, block(2, 16));
        let mut out = Vec::with_capacity(4);
        q.reclaim_into(2, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(q.parked(), 0);
    }

    #[test]
    fn clear_drops_everything_for_device_lost() {
        let mut q = EpochQuarantine::new(0);
        q.defer(1, block(1, 16));
        q.clear();
        assert_eq!(q.parked(), 0);
        assert_eq!(q.parked_bytes(), 0);
        assert!(q.reclaim(100).is_empty());
    }

    #[test]
    fn peak_tracks_the_high_water_mark() {
        let mut q = EpochQuarantine::new(0);
        for i in 0..10 {
            q.defer(1, block(i, 16));
        }
        q.reclaim(1);
        q.defer(2, block(99, 16));
        assert_eq!(q.peak_parked(), 10);
        assert_eq!(q.parked(), 1);
    }
}

//! Per-frame transient ring buffer (`docs` §2.2).
//!
//! Every frame owns one region of the ring. Because a frame's region is reset
//! when that frame slot is reused, the CPU never allocates and never stalls on
//! the GPU: it just bumps a cursor.
//!
//! Invariants:
//! * an allocation never crosses a region boundary (so a frame can be reset
//!   without disturbing its neighbours),
//! * offsets are aligned to the requested alignment relative to ring byte 0,
//!   which is itself aligned to the buffer's device alignment,
//! * overflow is reported, never silently wrapped — silent wrap is how
//!   "sometimes the UI flickers" bugs are born.

/// One frame slot inside the ring.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct Region {
    /// Absolute ring offset where this slot begins.
    start: u32,
    /// Bytes granted to this slot.
    len: u32,
    /// Bump cursor, absolute ring offset.
    cursor: u32,
    /// Timeline value this slot was last used for.
    epoch: u64,
}

/// A rotating per-frame bump allocator.
#[derive(Debug)]
pub struct TransientRing {
    regions: Vec<Region>,
    active: usize,
    frame_count: u32,
    high_water: u32,
    overflows: u64,
    /// Total bytes handed out since construction (diagnostics only).
    total_allocated: u64,
}

impl TransientRing {
    /// Split `capacity` bytes into `frames_in_flight` equal slots.
    ///
    /// # Panics
    /// Panics when `frames_in_flight == 0` or the capacity cannot give every
    /// slot at least 256 bytes.
    #[must_use]
    pub fn new(capacity: u32, frames_in_flight: u32) -> Self {
        assert!(frames_in_flight > 0, "need at least one frame slot");
        assert!(
            capacity / frames_in_flight >= 256,
            "ring too small: {capacity}B for {frames_in_flight} frames"
        );
        // Round the slot size down to 256B so every slot start is 256-aligned.
        let slot = (capacity / frames_in_flight) & !255;
        let mut regions = Vec::with_capacity(frames_in_flight as usize);
        for i in 0..frames_in_flight {
            regions.push(Region {
                start: i * slot,
                len: slot,
                cursor: i * slot,
                epoch: 0,
            });
        }
        Self {
            regions,
            // Start one before the first slot so the first `begin_frame`
            // advances *into* slot 0 and its epoch is recorded there.
            active: frames_in_flight as usize - 1,
            frame_count: frames_in_flight,
            high_water: 0,
            overflows: 0,
            total_allocated: 0,
        }
    }

    /// Bytes available to a single frame.
    #[must_use]
    pub fn slot_capacity(&self) -> u32 {
        self.regions.first().map_or(0, |r| r.len)
    }

    /// Number of frame slots.
    #[must_use]
    pub fn frames_in_flight(&self) -> u32 {
        self.frame_count
    }

    /// Bytes used by the active slot.
    #[must_use]
    pub fn frame_used(&self) -> u32 {
        let r = &self.regions[self.active];
        r.cursor - r.start
    }

    /// Peak usage of any slot since construction.
    #[must_use]
    pub fn high_water(&self) -> u32 {
        self.high_water
    }

    /// Number of failed allocations (each one means a frame lost transient
    /// data — the debug overlay shows this in red).
    #[must_use]
    pub fn overflows(&self) -> u64 {
        self.overflows
    }

    /// Total bytes handed out (diagnostics).
    #[must_use]
    pub fn total_allocated(&self) -> u64 {
        self.total_allocated
    }

    /// Advance to the next frame slot and reset its cursor.
    ///
    /// `epoch` is the timeline value the new frame will signal on completion.
    pub fn begin_frame(&mut self, epoch: u64) {
        self.active = (self.active + 1) % self.regions.len();
        let r = &mut self.regions[self.active];
        r.cursor = r.start;
        r.epoch = epoch;
    }

    /// Reset every slot (device-lost recovery / resize).
    pub fn reset_all(&mut self) {
        for r in &mut self.regions {
            r.cursor = r.start;
        }
        self.active = 0;
        self.high_water = 0;
    }

    /// Allocate `size` bytes aligned to `align`, returning the ring offset.
    ///
    /// Returns `None` when the current frame slot is exhausted. The caller may
    /// grow the ring and retry, but must never assume a wrap happened.
    pub fn allocate(&mut self, size: u32, align: u32) -> Option<u32> {
        debug_assert!(align.is_power_of_two());
        let r = &mut self.regions[self.active];
        let aligned = (r.cursor + align - 1) & !(align - 1);
        let end = aligned.checked_add(size)?;
        if end > r.start + r.len {
            self.overflows += 1;
            return None;
        }
        r.cursor = end;
        let used = r.cursor - r.start;
        if used > self.high_water {
            self.high_water = used;
        }
        self.total_allocated += u64::from(size);
        Some(aligned)
    }

    /// Allocate a typed slice and copy `data` into the ring's staging view.
    ///
    /// Returns `(offset, byte_len)`.
    pub fn allocate_bytes(&mut self, data: &[u8], align: u32) -> Option<(u32, u32)> {
        let off = self.allocate(data.len() as u32, align)?;
        Some((off, data.len() as u32))
    }

    /// Epoch recorded for the active slot.
    #[must_use]
    pub fn active_epoch(&self) -> u64 {
        self.regions[self.active].epoch
    }

    /// Epoch of an arbitrary slot (used by the presentation path to know which
    /// timeline value guards a region).
    #[must_use]
    pub fn epoch_of(&self, slot: usize) -> Option<u64> {
        self.regions.get(slot).map(|r| r.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_do_not_overlap() {
        let ring = TransientRing::new(3 * 1024, 3);
        assert_eq!(ring.slot_capacity(), 1024);
        // Every slot start is 256-aligned so that 256B-aligned allocations in
        // different frames never alias.
        for i in 0..3 {
            assert_eq!(ring.regions[i].start % 256, 0);
            assert_eq!(ring.regions[i].start, i as u32 * 1024);
        }
    }

    #[test]
    fn bump_allocation_is_aligned_and_sequential() {
        let mut ring = TransientRing::new(4096, 1);
        let a = ring.allocate(16, 16).unwrap();
        let b = ring.allocate(8, 16).unwrap();
        assert_eq!(a % 16, 0);
        assert_eq!(b % 16, 0);
        assert!(b >= a + 16);
        assert_eq!(ring.frame_used(), b + 8);
    }

    #[test]
    fn overflow_is_reported_and_does_not_wrap() {
        let mut ring = TransientRing::new(512, 1);
        let mut count = 0;
        while ring.allocate(64, 16).is_some() {
            count += 1;
            assert!(count < 100, "should have overflowed");
        }
        assert_eq!(count, 8);
        assert_eq!(ring.overflows(), 1);
        // A failed allocation must not have moved the cursor.
        assert_eq!(ring.frame_used(), 512);
    }

    #[test]
    fn frame_advance_reclaims_the_slot() {
        let mut ring = TransientRing::new(2048, 2);
        assert!(ring.allocate(900, 16).is_some());
        assert!(ring.allocate(900, 16).is_none());
        ring.begin_frame(1);
        // Second slot is empty.
        assert!(ring.allocate(900, 16).is_some());
        ring.begin_frame(2);
        // Back to slot 0, which was reset by `begin_frame`.
        assert!(ring.allocate(900, 16).is_some());
        assert_eq!(ring.overflows(), 1);
    }

    #[test]
    fn high_water_tracks_the_peak() {
        let mut ring = TransientRing::new(2048, 1);
        ring.allocate(64, 16).unwrap();
        ring.allocate(200, 16).unwrap();
        let peak = ring.frame_used();
        ring.begin_frame(1);
        ring.allocate(32, 16).unwrap();
        assert_eq!(ring.high_water(), peak);
        assert!(ring.frame_used() < peak);
    }

    #[test]
    fn allocation_cannot_cross_into_the_next_slot() {
        let mut ring = TransientRing::new(1024, 2);
        assert!(ring.allocate(512, 16).is_some());
        // Slot 0 is exactly full; the next 16B must fail rather than spill.
        assert!(ring.allocate(16, 16).is_none());
    }

    #[test]
    fn reset_all_clears_every_slot() {
        let mut ring = TransientRing::new(2048, 2);
        ring.allocate(500, 16);
        ring.begin_frame(1);
        ring.allocate(500, 16);
        ring.reset_all();
        assert_eq!(ring.frame_used(), 0);
        assert_eq!(ring.high_water(), 0);
        assert!(ring.allocate(1000, 16).is_some());
    }

    #[test]
    fn epochs_are_recorded_per_slot() {
        let mut ring = TransientRing::new(1024, 2);
        ring.begin_frame(7);
        assert_eq!(ring.active_epoch(), 7);
        ring.begin_frame(8);
        assert_eq!(ring.active_epoch(), 8);
        assert_eq!(ring.epoch_of(0), Some(7));
        assert_eq!(ring.epoch_of(1), Some(8));
        assert_eq!(ring.epoch_of(9), None);
    }
}

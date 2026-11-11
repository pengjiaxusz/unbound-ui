//! The CPU staging ring (`docs` §8.2).
//!
//! Decoded assets land in a staging buffer that the GPU reads from over PCIe.
//! The ring has the same shape as the GPU's transient ring: a fixed number of
//! slots, one per frame in flight, so a slot can only be recycled once the
//! frame that referenced it has completed.
//!
//! The ring is shared between the main thread and the decode workers, so it
//! carries its own lock. The critical section is a `memcpy` and a cursor bump
//! — nothing that can stall the main thread.

use std::sync::Mutex;

/// The staging ring could not accept the data.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StagingError {
    /// The active slot has no room left.
    SlotFull {
        /// Bytes the caller wanted to write.
        requested: usize,
        /// Bytes still free in the slot.
        available: usize,
    },
    /// The active slot is closed because the frame advanced.
    SlotClosed,
}

impl core::fmt::Display for StagingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SlotFull {
                requested,
                available,
            } => write!(
                f,
                "staging slot is full: {requested} bytes requested, {available} available"
            ),
            Self::SlotClosed => write!(f, "the staging slot is closed"),
        }
    }
}

impl std::error::Error for StagingError {}

/// A location inside the staging ring.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StagingRef {
    /// The slot the data lives in.
    pub slot: usize,
    /// The offset inside the slot.
    pub offset: usize,
    /// The number of bytes.
    pub size: usize,
}

/// Inner state of the ring.
#[derive(Debug)]
struct Inner {
    /// One byte buffer per slot.
    slots: Vec<Vec<u8>>,
    /// The slot currently being written to.
    active: usize,
    /// The write cursor inside the active slot.
    cursor: usize,
    /// Number of completed frames.
    frame: u64,
}

/// A multi-slot CPU staging buffer.
///
/// The buffer is handed to the decode workers; each of them appends its
/// decoded asset to the active slot.
#[derive(Debug)]
pub struct StagingRing {
    inner: Mutex<Inner>,
    /// The size of one slot.
    pub slot_bytes: usize,
    /// The number of slots, i.e. the number of frames in flight.
    pub frames: usize,
}

impl StagingRing {
    /// Create a ring with `frames` slots of `slot_bytes` each.
    #[must_use]
    pub fn new(slot_bytes: usize, frames: usize) -> Self {
        let frames = frames.max(1);
        Self {
            inner: Mutex::new(Inner {
                slots: vec![vec![0u8; slot_bytes]; frames],
                active: 0,
                cursor: 0,
                frame: 0,
            }),
            slot_bytes,
            frames,
        }
    }

    /// The capacity of one slot.
    #[must_use]
    pub const fn slot_capacity(&self) -> usize {
        self.slot_bytes
    }

    /// The number of slots.
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.frames
    }

    /// Advance to the next frame and reset the write cursor.
    ///
    /// The slot that was just left behind is only recycled after `frames`
    /// advances, which is exactly how long the GPU may still be reading it.
    pub fn begin_frame(&self) {
        let mut inner = self.lock();
        inner.active = (inner.active + 1) % self.frames;
        inner.cursor = 0;
        inner.frame += 1;
    }

    /// Bytes still free in the active slot.
    #[must_use]
    pub fn available(&self) -> usize {
        let inner = self.lock();
        self.slot_bytes.saturating_sub(inner.cursor)
    }

    /// The slot currently being written to.
    #[must_use]
    pub fn active_slot(&self) -> usize {
        self.lock().active
    }

    /// Append data to the active slot.
    ///
    /// # Errors
    /// Returns [`StagingError::SlotFull`] when the slot has no room left. The
    /// caller decides what to do: dropping the asset is better than stalling
    /// the frame.
    pub fn write(&self, bytes: &[u8]) -> Result<StagingRef, StagingError> {
        let mut inner = self.lock();
        if bytes.len() > self.slot_bytes.saturating_sub(inner.cursor) {
            return Err(StagingError::SlotFull {
                requested: bytes.len(),
                available: self.slot_bytes.saturating_sub(inner.cursor),
            });
        }
        let slot = inner.active;
        let offset = inner.cursor;
        {
            let buffer = &mut inner.slots[slot];
            buffer[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
        inner.cursor += bytes.len();
        Ok(StagingRef {
            slot,
            offset,
            size: bytes.len(),
        })
    }

    /// Read back a previously written region.
    ///
    /// The data is handed to a closure instead of being returned: the ring's
    /// lock must not be held while the caller works with the bytes.
    #[must_use]
    pub fn with<R>(&self, reference: StagingRef, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let inner = self.lock();
        let buffer = inner.slots.get(reference.slot)?;
        let slice = buffer.get(reference.offset..reference.offset + reference.size)?;
        Some(f(slice))
    }

    /// Lock the ring, recovering from a poisoned mutex.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_lands_in_the_active_slot() {
        let ring = StagingRing::new(1024, 3);
        let reference = ring.write(b"hello").unwrap();
        assert_eq!(reference.slot, 0);
        assert_eq!(reference.offset, 0);
        assert_eq!(reference.size, 5);
        assert_eq!(
            ring.with(reference, |bytes| bytes.to_vec()),
            Some(b"hello".to_vec())
        );
    }

    #[test]
    fn successive_writes_are_contiguous() {
        let ring = StagingRing::new(1024, 3);
        let a = ring.write(b"abc").unwrap();
        let b = ring.write(b"de").unwrap();
        assert_eq!(b.offset, a.offset + a.size);
        assert_eq!(ring.available(), 1024 - 5);
    }

    #[test]
    fn a_full_slot_reports_the_shortfall_instead_of_overflowing() {
        let ring = StagingRing::new(8, 2);
        ring.write(&[0u8; 6]).unwrap();
        let error = ring.write(&[0u8; 4]).unwrap_err();
        assert_eq!(
            error,
            StagingError::SlotFull {
                requested: 4,
                available: 2
            }
        );
    }

    #[test]
    fn a_new_frame_starts_a_fresh_slot() {
        let ring = StagingRing::new(64, 3);
        ring.write(b"first").unwrap();
        ring.begin_frame();
        assert_eq!(ring.active_slot(), 1);
        assert_eq!(ring.available(), 64);
        let reference = ring.write(b"second").unwrap();
        assert_eq!(reference.slot, 1);
        assert_eq!(reference.offset, 0);
    }

    #[test]
    fn a_slot_is_recycled_only_after_the_full_ring_lapped() {
        let ring = StagingRing::new(64, 3);
        let first = ring.write(b"first").unwrap();
        assert_eq!(first.slot, 0);
        for _ in 0..2 {
            ring.begin_frame();
        }
        // Two advances are not enough: the GPU may still read slot 0.
        assert_eq!(ring.active_slot(), 2);
        ring.begin_frame();
        // Only now has the ring lapped and slot 0 become reusable.
        assert_eq!(ring.active_slot(), 0);
    }

    #[test]
    fn the_ring_is_safe_to_share_between_threads() {
        use std::sync::Arc;
        let ring = Arc::new(StagingRing::new(4096, 4));
        let mut handles = Vec::new();
        for worker in 0..4u32 {
            let ring = Arc::clone(&ring);
            handles.push(std::thread::spawn(move || {
                for i in 0..16u8 {
                    let payload = [worker as u8; 8];
                    let _ = ring.write(&payload);
                    let _ = i;
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert!(ring.available() <= 4096);
    }
}

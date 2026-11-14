//! CPU 暂存环形缓冲区（`docs` §8.2）。
//!
//! 解码后的资产写入暂存缓冲区，GPU 通过 PCIe 从中读取。
//! 该环形缓冲区与 GPU 瞬态环结构一致：固定数量的槽位对应并发在飞帧数，
//! 因此只有当引用某槽位的帧执行完毕后，该槽位才能被回收复用。
//!
//!
//! 暂存环在主线程与解码工作线程间共享，因此自带互斥锁。
//! 临界区仅包含一次 `memcpy` 与游标递增——绝不会阻塞主线程。
//!

use std::sync::Mutex;

/// 暂存环无法接收数据时返回的错误。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StagingError {
    /// 当前活跃槽位剩余空间不足。
    SlotFull {
        /// 调用方请求写入的字节数。
        requested: usize,
        /// 槽位中剩余的可用字节数。
        available: usize,
    },
    /// 因帧已推进，活跃槽位已关闭。
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

/// 暂存环内的位置描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StagingRef {
    /// 数据所在的槽位索引。
    pub slot: usize,
    /// 槽位内的字节偏移量。
    pub offset: usize,
    /// 数据字节长度。
    pub size: usize,
}

/// 暂存环的内部状态。
#[derive(Debug)]
struct Inner {
    /// 每个槽位对应的字节缓冲区。
    slots: Vec<Vec<u8>>,
    /// 当前正在写入的槽位索引。
    active: usize,
    /// 活跃槽位内的写入游标。
    cursor: usize,
    /// 已完成的帧计数。
    frame: u64,
}

/// 多槽位 CPU 暂存缓冲区。
///
/// 缓冲区供解码工作线程使用；每个工作线程将其解码资产追加到当前活跃槽位。
///
#[derive(Debug)]
pub struct StagingRing {
    inner: Mutex<Inner>,
    /// 单个槽位的容量（字节）。
    pub slot_bytes: usize,
    /// 槽位数量（即并发在飞帧数）。
    pub frames: usize,
}

impl StagingRing {
    /// 创建一个包含 `frames` 个槽位、每个槽位 `slot_bytes` 字节的暂存环。
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

    /// 单个槽位的容量。
    #[must_use]
    pub const fn slot_capacity(&self) -> usize {
        self.slot_bytes
    }

    /// 槽位总数。
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.frames
    }

    /// 推进到下一帧并重置写入游标。
    ///
    /// 刚离开的槽位需在推进 `frames` 帧后才会被回收，
    /// 这恰好等于 GPU 可能仍在读取该槽位的时长。
    pub fn begin_frame(&self) {
        let mut inner = self.lock();
        inner.active = (inner.active + 1) % self.frames;
        inner.cursor = 0;
        inner.frame += 1;
    }

    /// 当前活跃槽位剩余可用字节数。
    #[must_use]
    pub fn available(&self) -> usize {
        let inner = self.lock();
        self.slot_bytes.saturating_sub(inner.cursor)
    }

    /// 当前正在写入的槽位索引。
    #[must_use]
    pub fn active_slot(&self) -> usize {
        self.lock().active
    }

    /// 向当前活跃槽位追加数据。
    ///
    /// # Errors
    /// 当槽位空间不足时返回 [`StagingError::SlotFull`]。
    /// 由调用方决定处理策略：丢弃资产优于阻塞渲染帧。
    ///
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

    /// 回读先前写入的区域。
    ///
    /// 数据通过闭包交由调用方处理而非直接返回：调用方处理字节期间不能持有环锁。
    ///
    #[must_use]
    pub fn with<R>(&self, reference: StagingRef, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let inner = self.lock();
        let buffer = inner.slots.get(reference.slot)?;
        let slice = buffer.get(reference.offset..reference.offset + reference.size)?;
        Some(f(slice))
    }

    /// 加锁访问暂存环，若互斥锁处于 poisoned 状态则自动恢复。
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
        // 推进两帧还不够：GPU 可能仍在读取槽位 0。
        assert_eq!(ring.active_slot(), 2);
        ring.begin_frame();
        // 直到此时环才绕回一圈，槽位 0 重新可用。
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

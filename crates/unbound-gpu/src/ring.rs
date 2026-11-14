//! 逐帧瞬态环形缓冲区（参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 每帧独占环中的一段区域。当该帧槽位被复用时对应区域整体重置，
//! 因此 CPU 无需动态分配，也不会因等待 GPU 而停顿：只需前移游标即可。
//!
//! 不变量：
//! * 单次分配绝不跨越区域边界（从而重置某帧时不会干扰相邻帧）；
//! * 偏移量相对于环字节 0 按请求对齐（环基址本身已满足设备对齐要求）；
//! * 溢出时显式报错，绝不静默回绕——静默回绕正是“UI 偶发闪烁”类 bug 的根源。

/// 环内的单个帧槽位。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct Region {
    /// 该槽位在环中的起始绝对偏移。
    start: u32,
    /// 分配给该槽位的字节容量。
    len: u32,
    /// 线性前移游标（环内绝对偏移）。
    cursor: u32,
    /// 该槽位上次使用对应的时间线值。
    epoch: u64,
}

/// 轮转式逐帧线性分配器。
#[derive(Debug)]
pub struct TransientRing {
    regions: Vec<Region>,
    active: usize,
    frame_count: u32,
    high_water: u32,
    overflows: u64,
    /// 自创建以来累计分配的总字节数（仅用于诊断）。
    total_allocated: u64,
}

impl TransientRing {
    /// 将 `capacity` 字节均分为 `frames_in_flight` 个槽位。
    ///
    /// # Panics
    /// 当 `frames_in_flight == 0` 或总容量不足以给每个槽位分配至少 256 字节时触发 panic。
    #[must_use]
    pub fn new(capacity: u32, frames_in_flight: u32) -> Self {
        assert!(frames_in_flight > 0, "need at least one frame slot");
        assert!(
            capacity / frames_in_flight >= 256,
            "ring too small: {capacity}B for {frames_in_flight} frames"
        );
        // 将槽位大小向下取整至 256B 的倍数，确保每个槽位起始均满足 256B 对齐。
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
            // 初始索引设为首个槽位之前，使首次 `begin_frame` 恰好推进至槽位 0 并记录其 epoch。
            active: frames_in_flight as usize - 1,
            frame_count: frames_in_flight,
            high_water: 0,
            overflows: 0,
            total_allocated: 0,
        }
    }

    /// 单帧可用的字节容量。
    #[must_use]
    pub fn slot_capacity(&self) -> u32 {
        self.regions.first().map_or(0, |r| r.len)
    }

    /// 帧槽位数量。
    #[must_use]
    pub fn frames_in_flight(&self) -> u32 {
        self.frame_count
    }

    /// 当前活跃槽位已使用的字节数。
    #[must_use]
    pub fn frame_used(&self) -> u32 {
        let r = &self.regions[self.active];
        r.cursor - r.start
    }

    /// 自创建以来任意槽位的峰值使用字节数。
    #[must_use]
    pub fn high_water(&self) -> u32 {
        self.high_water
    }

    /// 分配失败次数（每次失败意味着某帧丢失了瞬态数据——调试覆盖层会标红显示）。
    #[must_use]
    pub fn overflows(&self) -> u64 {
        self.overflows
    }

    /// 累计分配总字节数（诊断用）。
    #[must_use]
    pub fn total_allocated(&self) -> u64 {
        self.total_allocated
    }

    /// 推进到下一个帧槽位并重置其游标。
    ///
    /// `epoch` 为新帧完成时将触发的时间线值。
    pub fn begin_frame(&mut self, epoch: u64) {
        self.active = (self.active + 1) % self.regions.len();
        let r = &mut self.regions[self.active];
        r.cursor = r.start;
        r.epoch = epoch;
    }

    /// 重置所有槽位（用于设备丢失恢复或窗口尺寸变更）。
    pub fn reset_all(&mut self) {
        for r in &mut self.regions {
            r.cursor = r.start;
        }
        self.active = 0;
        self.high_water = 0;
    }

    /// 按 `align` 对齐分配 `size` 字节，返回环内偏移量。
    ///
    /// 当当前帧槽位耗尽时返回 `None`。调用方可以扩容后重试，但绝不能假定发生回绕。
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

    /// 分配类型化切片并将 `data` 拷贝到环的暂存视图中。
    ///
    /// 返回 `(offset, byte_len)`。
    pub fn allocate_bytes(&mut self, data: &[u8], align: u32) -> Option<(u32, u32)> {
        let off = self.allocate(data.len() as u32, align)?;
        Some((off, data.len() as u32))
    }

    /// 当前活跃槽位记录的 epoch。
    #[must_use]
    pub fn active_epoch(&self) -> u64 {
        self.regions[self.active].epoch
    }

    /// 获取指定槽位的 epoch（供呈现路径判断保护该区域的时间线值）。
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
        // 每个槽位起始均 256B 对齐，确保不同帧中的 256B 对齐分配互不混叠。
        for (i, region) in ring.regions.iter().enumerate() {
            assert_eq!(region.start % 256, 0);
            assert_eq!(region.start, i as u32 * 1024);
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
        // 分配失败时不得移动游标。
        assert_eq!(ring.frame_used(), 512);
    }

    #[test]
    fn frame_advance_reclaims_the_slot() {
        let mut ring = TransientRing::new(2048, 2);
        assert!(ring.allocate(900, 16).is_some());
        assert!(ring.allocate(900, 16).is_none());
        ring.begin_frame(1);
        // 第二个槽位为空。
        assert!(ring.allocate(900, 16).is_some());
        ring.begin_frame(2);
        // 回到槽位 0，其游标已被 `begin_frame` 重置。
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
        // 槽位 0 已恰好填满；接下来的 16B 分配必须失败而非溢出。
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

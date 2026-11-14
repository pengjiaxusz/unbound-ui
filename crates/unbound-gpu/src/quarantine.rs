//! 基于 Epoch 隔离的延迟释放队列（参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 在 N 帧同时在飞的情况下，GPU 可能仍在读取 CPU 已经“释放”的持久化池内存块。
//! 若立即将该块归还 TLSF，下一帧就会覆写仍在被采样的字节——这正是高负载下
//! 闪烁或画面撕裂的典型成因。
//!
//! 解决方式是将每个已释放的块隔离保存，直到*最后使用它*的帧对应的时间线信号量
//! 完成信号触发后，才重新放回空闲链表。

use std::collections::VecDeque;

/// 等待归还给分配器的单个内存块。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QuarantinedBlock {
    /// 分配器载荷偏移量。
    pub payload: u32,
    /// 分配时请求的字节大小（用于诊断与统计）。
    pub size: u32,
    /// 最后引用该内存块的帧槽位。
    pub frame_slot: u32,
}

/// 以时间线信号量值为键的隔离队列。
#[derive(Debug, Default)]
pub struct EpochQuarantine {
    /// 按 epoch 递增排列的桶；每个桶以原子方式整体排空。
    pending: VecDeque<(u64, Vec<QuarantinedBlock>)>,
    /// GPU 已确认完成的最高时间线值。
    completed_epoch: u64,
    /// 当前处于隔离中的块数量。
    parked: usize,
    /// 隔离块数量峰值（碎片化预警指标）。
    peak_parked: usize,
    /// 累计已实际回收的块总数。
    reclaimed: u64,
    /// 当前处于隔离中的字节数。
    parked_bytes: u64,
    /// 因队列超出安全上限而被拒绝入队的次数。
    dropped: u64,
}

impl EpochQuarantine {
    /// 安全上限：若未确认帧数超过此值，说明 GPU 已停顿。
    /// 此时继续保留已隔离块（绝不破坏数据）并向上报告。
    pub const MAX_PENDING_EPOCHS: usize = 8;

    #[must_use]
    pub fn new(completed_epoch: u64) -> Self {
        Self {
            completed_epoch,
            ..Default::default()
        }
    }

    /// 最近一次已完成帧的时间线值。
    #[must_use]
    pub fn completed_epoch(&self) -> u64 {
        self.completed_epoch
    }

    /// 当前隔离中的块数量。
    #[must_use]
    pub fn parked(&self) -> usize {
        self.parked
    }

    /// 隔离块数量历史峰值。
    #[must_use]
    pub fn peak_parked(&self) -> usize {
        self.peak_parked
    }

    /// 当前隔离中的字节数。
    #[must_use]
    pub fn parked_bytes(&self) -> u64 {
        self.parked_bytes
    }

    /// 自创建以来累计回收的块总数。
    #[must_use]
    pub fn reclaimed(&self) -> u64 {
        self.reclaimed
    }

    /// 等待确认的桶数量。
    #[must_use]
    pub fn pending_buckets(&self) -> usize {
        self.pending.len()
    }

    /// 当 GPU 落后过多导致触发隔离上限时返回 true。
    #[must_use]
    pub fn is_saturated(&self) -> bool {
        self.pending.len() >= Self::MAX_PENDING_EPOCHS
    }

    /// 隔离内存块直至 `epoch` 完成。
    ///
    /// 若隔离队列已饱和则返回 `false`；调用方此时必须泄漏该块
    /// （比破坏渲染帧更安全）并上报异常。
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

    /// 将 `epoch` 标记为 GPU 已完成，并按从旧到新顺序返回所有可复用的块。
    ///
    /// 调用方可在多次调用间复用返回向量（参见 [`Self::reclaim_into`]），
    /// 以保持稳态帧零堆分配。
    pub fn reclaim(&mut self, epoch: u64) -> Vec<QuarantinedBlock> {
        let mut out = Vec::new();
        self.reclaim_into(epoch, &mut out);
        out
    }

    /// 无分配变体：将可回收块追加到 `out`。
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

    /// 因队列饱和而被拒绝隔离的块总数。
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 丢弃所有隔离块且*不*归还给分配器。
    ///
    /// 用于设备丢失恢复场景，此时整个内存池会重建，单个旧块已无意义。
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

        // 帧 4 完成：尚无任何块可释放。
        assert!(q.reclaim(4).is_empty());
        assert_eq!(q.parked(), 2);

        // 帧 5 完成：仅释放隔离在 epoch 5 的块。
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
        // 已有桶保持不变。
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

//! GPU 时间戳查询。
//!
//! 设计文档中的验收表规定了 GPU 耗时预算——10 万个带阴影圆角画布节点 1.5 ms、
//! 1 万条 Bézier 连接线 0.8 ms。未经过测量的预算只是空谈，因此渲染器通过时间戳
//! 包围各阶段工作并报告真实耗时。
//!
//! 机制采用标准方式：`u64` 时间戳查询池、在目标区间前后调用 `vkCmdWriteTimestamp`，
//! 再通过 `vkGetQueryPoolResults` 读回宿主可见内存。原始值为 GPU 时钟滴答数，
//! 通过设备的 `timestampPeriod` 换算为纳秒。

use ash::vk;

use crate::device::{Gpu, GpuError};

/// 单帧按阶段拆分的 GPU 耗时。
///
/// 每个数值均为*累计值*：即从帧开始到该阶段结束经过的毫秒数。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FrameTiming {
    /// Scatter（增量上传）阶段结束时刻。
    pub scatter_ms: f32,
    /// 视口剔除阶段结束时刻。
    pub cull_ms: f32,
    /// 基数排序阶段结束时刻。
    pub sort_ms: f32,
    /// 间接绘制命令压缩阶段结束时刻。
    pub indirect_ms: f32,
    /// 图形渲染 Pass 结束时刻。
    pub graphics_ms: f32,
    /// 整帧结束时刻。
    pub total_ms: f32,
}

/// GPU 时间戳查询池。
#[derive(Debug)]
pub struct TimestampScope {
    pool: vk::QueryPool,
    /// 查询池中的时间戳数量。
    pub count: u32,
    /// 每个时间戳滴答对应的纳秒数。
    period: f32,
    /// 最近一次读回的原始 GPU 滴答值。
    ///
    /// 原始值以 `u64` 保留：它们是 10^18 量级的绝对时间戳，若先转为 `f32` 毫秒
    /// 会把本模块旨在测量的微小时间差完全舍入丢失。
    raw: Vec<u64>,
}

impl TimestampScope {
    /// 创建包含 `count` 个时间戳的作用域。
    ///
    /// # Errors
    /// 当查询池创建失败时返回 [`GpuError::Vulkan`]。
    pub fn new(gpu: &Gpu, count: u32) -> Result<Self, GpuError> {
        let count = count.max(1);
        let info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(count);
        let pool =
            unsafe { gpu.device.create_query_pool(&info, None) }.map_err(GpuError::Vulkan)?;
        Ok(Self {
            pool,
            count,
            period: gpu.properties.limits.timestamp_period,
            raw: Vec::new(),
        })
    }

    /// 原始时钟周期（每滴答对应的纳秒数）。
    #[must_use]
    pub const fn period(&self) -> f32 {
        self.period
    }

    /// 重置查询池中的所有时间戳。
    ///
    /// 当 `hostQueryReset` 可用时在宿主端执行重置：录制在命令缓冲区内的重置
    /// 可能与随后的写入发生竞态，驱动有权对两者重排序。
    ///
    /// 若设备不支持该特性，则必须将重置命令录制到命令缓冲区中以保证顺序先于
    /// 后续时间戳写入，此时请调用 [`Self::reset_in_command_buffer`]。
    ///
    /// # Errors
    /// 当宿主端无法重置查询池时返回 [`GpuError::UnsupportedFeatures`]。
    pub fn reset(&self, gpu: &Gpu) -> Result<(), GpuError> {
        if gpu.features.host_query_reset {
            // SAFETY: 查询池当前未被使用：使用过它的帧已退役。
            unsafe { gpu.device.reset_query_pool(self.pool, 0, self.count) };
            Ok(())
        } else {
            Err(GpuError::UnsupportedFeatures(vec!["hostQueryReset"]))
        }
    }

    /// 在命令缓冲区内录制查询池重置命令。
    ///
    /// 这是不支持 `hostQueryReset` 设备上的回退方案：重置必须先于后续时间戳写入，
    /// 唯有录制进命令缓冲区才能保证该顺序。
    ///
    /// # Safety
    /// 命令缓冲区必须处于录制状态、属于 `gpu`，且尚未包含针对本查询池的时间戳。
    pub unsafe fn reset_in_command_buffer(&self, gpu: &Gpu, cmd: vk::CommandBuffer) {
        // SAFETY: 调用方保证命令缓冲区处于录制状态且属于 `gpu`。
        unsafe {
            gpu.device
                .cmd_reset_query_pool(cmd, self.pool, 0, self.count);
        }
    }

    /// 向命令缓冲区写入一个时间戳。
    ///
    /// 管线阶段决定时间戳的采样时机。此处刻意避免使用 `ALL_COMMANDS`：
    /// 它会强制排空整条命令缓冲区，使标记失去定位慢速阶段的作用。
    ///
    /// # Safety
    /// 命令缓冲区必须处于录制状态且属于 `gpu`。
    pub unsafe fn write_at(
        &self,
        gpu: &Gpu,
        command_buffer: vk::CommandBuffer,
        index: u32,
        stage: vk::PipelineStageFlags,
    ) {
        // SAFETY: 调用方保证命令缓冲区处于录制状态且属于 `gpu`。
        unsafe {
            gpu.device
                .cmd_write_timestamp(command_buffer, stage, self.pool, index);
        }
    }

    /// 向命令缓冲区写入一个时间戳。
    ///
    /// # Safety
    /// 命令缓冲区必须处于录制状态且属于 `gpu`。
    pub unsafe fn write(&self, gpu: &Gpu, command_buffer: vk::CommandBuffer, index: u32) {
        // SAFETY: 调用方保证命令缓冲区处于录制状态。
        unsafe {
            self.write_at(
                gpu,
                command_buffer,
                index,
                vk::PipelineStageFlags::ALL_COMMANDS,
            )
        };
    }

    /// 读回所有时间戳。
    ///
    /// 返回以任意零点为基准的毫秒时间戳。
    ///
    /// # Errors
    /// 当无法获取查询结果时返回 [`GpuError::Vulkan`]。
    pub fn read(&mut self, gpu: &Gpu) -> Result<Vec<f32>, GpuError> {
        let mut raw = vec![0u64; self.count as usize];
        let result = unsafe {
            gpu.device.get_query_pool_results(
                self.pool,
                0,
                &mut raw,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
            )
        };
        result.map_err(GpuError::Vulkan)?;
        self.raw = raw;
        Ok(self.values())
    }

    /// 计算两个时间戳之间经过的毫秒数。
    ///
    /// 差值在原始 64 位滴答上完成，仅将*差值*转换为浮点数。
    ///
    /// 若该对时间戳未测量或结果不可用则返回 `None`。
    #[must_use]
    pub fn elapsed_ms(&self, from: usize, to: usize) -> Option<f32> {
        let start = self.raw.get(from).copied()?;
        let end = self.raw.get(to).copied()?;
        if end < start {
            return None;
        }
        Some((end - start) as f32 * self.period / 1_000_000.0)
    }

    /// 实际读回的时间戳数量。
    #[must_use]
    pub fn raw_len(&self) -> usize {
        self.raw.len()
    }

    /// 帧耗时明细（累计毫秒数）。
    #[must_use]
    pub fn frame_timing(&self) -> FrameTiming {
        let at = |index: usize| self.elapsed_ms(0, index).unwrap_or(0.0);
        FrameTiming {
            scatter_ms: at(1),
            cull_ms: at(2),
            sort_ms: at(3),
            indirect_ms: at(4),
            graphics_ms: at(5),
            total_ms: at(5),
        }
    }

    /// 最近一次读回的时间戳值（毫秒）。
    ///
    /// 该值为绝对值且数值很大，仅作调试参考；测量时间段请使用 [`Self::elapsed_ms`]。
    #[must_use]
    pub fn values(&self) -> Vec<f32> {
        self.raw
            .iter()
            .map(|value| *value as f32 * self.period / 1_000_000.0)
            .collect()
    }

    /// 销毁查询池。
    pub fn destroy(&self, gpu: &Gpu) {
        unsafe { gpu.device.destroy_query_pool(self.pool, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(raw: Vec<u64>) -> TimestampScope {
        TimestampScope {
            pool: vk::QueryPool::null(),
            count: 4,
            period: 1.0,
            raw,
        }
    }

    #[test]
    fn an_unmeasured_pair_has_no_duration() {
        assert_eq!(scope(Vec::new()).elapsed_ms(0, 1), None);
    }

    #[test]
    fn the_duration_is_the_difference_between_two_ticks() {
        let scope = scope(vec![10, 2_500_010, 10_000_010, 10_000_010]);
        assert_eq!(scope.elapsed_ms(0, 1), Some(2.5));
        assert_eq!(scope.elapsed_ms(1, 2), Some(7.5));
        assert_eq!(scope.elapsed_ms(2, 3), Some(0.0));
    }

    #[test]
    fn a_backwards_pair_is_rejected() {
        assert_eq!(scope(vec![12, 10]).elapsed_ms(0, 1), None);
    }

    #[test]
    fn a_large_absolute_timestamp_does_not_swallow_the_difference() {
        // 绝对 GPU 时间戳约为 10^18 滴答；直接用 `f32` 会把 2 毫秒的差值完全舍入掉。
        let base = 1_794_370_484_994_390_592u64;
        let scope = scope(vec![base, base + 2_000_000, base + 4_000_000]);
        assert_eq!(scope.elapsed_ms(0, 1), Some(2.0));
        assert_eq!(scope.elapsed_ms(0, 2), Some(4.0));
    }
}

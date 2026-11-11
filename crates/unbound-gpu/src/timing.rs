//! GPU timestamp queries.
//!
//! The acceptance table in the design document states GPU budgets — 1.5 ms
//! for 100 000 rounded, shadowed canvas nodes, 0.8 ms for 10 000 bézier
//! connectors. A budget that is not measured is a wish, so the renderer can
//! bracket its work with timestamps and report the real duration.
//!
//! The mechanism is the standard one: a query pool of `u64` timestamps,
//! `vkCmdWriteTimestamp` around the section of interest and
//! `vkGetQueryPoolResults` back into host-visible memory. The raw values are
//! GPU clock ticks; the device's `timestampPeriod` converts them to
//! nanoseconds.

use ash::vk;

use crate::device::{Gpu, GpuError};

/// The GPU time of one frame, broken down by stage.
///
/// Every number is *cumulative*: the milliseconds from the start of the frame
/// until the stage finished.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FrameTiming {
    /// The end of the scatter (the incremental upload) stage.
    pub scatter_ms: f32,
    /// The end of the viewport cull.
    pub cull_ms: f32,
    /// The end of the radix sort.
    pub sort_ms: f32,
    /// The end of the indirect command compaction.
    pub indirect_ms: f32,
    /// The end of the graphics pass.
    pub graphics_ms: f32,
    /// The end of the frame.
    pub total_ms: f32,
}

/// A pool of GPU timestamps.
#[derive(Debug)]
pub struct TimestampScope {
    pool: vk::QueryPool,
    /// The number of timestamps in the pool.
    pub count: u32,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// The last raw values read back, in GPU ticks.
    ///
    /// The raw values are kept as `u64`: they are absolute timestamps of the
    /// order of 10^18, and converting them to `f32` *milliseconds* first
    /// would round away the very differences the scope exists to measure.
    raw: Vec<u64>,
}

impl TimestampScope {
    /// Create a scope with `count` timestamps.
    ///
    /// # Errors
    /// Returns [`GpuError::Vulkan`] when the query pool cannot be created.
    pub fn new(gpu: &Gpu, count: u32) -> Result<Self, GpuError> {
        let count = count.max(1);
        let info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(count);
        let pool = unsafe { gpu.device.create_query_pool(&info, None) }
            .map_err(GpuError::Vulkan)?;
        Ok(Self {
            pool,
            count,
            period: gpu.properties.limits.timestamp_period,
            raw: Vec::new(),
        })
    }

    /// The raw period, in nanoseconds per tick.
    #[must_use]
    pub const fn period(&self) -> f32 {
        self.period
    }

    /// Reset every timestamp in the pool.
    ///
    /// The reset happens on the host when `hostQueryReset` is available: a
    /// reset recorded inside the command buffer races with the writes that
    /// follow it, and the driver is free to reorder the two.
    ///
    /// # Errors
    /// Returns [`GpuError::Vulkan`] when the pool cannot be reset.
    pub fn reset(&self, gpu: &Gpu) -> Result<(), GpuError> {
        if gpu.features.host_query_reset {
            // SAFETY: the pool is not in use: the frame that used it has
            // retired.
            unsafe { gpu.device.reset_query_pool(self.pool, 0, self.count) };
        } else {
            // Without the feature the reset has to be recorded. The caller
            // passes the command buffer so the reset is ordered before the
            // timestamps that follow it.
            return Err(GpuError::UnsupportedFeatures(vec![
                "hostQueryReset",
            ]));
        }
        Ok(())
    }

    /// Write a timestamp into the command buffer.
    ///
    /// The stage decides *when* the timestamp is taken. `ALL_COMMANDS` is
    /// deliberately avoided: it forces a full drain of the command buffer,
    /// which makes the markers useless for locating a slow stage.
    ///
    /// # Safety
    /// The command buffer must be in the recording state and must belong to
    /// `gpu`.
    pub unsafe fn write_at(
        &self,
        gpu: &Gpu,
        command_buffer: vk::CommandBuffer,
        index: u32,
        stage: vk::PipelineStageFlags,
    ) {
        // SAFETY: the caller guarantees the command buffer is recording and
        // belongs to `gpu`.
        unsafe {
            gpu.device
                .cmd_write_timestamp(command_buffer, stage, self.pool, index);
        }
    }

    /// Write a timestamp into the command buffer.
    ///
    /// # Safety
    /// The command buffer must be in the recording state and must belong to
    /// `gpu`.
    pub unsafe fn write(&self, gpu: &Gpu, command_buffer: vk::CommandBuffer, index: u32) {
        // SAFETY: the caller guarantees the command buffer is recording.
        unsafe { self.write_at(gpu, command_buffer, index, vk::PipelineStageFlags::ALL_COMMANDS) };
    }

    /// Read the timestamps back.
    ///
    /// Returns the timestamps in milliseconds since an arbitrary origin.
    ///
    /// # Errors
    /// Returns [`GpuError::Vulkan`] when the results cannot be fetched.
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

    /// The elapsed milliseconds between two timestamps.
    ///
    /// The subtraction happens on the raw 64-bit ticks; only the *difference*
    /// is converted to floating point.
    ///
    /// Returns `None` when the pair was not measured or the result is
    /// unavailable.
    #[must_use]
    pub fn elapsed_ms(&self, from: usize, to: usize) -> Option<f32> {
        let start = self.raw.get(from).copied()?;
        let end = self.raw.get(to).copied()?;
        if end < start {
            return None;
        }
        Some((end - start) as f32 * self.period / 1_000_000.0)
    }

    /// The number of timestamps that were read back.
    #[must_use]
    pub fn raw_len(&self) -> usize {
        self.raw.len()
    }

    /// The frame timing, in cumulative milliseconds.
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

    /// The last values read back, in milliseconds.
    ///
    /// The values are absolute and large, so they are only useful as a
    /// debugging aid; use [`Self::elapsed_ms`] to measure a duration.
    #[must_use]
    pub fn values(&self) -> Vec<f32> {
        self.raw
            .iter()
            .map(|value| *value as f32 * self.period / 1_000_000.0)
            .collect()
    }

    /// Destroy the query pool.
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
        // Absolute GPU timestamps are around 10^18 ticks; an `f32` would
        // round a two-millisecond difference away completely.
        let base = 1_794_370_484_994_390_592u64;
        let scope = scope(vec![base, base + 2_000_000, base + 4_000_000]);
        assert_eq!(scope.elapsed_ms(0, 1), Some(2.0));
        assert_eq!(scope.elapsed_ms(0, 2), Some(4.0));
    }
}

//! 流式加载管线（`docs` §8.2）。
//!
//! ```text
//! [disk / network I/O] ──> [decode worker pool] ──> [CPU staging ring]
//!                                                    │
//!                                                    ▼ (dedicated transfer queue)
//!                                          [GPU memory / bindless texture]
//! ```
//!
//! 主线程只负责*入队*任务。它从不解码、从不拷贝纹理，也从不等待文件：
//! [`AssetPipeline::request`] 仅执行通道发送，[`AssetPipeline::poll`] 则排空
//! 工作线程已完成的结果。这正是帧耗时独立于资产管线的关键。
//!
//!

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use std::sync::Arc;

use crate::decode::{Decoder, KindDecoder};
use crate::ring::{StagingRef, StagingRing};
use crate::transfer::{AssetError, AssetId, AssetKind, DecodedAsset};

/// 管线配置参数。
#[derive(Clone, Debug)]
pub struct PipelineConfig {
    /// 解码工作线程数量。
    pub workers: usize,
    /// 单个暂存槽位的字节大小。
    pub slot_bytes: usize,
    /// 暂存槽位数量（即并发在飞帧数）。
    pub frames: usize,
    /// 在 [`AssetPipeline::request`] 开始拒绝新任务前，
    /// 允许排队的最大请求数。
    pub queue_capacity: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            workers: 2,
            slot_bytes: 4 << 20,
            frames: 3,
            queue_capacity: 256,
        }
    }
}

/// 派发给工作线程的解码请求。
struct Job {
    id: AssetId,
    kind: AssetKind,
    bytes: Vec<u8>,
}

/// 单个解码任务的执行结果。
#[derive(Clone, Debug)]
pub struct Completed {
    /// 该任务所属的资产。
    pub id: AssetId,
    /// 解码成功时的载荷。
    pub asset: Option<DecodedAsset>,
    /// 载荷在暂存环中的落点位置。
    pub location: Option<StagingRef>,
    /// 解码失败时的错误信息。
    pub error: Option<String>,
}

impl Completed {
    /// 当资产已就绪可传输时返回 true。
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.asset.is_some() && self.location.is_some()
    }
}

/// 工作线程间的共享状态。
struct Shared {
    decoder: Box<dyn Decoder>,
    ring: StagingRing,
}

/// 资产流式加载管线。
pub struct AssetPipeline {
    jobs: Sender<Job>,
    done: Receiver<Completed>,
    shared: Arc<Shared>,
    /// 已入队但尚未上报结果的任务数量。
    inflight: usize,
    /// 解码失败的资产及其原因列表。
    failures: Vec<(AssetId, String)>,
}

impl AssetPipeline {
    /// 使用默认解码器启动管线。
    #[must_use]
    pub fn new(config: PipelineConfig) -> Self {
        Self::with_decoder(config, Box::new(KindDecoder::new()))
    }

    /// 使用自定义解码器启动管线。
    #[must_use]
    pub fn with_decoder(config: PipelineConfig, decoder: Box<dyn Decoder>) -> Self {
        let (jobs, job_rx) = crossbeam_channel::bounded(config.queue_capacity.max(1));
        let (done_tx, done) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared {
            decoder,
            ring: StagingRing::new(config.slot_bytes, config.frames),
        });
        let workers = config.workers.max(1);
        for _ in 0..workers {
            let job_rx = job_rx.clone();
            let done_tx = done_tx.clone();
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || worker_loop(job_rx, done_tx, shared));
        }
        // 丢弃原始接收端与发送端，以便在管线被丢弃时
        // 工作线程能感知到通道关闭。
        drop(job_rx);
        drop(done_tx);
        Self {
            jobs,
            done,
            shared,
            inflight: 0,
            failures: Vec::new(),
        }
    }

    /// 将资产加入解码队列。
    ///
    /// 该方法永不阻塞：队列满时会直接拒绝请求，
    /// 调用方可在下一帧重试。
    ///
    /// # Errors
    /// 当队列饱和时返回 [`AssetError::QueueFull`]。
    pub fn request(
        &mut self,
        id: AssetId,
        kind: AssetKind,
        bytes: Vec<u8>,
    ) -> Result<(), AssetError> {
        match self.jobs.try_send(Job { id, kind, bytes }) {
            Ok(()) => {
                self.inflight += 1;
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(AssetError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(AssetError::WorkerGone),
        }
    }

    /// 排空已完成的任务结果。
    ///
    /// # Errors
    /// 当所有工作线程均已退出时返回 [`AssetError::WorkerGone`]。
    pub fn poll(&mut self) -> Result<Vec<Completed>, AssetError> {
        let mut out = Vec::new();
        loop {
            match self.done.try_recv() {
                Ok(completed) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    if let Some(error) = &completed.error {
                        self.failures.push((completed.id, error.clone()));
                    }
                    out.push(completed);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.inflight > 0 {
                        return Err(AssetError::WorkerGone);
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    /// 阻塞直至所有未完成任务都已上报。
    ///
    /// 仅用于测试及“等待全部完成”的关闭路径：渲染循环绝不可调用此方法。
    ///
    ///
    /// # Errors
    /// 当工作线程退出时返回 [`AssetError::WorkerGone`]。
    pub fn drain(&mut self) -> Result<Vec<Completed>, AssetError> {
        let mut out = Vec::new();
        while self.inflight > 0 {
            match self.done.recv() {
                Ok(completed) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    if let Some(error) = &completed.error {
                        self.failures.push((completed.id, error.clone()));
                    }
                    out.push(completed);
                }
                Err(_) => return Err(AssetError::WorkerGone),
            }
        }
        Ok(out)
    }

    /// 工作线程写入的暂存环形缓冲区。
    #[must_use]
    pub fn staging(&self) -> &StagingRing {
        &self.shared.ring
    }

    /// 尚未上报结果的任务数量。
    #[must_use]
    pub const fn inflight(&self) -> usize {
        self.inflight
    }

    /// 解码失败的资产列表。
    #[must_use]
    pub fn failures(&self) -> &[(AssetId, String)] {
        &self.failures
    }

    /// 将暂存环推进到下一帧。
    pub fn begin_frame(&self) {
        self.shared.ring.begin_frame();
    }
}

/// 工作线程主循环。
fn worker_loop(jobs: Receiver<Job>, done: Sender<Completed>, shared: Arc<Shared>) {
    while let Ok(job) = jobs.recv() {
        let outcome = shared.decoder.decode(job.kind, &job.bytes);
        let completed = match outcome {
            Ok(asset) => {
                let location = shared.ring.write(&asset.bytes);
                Completed {
                    id: job.id,
                    asset: Some(asset),
                    location: location.ok(),
                    error: location.err().map(|error| error.to_string()),
                }
            }
            Err(error) => Completed {
                id: job.id,
                asset: None,
                location: None,
                error: Some(error.to_string()),
            },
        };
        if done.send(completed).is_err() {
            // 管线已被丢弃，停止工作。
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 消耗固定时长的模拟解码器，用于测试证明主线程不会等待解码。
    ///
    struct SlowDecoder {
        delay: Duration,
    }

    impl Decoder for SlowDecoder {
        fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError> {
            let start = Instant::now();
            while start.elapsed() < self.delay {
                std::hint::spin_loop();
            }
            Ok(DecodedAsset {
                kind,
                bytes: bytes.to_vec(),
                width: 1,
                height: 1,
            })
        }
    }

    fn config() -> PipelineConfig {
        PipelineConfig {
            workers: 2,
            slot_bytes: 64 * 1024,
            frames: 3,
            queue_capacity: 64,
        }
    }

    #[test]
    fn an_asset_is_decoded_in_the_background_and_staged() {
        let mut pipeline = AssetPipeline::new(config());
        pipeline
            .request(AssetId(1), AssetKind::Raw, b"hello".to_vec())
            .unwrap();
        let done = pipeline.drain().unwrap();
        assert_eq!(done.len(), 1);
        assert!(done[0].is_ready());
        let location = done[0].location.unwrap();
        assert_eq!(
            pipeline.staging().with(location, |bytes| bytes.to_vec()),
            Some(b"hello".to_vec()),
            "the decoded bytes must be in the staging ring"
        );
    }

    #[test]
    fn the_main_thread_never_waits_for_the_decode() {
        let mut pipeline = AssetPipeline::with_decoder(
            config(),
            Box::new(SlowDecoder {
                delay: Duration::from_millis(40),
            }),
        );
        let start = Instant::now();
        for index in 0..8 {
            pipeline
                .request(AssetId(index), AssetKind::Raw, vec![0u8; 16])
                .unwrap();
        }
        let elapsed = start.elapsed();
        // 8 个 40 ms 的解码共需 320 ms；入队操作绝不能等待其中任何耗时。
        //
        assert!(
            elapsed < Duration::from_millis(10),
            "enqueueing blocked for {elapsed:?}"
        );
        let done = pipeline.drain().unwrap();
        assert_eq!(done.len(), 8);
    }

    #[test]
    fn a_failing_decode_is_reported_without_killing_the_worker() {
        struct FailingDecoder;
        impl Decoder for FailingDecoder {
            fn decode(&self, _kind: AssetKind, _bytes: &[u8]) -> Result<DecodedAsset, AssetError> {
                Err(AssetError::Decode("nope".into()))
            }
        }
        let mut pipeline = AssetPipeline::with_decoder(config(), Box::new(FailingDecoder));
        pipeline
            .request(AssetId(7), AssetKind::Raw, b"x".to_vec())
            .unwrap();
        let done = pipeline.drain().unwrap();
        assert_eq!(done.len(), 1);
        assert!(!done[0].is_ready());
        assert!(done[0].error.is_some());
        assert_eq!(pipeline.failures().len(), 1);
    }

    #[test]
    fn a_full_queue_refuses_work_instead_of_blocking() {
        let mut pipeline = AssetPipeline::with_decoder(
            PipelineConfig {
                workers: 1,
                slot_bytes: 1024,
                frames: 2,
                queue_capacity: 2,
            },
            Box::new(SlowDecoder {
                delay: Duration::from_millis(50),
            }),
        );
        // 第一个请求占用工作线程，后续两个请求填满队列。
        let mut refused = 0;
        for index in 0..16 {
            if pipeline
                .request(AssetId(index), AssetKind::Raw, vec![0u8; 4])
                .is_err()
            {
                refused += 1;
            }
        }
        assert!(refused > 0, "a saturated queue must refuse work");
    }

    #[test]
    fn a_frame_advance_moves_the_staging_ring_to_the_next_slot() {
        let mut pipeline = AssetPipeline::new(config());
        pipeline
            .request(AssetId(1), AssetKind::Raw, b"a".to_vec())
            .unwrap();
        let done = pipeline.drain().unwrap();
        assert_eq!(done[0].location.unwrap().slot, 0);
        pipeline.begin_frame();
        pipeline
            .request(AssetId(2), AssetKind::Raw, b"b".to_vec())
            .unwrap();
        let done = pipeline.drain().unwrap();
        assert_eq!(done[0].location.unwrap().slot, 1);
    }
}

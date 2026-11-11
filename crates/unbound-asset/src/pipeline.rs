//! The streaming pipeline (`docs` §8.2).
//!
//! ```text
//! [disk / network I/O] ──> [decode worker pool] ──> [CPU staging ring]
//!                                                    │
//!                                                    ▼ (dedicated transfer queue)
//!                                          [GPU memory / bindless texture]
//! ```
//!
//! The main thread only ever *enqueues* work. It never decodes, never copies
//! a texture and never waits for a file: [`AssetPipeline::request`] is a
//! channel send, and [`AssetPipeline::poll`] drains whatever the workers have
//! finished. That is what keeps the frame time independent of the asset
//! pipeline.

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use std::sync::Arc;

use crate::decode::{Decoder, KindDecoder};
use crate::ring::{StagingRef, StagingRing};
use crate::transfer::{AssetError, AssetId, AssetKind, DecodedAsset};

/// How the pipeline is set up.
#[derive(Clone, Debug)]
pub struct PipelineConfig {
    /// Number of decode workers.
    pub workers: usize,
    /// Size of one staging slot in bytes.
    pub slot_bytes: usize,
    /// Number of staging slots (frames in flight).
    pub frames: usize,
    /// How many requests may be queued before [`AssetPipeline::request`]
    /// starts refusing work.
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

/// A request handed to a worker.
struct Job {
    id: AssetId,
    kind: AssetKind,
    bytes: Vec<u8>,
}

/// The result of a job.
#[derive(Clone, Debug)]
pub struct Completed {
    /// The asset the job belonged to.
    pub id: AssetId,
    /// The decoded payload, if the decode succeeded.
    pub asset: Option<DecodedAsset>,
    /// Where the payload landed in the staging ring.
    pub location: Option<StagingRef>,
    /// The failure, if the decode failed.
    pub error: Option<String>,
}

impl Completed {
    /// True when the asset is ready to be transferred.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.asset.is_some() && self.location.is_some()
    }
}

/// The shared state of the workers.
struct Shared {
    decoder: Box<dyn Decoder>,
    ring: StagingRing,
}

/// The asset streaming pipeline.
pub struct AssetPipeline {
    jobs: Sender<Job>,
    done: Receiver<Completed>,
    shared: Arc<Shared>,
    /// Number of jobs that have been enqueued but not yet reported.
    inflight: usize,
    /// The assets that failed, with the reason.
    failures: Vec<(AssetId, String)>,
}

impl AssetPipeline {
    /// Start the pipeline with the default decoder.
    #[must_use]
    pub fn new(config: PipelineConfig) -> Self {
        Self::with_decoder(config, Box::new(KindDecoder::new()))
    }

    /// Start the pipeline with a custom decoder.
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
        // The original receiver and sender are dropped so the workers see the
        // channel close when the pipeline is dropped.
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

    /// Enqueue an asset for decoding.
    ///
    /// This never blocks: when the queue is full the request is refused and
    /// the caller can retry on the next frame.
    ///
    /// # Errors
    /// Returns [`AssetError::QueueFull`] when the queue is saturated.
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

    /// Drain the finished jobs.
    ///
    /// # Errors
    /// Returns [`AssetError::WorkerGone`] when every worker has died.
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

    /// Block until every outstanding job has been reported.
    ///
    /// Only for tests and for a "wait for everything" shutdown path: the
    /// render loop must never call this.
    ///
    /// # Errors
    /// Returns [`AssetError::WorkerGone`] when a worker died.
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

    /// The staging ring the workers write into.
    #[must_use]
    pub fn staging(&self) -> &StagingRing {
        &self.shared.ring
    }

    /// Number of jobs that have not been reported yet.
    #[must_use]
    pub const fn inflight(&self) -> usize {
        self.inflight
    }

    /// The assets that failed to decode.
    #[must_use]
    pub fn failures(&self) -> &[(AssetId, String)] {
        &self.failures
    }

    /// Advance the staging ring to the next frame.
    pub fn begin_frame(&self) {
        self.shared.ring.begin_frame();
    }
}

/// The worker loop.
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
            // The pipeline was dropped; stop working.
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A decoder that burns a fixed amount of time, so the test can prove the
    /// main thread does not wait for it.
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
        // Eight 40 ms decodes take 320 ms of work; enqueueing must not wait
        // for any of it.
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
        // The first request occupies the worker, the next two fill the queue.
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

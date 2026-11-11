//! Async asset streaming (`docs` §8.2).
//!
//! The pipeline has four stages, and the crate models the three that live
//! outside the driver:
//!
//! 1. **Fetch** — the application hands the pipeline the bytes it already
//!    read, or the pipeline reads them from disk.
//! 2. **Decode** — a pool of worker threads turns the bytes into a GPU-ready
//!    payload. Nothing here ever runs on the thread that draws.
//! 3. **Stage** — the payload lands in a multi-slot CPU staging ring that
//!    the transfer queue reads from over PCIe.
//! 4. **Transfer** — a dedicated transfer queue copies the payload into GPU
//!    memory and signals a timeline semaphore; the graphics queue waits for
//!    that value before it samples the texture.
//!
//! The design constraint that shapes everything here: **the frame time must
//! not depend on the asset pipeline**. A request that cannot be queued is
//! refused, a decode that fails is reported, and an asset that is not ready
//! is drawn as a placeholder. Nothing ever blocks the render loop.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod decode;
pub mod pipeline;
pub mod ring;
pub mod transfer;

pub use decode::{Decoder, KindDecoder, PngDecoder, RawDecoder};
pub use pipeline::{AssetPipeline, Completed, PipelineConfig};
pub use ring::{StagingError, StagingRef, StagingRing};
pub use transfer::{
    ownership_barrier, AssetError, AssetId, AssetKind, AssetStatus, DecodedAsset, ImageLayout,
    OwnershipTransfer, Placeholder, QueueFamily, TimelineGate, TransferBarrier,
};

/// Read a file from disk, off the render thread.
///
/// The function is blocking on purpose — it is meant to be called from a
/// worker, never from the frame loop.
///
/// # Errors
/// Returns the I/O error when the file cannot be read.
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<Vec<u8>, AssetError> {
    std::fs::read(path).map_err(|error| AssetError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_reported_rather_than_panicking() {
        let error = read_file("this/file/does/not/exist.bin").unwrap_err();
        assert!(matches!(error, AssetError::Decode(_)));
    }

    #[test]
    fn a_file_on_disk_is_read_into_the_pipeline() {
        let path = std::env::temp_dir().join("unbound-asset-test.bin");
        std::fs::write(&path, b"payload").unwrap();
        let bytes = read_file(&path).unwrap();
        assert_eq!(bytes, b"payload");
        let _ = std::fs::remove_file(&path);
    }
}

//! Asset identity and the transfer hand-off (`docs` §8.2).
//!
//! The pipeline has three stages, and this module describes the boundary
//! between the last two:
//!
//! ```text
//! decode worker ──> CPU staging slot ──> transfer queue ──> GPU memory
//!                       (this module)      (ownership transfer + timeline)
//! ```

/// A stable identifier for an asset.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId(pub u32);

impl AssetId {
    /// The raw identifier.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// What kind of payload an asset carries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// A decoded image, stored as RGBA8.
    Image,
    /// A decoded vector scene.
    Vector,
    /// A parsed font.
    Font,
    /// Opaque bytes the application interprets itself.
    Raw,
}

/// The outcome of decoding an asset.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAsset {
    /// The kind of the asset.
    pub kind: AssetKind,
    /// The decoded payload.
    pub bytes: Vec<u8>,
    /// Pixel width, for images.
    pub width: u32,
    /// Pixel height, for images.
    pub height: u32,
}

impl DecodedAsset {
    /// A raw payload with no dimensions.
    #[must_use]
    pub fn raw(bytes: Vec<u8>) -> Self {
        Self {
            kind: AssetKind::Raw,
            bytes,
            width: 0,
            height: 0,
        }
    }
}

/// Errors raised by the asset pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum AssetError {
    /// The request queue is full; the caller must retry later.
    QueueFull,
    /// The payload could not be decoded.
    Decode(String),
    /// The staging ring had no room.
    Staging(crate::ring::StagingError),
    /// The worker thread died.
    WorkerGone,
}

impl core::fmt::Display for AssetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::QueueFull => write!(f, "the asset request queue is full"),
            Self::Decode(message) => write!(f, "decode failed: {message}"),
            Self::Staging(error) => write!(f, "{error}"),
            Self::WorkerGone => write!(f, "a decode worker died"),
        }
    }
}

impl std::error::Error for AssetError {}

/// A Vulkan queue family index.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueFamily(pub u32);

/// The image layouts the transfer path uses.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImageLayout {
    /// The layout of a freshly created image.
    Undefined,
    /// The source of a copy.
    TransferSrc,
    /// The destination of a copy.
    TransferDst,
    /// Sampled by a shader.
    ShaderReadOnly,
}

/// The barrier that hands a staged asset over to the graphics queue.
///
/// The numbers are the Vulkan stage/access bit values; they are spelled out
/// here so that the crate stays free of a Vulkan dependency while the
/// semantics stay checkable.
pub mod bits {
    /// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
    pub const TRANSFER: u32 = 0x0000_1000;
    /// `VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT`.
    pub const FRAGMENT_SHADER: u32 = 0x0080_0000;
    /// `VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`.
    pub const COMPUTE_SHADER: u32 = 0x0000_8000;
    /// `VK_ACCESS_TRANSFER_WRITE_BIT`.
    pub const TRANSFER_WRITE: u32 = 0x0000_0800;
    /// `VK_ACCESS_SHADER_READ_BIT`.
    pub const SHADER_READ: u32 = 0x0020_0000;
}

/// A queue-family ownership transfer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OwnershipTransfer {
    /// The queue family that owns the memory during the copy.
    pub from: QueueFamily,
    /// The queue family that owns the memory afterwards.
    pub to: QueueFamily,
    /// The old image layout.
    pub old_layout: ImageLayout,
    /// The new image layout.
    pub new_layout: ImageLayout,
    /// The timeline value at which the transfer is complete.
    pub timeline_value: u64,
}

/// The barrier description for an ownership transfer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TransferBarrier {
    /// Source pipeline stage.
    pub src_stage: u32,
    /// Destination pipeline stage.
    pub dst_stage: u32,
    /// Source access mask.
    pub src_access: u32,
    /// Destination access mask.
    pub dst_access: u32,
    /// The queue family that releases the memory.
    pub src_family: QueueFamily,
    /// The queue family that acquires the memory.
    pub dst_family: QueueFamily,
    /// The old layout.
    pub old_layout: ImageLayout,
    /// The new layout.
    pub new_layout: ImageLayout,
}

/// Build the barrier for an ownership transfer.
///
/// The staging buffer is written by the transfer queue and read by the
/// graphics queue, so the barrier must be a *release* on the transfer side
/// and an *acquire* on the graphics side.
#[must_use]
pub fn ownership_barrier(transfer: &OwnershipTransfer) -> TransferBarrier {
    TransferBarrier {
        src_stage: bits::TRANSFER,
        dst_stage: bits::FRAGMENT_SHADER | bits::COMPUTE_SHADER,
        src_access: bits::TRANSFER_WRITE,
        dst_access: bits::SHADER_READ,
        src_family: transfer.from,
        dst_family: transfer.to,
        old_layout: transfer.old_layout,
        new_layout: transfer.new_layout,
    }
}

/// A timeline semaphore gate.
///
/// The transfer queue signals a value when the DMA has finished; the graphics
/// queue waits for that value before it touches the memory. A wrong value is
/// a use-after-free in waiting to happen, so the gate is explicit.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimelineGate {
    value: u64,
}

impl Default for TimelineGate {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineGate {
    /// A gate that has signalled nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// The next value to signal.
    #[must_use]
    pub fn next_value(&self) -> u64 {
        self.value + 1
    }

    /// Signal that the transfer for `value` has completed.
    pub fn signal(&mut self, value: u64) {
        // Signals never go backwards: a late worker must not un-complete a
        // transfer that already finished.
        if value > self.value {
            self.value = value;
        }
    }

    /// The highest signalled value.
    #[must_use]
    pub const fn signalled(&self) -> u64 {
        self.value
    }

    /// True when the transfer for `value` has completed.
    #[must_use]
    pub const fn is_ready(&self, value: u64) -> bool {
        self.value >= value
    }
}

/// The state of an asset in the pipeline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AssetStatus {
    /// Still being fetched or decoded.
    Pending,
    /// Decoded and staged, waiting for the transfer queue.
    Staged,
    /// Transferred and usable by the renderer.
    Ready,
    /// The decode failed.
    Failed,
}

/// What the renderer should draw for an asset.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Placeholder {
    /// Draw the real asset.
    Real,
    /// Draw a low-resolution stand-in (a blurred or tinted rectangle).
    LowResolution,
    /// Draw a skeleton (an outline or a loading bar).
    Skeleton,
}

/// Choose what to draw for an asset that is not ready yet.
///
/// The rule from the design document: never block the frame. A pending asset
/// gets a placeholder; only a *ready* asset is drawn for real.
#[must_use]
pub const fn placeholder_for(status: AssetStatus) -> Placeholder {
    match status {
        AssetStatus::Ready => Placeholder::Real,
        AssetStatus::Pending | AssetStatus::Staged => Placeholder::Skeleton,
        AssetStatus::Failed => Placeholder::LowResolution,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_barrier_releases_on_the_transfer_side_and_acquires_on_the_graphics_side() {
        let transfer = OwnershipTransfer {
            from: QueueFamily(1),
            to: QueueFamily(0),
            old_layout: ImageLayout::TransferDst,
            new_layout: ImageLayout::ShaderReadOnly,
            timeline_value: 7,
        };
        let barrier = ownership_barrier(&transfer);
        assert_eq!(barrier.src_stage, bits::TRANSFER);
        assert_eq!(barrier.src_access, bits::TRANSFER_WRITE);
        assert_eq!(barrier.dst_access, bits::SHADER_READ);
        assert_eq!(barrier.src_family, QueueFamily(1));
        assert_eq!(barrier.dst_family, QueueFamily(0));
        assert_eq!(barrier.new_layout, ImageLayout::ShaderReadOnly);
    }

    #[test]
    fn the_gate_only_moves_forward() {
        let mut gate = TimelineGate::new();
        assert_eq!(gate.next_value(), 1);
        gate.signal(3);
        assert_eq!(gate.signalled(), 3);
        assert!(gate.is_ready(2));
        assert!(gate.is_ready(3));
        assert!(!gate.is_ready(4));
        // A late worker must not un-complete a transfer.
        gate.signal(1);
        assert_eq!(gate.signalled(), 3);
    }

    #[test]
    fn a_pending_asset_never_blocks_the_frame() {
        assert_eq!(placeholder_for(AssetStatus::Ready), Placeholder::Real);
        assert_eq!(placeholder_for(AssetStatus::Pending), Placeholder::Skeleton);
        assert_eq!(placeholder_for(AssetStatus::Staged), Placeholder::Skeleton);
        assert_eq!(
            placeholder_for(AssetStatus::Failed),
            Placeholder::LowResolution
        );
    }
}

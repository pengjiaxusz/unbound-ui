//! GPU backend: memory protocol, allocators and (later) the Vulkan 1.3 device.
//!
//! Module layout mirrors the milestones in `docs/gpu_ui_v2_0.md` §10.1:
//!
//! | module | milestone | responsibility |
//! | :--- | :--- | :--- |
//! | [`data`] | M1 | 64B primitive / push-constant memory protocol |
//! | [`tlsf`] | M1 | O(1) device-memory allocator |
//! | [`quarantine`] | M1 | epoch-delayed free list for frames in flight |
//! | [`ring`] | M1 | per-frame transient ring buffer |
//! | [`pool`] | M1 | persistent pool + sparse scatter patches |
//! | [`sort`] | M1 | 64-bit sort key encoding and CPU reference sort |
//! | [`device`] | M1 | Vulkan instance/device/queues/features |
//! | [`shader`] | M1 | compiled SPIR-V modules |
//! | [`pipeline`] | M1 | uber-shader + compute pipelines |
//! | [`frame`] | M1 | pass scheduling and indirect submission |
//!
//! The first six modules are pure CPU and are covered by unit tests that run on
//! any machine, including CI without a GPU.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod data;
pub mod debug;
pub mod device;
pub mod frame;
pub mod pipeline;
pub mod pool;
pub mod quarantine;
pub mod ring;
pub mod sdf;
pub mod shader;
pub mod sort;
pub mod surface;
pub mod swapchain;
pub mod atlas;
pub mod curve;
pub mod timing;
pub mod tlsf;

pub use sdf::SdfPipeline;
pub use debug::{is_device_lost, recover_with, RecoveryError, RecoveryPolicy, RenderDoc, ValidationCounter, VALIDATION};
pub use frame::{Camera, FrameStats, Renderer, SceneUpload};
pub use device::{Buffer, Gpu, GpuConfig, GpuError, GpuFeatures, MemoryKind, Timeline};
pub use data::{
    BlendMode, DrawIndirectCommand, FramePushConstants, GpuClipNode, GpuPayload,
    GpuPrimitiveInstance, GpuScatterPatch, GpuTransform, PassLayer, PrimFlags, PrimType,
    PushConstants, SortedEntry, SubElement, TransformFlags, CLIP_NONE, PRIMITIVE_STRIDE,
    TRANSFORM_ANIMATED_BIT,
};
pub use pool::{PersistentPool, PoolHandle, PoolStats, ScatterPatch};
pub use quarantine::{EpochQuarantine, QuarantinedBlock};
pub use ring::TransientRing;
pub use sort::SortKey;
pub use swapchain::{is_supported, PresentConfig, PresentMode, ResizeBehaviour};
pub use atlas::{AtlasKind, AtlasSet, AtlasTexture, AtlasUpload};
pub use curve::{cubic_to_draws, CurveDraw};
pub use surface::{Surface, Swapchain};
pub use timing::{FrameTiming, TimestampScope};
pub use tlsf::{AllocError, Tlsf, TlsfStats, MIN_BLOCK};

//! The render graph and the offscreen pool (`docs` §10.1, M3).
//!
//! The frosted-glass effect needs a scratch texture and a blur, every frame.
//! Doing that with ad-hoc allocations would make the frame time depend on
//! the driver, so the frame is declared as a graph instead:
//!
//! ```text
//! copy ──> blur ──> composite
//!   │                ▲
//!   └── backdrop ────┘
//! ```
//!
//! * [`RenderGraph`] declares the passes and their dependencies, and rejects a
//!   frame that reads a texture before it is written.
//! * [`CompiledGraph`] derives the *lifetime* of every transient texture, so
//!   the pool knows exactly when a target may be recycled.
//! * [`OffscreenPool`] hands out the physical targets and recycles them, so a
//!   steady-state frame allocates nothing.
//!
//! The crate is deliberately free of a Vulkan dependency: it decides *what*
//! the frame does, and the GPU crate executes it.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod graph;
pub mod pool;

pub use graph::{
    CompiledGraph, GraphError, Lifetime, Pass, PassKind, RenderGraph, Texture, TextureId,
    TextureRole,
};
pub use pool::{
    box_blur_passes, gaussian_kernel, kernel_weight, OffscreenPool, PooledTexture, TextureDesc,
    TextureFormat,
};

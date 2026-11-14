//! 渲染图（RenderGraph）与离屏纹理池（`docs` §10.1，M3）。
//!
//! 毛玻璃效果每帧都需要一张临时纹理与一次模糊操作。若每帧临时分配纹理，
//! 帧耗时将受限于驱动分配性能，因此改为将整帧声明为一张渲染图：
//!
//!
//! ```text
//! copy ──> blur ──> composite
//!   │                ▲
//!   └── backdrop ────┘
//! ```
//!
//! * [`RenderGraph`] 声明各 Pass 及其依赖关系，并拒绝在写入前读取纹理的帧。
//!
//! * [`CompiledGraph`] 推导每张瞬态纹理的*生命周期*，使纹理池能准确获知何时可复用渲染目标。
//!
//! * [`OffscreenPool`] 分发并回收物理渲染目标，从而实现稳态帧零分配。
//!
//!
//! 本 crate 刻意不依赖 Vulkan：它只决定帧*做什么*，具体执行由 GPU crate 负责。
//!

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

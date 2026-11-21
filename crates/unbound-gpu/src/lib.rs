//! GPU 后端：内存协议、分配器与 Vulkan 1.3 设备层。
//!
//! 模块布局按里程碑划分：
//!
//! | 模块 | 里程碑 | 职责 |
//! | :--- | :--- | :--- |
//! | [`data`] | M1 | 64B 图元 / Push Constant 内存协议 |
//! | [`tlsf`] | M1 | O(1) 设备内存分配器 |
//! | [`quarantine`] | M1 | 面向在飞帧的 epoch 延迟释放队列 |
//! | [`ring`] | M1 | 逐帧瞬态环形缓冲区 |
//! | [`pool`] | M1 | 持久化池 + 稀疏散布更新补丁 |
//! | [`sort`] | M1 | 64 位排序键编码与 CPU 参考排序 |
//! | [`device`] | M1 | Vulkan 实例/设备/特性初始化与诊断 |
//! | [`shader`] | M1 | 已编译的 SPIR-V 模块 |
//! | [`pipeline`] | M1 | Uber 着色器与计算管线 |
//! | [`frame`] | M1 | Pass 调度与间接绘制提交 |
//!
//! 前六个模块为纯 CPU 实现，覆盖单元测试，可在无 GPU 的任意机器（含 CI）上运行。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod atlas;
pub mod curve;
pub mod data;
pub mod debug;
pub mod device;
pub mod frame;
pub mod inspector;
pub mod pipeline;
pub mod pool;
pub mod quarantine;
pub mod ring;
pub mod sdf;
pub mod shader;
pub mod sort;
pub mod surface;
pub mod surface_extensions;
pub mod swapchain;
pub mod timing;
pub mod tlsf;

pub use atlas::{AtlasKind, AtlasSet, AtlasTexture, AtlasUpload};
pub use curve::{cubic_to_draws, CurveDraw};
pub use data::{
    BlendMode, DrawIndirectCommand, FramePushConstants, GpuClipNode, GpuPayload,
    GpuPrimitiveInstance, GpuScatterPatch, GpuTransform, PassLayer, PrimFlags, PrimType,
    PushConstants, SortedEntry, SubElement, TransformFlags, CLIP_NONE, PRIMITIVE_STRIDE,
    TRANSFORM_ANIMATED_BIT,
};
pub use debug::{
    is_device_lost, recover_with, RecoveryError, RecoveryPolicy, RenderDoc, ValidationCounter,
    VALIDATION,
};
pub use device::{
    explain_vk_result, filter_instance_extensions, is_13_backport_device_extension,
    is_optional_device_extension, is_optional_instance_extension, last_init_diagnostics,
    resolve_device_extensions, Buffer, DeviceExtensionResolution, Gpu, GpuConfig, GpuError,
    GpuFeatures, GpuInitDiagnostics, InstanceExtensionResolution, MemoryKind, Timeline,
};
pub use frame::{Camera, FrameStats, ManagedExternalTextureDesc, Renderer, SceneUpload};
pub use pool::{PersistentPool, PoolHandle, PoolStats, ScatterPatch};
pub use quarantine::{EpochQuarantine, QuarantinedBlock};
pub use ring::TransientRing;
pub use sdf::SdfPipeline;
pub use sort::SortKey;
pub use surface::{Surface, Swapchain};
pub use swapchain::{is_supported, PresentConfig, PresentMode, ResizeBehaviour};
pub use timing::{FrameTiming, TimestampScope};
pub use tlsf::{AllocError, Tlsf, TlsfStats, MIN_BLOCK};

pub use inspector::{
    ChannelMask, CustomViewportPass, ExtendedExternalSlots, ExternalImageDesc,
    ExternalMemoryHandleKind, ExternalMemoryImporter, ImportedExternalTexture,
    TextureInspectorParams, ViewportCamera3D, ViewportRenderPassDesc, EXTENDED_EXTERNAL_SLOTS,
};
pub use pipeline::{
    EXTERNAL_TEXTURE_BASE, EXTERNAL_TEXTURE_SLOTS, MAX_ATLAS_LAYERS, MAX_RGBA_BINDLESS_SLOTS,
};

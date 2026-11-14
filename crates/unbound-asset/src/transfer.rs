//! 资产标识与传输交接（`docs` §8.2）。
//!
//! 管线包含三个阶段，本模块描述最后两个阶段之间的边界：
//!
//!
//! ```text
//! decode worker ──> CPU staging slot ──> transfer queue ──> GPU memory
//!                       (this module)      (ownership transfer + timeline)
//! ```

/// 资产的稳定标识符。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId(pub u32);

impl AssetId {
    /// 原始标识符值。
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// 资产携带的载荷类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// 已解码图像，以 RGBA8 格式存储。
    Image,
    /// 已解码矢量场景。
    Vector,
    /// 已解析字体。
    Font,
    /// 由应用自行解释的不透明字节流。
    Raw,
}

/// 资产解码结果。
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAsset {
    /// 资产类型。
    pub kind: AssetKind,
    /// 解码后的载荷数据。
    pub bytes: Vec<u8>,
    /// 图像宽度（像素）。
    pub width: u32,
    /// 图像高度（像素）。
    pub height: u32,
}

impl DecodedAsset {
    /// 构造无尺寸信息的原始载荷。
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

/// 资产管线抛出的错误。
#[derive(Clone, Debug, PartialEq)]
pub enum AssetError {
    /// 请求队列已满；调用方需稍后重试。
    QueueFull,
    /// 载荷解码失败。
    Decode(String),
    /// 暂存环空间不足。
    Staging(crate::ring::StagingError),
    /// 工作线程已退出。
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

/// Vulkan 队列族索引。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueFamily(pub u32);

/// 传输路径使用的图像布局。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImageLayout {
    /// 新创建图像的初始布局。
    Undefined,
    /// 拷贝源布局。
    TransferSrc,
    /// 拷贝目标布局。
    TransferDst,
    /// 着色器采样布局。
    ShaderReadOnly,
}

/// 将已暂存资产移交至图形队列的内存屏障。
///
/// 数值为 Vulkan 阶段/访问位掩码；在此显式写出是为了让本 crate
/// 在不引入 Vulkan 依赖的同时保持语义可校验。
///
pub mod bits {
    /// Vulkan 管线阶段：`VK_PIPELINE_STAGE_TRANSFER_BIT`。
    pub const TRANSFER: u32 = 0x0000_1000;
    /// Vulkan 管线阶段：`VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT`。
    pub const FRAGMENT_SHADER: u32 = 0x0080_0000;
    /// Vulkan 管线阶段：`VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`。
    pub const COMPUTE_SHADER: u32 = 0x0000_8000;
    /// Vulkan 访问掩码：`VK_ACCESS_TRANSFER_WRITE_BIT`。
    pub const TRANSFER_WRITE: u32 = 0x0000_0800;
    /// Vulkan 访问掩码：`VK_ACCESS_SHADER_READ_BIT`。
    pub const SHADER_READ: u32 = 0x0020_0000;
}

/// 队列族所有权转移描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OwnershipTransfer {
    /// 拷贝期间持有内存所有权的队列族。
    pub from: QueueFamily,
    /// 拷贝完成后接管内存所有权的队列族。
    pub to: QueueFamily,
    /// 旧图像布局。
    pub old_layout: ImageLayout,
    /// 新图像布局。
    pub new_layout: ImageLayout,
    /// 传输完成时对应的时间线信号量值。
    pub timeline_value: u64,
}

/// 所有权转移对应的屏障描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TransferBarrier {
    /// 源管线阶段。
    pub src_stage: u32,
    /// 目标管线阶段。
    pub dst_stage: u32,
    /// 源访问掩码。
    pub src_access: u32,
    /// 目标访问掩码。
    pub dst_access: u32,
    /// 释放内存所有权的队列族。
    pub src_family: QueueFamily,
    /// 获取内存所有权的队列族。
    pub dst_family: QueueFamily,
    /// 旧布局。
    pub old_layout: ImageLayout,
    /// 新布局。
    pub new_layout: ImageLayout,
}

/// 构建所有权转移屏障。
///
/// 暂存缓冲区由传输队列写入、由图形队列读取，
/// 因此屏障必须在传输侧执行*释放（release）*，在图形侧执行*获取（acquire）*。
///
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

/// 时间线信号量门控。
///
/// 传输队列在 DMA 完成后触发信号量值；图形队列在访问内存前等待该值。
/// 错误的值会导致潜在的释放后使用（use-after-free），因此门控逻辑需显式建模。
///
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
    /// 创建一个尚未触发任何信号的门控。
    #[must_use]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// 下一个待触发的信号量值。
    #[must_use]
    pub fn next_value(&self) -> u64 {
        self.value + 1
    }

    /// 标记 `value` 对应的传输已完成。
    pub fn signal(&mut self, value: u64) {
        // 信号量值永不倒退：迟到的工作线程不能撤销已完成的传输。
        //
        if value > self.value {
            self.value = value;
        }
    }

    /// 当前已触发的最高信号量值。
    #[must_use]
    pub const fn signalled(&self) -> u64 {
        self.value
    }

    /// 当 `value` 对应的传输已完成时返回 true。
    #[must_use]
    pub const fn is_ready(&self, value: u64) -> bool {
        self.value >= value
    }
}

/// 资产在管线中的状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AssetStatus {
    /// 仍在获取或解码中。
    Pending,
    /// 已解码并暂存，等待传输队列。
    Staged,
    /// 已传输完成，可供渲染器使用。
    Ready,
    /// 解码失败。
    Failed,
}

/// 渲染器应对资产采取的绘制策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Placeholder {
    /// 绘制真实资产。
    Real,
    /// 绘制低分辨率替身（模糊或着色矩形）。
    LowResolution,
    /// 绘制骨架屏（轮廓或加载条）。
    Skeleton,
}

/// 为尚未就绪的资产选择绘制策略。
///
/// 设计文档规则：绝不阻塞帧。未就绪的资产使用占位符；只有*已就绪*的资产才按真实内容绘制。
///
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
        // 迟到的工作线程不能撤销已完成的传输。
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

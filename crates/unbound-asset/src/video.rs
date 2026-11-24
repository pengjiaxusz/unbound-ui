//! 视频帧载荷与最新帧信箱建模。
//!
//! 本模块提供与图形后端无关的纯 CPU 视频帧元数据、平面布局校验、外部 GPU 句柄描述，
//! 以及面向高频解码线程与渲染线程之间的无阻塞 Latest-Wins 帧传递信箱。

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// 视频帧像素格式（4:2:0 色度子采样）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoPixelFormat {
    /// 8-bit 半平面格式：Y 平面（R8）+ UV 交错平面（R8G8）。
    Nv12,
    /// 10-bit 高位对齐半平面格式：Y 平面（R16）+ UV 交错平面（R16G16）。
    P010,
    /// 8-bit 三平面格式：Y、U、V 三个独立单通道平面（R8）。
    I420,
}

impl VideoPixelFormat {
    /// 返回该像素格式包含的平面数量。
    #[must_use]
    pub const fn plane_count(self) -> usize {
        match self {
            Self::Nv12 | Self::P010 => 2,
            Self::I420 => 3,
        }
    }

    /// 判断该像素格式是否为 10-bit 色深。
    #[must_use]
    pub const fn is_10bit(self) -> bool {
        matches!(self, Self::P010)
    }

    /// 返回传递给 GPU 着色器的像素格式编码。
    #[must_use]
    pub const fn gpu_format_code(self) -> u32 {
        match self {
            Self::Nv12 => 0,
            Self::P010 => 1,
            Self::I420 => 2,
        }
    }

    /// 计算指定平面在给定视频分辨率下的期望纹理维度（宽、高）。
    ///
    /// 当 `plane_index` 超出当前格式的平面数量时返回 `None`。
    #[must_use]
    pub const fn expected_plane_dims(
        self,
        width: u32,
        height: u32,
        plane_index: usize,
    ) -> Option<(u32, u32)> {
        match (self, plane_index) {
            (_, 0) => Some((width, height)),
            (Self::Nv12 | Self::P010, 1) | (Self::I420, 1 | 2) => Some((width / 2, height / 2)),
            _ => None,
        }
    }

    /// 计算指定平面在给定视频宽度下的最小行步长（字节）。
    ///
    /// - NV12：平面 0 为 `width`，平面 1 为 `width`（每 UV 采样点 2 字节）。
    /// - P010：平面 0 为 `width * 2`，平面 1 为 `width * 2`（每 UV 采样点 4 字节）。
    /// - I420：平面 0 为 `width`，平面 1 与平面 2 均为 `width / 2`。
    #[must_use]
    pub const fn min_stride_bytes(self, width: u32, plane_index: usize) -> Option<usize> {
        let w = width as usize;
        match (self, plane_index) {
            (Self::Nv12, 0 | 1) | (Self::I420, 0) => Some(w),
            (Self::P010, 0 | 1) => w.checked_mul(2),
            (Self::I420, 1 | 2) => Some(w / 2),
            _ => None,
        }
    }

    /// 估算源视频平面在 GPU 显存中占用的字节数（VRAM）。
    #[must_use]
    pub const fn estimated_source_vram_bytes(self, width: u32, height: u32) -> u64 {
        let w = width as u64;
        let h = height as u64;
        let y_samples = w.saturating_mul(h);
        let chroma_samples = (w / 2).saturating_mul(h / 2);
        match self {
            Self::Nv12 | Self::I420 => y_samples.saturating_add(chroma_samples.saturating_mul(2)),
            Self::P010 => y_samples
                .saturating_mul(2)
                .saturating_add(chroma_samples.saturating_mul(4)),
        }
    }
}

/// YUV 转 RGB 色彩矩阵标准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoColorMatrix {
    /// 标清视频标准 BT.601。
    Bt601,
    /// 高清视频标准 BT.709（默认值）。
    #[default]
    Bt709,
    /// 超高清与 HDR 视频标准 BT.2020。
    Bt2020,
}

impl VideoColorMatrix {
    /// 返回传递给 GPU 着色器的色彩矩阵编码。
    #[must_use]
    pub const fn gpu_matrix_code(self) -> u32 {
        match self {
            Self::Bt601 => 0,
            Self::Bt709 => 1,
            Self::Bt2020 => 2,
        }
    }
}

/// 视频色彩量化范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoColorRange {
    /// 有限范围（TV 范围，默认值）。
    #[default]
    Limited,
    /// 全范围（PC 范围）。
    Full,
}

impl VideoColorRange {
    /// 返回传递给 GPU 着色器的色彩范围编码。
    #[must_use]
    pub const fn gpu_range_code(self) -> u32 {
        match self {
            Self::Limited => 0,
            Self::Full => 1,
        }
    }
}

/// 视频光电传递函数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoTransferFunction {
    /// 标准动态范围 SDR（默认值）。
    #[default]
    Sdr,
    /// HDR10 使用的感知量化器 PQ（SMPTE ST 2084）。
    Pq,
    /// 广播级 HDR 使用的混合对数伽马 HLG。
    Hlg,
}

impl VideoTransferFunction {
    /// 返回传递给 GPU 着色器的传递函数编码。
    #[must_use]
    pub const fn gpu_transfer_code(self) -> u32 {
        match self {
            Self::Sdr => 0,
            Self::Pq => 1,
            Self::Hlg => 2,
        }
    }
}

/// 完整的视频色彩空间描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VideoColorSpace {
    /// 色彩矩阵。
    pub matrix: VideoColorMatrix,
    /// 色彩量化范围。
    pub range: VideoColorRange,
    /// 光电传递函数。
    pub transfer: VideoTransferFunction,
}

impl VideoColorSpace {
    /// 使用指定的矩阵、范围和传递函数构造色彩空间。
    #[must_use]
    pub const fn new(
        matrix: VideoColorMatrix,
        range: VideoColorRange,
        transfer: VideoTransferFunction,
    ) -> Self {
        Self {
            matrix,
            range,
            transfer,
        }
    }

    /// 构造 BT.601 有限范围 SDR 色彩空间。
    #[must_use]
    pub const fn bt601_limited() -> Self {
        Self::new(
            VideoColorMatrix::Bt601,
            VideoColorRange::Limited,
            VideoTransferFunction::Sdr,
        )
    }

    /// 构造 BT.709 有限范围 SDR 色彩空间。
    #[must_use]
    pub const fn bt709_limited() -> Self {
        Self::new(
            VideoColorMatrix::Bt709,
            VideoColorRange::Limited,
            VideoTransferFunction::Sdr,
        )
    }

    /// 构造 BT.709 全范围 SDR 色彩空间。
    #[must_use]
    pub const fn bt709_full() -> Self {
        Self::new(
            VideoColorMatrix::Bt709,
            VideoColorRange::Full,
            VideoTransferFunction::Sdr,
        )
    }

    /// 构造 BT.2020 有限范围 PQ（HDR10）色彩空间。
    #[must_use]
    pub const fn bt2020_pq() -> Self {
        Self::new(
            VideoColorMatrix::Bt2020,
            VideoColorRange::Limited,
            VideoTransferFunction::Pq,
        )
    }

    /// 构造 BT.2020 有限范围 HLG（HDR）色彩空间。
    #[must_use]
    pub const fn bt2020_hlg() -> Self {
        Self::new(
            VideoColorMatrix::Bt2020,
            VideoColorRange::Limited,
            VideoTransferFunction::Hlg,
        )
    }
}

/// CPU 缓冲区内单个视频平面的布局描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFramePlane {
    /// 该平面在共享字节缓冲区中的起始偏移量（字节）。
    pub offset_bytes: usize,
    /// 该平面相邻两行之间的跨度（字节）。
    pub stride_bytes: usize,
    /// 该平面的纹理宽度（像素/采样点）。
    pub width: u32,
    /// 该平面的纹理高度（像素/采样点）。
    pub height: u32,
}

/// 外部 GPU 共享内存句柄类型（面向 Windows DXGI / Direct3D 互操作）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoExternalHandleKind {
    /// Win32 NT 共享句柄。
    OpaqueWin32,
    /// Win32 KMT 传统共享句柄。
    OpaqueWin32Kmt,
    /// D3D11 纹理 NT 共享句柄。
    D3D11Texture,
    /// D3D11 纹理 KMT 共享句柄。
    D3D11TextureKmt,
    /// D3D12 资源共享句柄。
    D3D12Resource,
    /// D3D12 堆共享句柄。
    D3D12Heap,
}

/// 外部 GPU 共享纹理句柄描述符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoExternalHandleDesc {
    /// 外部句柄类型。
    pub kind: VideoExternalHandleKind,
    /// 原始操作系统句柄值。
    pub raw_handle: isize,
    /// 底层显存分配大小（字节，若由驱动查询可为 0）。
    pub allocation_size: u64,
    /// 纹理数组或子资源索引。
    pub subresource_index: u32,
    /// 同步围栏值。
    pub fence_value: u64,
}

/// 视频帧底层数据存储方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoFrameStorage {
    /// 位于 CPU 内存中的连续多平面缓冲区。
    CpuPlanes {
        /// 共享的原始帧字节数据。
        data: Arc<[u8]>,
        /// 各平面的偏移量、步长与维度描述。
        planes: Vec<VideoFramePlane>,
    },
    /// 来自硬件解码器的零拷贝外部 GPU 句柄，可选携带 CPU 回退平面。
    ExternalHandle {
        /// 外部句柄描述信息。
        handle: VideoExternalHandleDesc,
        /// 当外部句柄导入不可用时的可选 CPU 回退平面数据。
        fallback_cpu: Option<(Arc<[u8]>, Vec<VideoFramePlane>)>,
    },
}

/// 视频帧构造或校验失败时返回的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoFrameError {
    /// 视频帧宽度或高度为零。
    ZeroDimensions {
        /// 实际宽度。
        width: u32,
        /// 实际高度。
        height: u32,
    },
    /// 4:2:0 像素格式要求偶数宽高，但传入了奇数尺寸。
    OddDimensions {
        /// 实际宽度。
        width: u32,
        /// 实际高度。
        height: u32,
        /// 对应的像素格式。
        format: VideoPixelFormat,
    },
    /// 平面数量与像素格式要求不符。
    InvalidPlaneCount {
        /// 对应的像素格式。
        format: VideoPixelFormat,
        /// 期望的平面数量。
        expected: usize,
        /// 实际提供的平面数量。
        actual: usize,
    },
    /// 指定平面的宽高与期望维度不匹配。
    PlaneDimensionsMismatch {
        /// 出错的平面索引。
        plane: usize,
        /// 期望维度（宽、高）。
        expected: (u32, u32),
        /// 实际维度（宽、高）。
        actual: (u32, u32),
    },
    /// 指定平面的行步长小于像素格式要求的最小步长。
    PlaneStrideTooSmall {
        /// 出错的平面索引。
        plane: usize,
        /// 实际行步长（字节）。
        stride: usize,
        /// 要求的最小行步长（字节）。
        min_stride: usize,
    },
    /// 指定平面的字节区间超出底层缓冲区长度。
    PlaneOutOfBounds {
        /// 出错的平面索引。
        plane: usize,
        /// 计算得到的平面结束偏移（字节）。
        end: usize,
        /// 底层缓冲区总长度（字节）。
        data_len: usize,
    },
    /// 外部 GPU 共享句柄无效（例如空句柄或无效哨兵值）。
    InvalidExternalHandle,
}

impl fmt::Display for VideoFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimensions { width, height } => {
                write!(f, "video frame has zero dimensions: {width}x{height}")
            }
            Self::OddDimensions {
                width,
                height,
                format,
            } => write!(
                f,
                "video frame format {format:?} requires even dimensions, got {width}x{height}"
            ),
            Self::InvalidPlaneCount {
                format,
                expected,
                actual,
            } => write!(
                f,
                "invalid plane count for {format:?}: expected {expected}, got {actual}"
            ),
            Self::PlaneDimensionsMismatch {
                plane,
                expected,
                actual,
            } => write!(
                f,
                "plane {plane} dimensions mismatch: expected {}x{}, got {}x{}",
                expected.0, expected.1, actual.0, actual.1
            ),
            Self::PlaneStrideTooSmall {
                plane,
                stride,
                min_stride,
            } => write!(
                f,
                "plane {plane} stride {stride} is smaller than minimum required stride {min_stride}"
            ),
            Self::PlaneOutOfBounds {
                plane,
                end,
                data_len,
            } => write!(
                f,
                "plane {plane} range ends at {end}, exceeding buffer length {data_len}"
            ),
            Self::InvalidExternalHandle => write!(f, "invalid external video handle"),
        }
    }
}

impl std::error::Error for VideoFrameError {}

/// 解码后待上传或导入 GPU 的完整视频帧载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFramePayload {
    /// 视频帧逻辑宽度（像素）。
    pub width: u32,
    /// 视频帧逻辑高度（像素）。
    pub height: u32,
    /// 源视频像素格式。
    pub format: VideoPixelFormat,
    /// 视频色彩空间信息。
    pub color_space: VideoColorSpace,
    /// 显示时间戳（微秒）。
    pub pts_us: i64,
    /// 底层帧数据存储。
    pub storage: VideoFrameStorage,
}

impl VideoFramePayload {
    /// 从自定义 CPU 多平面缓冲区构造视频帧载荷并执行完整性校验。
    pub fn from_cpu_planes(
        width: u32,
        height: u32,
        format: VideoPixelFormat,
        color_space: VideoColorSpace,
        pts_us: i64,
        data: impl Into<Arc<[u8]>>,
        planes: Vec<VideoFramePlane>,
    ) -> Result<Self, VideoFrameError> {
        let payload = Self {
            width,
            height,
            format,
            color_space,
            pts_us,
            storage: VideoFrameStorage::CpuPlanes {
                data: data.into(),
                planes,
            },
        };
        payload.validate()?;
        Ok(payload)
    }

    /// 从紧凑排列的 NV12 缓冲区构造视频帧载荷并执行校验。
    pub fn from_nv12(
        width: u32,
        height: u32,
        color_space: VideoColorSpace,
        pts_us: i64,
        data: impl Into<Arc<[u8]>>,
    ) -> Result<Self, VideoFrameError> {
        validate_frame_dimensions(width, height, VideoPixelFormat::Nv12)?;
        let y_stride = width as usize;
        let y_size = y_stride.saturating_mul(height as usize);
        let planes = vec![
            VideoFramePlane {
                offset_bytes: 0,
                stride_bytes: y_stride,
                width,
                height,
            },
            VideoFramePlane {
                offset_bytes: y_size,
                stride_bytes: y_stride,
                width: width / 2,
                height: height / 2,
            },
        ];
        Self::from_cpu_planes(
            width,
            height,
            VideoPixelFormat::Nv12,
            color_space,
            pts_us,
            data,
            planes,
        )
    }

    /// 从紧凑排列的 P010 缓冲区构造视频帧载荷并执行校验。
    pub fn from_p010(
        width: u32,
        height: u32,
        color_space: VideoColorSpace,
        pts_us: i64,
        data: impl Into<Arc<[u8]>>,
    ) -> Result<Self, VideoFrameError> {
        validate_frame_dimensions(width, height, VideoPixelFormat::P010)?;
        let stride = (width as usize).saturating_mul(2);
        let y_size = stride.saturating_mul(height as usize);
        let planes = vec![
            VideoFramePlane {
                offset_bytes: 0,
                stride_bytes: stride,
                width,
                height,
            },
            VideoFramePlane {
                offset_bytes: y_size,
                stride_bytes: stride,
                width: width / 2,
                height: height / 2,
            },
        ];
        Self::from_cpu_planes(
            width,
            height,
            VideoPixelFormat::P010,
            color_space,
            pts_us,
            data,
            planes,
        )
    }

    /// 从紧凑排列的 I420 缓冲区构造视频帧载荷并执行校验。
    pub fn from_i420(
        width: u32,
        height: u32,
        color_space: VideoColorSpace,
        pts_us: i64,
        data: impl Into<Arc<[u8]>>,
    ) -> Result<Self, VideoFrameError> {
        validate_frame_dimensions(width, height, VideoPixelFormat::I420)?;
        let y_stride = width as usize;
        let y_size = y_stride.saturating_mul(height as usize);
        let uv_width = width / 2;
        let uv_height = height / 2;
        let uv_stride = uv_width as usize;
        let uv_size = uv_stride.saturating_mul(uv_height as usize);
        let planes = vec![
            VideoFramePlane {
                offset_bytes: 0,
                stride_bytes: y_stride,
                width,
                height,
            },
            VideoFramePlane {
                offset_bytes: y_size,
                stride_bytes: uv_stride,
                width: uv_width,
                height: uv_height,
            },
            VideoFramePlane {
                offset_bytes: y_size.saturating_add(uv_size),
                stride_bytes: uv_stride,
                width: uv_width,
                height: uv_height,
            },
        ];
        Self::from_cpu_planes(
            width,
            height,
            VideoPixelFormat::I420,
            color_space,
            pts_us,
            data,
            planes,
        )
    }

    /// 从外部 GPU 共享句柄构造视频帧载荷并执行校验。
    pub fn from_external_handle(
        width: u32,
        height: u32,
        format: VideoPixelFormat,
        color_space: VideoColorSpace,
        pts_us: i64,
        handle: VideoExternalHandleDesc,
    ) -> Result<Self, VideoFrameError> {
        let payload = Self {
            width,
            height,
            format,
            color_space,
            pts_us,
            storage: VideoFrameStorage::ExternalHandle {
                handle,
                fallback_cpu: None,
            },
        };
        payload.validate()?;
        Ok(payload)
    }

    /// 从外部 GPU 共享句柄及可选 CPU 回退平面构造视频帧载荷并执行校验。
    #[allow(clippy::too_many_arguments)]
    pub fn with_external_handle_and_fallback(
        width: u32,
        height: u32,
        format: VideoPixelFormat,
        color_space: VideoColorSpace,
        pts_us: i64,
        handle: VideoExternalHandleDesc,
        fallback_data: impl Into<Arc<[u8]>>,
        fallback_planes: Vec<VideoFramePlane>,
    ) -> Result<Self, VideoFrameError> {
        let payload = Self {
            width,
            height,
            format,
            color_space,
            pts_us,
            storage: VideoFrameStorage::ExternalHandle {
                handle,
                fallback_cpu: Some((fallback_data.into(), fallback_planes)),
            },
        };
        payload.validate()?;
        Ok(payload)
    }

    /// 校验当前视频帧载荷的尺寸、平面布局、步长、缓冲区边界与外部句柄有效性。
    pub fn validate(&self) -> Result<(), VideoFrameError> {
        validate_frame_dimensions(self.width, self.height, self.format)?;
        match &self.storage {
            VideoFrameStorage::CpuPlanes { data, planes } => {
                validate_cpu_planes(self.width, self.height, self.format, data, planes)
            }
            VideoFrameStorage::ExternalHandle {
                handle,
                fallback_cpu,
            } => {
                if handle.raw_handle == 0 || handle.raw_handle == -1 {
                    return Err(VideoFrameError::InvalidExternalHandle);
                }
                if let Some((data, planes)) = fallback_cpu {
                    validate_cpu_planes(self.width, self.height, self.format, data, planes)?;
                }
                Ok(())
            }
        }
    }

    /// 计算色彩转换后目标 RGBA8 纹理所需的字节数（`width * height * 4`）。
    #[must_use]
    pub const fn target_rgba_bytes(&self) -> u64 {
        (self.width as u64)
            .saturating_mul(self.height as u64)
            .saturating_mul(4)
    }

    /// 估算该帧对应的总 VRAM 占用（目标 RGBA 纹理与源 YUV 平面之和）。
    #[must_use]
    pub const fn estimated_vram_bytes(&self) -> u64 {
        self.target_rgba_bytes().saturating_add(
            self.format
                .estimated_source_vram_bytes(self.width, self.height),
        )
    }
}

/// 校验帧宽高非零且满足 4:2:0 偶数约束。
const fn validate_frame_dimensions(
    width: u32,
    height: u32,
    format: VideoPixelFormat,
) -> Result<(), VideoFrameError> {
    if width == 0 || height == 0 {
        return Err(VideoFrameError::ZeroDimensions { width, height });
    }
    if width % 2 != 0 || height % 2 != 0 {
        return Err(VideoFrameError::OddDimensions {
            width,
            height,
            format,
        });
    }
    Ok(())
}

/// 校验 CPU 多平面缓冲区的平面数量、维度、行步长与字节范围。
fn validate_cpu_planes(
    width: u32,
    height: u32,
    format: VideoPixelFormat,
    data: &[u8],
    planes: &[VideoFramePlane],
) -> Result<(), VideoFrameError> {
    let expected_planes = format.plane_count();
    if planes.len() != expected_planes {
        return Err(VideoFrameError::InvalidPlaneCount {
            format,
            expected: expected_planes,
            actual: planes.len(),
        });
    }

    for (index, plane) in planes.iter().enumerate() {
        let Some(expected_dims) = format.expected_plane_dims(width, height, index) else {
            return Err(VideoFrameError::InvalidPlaneCount {
                format,
                expected: expected_planes,
                actual: planes.len(),
            });
        };
        let actual_dims = (plane.width, plane.height);
        if actual_dims != expected_dims {
            return Err(VideoFrameError::PlaneDimensionsMismatch {
                plane: index,
                expected: expected_dims,
                actual: actual_dims,
            });
        }

        let Some(min_stride) = format.min_stride_bytes(width, index) else {
            return Err(VideoFrameError::PlaneStrideTooSmall {
                plane: index,
                stride: plane.stride_bytes,
                min_stride: usize::MAX,
            });
        };
        if plane.stride_bytes < min_stride {
            return Err(VideoFrameError::PlaneStrideTooSmall {
                plane: index,
                stride: plane.stride_bytes,
                min_stride,
            });
        }

        let plane_bytes = plane.stride_bytes.saturating_mul(plane.height as usize);
        let end = plane.offset_bytes.saturating_add(plane_bytes);
        if end > data.len() {
            return Err(VideoFrameError::PlaneOutOfBounds {
                plane: index,
                end,
                data_len: data.len(),
            });
        }
    }

    Ok(())
}

/// 线程安全的 Latest-Wins 单槽位视频帧信箱。
///
/// 解码线程可持续推送最新解码帧；若渲染线程尚未消费上一帧，新帧将直接覆盖旧帧
/// 并累加丢帧计数，从而保证渲染线程始终只消耗最新一帧且永不积压。
#[derive(Debug, Default)]
pub struct LatestFrameMailbox {
    /// 受互斥锁保护的待消费帧槽位。
    slot: Mutex<Option<VideoFramePayload>>,
    /// 累计提交的帧总数。
    submitted_count: AtomicU64,
    /// 累计被覆盖或清空丢弃的未读帧总数。
    dropped_count: AtomicU64,
    /// 自上次调用 `take_dropped_delta` 以来的丢帧增量。
    dropped_delta: AtomicU64,
}

impl LatestFrameMailbox {
    /// 创建一个空的最新帧信箱。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            submitted_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
            dropped_delta: AtomicU64::new(0),
        }
    }

    /// 推送一帧到信箱中；若覆盖了尚未被消费的旧帧则返回 `true` 并累加丢帧计数。
    pub fn push(&self, frame: VideoFramePayload) -> bool {
        self.submitted_count.fetch_add(1, Ordering::Relaxed);
        let mut slot = self.lock_slot();
        let overwritten = slot.replace(frame).is_some();
        if overwritten {
            self.dropped_count.fetch_add(1, Ordering::Relaxed);
            self.dropped_delta.fetch_add(1, Ordering::Relaxed);
        }
        overwritten
    }

    /// 取出当前待消费的最新视频帧，若信箱为空则返回 `None`。
    #[must_use]
    pub fn take_latest(&self) -> Option<VideoFramePayload> {
        self.lock_slot().take()
    }

    /// 检查信箱中是否存在尚未消费的视频帧。
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.lock_slot().is_some()
    }

    /// 返回累计丢弃的未读帧总数。
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// 返回累计提交到信箱的帧总数。
    #[must_use]
    pub fn submitted_count(&self) -> u64 {
        self.submitted_count.load(Ordering::Relaxed)
    }

    /// 原子读取并清零自上次采样以来新增的丢帧数量。
    #[must_use]
    pub fn take_dropped_delta(&self) -> u64 {
        self.dropped_delta.swap(0, Ordering::AcqRel)
    }

    /// 清空信箱中尚未消费的帧，返回本次被丢弃的帧数量（0 或 1）。
    pub fn clear(&self) -> u64 {
        let mut slot = self.lock_slot();
        if slot.take().is_some() {
            self.dropped_count.fetch_add(1, Ordering::Relaxed);
            self.dropped_delta.fetch_add(1, Ordering::Relaxed);
            1
        } else {
            0
        }
    }

    /// 获取内部槽位互斥锁，并在中毒时恢复访问。
    fn lock_slot(&self) -> MutexGuard<'_, Option<VideoFramePayload>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    fn sample_nv12_frame(width: u32, height: u32, pts_us: i64) -> VideoFramePayload {
        let len = (width as usize) * (height as usize) * 3 / 2;
        VideoFramePayload::from_nv12(
            width,
            height,
            VideoColorSpace::bt709_limited(),
            pts_us,
            vec![0x10u8; len],
        )
        .expect("valid NV12 frame")
    }

    #[test]
    fn pixel_format_metadata_and_vram_estimates_match_contract() {
        assert_eq!(VideoPixelFormat::Nv12.plane_count(), 2);
        assert_eq!(VideoPixelFormat::P010.plane_count(), 2);
        assert_eq!(VideoPixelFormat::I420.plane_count(), 3);

        assert!(!VideoPixelFormat::Nv12.is_10bit());
        assert!(VideoPixelFormat::P010.is_10bit());
        assert!(!VideoPixelFormat::I420.is_10bit());

        assert_eq!(VideoPixelFormat::Nv12.gpu_format_code(), 0);
        assert_eq!(VideoPixelFormat::P010.gpu_format_code(), 1);
        assert_eq!(VideoPixelFormat::I420.gpu_format_code(), 2);

        assert_eq!(
            VideoPixelFormat::Nv12.expected_plane_dims(1920, 1080, 0),
            Some((1920, 1080))
        );
        assert_eq!(
            VideoPixelFormat::Nv12.expected_plane_dims(1920, 1080, 1),
            Some((960, 540))
        );
        assert_eq!(
            VideoPixelFormat::Nv12.expected_plane_dims(1920, 1080, 2),
            None
        );

        assert_eq!(
            VideoPixelFormat::I420.expected_plane_dims(1920, 1080, 2),
            Some((960, 540))
        );
        assert_eq!(
            VideoPixelFormat::I420.expected_plane_dims(1920, 1080, 3),
            None
        );

        assert_eq!(VideoPixelFormat::Nv12.min_stride_bytes(1920, 0), Some(1920));
        assert_eq!(VideoPixelFormat::Nv12.min_stride_bytes(1920, 1), Some(1920));
        assert_eq!(VideoPixelFormat::Nv12.min_stride_bytes(1920, 2), None);

        assert_eq!(VideoPixelFormat::P010.min_stride_bytes(1920, 0), Some(3840));
        assert_eq!(VideoPixelFormat::P010.min_stride_bytes(1920, 1), Some(3840));
        assert_eq!(VideoPixelFormat::P010.min_stride_bytes(1920, 2), None);

        assert_eq!(VideoPixelFormat::I420.min_stride_bytes(1920, 0), Some(1920));
        assert_eq!(VideoPixelFormat::I420.min_stride_bytes(1920, 1), Some(960));
        assert_eq!(VideoPixelFormat::I420.min_stride_bytes(1920, 2), Some(960));
        assert_eq!(VideoPixelFormat::I420.min_stride_bytes(1920, 3), None);

        assert_eq!(
            VideoPixelFormat::Nv12.estimated_source_vram_bytes(1920, 1080),
            3_110_400
        );
        assert_eq!(
            VideoPixelFormat::P010.estimated_source_vram_bytes(1920, 1080),
            6_220_800
        );
        assert_eq!(
            VideoPixelFormat::I420.estimated_source_vram_bytes(1920, 1080),
            3_110_400
        );
    }

    #[test]
    fn color_space_presets_and_gpu_codes_are_consistent() {
        assert_eq!(VideoColorMatrix::Bt601.gpu_matrix_code(), 0);
        assert_eq!(VideoColorMatrix::Bt709.gpu_matrix_code(), 1);
        assert_eq!(VideoColorMatrix::Bt2020.gpu_matrix_code(), 2);

        assert_eq!(VideoColorRange::Limited.gpu_range_code(), 0);
        assert_eq!(VideoColorRange::Full.gpu_range_code(), 1);

        assert_eq!(VideoTransferFunction::Sdr.gpu_transfer_code(), 0);
        assert_eq!(VideoTransferFunction::Pq.gpu_transfer_code(), 1);
        assert_eq!(VideoTransferFunction::Hlg.gpu_transfer_code(), 2);

        assert_eq!(VideoColorSpace::default(), VideoColorSpace::bt709_limited());
        assert_eq!(
            VideoColorSpace::bt601_limited(),
            VideoColorSpace::new(
                VideoColorMatrix::Bt601,
                VideoColorRange::Limited,
                VideoTransferFunction::Sdr
            )
        );
        assert_eq!(
            VideoColorSpace::bt709_full(),
            VideoColorSpace::new(
                VideoColorMatrix::Bt709,
                VideoColorRange::Full,
                VideoTransferFunction::Sdr
            )
        );
        assert_eq!(
            VideoColorSpace::bt2020_pq(),
            VideoColorSpace::new(
                VideoColorMatrix::Bt2020,
                VideoColorRange::Limited,
                VideoTransferFunction::Pq
            )
        );
        assert_eq!(
            VideoColorSpace::bt2020_hlg(),
            VideoColorSpace::new(
                VideoColorMatrix::Bt2020,
                VideoColorRange::Limited,
                VideoTransferFunction::Hlg
            )
        );
    }

    #[test]
    fn packed_nv12_p010_and_i420_constructors_succeed_and_compute_vram() {
        let nv12 = VideoFramePayload::from_nv12(
            16,
            8,
            VideoColorSpace::bt709_limited(),
            1000,
            vec![0u8; 16 * 8 * 3 / 2],
        )
        .expect("valid NV12 payload");
        assert_eq!(nv12.target_rgba_bytes(), 16 * 8 * 4);
        assert_eq!(nv12.estimated_vram_bytes(), 16 * 8 * 4 + 16 * 8 * 3 / 2);

        let p010 = VideoFramePayload::from_p010(
            16,
            8,
            VideoColorSpace::bt2020_pq(),
            2000,
            vec![0u8; 16 * 8 * 3],
        )
        .expect("valid P010 payload");
        assert_eq!(p010.target_rgba_bytes(), 16 * 8 * 4);
        assert_eq!(p010.estimated_vram_bytes(), 16 * 8 * 4 + 16 * 8 * 3);

        let i420 = VideoFramePayload::from_i420(
            16,
            8,
            VideoColorSpace::bt601_limited(),
            3000,
            vec![0u8; 16 * 8 * 3 / 2],
        )
        .expect("valid I420 payload");
        assert_eq!(i420.target_rgba_bytes(), 16 * 8 * 4);
        assert_eq!(i420.estimated_vram_bytes(), 16 * 8 * 4 + 16 * 8 * 3 / 2);
    }

    #[test]
    fn padded_stride_cpu_planes_are_accepted() {
        // 宽 16 高 8 的 NV12，行步长按 32 字节对齐。
        let y_stride = 32;
        let uv_offset = y_stride * 8;
        let uv_stride = 32;
        let total_len = uv_offset + uv_stride * 4;
        let payload = VideoFramePayload::from_cpu_planes(
            16,
            8,
            VideoPixelFormat::Nv12,
            VideoColorSpace::bt709_full(),
            42,
            vec![0u8; total_len],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: y_stride,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: uv_offset,
                    stride_bytes: uv_stride,
                    width: 8,
                    height: 4,
                },
            ],
        );
        assert!(payload.is_ok());
    }

    #[test]
    fn zero_and_odd_dimensions_are_rejected() {
        let zero_w =
            VideoFramePayload::from_nv12(0, 8, VideoColorSpace::bt709_limited(), 0, Vec::new());
        assert_eq!(
            zero_w.unwrap_err(),
            VideoFrameError::ZeroDimensions {
                width: 0,
                height: 8
            }
        );

        let zero_h =
            VideoFramePayload::from_p010(16, 0, VideoColorSpace::bt2020_pq(), 0, Vec::new());
        assert_eq!(
            zero_h.unwrap_err(),
            VideoFrameError::ZeroDimensions {
                width: 16,
                height: 0
            }
        );

        let odd_w = VideoFramePayload::from_i420(
            15,
            8,
            VideoColorSpace::bt709_limited(),
            0,
            vec![0u8; 256],
        );
        assert_eq!(
            odd_w.unwrap_err(),
            VideoFrameError::OddDimensions {
                width: 15,
                height: 8,
                format: VideoPixelFormat::I420,
            }
        );

        let odd_h = VideoFramePayload::from_nv12(
            16,
            9,
            VideoColorSpace::bt709_limited(),
            0,
            vec![0u8; 256],
        );
        assert_eq!(
            odd_h.unwrap_err(),
            VideoFrameError::OddDimensions {
                width: 16,
                height: 9,
                format: VideoPixelFormat::Nv12,
            }
        );
    }

    #[test]
    fn plane_count_dims_stride_and_bounds_errors_are_detected() {
        // 平面数量错误。
        let bad_count = VideoFramePayload::from_cpu_planes(
            16,
            8,
            VideoPixelFormat::I420,
            VideoColorSpace::bt709_limited(),
            0,
            vec![0u8; 256],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: 16,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: 128,
                    stride_bytes: 8,
                    width: 8,
                    height: 4,
                },
            ],
        );
        assert_eq!(
            bad_count.unwrap_err(),
            VideoFrameError::InvalidPlaneCount {
                format: VideoPixelFormat::I420,
                expected: 3,
                actual: 2,
            }
        );

        // 平面维度不匹配。
        let bad_dims = VideoFramePayload::from_cpu_planes(
            16,
            8,
            VideoPixelFormat::Nv12,
            VideoColorSpace::bt709_limited(),
            0,
            vec![0u8; 256],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: 16,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: 128,
                    stride_bytes: 16,
                    width: 16,
                    height: 4,
                },
            ],
        );
        assert_eq!(
            bad_dims.unwrap_err(),
            VideoFrameError::PlaneDimensionsMismatch {
                plane: 1,
                expected: (8, 4),
                actual: (16, 4),
            }
        );

        // P010 步长过小（需要至少 32 字节，仅提供 16 字节）。
        let bad_stride = VideoFramePayload::from_cpu_planes(
            16,
            8,
            VideoPixelFormat::P010,
            VideoColorSpace::bt2020_pq(),
            0,
            vec![0u8; 512],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: 32,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: 256,
                    stride_bytes: 16,
                    width: 8,
                    height: 4,
                },
            ],
        );
        assert_eq!(
            bad_stride.unwrap_err(),
            VideoFrameError::PlaneStrideTooSmall {
                plane: 1,
                stride: 16,
                min_stride: 32,
            }
        );

        // 缓冲区越界。
        let oob = VideoFramePayload::from_nv12(
            16,
            8,
            VideoColorSpace::bt709_limited(),
            0,
            vec![0u8; 16 * 8 * 3 / 2 - 1],
        );
        assert_eq!(
            oob.unwrap_err(),
            VideoFrameError::PlaneOutOfBounds {
                plane: 1,
                end: 192,
                data_len: 191,
            }
        );
    }

    #[test]
    fn external_handle_validation_and_fallback_planes() {
        let invalid_null = VideoExternalHandleDesc {
            kind: VideoExternalHandleKind::D3D11Texture,
            raw_handle: 0,
            allocation_size: 4096,
            subresource_index: 0,
            fence_value: 1,
        };
        assert_eq!(
            VideoFramePayload::from_external_handle(
                16,
                8,
                VideoPixelFormat::Nv12,
                VideoColorSpace::bt709_limited(),
                10,
                invalid_null,
            )
            .unwrap_err(),
            VideoFrameError::InvalidExternalHandle
        );

        let invalid_sentinel = VideoExternalHandleDesc {
            kind: VideoExternalHandleKind::D3D12Resource,
            raw_handle: -1,
            allocation_size: 4096,
            subresource_index: 0,
            fence_value: 1,
        };
        assert_eq!(
            VideoFramePayload::from_external_handle(
                16,
                8,
                VideoPixelFormat::P010,
                VideoColorSpace::bt2020_pq(),
                20,
                invalid_sentinel,
            )
            .unwrap_err(),
            VideoFrameError::InvalidExternalHandle
        );

        let valid_handle = VideoExternalHandleDesc {
            kind: VideoExternalHandleKind::OpaqueWin32,
            raw_handle: 0x1234,
            allocation_size: 192,
            subresource_index: 0,
            fence_value: 7,
        };
        assert!(VideoFramePayload::from_external_handle(
            16,
            8,
            VideoPixelFormat::Nv12,
            VideoColorSpace::bt709_limited(),
            30,
            valid_handle,
        )
        .is_ok());

        // 带无效 CPU 回退平面的外部句柄也应报错。
        let bad_fallback = VideoFramePayload::with_external_handle_and_fallback(
            16,
            8,
            VideoPixelFormat::Nv12,
            VideoColorSpace::bt709_limited(),
            40,
            valid_handle,
            vec![0u8; 64],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: 16,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: 128,
                    stride_bytes: 16,
                    width: 8,
                    height: 4,
                },
            ],
        );
        assert_eq!(
            bad_fallback.unwrap_err(),
            VideoFrameError::PlaneOutOfBounds {
                plane: 0,
                end: 128,
                data_len: 64,
            }
        );

        // 带有效 CPU 回退平面的外部句柄应成功通过校验。
        let good_fallback = VideoFramePayload::with_external_handle_and_fallback(
            16,
            8,
            VideoPixelFormat::Nv12,
            VideoColorSpace::bt709_limited(),
            50,
            valid_handle,
            vec![0u8; 192],
            vec![
                VideoFramePlane {
                    offset_bytes: 0,
                    stride_bytes: 16,
                    width: 16,
                    height: 8,
                },
                VideoFramePlane {
                    offset_bytes: 128,
                    stride_bytes: 16,
                    width: 8,
                    height: 4,
                },
            ],
        );
        assert!(good_fallback.is_ok());
    }

    #[test]
    fn latest_frame_mailbox_overwrites_unread_frames_and_tracks_drops() {
        assert_send_sync::<LatestFrameMailbox>();

        let mailbox = LatestFrameMailbox::new();
        assert!(!mailbox.has_pending());
        assert_eq!(mailbox.submitted_count(), 0);
        assert_eq!(mailbox.dropped_count(), 0);
        assert_eq!(mailbox.take_dropped_delta(), 0);
        assert!(mailbox.take_latest().is_none());

        // 连续超速提交 4 帧，仅第一帧不触发覆盖，其余 3 帧覆盖未读旧帧。
        assert!(!mailbox.push(sample_nv12_frame(16, 8, 100)));
        assert!(mailbox.push(sample_nv12_frame(16, 8, 200)));
        assert!(mailbox.push(sample_nv12_frame(16, 8, 300)));
        assert!(mailbox.push(sample_nv12_frame(16, 8, 400)));

        assert!(mailbox.has_pending());
        assert_eq!(mailbox.submitted_count(), 4);
        assert_eq!(mailbox.dropped_count(), 3);
        assert_eq!(mailbox.take_dropped_delta(), 3);
        assert_eq!(mailbox.take_dropped_delta(), 0);

        let latest = mailbox.take_latest().expect("latest frame present");
        assert_eq!(latest.pts_us, 400);
        assert!(!mailbox.has_pending());

        // 再次超速提交 2 帧，验证累计与增量计数。
        assert!(!mailbox.push(sample_nv12_frame(16, 8, 500)));
        assert!(mailbox.push(sample_nv12_frame(16, 8, 600)));
        assert_eq!(mailbox.submitted_count(), 6);
        assert_eq!(mailbox.dropped_count(), 4);
        assert_eq!(mailbox.take_dropped_delta(), 1);

        // 清空待读帧应返回 1 并累加丢帧计数，重复清空返回 0。
        assert_eq!(mailbox.clear(), 1);
        assert_eq!(mailbox.clear(), 0);
        assert!(!mailbox.has_pending());
        assert_eq!(mailbox.dropped_count(), 5);
        assert_eq!(mailbox.take_dropped_delta(), 1);
    }

    #[test]
    fn error_display_messages_are_non_empty_english() {
        let err = VideoFrameError::InvalidExternalHandle;
        assert!(!err.to_string().is_empty());
        assert!(err.to_string().is_ascii());
    }
}

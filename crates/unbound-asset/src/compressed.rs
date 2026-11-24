//! 纯 CPU 压缩纹理（BC7）载荷校验、分级多页货架图集分配器、帧级上传节拍队列与软解降级。
//!
//! 本模块严格保持零 GPU 依赖与零 `unsafe` 代码，为上层 `unbound-app` 与 `unbound-gpu`
//! 提供确定性的 BC7 压缩纹理建模能力：
//!
//! 1. **格式与载荷校验**：强校验 4×4 块对齐、非零尺寸与精确字节长度（1 B/px）。
//! 2. **大小分流与多页货架分配**：`TieredShelfMultiPageAllocator` 支持固定 `2048×2048`
//!    多页扩展、4px 块对齐高度档位与释放即时合并相邻 `free_spans`。
//! 3. **帧级上传节拍队列**：`CompressedUploadQueue` 按 `UploadPacingBudget` 双维度封顶
//!    （条数 + 字节数）按 FIFO 排空，支持按 `AssetId` 去重覆写与离屏取消。
//! 4. **纯 Rust 软解降级**：`decode_bc7_to_rgba8_fallback` 对 Mode 6 块精确解码为 RGBA8，
//!    供无 BC7 硬件或无头环境安全回退。

use std::collections::{HashMap, VecDeque};

use crate::transfer::{AssetError, AssetId};

/// BC7 压缩块的像素边长（4×4 像素块）。
pub const BC7_BLOCK_DIM: u32 = 4;

/// 单个 BC7 压缩块的字节大小（16 字节 / 128 位）。
pub const BC7_BLOCK_BYTES: usize = 16;

/// BC7 多页动态图集单页的像素边长（2048×2048）。
pub const BC7_ATLAS_PAGE_SIZE: u32 = 2048;

/// BC7 多页动态图集单页占用的显存字节数（精确 1 B/px，共 4 MiB）。
pub const BC7_ATLAS_PAGE_BYTES: u64 = (BC7_ATLAS_PAGE_SIZE as u64) * (BC7_ATLAS_PAGE_SIZE as u64);

/// 默认压缩纹理路由阈值（宽高均不超过 512px 进入多页图集，否则走独立外部槽）。
pub const DEFAULT_COMPRESSED_ATLAS_THRESHOLD: u32 = 512;

/// 默认 4px 块对齐分级货架高度档位。
pub const DEFAULT_SHELF_HEIGHT_TIERS: [u32; 4] = [64, 128, 256, 512];

/// Khronos BC7 Mode 6 透明黑块（`r=0, g=0, b=0, a=0`），用于新图集页清零。
pub const BC7_MODE6_TRANSPARENT_BLACK_BLOCK: [u8; 16] =
    [0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// 每帧默认上传条数封顶。
const DEFAULT_MAX_UPLOADS_PER_FRAME: usize = 16;

/// 每帧默认上传字节数封顶（2 MiB）。
const DEFAULT_MAX_BYTES_PER_FRAME: usize = 2 * 1024 * 1024;

/// 待传队列默认容量上限。
const DEFAULT_UPLOAD_QUEUE_CAPACITY: usize = 4096;

/// BC7 4-bit 索引插值权重表（Khronos Data Format Specification）。
const BC7_WEIGHTS4: [u16; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// 受支持的压缩像素格式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompressedPixelFormat {
    /// BC7 无符号归一化块压缩格式（对应 Vulkan `VK_FORMAT_BC7_UNORM_BLOCK`）。
    Bc7Unorm,
}

impl CompressedPixelFormat {
    /// 返回单个压缩块的像素边长。
    #[must_use]
    pub const fn block_dim(self) -> u32 {
        match self {
            Self::Bc7Unorm => BC7_BLOCK_DIM,
        }
    }

    /// 返回单个压缩块占用的字节数。
    #[must_use]
    pub const fn block_bytes(self) -> usize {
        match self {
            Self::Bc7Unorm => BC7_BLOCK_BYTES,
        }
    }

    /// 计算给定尺寸对应的精确压缩字节长度。
    ///
    /// 若尺寸为零、未按压缩块边长对齐或乘法溢出，则返回 `None`。
    #[must_use]
    pub fn expected_byte_len(self, width: u32, height: u32) -> Option<usize> {
        let dim = self.block_dim();
        if width == 0 || height == 0 || width % dim != 0 || height % dim != 0 {
            return None;
        }
        let blocks_x = usize::try_from(width / dim).ok()?;
        let blocks_y = usize::try_from(height / dim).ok()?;
        blocks_x
            .checked_mul(blocks_y)?
            .checked_mul(self.block_bytes())
    }
}

/// 压缩纹理校验或图集分配期间抛出的错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompressedTextureError {
    /// 宽度或高度为零。
    ZeroDimension {
        /// 请求宽度（像素）。
        width: u32,
        /// 请求高度（像素）。
        height: u32,
    },
    /// 宽度或高度未按压缩块边长对齐。
    UnalignedDimension {
        /// 请求宽度（像素）。
        width: u32,
        /// 请求高度（像素）。
        height: u32,
        /// 要求的块边长（像素）。
        block_dim: u32,
    },
    /// 字节长度与块尺寸计算值不匹配。
    ByteLengthMismatch {
        /// 期望字节数。
        expected: usize,
        /// 实际字节数。
        actual: usize,
        /// 纹理宽度（像素）。
        width: u32,
        /// 纹理高度（像素）。
        height: u32,
    },
    /// 图集区域超出页边界或无可用高度档位。
    InvalidRegion {
        /// 区域起始 X 坐标（像素）。
        x: u32,
        /// 区域起始 Y 坐标（像素）。
        y: u32,
        /// 区域宽度（像素）。
        width: u32,
        /// 区域高度（像素）。
        height: u32,
        /// 图集页边长（像素）。
        page_size: u32,
    },
}

impl core::fmt::Display for CompressedTextureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroDimension { width, height } => {
                write!(
                    f,
                    "compressed texture dimensions must be non-zero, got {width}x{height}"
                )
            }
            Self::UnalignedDimension {
                width,
                height,
                block_dim,
            } => {
                write!(
                    f,
                    "compressed texture dimensions {width}x{height} are not aligned to {block_dim}x{block_dim} blocks"
                )
            }
            Self::ByteLengthMismatch {
                expected,
                actual,
                width,
                height,
            } => {
                write!(
                    f,
                    "compressed texture byte length mismatch for {width}x{height}: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidRegion {
                x,
                y,
                width,
                height,
                page_size,
            } => {
                write!(
                    f,
                    "invalid atlas region ({x}, {y}, {width}x{height}) for page size {page_size}"
                )
            }
        }
    }
}

impl std::error::Error for CompressedTextureError {}

/// 已通过格式、对齐与字节长度校验的压缩纹理载荷。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedTexturePayload {
    format: CompressedPixelFormat,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl CompressedTexturePayload {
    /// 构造并严格校验压缩纹理载荷。
    ///
    /// # Errors
    /// 当宽高为零、未按 4×4 块对齐或字节长度不匹配时返回 [`CompressedTextureError`]。
    pub fn new(
        format: CompressedPixelFormat,
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, CompressedTextureError> {
        if width == 0 || height == 0 {
            return Err(CompressedTextureError::ZeroDimension { width, height });
        }
        let block_dim = format.block_dim();
        if width % block_dim != 0 || height % block_dim != 0 {
            return Err(CompressedTextureError::UnalignedDimension {
                width,
                height,
                block_dim,
            });
        }
        let Some(expected) = format.expected_byte_len(width, height) else {
            return Err(CompressedTextureError::ByteLengthMismatch {
                expected: usize::MAX,
                actual: bytes.len(),
                width,
                height,
            });
        };
        if bytes.len() != expected {
            return Err(CompressedTextureError::ByteLengthMismatch {
                expected,
                actual: bytes.len(),
                width,
                height,
            });
        }
        Ok(Self {
            format,
            width,
            height,
            bytes,
        })
    }

    /// 构造 `CompressedPixelFormat::Bc7Unorm` 载荷的便捷入口。
    ///
    /// # Errors
    /// 当宽高为零、未按 4×4 块对齐或字节长度不匹配时返回 [`CompressedTextureError`]。
    pub fn bc7_unorm(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, CompressedTextureError> {
        Self::new(CompressedPixelFormat::Bc7Unorm, width, height, bytes)
    }

    /// 返回压缩像素格式。
    #[must_use]
    pub const fn format(&self) -> CompressedPixelFormat {
        self.format
    }

    /// 返回纹理宽度（像素）。
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 返回纹理高度（像素）。
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 返回底层压缩块字节切片。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 消费载荷并取回底层字节缓冲区。
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// 返回该载荷在显存中占用的精确字节数（BC7 为精确 1 B/px）。
    #[must_use]
    pub fn vram_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
}

/// 构造符合 Khronos BC7 Mode 6 规范的纯色 16 字节压缩块。
///
/// Mode 6 位布局（小端 128-bit）：
/// - bits `0..7`：模式位 `0b100_0000`（`0x40`）
/// - bits `7..63`：`R0/R1/G0/G1/B0/B1/A0/A1` 各 7 bit
/// - bit `63` / `64`：共享端点低位 `P0` / `P1` 各 1 bit
/// - bits `65..128`：16 个像素插值索引（63 bit，纯色块全为 0）
///
/// 当 `r=0, g=0, b=0, a=0` 时，输出严格等于 [`BC7_MODE6_TRANSPARENT_BLACK_BLOCK`]。
#[must_use]
pub fn bc7_solid_mode6_block(r: u8, g: u8, b: u8, a: u8) -> [u8; 16] {
    let r7 = u128::from(r >> 1);
    let g7 = u128::from(g >> 1);
    let b7 = u128::from(b >> 1);
    let a7 = u128::from(a >> 1);

    // 四个通道共享同一个 1-bit p-bit；采用多数表决以最小化量化误差，同时保证全 0 严格为 0。
    let lsb_ones = (r & 1) + (g & 1) + (b & 1) + (a & 1);
    let p = u128::from(lsb_ones >= 2);

    let bits: u128 = (1u128 << 6)
        | (r7 << 7)
        | (r7 << 14)
        | (g7 << 21)
        | (g7 << 28)
        | (b7 << 35)
        | (b7 << 42)
        | (a7 << 49)
        | (a7 << 56)
        | (p << 63)
        | (p << 64);

    bits.to_le_bytes()
}

/// 纯 Rust 安全软解函数：将 BC7 载荷解码为 RGBA8 像素缓冲区。
///
/// 对 Mode 6 块按 Khronos BC7 规范精确解码所有 16 个像素；对其他有效模式输出确定性、
/// 非崩溃的中性 RGBA8 像素；对保留无效模式按规范输出透明黑，供无 BC7 硬件安全降级使用。
#[must_use]
pub fn decode_bc7_to_rgba8_fallback(payload: &CompressedTexturePayload) -> Vec<u8> {
    let width = payload.width() as usize;
    let height = payload.height() as usize;
    let blocks_x = width / (BC7_BLOCK_DIM as usize);
    let blocks_y = height / (BC7_BLOCK_DIM as usize);
    let mut rgba = vec![0u8; width * height * 4];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let block_idx = by * blocks_x + bx;
            let offset = block_idx * BC7_BLOCK_BYTES;
            let mut block = [0u8; BC7_BLOCK_BYTES];
            block.copy_from_slice(&payload.bytes()[offset..offset + BC7_BLOCK_BYTES]);
            let pixels = decode_bc7_block_rgba8(block);

            for py in 0..4usize {
                for px in 0..4usize {
                    let dst_x = bx * 4 + px;
                    let dst_y = by * 4 + py;
                    let dst_offset = (dst_y * width + dst_x) * 4;
                    rgba[dst_offset..dst_offset + 4].copy_from_slice(&pixels[py * 4 + px]);
                }
            }
        }
    }

    rgba
}

/// 解码单个 16 字节 BC7 压缩块为 4×4 RGBA8 像素。
fn decode_bc7_block_rgba8(block: [u8; 16]) -> [[u8; 4]; 16] {
    let bits = u128::from_le_bytes(block);
    let mode = bits.trailing_zeros();

    if mode == 6 {
        let r0 = ((bits >> 7) & 0x7F) as u8;
        let r1 = ((bits >> 14) & 0x7F) as u8;
        let g0 = ((bits >> 21) & 0x7F) as u8;
        let g1 = ((bits >> 28) & 0x7F) as u8;
        let b0 = ((bits >> 35) & 0x7F) as u8;
        let b1 = ((bits >> 42) & 0x7F) as u8;
        let a0 = ((bits >> 49) & 0x7F) as u8;
        let a1 = ((bits >> 56) & 0x7F) as u8;
        let p0 = ((bits >> 63) & 1) as u8;
        let p1 = ((bits >> 64) & 1) as u8;

        let ep0 = [
            (r0 << 1) | p0,
            (g0 << 1) | p0,
            (b0 << 1) | p0,
            (a0 << 1) | p0,
        ];
        let ep1 = [
            (r1 << 1) | p1,
            (g1 << 1) | p1,
            (b1 << 1) | p1,
            (a1 << 1) | p1,
        ];

        let mut out = [[0u8; 4]; 16];
        for (i, pixel) in out.iter_mut().enumerate() {
            let idx = if i == 0 {
                ((bits >> 65) & 0x7) as usize
            } else {
                ((bits >> (68 + (i - 1) * 4)) & 0xF) as usize
            };
            let w = BC7_WEIGHTS4[idx];
            for ch in 0..4 {
                let interpolated =
                    ((64 - w) * u16::from(ep0[ch]) + w * u16::from(ep1[ch]) + 32) >> 6;
                pixel[ch] = interpolated as u8;
            }
        }
        return out;
    }

    if mode >= 8 {
        // Khronos 规范：高 8 位均无模式置位（含全 0 块）视为保留块，解码为透明黑。
        return [[0, 0, 0, 0]; 16];
    }

    // 其余有效模式（Mode 0..=5、Mode 7）：安全输出确定性的非黑 RGBA8 降级像素。
    let r = block[1] ^ block[5];
    let g = block[2] ^ block[6];
    let b = block[3] ^ block[7];
    [[r, g, b, 0xFF]; 16]
}

/// 压缩纹理尺寸分流目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CompressedTextureRoute {
    /// 宽高均在阈值以内，进入多页动态图集。
    AtlasPage,
    /// 任一边超过阈值，路由至独立 Bindless 外部纹理槽。
    ExternalStandalone,
}

/// 根据尺寸与阈值决定压缩纹理应进入多页图集还是独立外部槽。
#[must_use]
pub const fn route_compressed_texture(
    width: u32,
    height: u32,
    threshold: u32,
) -> CompressedTextureRoute {
    if width <= threshold && height <= threshold {
        CompressedTextureRoute::AtlasPage
    } else {
        CompressedTextureRoute::ExternalStandalone
    }
}

/// 动态多页图集的稳定逻辑页标识。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtlasPageId(pub u32);

/// 多页货架图集中的单条已分配区域描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasAllocation {
    /// 所属逻辑图集页标识。
    pub page: AtlasPageId,
    /// 区域左上角 X 坐标（像素，4px 对齐）。
    pub x: u32,
    /// 区域左上角 Y 坐标（像素，4px 对齐）。
    pub y: u32,
    /// 条目实际宽度（像素）。
    pub width: u32,
    /// 条目实际高度（像素）。
    pub height: u32,
    /// 所属货架档位高度（像素）。
    pub shelf_height: u32,
    /// 所属图集页边长（像素）。
    pub page_size: u32,
}

impl AtlasAllocation {
    /// 计算该分配区域在图集页中的归一化 UV 边界 `[u0, v0, u1, v1]`。
    #[must_use]
    pub fn uv_bounds(&self) -> [f32; 4] {
        if self.page_size == 0 {
            return [0.0, 0.0, 0.0, 0.0];
        }
        let inv = 1.0 / (self.page_size as f32);
        [
            self.x as f32 * inv,
            self.y as f32 * inv,
            (self.x + self.width) as f32 * inv,
            (self.y + self.height) as f32 * inv,
        ]
    }
}

/// 单个货架内的水平空闲区间（严格按 `x` 升序维护且两两不相邻）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct FreeSpan {
    x: u32,
    width: u32,
}

/// 图集页内的一条定高水平货架。
#[derive(Clone, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    free_spans: Vec<FreeSpan>,
}

/// 逻辑多页图集中的单页状态。
#[derive(Clone, Debug)]
struct AtlasPage {
    id: AtlasPageId,
    next_y: u32,
    active_count: usize,
    shelves: Vec<Shelf>,
}

/// 4px 块对齐分级高度货架多页分配器。
///
/// 支持任意长宽比条目装箱、按需跨页扩展、释放即时合并相邻水平 `free_spans`（杜绝墓碑泄漏），
/// 以及空页识别与回收。
#[derive(Clone, Debug)]
pub struct TieredShelfMultiPageAllocator {
    page_size: u32,
    tiers: Vec<u32>,
    next_page_id: u32,
    pages: Vec<AtlasPage>,
}

impl Default for TieredShelfMultiPageAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl TieredShelfMultiPageAllocator {
    /// 使用默认配置（`2048×2048` 页、档位 `[64, 128, 256, 512]`）构造分配器。
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(BC7_ATLAS_PAGE_SIZE, DEFAULT_SHELF_HEIGHT_TIERS.to_vec())
    }

    /// 使用自定义页尺寸与高度档位列表构造分配器。
    ///
    /// 非法档位（零值、未按 4px 对齐或超过页尺寸）会被自动过滤，剩余档位按升序去重。
    #[must_use]
    pub fn with_config(page_size: u32, tiers: Vec<u32>) -> Self {
        let mut normalized: Vec<u32> = tiers
            .into_iter()
            .filter(|&tier| tier > 0 && tier % BC7_BLOCK_DIM == 0 && tier <= page_size)
            .collect();
        normalized.sort_unstable();
        normalized.dedup();
        Self {
            page_size,
            tiers: normalized,
            next_page_id: 0,
            pages: Vec::new(),
        }
    }

    /// 为指定尺寸分配一块 4px 对齐的图集区域，返回 `(分配结果, 是否新建了图集页)`。
    ///
    /// 分配优先级：
    /// 1. 在已有页的对应档位货架的 `free_spans` 中执行最佳适配（best-fit）；
    /// 2. 在已有页剩余纵向空间中开辟新货架；
    /// 3. 创建新页并在其顶部开辟首个货架。
    ///
    /// # Errors
    /// 当尺寸为零、未按 4px 对齐、超出页尺寸或无可用高度档位时返回 [`CompressedTextureError`]。
    pub fn allocate(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(AtlasAllocation, bool), CompressedTextureError> {
        if width == 0 || height == 0 {
            return Err(CompressedTextureError::ZeroDimension { width, height });
        }
        if width % BC7_BLOCK_DIM != 0 || height % BC7_BLOCK_DIM != 0 {
            return Err(CompressedTextureError::UnalignedDimension {
                width,
                height,
                block_dim: BC7_BLOCK_DIM,
            });
        }
        if self.page_size == 0 || width > self.page_size || height > self.page_size {
            return Err(CompressedTextureError::InvalidRegion {
                x: 0,
                y: 0,
                width,
                height,
                page_size: self.page_size,
            });
        }

        let Some(shelf_height) = self.tiers.iter().copied().find(|&tier| tier >= height) else {
            return Err(CompressedTextureError::InvalidRegion {
                x: 0,
                y: 0,
                width,
                height,
                page_size: self.page_size,
            });
        };

        // 优先级 1：在已有页的同档位货架中查找可容纳的最佳空闲水平区间。
        let mut best_fit: Option<(usize, usize, usize, u32)> = None;
        for (page_idx, page) in self.pages.iter().enumerate() {
            for (shelf_idx, shelf) in page.shelves.iter().enumerate() {
                if shelf.height != shelf_height {
                    continue;
                }
                for (span_idx, span) in shelf.free_spans.iter().enumerate() {
                    if span.width < width {
                        continue;
                    }
                    let waste = span.width - width;
                    if best_fit.is_none_or(|(_, _, _, best_waste)| waste < best_waste) {
                        best_fit = Some((page_idx, shelf_idx, span_idx, waste));
                        if waste == 0 {
                            break;
                        }
                    }
                }
            }
        }

        if let Some((page_idx, shelf_idx, span_idx, _)) = best_fit {
            let page = &mut self.pages[page_idx];
            let shelf = &mut page.shelves[shelf_idx];
            let span = shelf.free_spans[span_idx];
            let alloc_x = span.x;
            if span.width == width {
                shelf.free_spans.remove(span_idx);
            } else {
                shelf.free_spans[span_idx] = FreeSpan {
                    x: span.x + width,
                    width: span.width - width,
                };
            }
            page.active_count += 1;
            return Ok((
                AtlasAllocation {
                    page: page.id,
                    x: alloc_x,
                    y: shelf.y,
                    width,
                    height,
                    shelf_height,
                    page_size: self.page_size,
                },
                false,
            ));
        }

        // 优先级 2：在已有页中开辟新货架。
        for page in &mut self.pages {
            if page.next_y + shelf_height <= self.page_size {
                let shelf_y = page.next_y;
                page.next_y += shelf_height;
                let mut free_spans = Vec::new();
                if width < self.page_size {
                    free_spans.push(FreeSpan {
                        x: width,
                        width: self.page_size - width,
                    });
                }
                page.shelves.push(Shelf {
                    y: shelf_y,
                    height: shelf_height,
                    free_spans,
                });
                page.active_count += 1;
                return Ok((
                    AtlasAllocation {
                        page: page.id,
                        x: 0,
                        y: shelf_y,
                        width,
                        height,
                        shelf_height,
                        page_size: self.page_size,
                    },
                    false,
                ));
            }
        }

        // 优先级 3：创建新图集页并开辟首个货架。
        let page_id = AtlasPageId(self.next_page_id);
        self.next_page_id += 1;
        let mut free_spans = Vec::new();
        if width < self.page_size {
            free_spans.push(FreeSpan {
                x: width,
                width: self.page_size - width,
            });
        }
        self.pages.push(AtlasPage {
            id: page_id,
            next_y: shelf_height,
            active_count: 1,
            shelves: vec![Shelf {
                y: 0,
                height: shelf_height,
                free_spans,
            }],
        });

        Ok((
            AtlasAllocation {
                page: page_id,
                x: 0,
                y: 0,
                width,
                height,
                shelf_height,
                page_size: self.page_size,
            },
            true,
        ))
    }

    /// 释放一条分配记录并立即合并相邻空闲水平区间（`free_spans`）。
    ///
    /// 若页或货架不存在、区间越界或与现有空闲区间重叠（重复释放），则返回 `false`。
    pub fn free(&mut self, alloc: AtlasAllocation) -> bool {
        if alloc.width == 0 || alloc.height == 0 {
            return false;
        }
        let Some(alloc_end) = alloc.x.checked_add(alloc.width) else {
            return false;
        };
        if alloc_end > self.page_size {
            return false;
        }

        let Some(page) = self.pages.iter_mut().find(|p| p.id == alloc.page) else {
            return false;
        };
        if page.active_count == 0 {
            return false;
        }
        let Some(shelf) = page
            .shelves
            .iter_mut()
            .find(|s| s.y == alloc.y && s.height == alloc.shelf_height)
        else {
            return false;
        };

        let idx = shelf.free_spans.partition_point(|span| span.x < alloc.x);

        // 校验释放区间不与左右已有空闲区间重叠，防止重复释放破坏不变量。
        if idx > 0 {
            let left = shelf.free_spans[idx - 1];
            if left.x + left.width > alloc.x {
                return false;
            }
        }
        if idx < shelf.free_spans.len() {
            let right = shelf.free_spans[idx];
            if alloc_end > right.x {
                return false;
            }
        }

        let merge_left =
            idx > 0 && shelf.free_spans[idx - 1].x + shelf.free_spans[idx - 1].width == alloc.x;
        let merge_right = idx < shelf.free_spans.len() && alloc_end == shelf.free_spans[idx].x;

        match (merge_left, merge_right) {
            (true, true) => {
                let right_width = shelf.free_spans[idx].width;
                shelf.free_spans[idx - 1].width += alloc.width + right_width;
                shelf.free_spans.remove(idx);
            }
            (true, false) => {
                shelf.free_spans[idx - 1].width += alloc.width;
            }
            (false, true) => {
                shelf.free_spans[idx].x = alloc.x;
                shelf.free_spans[idx].width += alloc.width;
            }
            (false, false) => {
                shelf.free_spans.insert(
                    idx,
                    FreeSpan {
                        x: alloc.x,
                        width: alloc.width,
                    },
                );
            }
        }

        page.active_count -= 1;
        true
    }

    /// 返回当前存活的逻辑图集页总数。
    #[must_use]
    pub fn active_page_count(&self) -> usize {
        self.pages.len()
    }

    /// 返回当前所有存活逻辑页的标识列表。
    #[must_use]
    pub fn page_ids(&self) -> Vec<AtlasPageId> {
        self.pages.iter().map(|p| p.id).collect()
    }

    /// 返回指定页内的活跃分配条目数（页不存在时返回 0）。
    #[must_use]
    pub fn page_active_count(&self, page: AtlasPageId) -> usize {
        self.pages
            .iter()
            .find(|p| p.id == page)
            .map_or(0, |p| p.active_count)
    }

    /// 判断指定页是否存在且已无任何活跃条目。
    #[must_use]
    pub fn is_page_empty(&self, page: AtlasPageId) -> bool {
        self.pages
            .iter()
            .find(|p| p.id == page)
            .is_some_and(|p| p.active_count == 0)
    }

    /// 返回当前所有已变空（活跃条目数为 0）的图集页标识列表。
    #[must_use]
    pub fn empty_pages(&self) -> Vec<AtlasPageId> {
        self.pages
            .iter()
            .filter(|p| p.active_count == 0)
            .map(|p| p.id)
            .collect()
    }

    /// 仅当指定页存在且为空时将其移除并返回 `true`。
    pub fn remove_empty_page(&mut self, page: AtlasPageId) -> bool {
        if let Some(pos) = self
            .pages
            .iter()
            .position(|p| p.id == page && p.active_count == 0)
        {
            self.pages.remove(pos);
            true
        } else {
            false
        }
    }

    /// 无条件移除指定逻辑页（例如设备丢失重建时清理）并返回是否找到。
    pub fn remove_page(&mut self, page: AtlasPageId) -> bool {
        if let Some(pos) = self.pages.iter().position(|p| p.id == page) {
            self.pages.remove(pos);
            true
        } else {
            false
        }
    }

    /// 清空全部逻辑页（保留递增页 ID 计数器以避免与旧句柄混淆）。
    pub fn clear(&mut self) {
        self.pages.clear();
    }

    /// 返回指定页内所有货架的 `free_spans` 区间总数；若页不存在则返回 `None`。
    #[must_use]
    pub fn page_free_span_count(&self, page: AtlasPageId) -> Option<usize> {
        self.pages
            .iter()
            .find(|p| p.id == page)
            .map(|p| p.shelves.iter().map(|s| s.free_spans.len()).sum())
    }
}

/// 单帧压缩纹理上传节拍预算（条数与字节数双重封顶）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UploadPacingBudget {
    /// 单帧允许出队的最大纹理条数。
    pub max_uploads: usize,
    /// 单帧允许出队的最大压缩字节总数。
    pub max_bytes: usize,
}

impl Default for UploadPacingBudget {
    fn default() -> Self {
        Self {
            max_uploads: DEFAULT_MAX_UPLOADS_PER_FRAME,
            max_bytes: DEFAULT_MAX_BYTES_PER_FRAME,
        }
    }
}

impl UploadPacingBudget {
    /// 构造自定义双维度帧级上传配额。
    #[must_use]
    pub const fn new(max_uploads: usize, max_bytes: usize) -> Self {
        Self {
            max_uploads,
            max_bytes,
        }
    }

    /// 构造不设限制的上传预算（用于批量预热或无节拍测试场景）。
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_uploads: usize::MAX,
            max_bytes: usize::MAX,
        }
    }
}

/// 队列中等待上传的单条压缩纹理任务。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCompressedUpload {
    /// 资产唯一标识。
    pub id: AssetId,
    /// 已校验的压缩纹理载荷。
    pub payload: CompressedTexturePayload,
    /// 单调递增版本号，用于区分同 `AssetId` 的覆写提交。
    pub generation: u64,
}

/// 单帧按节拍排空后产出的压缩纹理上传批次。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedAtlasUploadBatch {
    /// 本帧准许上传的条目列表（严格 FIFO 顺序）。
    pub items: Vec<PendingCompressedUpload>,
    /// 本批次条目占用的压缩字节总数。
    pub total_bytes: usize,
}

/// 有界、可取消、支持按 `AssetId` 去重覆写的帧级压缩纹理待传队列。
#[derive(Clone, Debug)]
pub struct CompressedUploadQueue {
    capacity: usize,
    next_generation: u64,
    order: VecDeque<AssetId>,
    entries: HashMap<AssetId, PendingCompressedUpload>,
}

impl Default for CompressedUploadQueue {
    fn default() -> Self {
        Self::new(DEFAULT_UPLOAD_QUEUE_CAPACITY)
    }
}

impl CompressedUploadQueue {
    /// 构造指定容量上限的待传队列。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_generation: 0,
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// 将压缩纹理载荷入队；若同 `AssetId` 已在队列中则就地覆写载荷并递增版本号。
    ///
    /// # Errors
    /// 当该 `AssetId` 尚未在队列中且队列已达容量上限时，返回 [`AssetError::QueueFull`]。
    pub fn enqueue(
        &mut self,
        id: AssetId,
        payload: CompressedTexturePayload,
    ) -> Result<u64, AssetError> {
        if let Some(existing) = self.entries.get_mut(&id) {
            self.next_generation += 1;
            existing.payload = payload;
            existing.generation = self.next_generation;
            return Ok(self.next_generation);
        }

        if self.entries.len() >= self.capacity {
            return Err(AssetError::QueueFull);
        }

        self.next_generation += 1;
        let generation = self.next_generation;
        self.order.push_back(id);
        self.entries.insert(
            id,
            PendingCompressedUpload {
                id,
                payload,
                generation,
            },
        );
        Ok(generation)
    }

    /// 立即取消指定 `AssetId` 的待传任务（例如条目离屏时跳过上传）。
    pub fn cancel(&mut self, id: AssetId) -> bool {
        if self.entries.remove(&id).is_some() {
            self.order.retain(|&queued_id| queued_id != id);
            true
        } else {
            false
        }
    }

    /// 检查指定 `AssetId` 是否正在等待上传。
    #[must_use]
    pub fn contains(&self, id: AssetId) -> bool {
        self.entries.contains_key(&id)
    }

    /// 返回当前队列中待上传条目数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 判断待传队列是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清空全部待传条目。
    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    /// 按 `budget` 的条数与字节数双重封顶从队首 FIFO 排空一个上传批次。
    ///
    /// 超出本帧配额的条目安全留在队列中，顺延至后续帧继续排空。
    pub fn drain_batch(&mut self, budget: UploadPacingBudget) -> CompressedAtlasUploadBatch {
        let mut items = Vec::new();
        let mut total_bytes = 0usize;

        if budget.max_uploads == 0 || budget.max_bytes == 0 {
            return CompressedAtlasUploadBatch { items, total_bytes };
        }

        while items.len() < budget.max_uploads {
            let Some(&front_id) = self.order.front() else {
                break;
            };
            let Some(entry) = self.entries.get(&front_id) else {
                self.order.pop_front();
                continue;
            };
            let entry_bytes = entry.payload.bytes().len();
            if total_bytes.saturating_add(entry_bytes) > budget.max_bytes {
                break;
            }
            self.order.pop_front();
            if let Some(removed) = self.entries.remove(&front_id) {
                total_bytes += entry_bytes;
                items.push(removed);
            }
        }

        CompressedAtlasUploadBatch { items, total_bytes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造指定尺寸的纯色 BC7 Mode 6 测试载荷。
    fn make_solid_bc7_payload(
        width: u32,
        height: u32,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    ) -> CompressedTexturePayload {
        let block = bc7_solid_mode6_block(r, g, b, a);
        let block_count = ((width / BC7_BLOCK_DIM) * (height / BC7_BLOCK_DIM)) as usize;
        let mut bytes = Vec::with_capacity(block_count * BC7_BLOCK_BYTES);
        for _ in 0..block_count {
            bytes.extend_from_slice(&block);
        }
        CompressedTexturePayload::bc7_unorm(width, height, bytes).unwrap()
    }

    #[test]
    fn payload_rejects_zero_unaligned_and_mismatched_byte_lengths() {
        assert_eq!(
            CompressedTexturePayload::bc7_unorm(0, 4, Vec::new()),
            Err(CompressedTextureError::ZeroDimension {
                width: 0,
                height: 4,
            })
        );
        assert_eq!(
            CompressedTexturePayload::bc7_unorm(4, 0, Vec::new()),
            Err(CompressedTextureError::ZeroDimension {
                width: 4,
                height: 0,
            })
        );
        assert_eq!(
            CompressedTexturePayload::bc7_unorm(6, 4, vec![0; 32]),
            Err(CompressedTextureError::UnalignedDimension {
                width: 6,
                height: 4,
                block_dim: 4,
            })
        );
        assert_eq!(
            CompressedTexturePayload::bc7_unorm(4, 5, vec![0; 32]),
            Err(CompressedTextureError::UnalignedDimension {
                width: 4,
                height: 5,
                block_dim: 4,
            })
        );
        assert_eq!(
            CompressedTexturePayload::bc7_unorm(8, 8, vec![0; 60]),
            Err(CompressedTextureError::ByteLengthMismatch {
                expected: 64,
                actual: 60,
                width: 8,
                height: 8,
            })
        );

        let valid = make_solid_bc7_payload(128, 64, 10, 20, 30, 255);
        assert_eq!(valid.format(), CompressedPixelFormat::Bc7Unorm);
        assert_eq!(valid.width(), 128);
        assert_eq!(valid.height(), 64);
        assert_eq!(valid.vram_bytes(), 128 * 64);
        assert_eq!(
            CompressedPixelFormat::Bc7Unorm.expected_byte_len(2048, 2048),
            Some(BC7_ATLAS_PAGE_BYTES as usize)
        );
        assert_eq!(
            CompressedPixelFormat::Bc7Unorm.expected_byte_len(0, 4),
            None
        );
        assert_eq!(
            CompressedPixelFormat::Bc7Unorm.expected_byte_len(5, 4),
            None
        );
    }

    #[test]
    fn bc7_mode6_transparent_black_and_fallback_decode_roundtrip() {
        assert_eq!(
            bc7_solid_mode6_block(0, 0, 0, 0),
            BC7_MODE6_TRANSPARENT_BLACK_BLOCK
        );

        // 验证纯透明黑块与纯白块无损往返。
        let black_payload = make_solid_bc7_payload(4, 4, 0, 0, 0, 0);
        let black_rgba = decode_bc7_to_rgba8_fallback(&black_payload);
        assert_eq!(black_rgba.len(), 4 * 4 * 4);
        assert!(black_rgba.chunks_exact(4).all(|px| px == [0, 0, 0, 0]));

        let white_payload = make_solid_bc7_payload(4, 4, 255, 255, 255, 255);
        let white_rgba = decode_bc7_to_rgba8_fallback(&white_payload);
        assert!(white_rgba
            .chunks_exact(4)
            .all(|px| px == [255, 255, 255, 255]));

        // 验证任意颜色 Mode 6 纯色块误差不超过 1 LSB（共享 p-bit 的理论极限）。
        let target = [200u8, 121, 64, 255];
        let color_payload =
            make_solid_bc7_payload(8, 4, target[0], target[1], target[2], target[3]);
        let decoded = decode_bc7_to_rgba8_fallback(&color_payload);
        assert_eq!(decoded.len(), 8 * 4 * 4);
        for px in decoded.chunks_exact(4) {
            for ch in 0..4 {
                let diff = px[ch].abs_diff(target[ch]);
                assert!(diff <= 1, "channel {ch} diff {diff} exceeded 1 LSB");
            }
        }

        // 验证其他模式（如 Mode 0）与保留无效块（全 0）均安全解出确定性像素且不崩溃。
        let mode0_payload = CompressedTexturePayload::bc7_unorm(
            4,
            4,
            vec![0x01, 0x22, 0x44, 0x66, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let mode0_rgba = decode_bc7_to_rgba8_fallback(&mode0_payload);
        assert_eq!(mode0_rgba.len(), 64);
        assert!(mode0_rgba.chunks_exact(4).all(|px| px[3] == 0xFF));

        let reserved_payload = CompressedTexturePayload::bc7_unorm(4, 4, vec![0u8; 16]).unwrap();
        let reserved_rgba = decode_bc7_to_rgba8_fallback(&reserved_payload);
        assert!(reserved_rgba.chunks_exact(4).all(|px| px == [0, 0, 0, 0]));
    }

    #[test]
    fn route_compressed_texture_splits_atlas_and_external_at_threshold() {
        assert_eq!(
            route_compressed_texture(512, 512, DEFAULT_COMPRESSED_ATLAS_THRESHOLD),
            CompressedTextureRoute::AtlasPage
        );
        assert_eq!(
            route_compressed_texture(256, 512, DEFAULT_COMPRESSED_ATLAS_THRESHOLD),
            CompressedTextureRoute::AtlasPage
        );
        assert_eq!(
            route_compressed_texture(516, 256, DEFAULT_COMPRESSED_ATLAS_THRESHOLD),
            CompressedTextureRoute::ExternalStandalone
        );
        assert_eq!(
            route_compressed_texture(256, 516, DEFAULT_COMPRESSED_ATLAS_THRESHOLD),
            CompressedTextureRoute::ExternalStandalone
        );
        assert_eq!(
            route_compressed_texture(1024, 1024, DEFAULT_COMPRESSED_ATLAS_THRESHOLD),
            CompressedTextureRoute::ExternalStandalone
        );
    }

    #[test]
    fn allocator_packs_arbitrary_aspects_expands_pages_and_coalesces_free_spans() {
        let mut alloc = TieredShelfMultiPageAllocator::new();

        // 验证任意 4px 对齐长宽比（极宽、极窄、方形）正确落入对应档位并输出合法 UV。
        let (wide, created0) = alloc.allocate(512, 32).unwrap();
        assert!(created0);
        assert_eq!(wide.shelf_height, 64);
        assert_eq!(wide.uv_bounds(), [0.0, 0.0, 0.25, 32.0 / 2048.0]);

        let (tall, created1) = alloc.allocate(20, 300).unwrap();
        assert!(!created1);
        assert_eq!(tall.page, wide.page);
        assert_eq!(tall.shelf_height, 512);

        // 释放宽条与高条，验证页立即变空且所有货架的 free_spans 合并为每货架 1 个完整区间。
        assert!(alloc.free(wide));
        assert!(!alloc.free(wide));
        assert!(alloc.free(tall));
        assert!(alloc.is_page_empty(wide.page));
        assert_eq!(alloc.empty_pages(), vec![wide.page]);
        assert_eq!(alloc.page_free_span_count(wide.page), Some(2));
        assert!(alloc.remove_empty_page(wide.page));
        assert_eq!(alloc.active_page_count(), 0);

        // 高密度跨页扩展：2048×2048 单页最多容纳 4×4 = 16 个 512×512 条目，第 17 个必须开辟新页。
        let mut page0_items = Vec::new();
        for i in 0..16 {
            let (entry, created) = alloc.allocate(512, 512).unwrap();
            assert_eq!(created, i == 0);
            page0_items.push(entry);
        }
        assert_eq!(alloc.active_page_count(), 1);
        let page0 = page0_items[0].page;
        assert_eq!(alloc.page_active_count(page0), 16);

        let (entry17, created_page1) = alloc.allocate(512, 512).unwrap();
        assert!(created_page1);
        assert_ne!(entry17.page, page0);
        assert_eq!(alloc.active_page_count(), 2);

        // 释放第 1 页所有条目后回收该空页，不影响第 2 页。
        for item in page0_items {
            assert!(alloc.free(item));
        }
        assert!(alloc.is_page_empty(page0));
        assert!(!alloc.is_page_empty(entry17.page));
        assert!(alloc.remove_empty_page(page0));
        assert!(!alloc.remove_empty_page(entry17.page));
        assert_eq!(alloc.page_ids(), vec![entry17.page]);
    }

    #[test]
    fn allocator_repeated_alloc_free_cycles_produce_zero_fragmentation_or_extra_pages() {
        let mut alloc = TieredShelfMultiPageAllocator::new();
        let (seed, created) = alloc.allocate(256, 128).unwrap();
        assert!(created);
        let page = seed.page;
        assert!(alloc.free(seed));
        assert_eq!(alloc.page_free_span_count(page), Some(1));

        // 连续 2000 轮交替分配/乱序释放，绝不新建第二页且释放后 free_spans 严格合并回 1。
        for _ in 0..2000 {
            let (a, c1) = alloc.allocate(128, 100).unwrap();
            let (b, c2) = alloc.allocate(256, 128).unwrap();
            let (c, c3) = alloc.allocate(384, 96).unwrap();
            let (d, c4) = alloc.allocate(512, 120).unwrap();
            assert!(!c1 && !c2 && !c3 && !c4);

            // 故意以非连续顺序释放以触发左合并、右合并与双侧桥接合并。
            assert!(alloc.free(b));
            assert!(alloc.free(d));
            assert!(alloc.free(a));
            assert!(alloc.free(c));
        }

        assert_eq!(alloc.active_page_count(), 1);
        assert_eq!(alloc.page_active_count(page), 0);
        assert_eq!(alloc.page_free_span_count(page), Some(1));
    }

    #[test]
    fn upload_queue_enforces_count_and_byte_budgets_dedup_cancel_and_capacity() {
        let mut queue = CompressedUploadQueue::new(3);
        let p_small = make_solid_bc7_payload(64, 64, 10, 20, 30, 255); // 4096 bytes
        let p_updated = make_solid_bc7_payload(64, 64, 40, 50, 60, 255); // 4096 bytes
        let p_large = make_solid_bc7_payload(128, 128, 70, 80, 90, 255); // 16384 bytes

        let g1 = queue.enqueue(AssetId(1), p_small.clone()).unwrap();
        let g2 = queue.enqueue(AssetId(2), p_large.clone()).unwrap();
        let g3 = queue.enqueue(AssetId(3), p_small.clone()).unwrap();
        assert!(g1 < g2 && g2 < g3);
        assert_eq!(queue.len(), 3);

        // 队列满时拒收新 AssetId。
        assert_eq!(
            queue.enqueue(AssetId(4), p_small.clone()),
            Err(AssetError::QueueFull)
        );

        // 已在队列中的 AssetId 允许去重覆写且递增版本号、不增加队列长度。
        let g1_overwrite = queue.enqueue(AssetId(1), p_updated.clone()).unwrap();
        assert!(g1_overwrite > g3);
        assert_eq!(queue.len(), 3);

        // 离屏取消 AssetId(2) 后可重新入队 AssetId(4)。
        assert!(queue.cancel(AssetId(2)));
        assert!(!queue.contains(AssetId(2)));
        assert!(!queue.cancel(AssetId(2)));
        queue.enqueue(AssetId(4), p_large.clone()).unwrap();
        assert_eq!(queue.len(), 3);

        // 字节数封顶测试：允许最多 10 条，但字节上限 8192 仅放行前两条 4096B（AssetId 1 与 3）。
        let batch1 = queue.drain_batch(UploadPacingBudget::new(10, 8192));
        assert_eq!(batch1.items.len(), 2);
        assert_eq!(batch1.total_bytes, 8192);
        assert_eq!(batch1.items[0].id, AssetId(1));
        assert_eq!(batch1.items[0].generation, g1_overwrite);
        assert_eq!(batch1.items[0].payload, p_updated);
        assert_eq!(batch1.items[1].id, AssetId(3));
        assert_eq!(queue.len(), 1);

        // 下一帧：若字节预算不足以容纳 16384B 条目则继续安全顺延。
        let batch_blocked = queue.drain_batch(UploadPacingBudget::new(4, 8192));
        assert!(batch_blocked.items.is_empty());
        assert_eq!(batch_blocked.total_bytes, 0);
        assert_eq!(queue.len(), 1);

        // 再下一帧：条数封顶 1 且字节充足时放行剩余条目。
        let batch2 = queue.drain_batch(UploadPacingBudget::new(1, 32768));
        assert_eq!(batch2.items.len(), 1);
        assert_eq!(batch2.items[0].id, AssetId(4));
        assert_eq!(batch2.total_bytes, 16384);
        assert!(queue.is_empty());
    }
}

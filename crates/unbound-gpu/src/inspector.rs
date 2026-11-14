//! 外部显存句柄导入协议、2D 纹理通道检视器参数与 3D 视口自定义渲染钩子（阶段 1 WP1）。
//!
//! 面向两大宿主的专业图形需求：
//! * **下游渲染工具**：需要零拷贝导入外部进程/帧捕获的 `VkImage`（Win32 `HANDLE` /
//!   D3D11/D3D12 共享纹理 / DXGI 句柄），并提供 R/G/B/A 单通道过滤、曝光、Gamma、Mip 层级、
//!   棋盘格背景与伪彩色范围映射；
//! * **下游资产应用（纯 Rust 资产管理器）**：需要同时驻留上百张缩略图纹理（256 槽扩展 Bindless
//!   分配器），并在 UI 矩形内嵌入自定义 3D 网格/材质球预览 Pass。

use unbound_math::{Rect, Vec2};

/// 扩展外部纹理槽位容量（支持 下游资产应用 大规模缩略图与 下游渲染工具 多Pass RT 检视）。
pub const EXTENDED_EXTERNAL_SLOTS: u32 = 256;

/// 跨 API / 跨进程外部显存句柄类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExternalMemoryHandleKind {
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT`
    OpaqueWin32,
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_KMT_BIT`
    OpaqueWin32Kmt,
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_TEXTURE_BIT`
    D3D11Texture,
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_TEXTURE_KMT_BIT`
    D3D11TextureKmt,
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE_BIT`
    D3D12Resource,
    /// `VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_HEAP_BIT`
    D3D12Heap,
}

impl ExternalMemoryHandleKind {
    /// 该句柄类型在 Vulkan 规范下是否通常要求专用分配（`VkMemoryDedicatedAllocateInfo`）。
    #[must_use]
    pub const fn requires_dedicated_allocation(self) -> bool {
        matches!(
            self,
            Self::D3D11Texture | Self::D3D11TextureKmt | Self::D3D12Resource
        )
    }

    /// 对应的 Vulkan 扩展名。
    #[must_use]
    pub const fn required_extension_name(self) -> &'static str {
        "VK_KHR_external_memory_win32"
    }
}

/// 外部共享纹理导入描述符。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExternalImageDesc {
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
    /// Mipmap 层级总数。
    pub mip_levels: u32,
    /// 数组图层数（Cubemap 为 6）。
    pub array_layers: u32,
    /// Vulkan `VkFormat` 原始枚举值（例如 `R8G8B8A8_UNORM = 37`，`R16G16B16A16_SFLOAT = 97`）。
    pub vk_format_raw: i32,
    /// 外部显存句柄类型。
    pub handle_kind: ExternalMemoryHandleKind,
    /// 操作系统原始句柄值（Windows `HANDLE` 强转为 `isize`）。
    pub raw_handle: isize,
    /// 导入分配的字节大小。
    pub allocation_size: u64,
}

impl ExternalImageDesc {
    /// 校验导入描述符的几何与句柄合法性。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.width == 0 || self.height == 0 {
            return Err("external image dimensions must be non-zero");
        }
        if self.mip_levels == 0 || self.array_layers == 0 {
            return Err("external image mip_levels and array_layers must be >= 1");
        }
        if self.raw_handle == 0 || self.raw_handle == -1 {
            return Err("external memory handle cannot be null or INVALID_HANDLE_VALUE");
        }
        if self.allocation_size == 0 {
            return Err("external memory allocation_size must be > 0");
        }
        Ok(())
    }
}

/// 支持最多 256 个槽位的扩展外部纹理分配器（位图实现，O(1) 分配与释放，零堆分配）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtendedExternalSlots {
    words: [u64; 4],
    base_slot: u32,
}

impl ExtendedExternalSlots {
    /// 以指定起始 Bindless 槽位创建 256 槽位分配器。
    #[must_use]
    pub const fn new(base_slot: u32) -> Self {
        Self {
            words: [0; 4],
            base_slot,
        }
    }

    /// 起始槽位偏移。
    #[must_use]
    pub const fn base_slot(&self) -> u32 {
        self.base_slot
    }

    /// 总槽位容量（256）。
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        EXTENDED_EXTERNAL_SLOTS
    }

    /// 当前已分配的槽位数量。
    #[must_use]
    pub const fn used(&self) -> u32 {
        self.words[0].count_ones()
            + self.words[1].count_ones()
            + self.words[2].count_ones()
            + self.words[3].count_ones()
    }

    /// 判断槽位是否属于本分配器管辖范围。
    #[must_use]
    pub const fn owns(&self, slot: u32) -> bool {
        slot >= self.base_slot && slot < self.base_slot + EXTENDED_EXTERNAL_SLOTS
    }

    /// 分配最小的空闲槽位。
    pub fn allocate(&mut self) -> Option<u32> {
        for (word_idx, word) in self.words.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = (!*word).trailing_zeros();
                *word |= 1u64 << bit;
                return Some(self.base_slot + (word_idx as u32) * 64 + bit);
            }
        }
        None
    }

    /// 释放已分配的槽位。
    pub fn release(&mut self, slot: u32) -> bool {
        if !self.owns(slot) {
            return false;
        }
        let rel = slot - self.base_slot;
        let word_idx = (rel / 64) as usize;
        let bit = rel % 64;
        let mask = 1u64 << bit;
        if self.words[word_idx] & mask == 0 {
            return false;
        }
        self.words[word_idx] &= !mask;
        true
    }

    /// 清空全部已分配槽位。
    pub fn clear(&mut self) {
        self.words = [0; 4];
    }
}

/// 2D 纹理检视器的通道掩码模式（服务于 下游渲染工具 RT 检视与 下游资产应用 贴图通道预览）。
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChannelMask {
    /// 标准 RGBA 四通道合成。
    #[default]
    Rgba = 0,
    /// 忽略 Alpha，强制不透明显示 RGB。
    Rgb = 1,
    /// 仅显示 R 通道（灰度广播至 RGB）。
    Red = 2,
    /// 仅显示 G 通道（灰度广播至 RGB）。
    Green = 3,
    /// 仅显示 B 通道（灰度广播至 RGB）。
    Blue = 4,
    /// 仅显示 Alpha 通道（灰度广播至 RGB）。
    Alpha = 5,
    /// 感知亮度灰度显示（Rec.709 线性系数）。
    Luminance = 6,
}

/// 2D 纹理通道检视器参数。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TextureInspectorParams {
    /// 激活的通道过滤掩码。
    pub channel_mask: ChannelMask,
    /// 曝光补偿（EV 档位，实际乘数为 `2^exposure`）。
    pub exposure: f32,
    /// 显示 Gamma（默认 `1.0`；非 1.0 时执行 `pow(c, 1.0 / gamma)`）。
    pub gamma: f32,
    /// 采样 Mipmap 层级。
    pub mip_level: f32,
    /// 采样数组图层 / Cubemap 面索引。
    pub array_layer: u32,
    /// 透明区域棋盘格像素尺寸（`0.0` 表示关闭棋盘格）。
    pub checkerboard_size: f32,
    /// 值域映射下界（用于深度图/HDR 归一化：`c = (c - min) / (max - min)`）。
    pub range_min: f32,
    /// 值域映射上界。
    pub range_max: f32,
}

impl Default for TextureInspectorParams {
    fn default() -> Self {
        Self {
            channel_mask: ChannelMask::Rgba,
            exposure: 0.0,
            gamma: 1.0,
            mip_level: 0.0,
            array_layer: 0,
            checkerboard_size: 8.0,
            range_min: 0.0,
            range_max: 1.0,
        }
    }
}

impl TextureInspectorParams {
    /// 对单个线性 RGBA 纹素执行与 GPU 着色器完全一致的通道检视、值域重映射、曝光与 Gamma 变换。
    #[must_use]
    pub fn evaluate_texel(&self, rgba: [f32; 4]) -> [f32; 4] {
        let span = (self.range_max - self.range_min).max(1e-6);
        let norm = |v: f32| (v - self.range_min) / span;
        let r = norm(rgba[0]);
        let g = norm(rgba[1]);
        let b = norm(rgba[2]);
        let a = rgba[3].clamp(0.0, 1.0);

        let exp_scale = self.exposure.exp2();
        let inv_gamma = 1.0 / self.gamma.max(0.05);
        let tone = |v: f32| {
            let scaled = (v * exp_scale).max(0.0);
            if (inv_gamma - 1.0).abs() > 1e-4 {
                scaled.powf(inv_gamma).clamp(0.0, 1.0)
            } else {
                scaled.clamp(0.0, 1.0)
            }
        };

        match self.channel_mask {
            ChannelMask::Rgba => [tone(r), tone(g), tone(b), a],
            ChannelMask::Rgb => [tone(r), tone(g), tone(b), 1.0],
            ChannelMask::Red => {
                let v = tone(r);
                [v, v, v, 1.0]
            }
            ChannelMask::Green => {
                let v = tone(g);
                [v, v, v, 1.0]
            }
            ChannelMask::Blue => {
                let v = tone(b);
                [v, v, v, 1.0]
            }
            ChannelMask::Alpha => {
                let v = norm(rgba[3]).clamp(0.0, 1.0);
                [v, v, v, 1.0]
            }
            ChannelMask::Luminance => {
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let v = tone(luma);
                [v, v, v, 1.0]
            }
        }
    }
}

/// 3D 视口轨道相机参数（用于 下游资产应用 3D 模型/材质球预览与 下游渲染工具 网格检视）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ViewportCamera3D {
    /// 水平旋转角（弧度）。
    pub orbit_yaw: f32,
    /// 俯仰角（弧度，钳制在 `[-1.5, 1.5]`）。
    pub orbit_pitch: f32,
    /// 相机到目标点的距离。
    pub distance: f32,
    /// 观察目标中心点。
    pub target: [f32; 3],
    /// 垂直视场角（弧度）。
    pub fov_y_rad: f32,
    /// 近裁剪面。
    pub near: f32,
    /// 远裁剪面。
    pub far: f32,
}

impl Default for ViewportCamera3D {
    fn default() -> Self {
        Self {
            orbit_yaw: 0.6,
            orbit_pitch: 0.35,
            distance: 3.5,
            target: [0.0, 0.0, 0.0],
            fov_y_rad: core::f32::consts::FRAC_PI_4,
            near: 0.05,
            far: 1000.0,
        }
    }
}

impl ViewportCamera3D {
    /// 根据鼠标拖拽增量旋转轨道相机。
    pub fn orbit(&mut self, delta: Vec2, sensitivity: f32) {
        self.orbit_yaw += delta.x * sensitivity;
        self.orbit_pitch = (self.orbit_pitch + delta.y * sensitivity).clamp(-1.5, 1.5);
    }

    /// 根据滚轮增量缩放观察距离。
    pub fn dolly(&mut self, zoom_factor: f32) {
        self.distance = (self.distance * zoom_factor).clamp(0.1, 500.0);
    }

    /// 计算相机在世界空间中的眼睛坐标。
    #[must_use]
    pub fn eye_position(&self) -> [f32; 3] {
        let cp = self.orbit_pitch.cos();
        let sp = self.orbit_pitch.sin();
        let cy = self.orbit_yaw.cos();
        let sy = self.orbit_yaw.sin();
        [
            self.target[0] + self.distance * cp * sy,
            self.target[1] + self.distance * sp,
            self.target[2] + self.distance * cp * cy,
        ]
    }
}

/// 嵌入 UI 布局矩形中的自定义 3D 视口渲染 Pass 描述符。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ViewportRenderPassDesc {
    /// 视口唯一标识。
    pub viewport_id: u64,
    /// UI 布局分配的屏幕矩形。
    pub bounds: Rect,
    /// 绑定的外部颜色纹理槽位。
    pub bindless_slot: u32,
    /// 3D 轨道相机参数。
    pub camera: ViewportCamera3D,
    /// 是否显示线框叠加。
    pub wireframe_overlay: bool,
    /// 是否显示世界坐标网格。
    pub show_grid: bool,
}

/// 已导入的外部共享纹理句柄包（持有 `VkImage`、`VkDeviceMemory`、`VkImageView` 与分配的 Bindless 槽位）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImportedExternalTexture {
    /// 分配的 Bindless 纹理槽位（`48..304`）。
    pub bindless_slot: u32,
    /// 导入的 Vulkan 图像句柄。
    pub image: ash::vk::Image,
    /// 导入的外部设备内存句柄。
    pub memory: ash::vk::DeviceMemory,
    /// 供着色器采样的图像视图。
    pub view: ash::vk::ImageView,
    /// 像素尺寸 `(width, height)`。
    pub extent: (u32, u32),
}

/// 跨进程 / 跨 API（D3D11/D3D12/Vulkan）Win32 NT 显存句柄零拷贝导入辅助器。
#[derive(Debug, Default, Clone, Copy)]
pub struct ExternalMemoryImporter;

impl ExternalMemoryImporter {
    /// 将 [`ExternalMemoryHandleKind`] 映射为 Vulkan `VkExternalMemoryHandleTypeFlags`。
    #[must_use]
    pub const fn vk_handle_type(
        kind: ExternalMemoryHandleKind,
    ) -> ash::vk::ExternalMemoryHandleTypeFlags {
        match kind {
            ExternalMemoryHandleKind::OpaqueWin32 => {
                ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
            }
            ExternalMemoryHandleKind::OpaqueWin32Kmt => {
                ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32_KMT
            }
            ExternalMemoryHandleKind::D3D11Texture => {
                ash::vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE
            }
            ExternalMemoryHandleKind::D3D11TextureKmt => {
                ash::vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE_KMT
            }
            ExternalMemoryHandleKind::D3D12Resource => {
                ash::vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE
            }
            ExternalMemoryHandleKind::D3D12Heap => {
                ash::vk::ExternalMemoryHandleTypeFlags::D3D12_HEAP
            }
        }
    }

    /// 校验外部图像描述符并返回对应的 Vulkan 格式与句柄标志。
    pub fn prepare_import(
        desc: &ExternalImageDesc,
    ) -> Result<(ash::vk::Format, ash::vk::ExternalMemoryHandleTypeFlags), &'static str> {
        desc.validate()?;
        let format = ash::vk::Format::from_raw(desc.vk_format_raw);
        let handle_type = Self::vk_handle_type(desc.handle_kind);
        Ok((format, handle_type))
    }
}

/// 嵌入 `unbound-ui` 帧管线的自定义 2D/3D 视口渲染钩子（供 下游渲染工具 零拷贝注入）。
pub trait CustomViewportPass: Send + Sync + core::fmt::Debug {
    /// 视口唯一标识。
    fn viewport_id(&self) -> u64;

    /// 视口在 UI 窗口中的裁剪矩形（物理像素）。
    fn viewport_bounds(&self) -> Rect;

    /// 在 UI 主 Pass 合成阶段录制自定义命令（如 2D 纹理通道检视器全屏 Quad 或 3D 网格/线框预览）。
    fn record_pass(
        &mut self,
        device: &ash::Device,
        cmd: ash::vk::CommandBuffer,
        target_extent: (u32, u32),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_external_slots_allocate_all_256_and_reclaim() {
        let mut slots = ExtendedExternalSlots::new(64);
        assert_eq!(slots.capacity(), 256);
        assert_eq!(slots.used(), 0);

        let mut allocated = Vec::new();
        while let Some(s) = slots.allocate() {
            assert!(slots.owns(s));
            allocated.push(s);
        }
        assert_eq!(allocated.len(), 256);
        assert_eq!(slots.used(), 256);
        assert_eq!(slots.allocate(), None);

        assert!(slots.release(allocated[137]));
        assert!(!slots.release(allocated[137]));
        assert_eq!(slots.used(), 255);
        assert_eq!(slots.allocate(), Some(allocated[137]));
    }

    #[test]
    fn texture_inspector_channel_mask_and_exposure_work() {
        let mut params = TextureInspectorParams {
            channel_mask: ChannelMask::Red,
            exposure: 1.0, // 2^1 = 2x
            ..Default::default()
        };
        let out = params.evaluate_texel([0.25, 0.8, 0.1, 0.5]);
        assert!((out[0] - 0.5).abs() < 1e-5);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        assert_eq!(out[3], 1.0);

        params.channel_mask = ChannelMask::Alpha;
        let alpha_out = params.evaluate_texel([0.25, 0.8, 0.1, 0.4]);
        assert!((alpha_out[0] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn external_image_desc_validates_parameters() {
        let valid = ExternalImageDesc {
            width: 1920,
            height: 1080,
            mip_levels: 1,
            array_layers: 1,
            vk_format_raw: 37,
            handle_kind: ExternalMemoryHandleKind::D3D11Texture,
            raw_handle: 0x1234,
            allocation_size: 1920 * 1080 * 4,
        };
        assert!(valid.validate().is_ok());
        assert!(valid.handle_kind.requires_dedicated_allocation());

        let invalid = ExternalImageDesc {
            raw_handle: 0,
            ..valid
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn viewport_camera_orbit_clamps_pitch_and_computes_eye() {
        let mut cam = ViewportCamera3D::default();
        cam.orbit(Vec2::new(1.0, 100.0), 0.1);
        assert!(cam.orbit_pitch <= 1.5);
        let eye = cam.eye_position();
        assert!(eye.iter().all(|v| v.is_finite()));
    }
}

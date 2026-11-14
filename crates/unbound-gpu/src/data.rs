//! CPU ⇄ GPU 内存协议（参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 此处所有结构体均为 `#[repr(C)]`、`Pod`，并按 CPU 缓存行大小对齐：
//!
//! * 单个图元绝不跨越两条缓存行（1 个实例 == 1 条 64B 缓存行）；
//! * 着色器只需单条 `shl 6` 指令即可索引 `primitives[i]`；
//! * `FramePushConstants` 精简紧凑，远低于所有实现保证的 128B Push Constant 下限。
//!
//! 字段偏移由单元测试锁定：若无意中调整字段顺序，会导致难以察觉的渲染损坏。

use bytemuck::{Pod, Zeroable};

use unbound_math::{f16_to_f32, f32_to_f16};

/// 单个 [`GpuPrimitiveInstance`] 的字节数——亦为全引擎调优所依据的缓存行大小。
pub const PRIMITIVE_STRIDE: usize = 64;

/// 图元类型，打包存储在 `prim_type_and_flags` 的低 8 位。
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PrimType {
    /// 带可选边框与解析阴影的圆角矩形。
    #[default]
    RoundedRect = 0,
    /// 纹理四边形（图像图集页，Bindless 图层索引存放在边框槽位）。
    Image = 1,
    /// 采样 `Atlas_R8` 的字形四边形（支持亚像素相位）。
    Glyph = 2,
    /// 来自 `Atlas_RGBA8` 的彩色字形（Emoji / MTSDF）。
    GlyphColor = 3,
    /// 以紧凑有向包围盒（OBB）渲染的二次 Bézier 曲线段（参见 `docs/architecture/gpu-renderer.md`）。
    Curve = 4,
    /// 空心圆角矩形，用于焦点环与选区轮廓。
    Stroke = 5,
    /// 纯阴影实例（无填充），用于独立浮层。
    Shadow = 6,
    /// 实心分隔线。
    Line = 7,
}

impl PrimType {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Image,
            2 => Self::Glyph,
            3 => Self::GlyphColor,
            4 => Self::Curve,
            5 => Self::Stroke,
            6 => Self::Shadow,
            7 => Self::Line,
            _ => Self::RoundedRect,
        }
    }

    /// 当图元的片元着色器需要读取字形图集并应用伽马/字干粗细补偿时返回 true。
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(self, Self::Glyph | Self::GlyphColor)
    }
}

/// 混合模式，打包存储在 `prim_type_and_flags` 的第 8..16 位。
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// 标准 Source-Over 混合（GPU 侧按非预乘输入处理）。
    #[default]
    Normal = 0,
    /// 加法混合（Additive）——用于光标拖尾与诊断覆盖层。
    Additive = 1,
    /// 正片叠底（Multiply）——用于画布正片叠底图层。
    Multiply = 2,
    /// 替换模式（Replace）——完全不混合（不透明核心预 Pass 仅写深度，
    /// 关闭 MSAA 时的首个不透明层也可使用替换模式）。
    Replace = 3,
}

impl BlendMode {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Additive,
            2 => Self::Multiply,
            3 => Self::Replace,
            _ => Self::Normal,
        }
    }
}

/// 单个组件内部的子元素顺序（排序键的第 7..0 位，参见 `docs/architecture/gpu-renderer.md`）。
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubElement {
    #[default]
    Shadow = 0,
    Background = 1,
    Content = 2,
    InnerShadow = 3,
    Border = 4,
    FocusRing = 5,
}

/// Portal / Pass 图层，位于 64 位排序键的最高 8 位。
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PassLayer {
    #[default]
    Canvas = 0,
    Ui = 1,
    /// 下拉菜单、工具提示、右键菜单。
    Overlay = 2,
    /// 模态对话框。
    Modal = 3,
    /// 调试/诊断覆盖层始终置顶。
    Debug = 255,
}

impl PassLayer {
    /// Portal 从第 2 层开始，最高可达 254。
    #[must_use]
    pub const fn portal(depth: u8) -> Self {
        // `depth` 钳制在 254，使 255 始终保留给诊断覆盖层。
        if depth > 254 {
            Self::Debug
        } else {
            Self::from_u8(2u8.saturating_add(depth))
        }
    }

    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Canvas,
            1 => Self::Ui,
            255 => Self::Debug,
            _ => Self::Overlay,
        }
    }
}

/// 帧 Push Constant 头部——每帧唯一的 CPU→GPU 载荷（参见 `docs/architecture/gpu-renderer.md`）。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FramePushConstants {
    /// `GpuPrimitiveInstance` 数组的物理设备地址（BDA）。
    pub primitives_bda: u64,
    /// `GpuTransform` 数组的物理设备地址（BDA）。
    pub transforms_bda: u64,
    /// `GpuClipNode` 数组的物理设备地址（BDA）。
    pub clips_bda: u64,
    /// RGBA16F 主题颜色查找表（LUT）的物理设备地址。
    pub theme_lut_bda: u64,
    /// 曲线/渐变载荷数组的物理设备地址。
    pub payloads_bda: u64,
    /// 包含相机原点的世界 chunk ID。
    pub camera_chunk: [i32; 2],
    /// 相机在该 chunk 内的局部位置，范围 `[0, 4096)`。
    pub camera_local: [f32; 2],
    /// 物理像素视口尺寸。
    pub viewport_size: [f32; 2],
    /// 画布缩放因子（1.0 == 100%）。
    pub camera_zoom: f32,
    /// 操作系统 DPI × 应用缩放比。
    pub dpi_scale: f32,
}

impl FramePushConstants {
    /// Push Constant 范围大小契约。
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;
}

/// 单个 64 字节图元实例，字段排布严格遵循 `docs/architecture/gpu-renderer.md` 规范。
#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPrimitiveInstance {
    /// 世界 chunk ID。静态 UI 图元恒为 `[0, 0]`。
    pub chunk_id: [i32; 2],
    /// 在 chunk 内的局部坐标，范围 `[0.0, 4096.0)`。
    pub local_pos: [f32; 2],
    /// 逻辑像素尺寸。
    pub size: [f32; 2],
    /// 四角圆角半径：左上、右上、右下、左下（f16）。
    pub corner_radii: [u16; 4],
    /// 边框宽度（f16）。
    pub border_width: u16,
    /// 解析阴影模糊半径（f16，0 表示无阴影）。
    pub shadow_blur: u16,
    /// `transforms_bda` 中的索引；第 31 位标记是否处于动画中。
    pub transform_idx: u32,
    /// `clips_bda` 中的索引。
    pub clip_idx: u32,
    /// 主题颜色令牌或打包后的图集 UV 索引。
    pub fill_token_or_uv: u32,
    /// 边框颜色令牌或 Bindless 纹理/图集图层索引。
    pub border_token_or_tex: u32,
    /// `[8 位类型 | 8 位混合模式 | 16 位标志]`。
    pub prim_type_and_flags: u32,
    /// `[16 位归一化深度 | 16 位 UNORM 不透明度]`。
    pub z_and_opacity: u32,
    /// 曲线与渐变在 `payloads_bda` 中的索引。
    pub extra_payload_idx: u32,
}

/// `transform_idx` 的第 31 位：图元正在动画中，必须跳过整数像素对齐（参见 `docs/architecture/gpu-renderer.md`）。
pub const TRANSFORM_ANIMATED_BIT: u32 = 1 << 31;
/// 剥离动画标志位以提取真实变换表索引的掩码。
pub const TRANSFORM_INDEX_MASK: u32 = !TRANSFORM_ANIMATED_BIT;

impl GpuPrimitiveInstance {
    /// 打包 `prim_type_and_flags` 字。
    #[must_use]
    pub const fn pack_type(prim: PrimType, blend: BlendMode, flags: u16) -> u32 {
        (prim as u32) | ((blend as u32) << 8) | ((flags as u32) << 16)
    }

    /// 解包为 `[type, blend, flags]`。
    #[must_use]
    pub const fn unpack_type(word: u32) -> (PrimType, BlendMode, u16) {
        (
            PrimType::from_u8((word & 0xff) as u8),
            BlendMode::from_u8(((word >> 8) & 0xff) as u8),
            (word >> 16) as u16,
        )
    }

    #[inline]
    #[must_use]
    pub const fn prim_type(&self) -> PrimType {
        PrimType::from_u8((self.prim_type_and_flags & 0xff) as u8)
    }

    #[inline]
    #[must_use]
    pub const fn blend_mode(&self) -> BlendMode {
        BlendMode::from_u8(((self.prim_type_and_flags >> 8) & 0xff) as u8)
    }

    /// `[16 位深度 | 16 位不透明度]`。
    #[must_use]
    pub const fn pack_z_opacity(depth: u16, opacity: f32) -> u32 {
        let o = (opacity.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        ((depth as u32) << 16) | (o & 0xffff)
    }

    #[inline]
    #[must_use]
    pub const fn depth_u16(&self) -> u16 {
        (self.z_and_opacity >> 16) as u16
    }

    /// 从 UNORM16 解码的 `[0, 1]` 不透明度。
    #[inline]
    #[must_use]
    pub fn opacity(&self) -> f32 {
        (self.z_and_opacity & 0xffff) as f32 / 65535.0
    }

    /// `[0, 1]` 反向 Z 深度（1.0 == 最近）。
    #[inline]
    #[must_use]
    pub fn depth(&self) -> f32 {
        self.depth_u16() as f32 / 65535.0
    }

    /// 当图元参与独立动画通道时返回 true。
    #[inline]
    #[must_use]
    pub const fn is_animated(&self) -> bool {
        self.transform_idx & TRANSFORM_ANIMATED_BIT != 0
    }

    /// 剥离动画标志位后的真实变换表索引。
    #[inline]
    #[must_use]
    pub const fn transform_index(&self) -> u32 {
        self.transform_idx & TRANSFORM_INDEX_MASK
    }

    #[inline]
    #[must_use]
    pub fn radii(&self) -> [f32; 4] {
        [
            f16_to_f32(self.corner_radii[0]),
            f16_to_f32(self.corner_radii[1]),
            f16_to_f32(self.corner_radii[2]),
            f16_to_f32(self.corner_radii[3]),
        ]
    }

    #[inline]
    pub fn set_radii(&mut self, radii: [f32; 4]) {
        self.corner_radii = [
            f32_to_f16(radii[0]),
            f32_to_f16(radii[1]),
            f32_to_f16(radii[2]),
            f32_to_f16(radii[3]),
        ];
    }

    #[inline]
    #[must_use]
    pub fn border_width(&self) -> f32 {
        f16_to_f32(self.border_width)
    }

    #[inline]
    pub fn set_border_width(&mut self, w: f32) {
        self.border_width = f32_to_f16(w);
    }

    #[inline]
    #[must_use]
    pub fn shadow_blur(&self) -> f32 {
        f16_to_f32(self.shadow_blur)
    }

    #[inline]
    pub fn set_shadow_blur(&mut self, b: f32) {
        self.shadow_blur = f32_to_f16(b);
    }

    /// chunk 局部空间下的轴对齐包围盒，已按阴影模糊半径外扩，确保剔除计算着色器保留纯阴影几何体。
    #[inline]
    #[must_use]
    pub fn bounds_with_shadow(&self) -> ([f32; 2], [f32; 2]) {
        let pad = self.shadow_blur() * 3.0;
        (
            [self.local_pos[0] - pad, self.local_pos[1] - pad],
            [
                self.local_pos[0] + self.size[0] + pad,
                self.local_pos[1] + self.size[1] + pad,
            ],
        )
    }
}

/// 64B 裁剪节点：一个折叠后的硬裁剪 AABB 外加**两个**圆角父级槽位。
///
/// 字段顺序与 `common.glsl` 中的 `ClipNode` **完全一致**。宽成员须置于前方，
/// 因为 std430 要求 `vec4` 按 16 字节对齐：若交错排布 `vec4` 与 `uvec2`，
/// 着色器侧步长会变成 80 字节而本 `repr(C)` 结构体仍为 64 字节，
/// 导致索引大于 0 的所有裁剪节点均读取错位。
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuClipNode {
    /// 折叠后的硬裁剪矩形 `(min_x, min_y, max_x, max_y)`。
    pub aabb_clip: [f32; 4],
    /// 第一个圆角父级 `(center_x, center_y, half_w, half_h)`。
    pub rounded_clip_0: [f32; 4],
    /// 第二个圆角父级。
    pub rounded_clip_1: [f32; 4],
    /// 第一个圆角父级的四个圆角半径（f16）。
    pub radii_0: [u16; 4],
    /// 第二个圆角父级的四个圆角半径（f16）。
    pub radii_1: [u16; 4],
}

/// 表示“无裁剪”的哨兵裁剪索引。
pub const CLIP_NONE: u32 = u32::MAX;

impl GpuClipNode {
    /// 仅按 `aabb` 裁剪、无圆角父级的裁剪节点。
    #[must_use]
    pub fn from_aabb(aabb: [f32; 4]) -> Self {
        Self {
            aabb_clip: aabb,
            // 零半径且范围极大的退化圆角裁剪在着色器 SDF 测试中为空操作。
            rounded_clip_0: [0.0, 0.0, 1.0e9, 1.0e9],
            radii_0: [0; 4],
            rounded_clip_1: [0.0, 0.0, 1.0e9, 1.0e9],
            radii_1: [0; 4],
        }
    }

    /// 当折叠后的 AABB 为空（全部被剔除）时返回 true。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aabb_clip[0] >= self.aabb_clip[2] || self.aabb_clip[1] >= self.aabb_clip[3]
    }
}

/// 独立动画通道的 32B 变换记录。
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuTransform {
    /// `m00, m10, m01, m11`（列主序 2×2 矩阵）。
    pub linear: [f32; 4],
    /// 逻辑像素平移量。
    pub translation: [f32; 2],
    /// 当变换为单位矩阵且可完全跳过时为 `1.0`。
    pub scale_hint: f32,
    /// 位打包标志（参见 [`TransformFlags`]）。
    pub flags: u32,
}

impl Default for GpuTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl GpuTransform {
    pub const IDENTITY: Self = Self {
        linear: [1.0, 0.0, 0.0, 1.0],
        translation: [0.0, 0.0],
        scale_hint: 1.0,
        flags: 0,
    };

    #[must_use]
    pub fn from_affine(m: unbound_math::Affine2) -> Self {
        Self {
            linear: [m.m00, m.m10, m.m01, m.m11],
            translation: [m.tx, m.ty],
            scale_hint: m.max_scale(),
            flags: 0,
        }
    }

    #[must_use]
    pub fn to_affine(self) -> unbound_math::Affine2 {
        unbound_math::Affine2 {
            m00: self.linear[0],
            m10: self.linear[1],
            m01: self.linear[2],
            m11: self.linear[3],
            tx: self.translation[0],
            ty: self.translation[1],
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.linear[0] == 1.0
            && self.linear[1] == 0.0
            && self.linear[2] == 0.0
            && self.linear[3] == 1.0
            && self.translation[0] == 0.0
            && self.translation[1] == 0.0
    }
}

/// 存储在 [`GpuTransform::flags`] 中的标志位。
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct TransformFlags(pub u32);

impl TransformFlags {
    pub const EMPTY: Self = Self(0);
    /// 图元正在动画中：跳过像素对齐。
    pub const ANIMATED: Self = Self(1 << 0);
    /// 缩放为均匀缩放，着色器可走低开销标量路径。
    pub const UNIFORM_SCALE: Self = Self(1 << 1);
    /// 变换在 X 轴存在镜像翻转。
    pub const MIRROR_X: Self = Self(1 << 2);
    /// 完全跳过变换（顶点着色器中的单位矩阵快速路径）。
    pub const SKIP: Self = Self(1 << 3);

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl core::ops::BitOr for TransformFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// 曲线与渐变的 48B 载荷头。
///
/// 对于 [`PrimType::Curve`]：`p0`/`p1`/`p2` 为图元局部空间中的二次控制多边形顶点。
/// 对于渐变：`p0`/`p1` 为端点，`p2.x` 携带色标信息。
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPayload {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
    /// 逻辑像素描边宽度。
    pub line_width: f32,
    /// 渐变色标偏移 / 曲线标志。
    pub extra: u32,
    /// 打包标志位。
    pub flags: u32,
    /// 显式尾部填充：3 个字使结构体严格保持 48B 且无隐式填充（`Pod` 类型不得含隐式填充）。
    pub _reserved: [u32; 3],
}

impl GpuPayload {
    #[must_use]
    pub fn curve(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], line_width: f32) -> Self {
        Self {
            p0,
            p1,
            p2,
            line_width,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        }
    }

    /// 从 `from` 到 `to` 的线性渐变，两者均为图元包围盒内的归一化局部坐标（`0..1`）。
    ///
    /// 色标为主题令牌：`from` 处为 `token0`，`to` 处为 `token1`。`exponent` 调节插值
    /// 偏置（`1.0` 为线性）——有助于在线性 sRGB 交换链上实现感知均匀的渐变过渡。
    #[must_use]
    pub fn gradient_linear(
        from: [f32; 2],
        to: [f32; 2],
        token0: u32,
        token1: u32,
        exponent: f32,
    ) -> Self {
        Self::gradient(from, to, token0, token1, exponent)
    }

    /// 以 `center` 为圆心、各轴半径为 `radius` 的径向渐变。
    #[must_use]
    pub fn gradient_radial(
        center: [f32; 2],
        radius: [f32; 2],
        token0: u32,
        token1: u32,
        exponent: f32,
    ) -> Self {
        Self::gradient(center, radius, token0, token1, exponent)
    }

    fn gradient(p0: [f32; 2], p1: [f32; 2], token0: u32, token1: u32, exponent: f32) -> Self {
        // `p2.x` 携带两个色标令牌：低 16 位为起点，高 16 位为终点。
        // `p2.y` 为插值指数。着色器通过 `floatBitsToUint` 按相同方式解包。
        let packed = (token0 & 0xFFFF) | ((token1 & 0xFFFF) << 16);
        Self {
            p0,
            p1,
            p2: [f32::from_bits(packed), exponent],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        }
    }

    /// 提取由 [`Self::gradient_linear`] / [`Self::gradient_radial`] 写入的两个渐变色标令牌。
    #[must_use]
    pub fn gradient_tokens(&self) -> (u32, u32) {
        let packed = self.p2[0].to_bits();
        (packed & 0xFFFF, packed >> 16)
    }

    /// 返回渐变插值指数。
    #[must_use]
    pub fn gradient_exponent(&self) -> f32 {
        self.p2[1]
    }
}

/// 间接绘制命令；内存布局与 `VkDrawIndirectCommand` 兼容。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DrawIndirectCommand {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

impl DrawIndirectCommand {
    #[must_use]
    pub const fn new(instance_count: u32, first_instance: u32) -> Self {
        Self {
            vertex_count: 4,
            instance_count,
            first_vertex: 0,
            first_instance,
        }
    }
}

/// 存储在 `prim_type_and_flags` 高 16 位中的标志。
///
/// 第 0..2 位为**子元素顺序**，同时构成 64 位排序键的低字节，
/// 确保节点阴影始终在背景之后、边框始终在最前。
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PrimFlags(pub u16);

impl PrimFlags {
    pub const EMPTY: Self = Self(0);
    /// 子元素顺序掩码（`SubElement`）。
    pub const SUB_ELEMENT_MASK: Self = Self(0b111);
    /// 字形位于深色表面上：启用字干加粗补偿。
    pub const DARK_BG: Self = Self(1 << 3);
    /// 将 RGBA 图集作为多通道距离场（MTSDF）采样。
    pub const MSDF: Self = Self(1 << 4);
    /// 文本行被截断；着色器可柔化尾部边缘。
    pub const ELLIPSIS: Self = Self(1 << 5);
    /// 使用水平亚像素相位光栅化（`Atlas_R8` 相位层）。
    pub const SUBPIXEL: Self = Self(1 << 6);
    /// 该图元位于不透明核心顶层（供预 Pass 使用）。
    pub const OPAQUE_CORE: Self = Self(1 << 7);
    /// 使用 `extra_payload_idx` 中存储的线性渐变填充。
    ///
    /// 第 5、6 位已被 `ELLIPSIS`/`SUBPIXEL` 占用，故渐变标记从第 8 位开始。
    /// `common.glsl` 与此值严格镜像。
    pub const GRADIENT_LINEAR: Self = Self(1 << 8);
    /// 使用径向渐变填充（圆心 `p0`，半径向量 `p1`）。
    pub const GRADIENT_RADIAL: Self = Self(1 << 9);
    /// 该图元为 `InnerShadow` 子元素：`shadow_blur` 为内阴影模糊半径，填充令牌为阴影颜色。
    pub const INNER_SHADOW: Self = Self(1 << 10);

    /// 任意渐变标记掩码。
    pub const GRADIENT_ANY: Self = Self(Self::GRADIENT_LINEAR.0 | Self::GRADIENT_RADIAL.0);

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// 根据子元素类型与附加标志构建标志字。
    #[must_use]
    pub const fn with_sub(sub: SubElement, extra: Self) -> Self {
        Self((sub as u16) | extra.0)
    }

    /// 提取编码在低 3 位中的子元素类型。
    #[must_use]
    pub const fn sub_element(self) -> u8 {
        (self.0 & Self::SUB_ELEMENT_MASK.0) as u8
    }
}

impl core::ops::BitOr for PrimFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// 所有管线共用的**唯一** 128 字节 Push Constant 块。
///
/// 前 72 字节为 `FramePushConstants`；尾部携带计算阶段的暂存地址与计数器。
/// 图形管线推送整块并将尾部清零，从而使单个 `VkPipelineLayout` 即可服务于
/// uber 着色器与全部六个计算阶段（且严格位于 128 字节保证下限之内）。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PushConstants {
    pub frame: FramePushConstants,
    /// 当前计算调度的元素数量。
    pub prim_count: u32,
    /// 基数排序位偏移（0、8、16、… 56）。
    pub digit: u32,
    /// 当前排序 Pass 中 256 元素块的数量。
    pub block_count: u32,
    /// 通用暂存字（各阶段含义不同）。
    pub aux: u32,
    pub scratch_bda: u64,
    pub scratch_bda2: u64,
    pub scratch_bda3: u64,
    pub scratch_bda4: u64,
    pub scratch_bda5: u64,
}

impl PushConstants {
    /// Push Constant 范围字节大小。
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;

    /// 仅构建包含帧前缀的 Push Constant 块（供大多数调用方使用）。
    #[must_use]
    pub fn from_frame(frame: FramePushConstants) -> Self {
        Self {
            frame,
            ..Default::default()
        }
    }
}

/// Dual-Kawase 模糊 Pass 的 Push Constant。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BlurPush {
    /// UV 空间源区域：`(x, y, width, height)`。
    pub uv_rect: [f32; 4],
    /// 目标像素尺寸。
    pub dst_size: [f32; 2],
    /// 逻辑像素模糊半径。
    pub radius: f32,
    /// 当前降采样/升采样迭代轮次。
    pub iteration: u32,
}

impl BlurPush {
    /// 模糊管线布局的 Push Constant 大小契约。
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;
}

/// 背景虚化合成 Pass 的 Push Constant。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BackdropCompositePush {
    /// 物理像素矩形：`(x, y, width, height)`。
    pub rect: [f32; 4],
    /// 圆角半径：左上、右上、右下、左下。
    pub radii: [f32; 4],
}

impl BackdropCompositePush {
    /// 背景合成管线布局的 Push Constant 大小契约。
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;
}

/// 单个增量散布补丁，内存布局与 `scatter.comp` 兼容。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuScatterPatch {
    pub dst_offset: u32,
    pub src_offset: u32,
    pub len: u32,
    pub _pad: u32,
}

/// 剔除 Pass 生成的 `(key, primitive_index)` 二元组。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct SortedEntry {
    pub key: u64,
    pub index: u32,
    pub _pad: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn push_constants_fit_one_block() {
        // 声明的 9 个字段合计 72B（5 个 BDA 指针 + 3 个 vec2 + 2 个标量），
        // 所有字段均必需，且 72B 仍远低于 128B 保证下限。
        assert_eq!(size_of::<FramePushConstants>(), 72);
        assert_eq!(align_of::<FramePushConstants>(), 8);
        assert_eq!(FramePushConstants::SIZE, 72);
    }

    #[test]
    fn push_constant_offsets_are_pinned() {
        assert_eq!(offset_of!(FramePushConstants, primitives_bda), 0);
        assert_eq!(offset_of!(FramePushConstants, transforms_bda), 8);
        assert_eq!(offset_of!(FramePushConstants, clips_bda), 16);
        assert_eq!(offset_of!(FramePushConstants, theme_lut_bda), 24);
        assert_eq!(offset_of!(FramePushConstants, payloads_bda), 32);
        assert_eq!(offset_of!(FramePushConstants, camera_chunk), 40);
        assert_eq!(offset_of!(FramePushConstants, camera_local), 48);
        assert_eq!(offset_of!(FramePushConstants, viewport_size), 56);
    }

    #[test]
    fn primitive_is_one_cache_line() {
        assert_eq!(size_of::<GpuPrimitiveInstance>(), PRIMITIVE_STRIDE);
        assert_eq!(align_of::<GpuPrimitiveInstance>(), 64);
        // 每个字段均落入同一条 64B 缓存行内。
        assert_eq!(offset_of!(GpuPrimitiveInstance, chunk_id), 0);
        assert_eq!(offset_of!(GpuPrimitiveInstance, local_pos), 8);
        assert_eq!(offset_of!(GpuPrimitiveInstance, size), 16);
        assert_eq!(offset_of!(GpuPrimitiveInstance, corner_radii), 24);
        assert_eq!(offset_of!(GpuPrimitiveInstance, border_width), 32);
        assert_eq!(offset_of!(GpuPrimitiveInstance, shadow_blur), 34);
        assert_eq!(offset_of!(GpuPrimitiveInstance, transform_idx), 36);
        assert_eq!(offset_of!(GpuPrimitiveInstance, clip_idx), 40);
        assert_eq!(offset_of!(GpuPrimitiveInstance, fill_token_or_uv), 44);
        assert_eq!(offset_of!(GpuPrimitiveInstance, border_token_or_tex), 48);
        assert_eq!(offset_of!(GpuPrimitiveInstance, prim_type_and_flags), 52);
        assert_eq!(offset_of!(GpuPrimitiveInstance, z_and_opacity), 56);
        assert_eq!(offset_of!(GpuPrimitiveInstance, extra_payload_idx), 60);
    }

    #[test]
    fn arrays_of_primitives_are_tightly_packed() {
        let mut v = vec![GpuPrimitiveInstance::default(); 16];
        for (i, p) in v.iter_mut().enumerate() {
            p.local_pos = [i as f32, 0.0];
        }
        let bytes: &[u8] = bytemuck::cast_slice(&v);
        assert_eq!(bytes.len(), 16 * PRIMITIVE_STRIDE);
        // 步长校验：第 3 个实例的 local_pos.x 位于 3*64 + 8。
        let off = 3 * PRIMITIVE_STRIDE + 8;
        assert_eq!(
            f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            3.0
        );
    }

    #[test]
    fn type_and_blend_packing_round_trips() {
        let w = GpuPrimitiveInstance::pack_type(PrimType::Curve, BlendMode::Additive, 0xbeef);
        assert_eq!(
            GpuPrimitiveInstance::unpack_type(w),
            (PrimType::Curve, BlendMode::Additive, 0xbeef)
        );
    }

    #[test]
    fn opacity_and_depth_pack_without_crosstalk() {
        let p = GpuPrimitiveInstance {
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(0xabcd, 0.5),
            ..Default::default()
        };
        assert_eq!(p.depth_u16(), 0xabcd);
        assert!((p.opacity() - 0.5).abs() < 2e-5);
        assert!((p.depth() - 0.670_58).abs() < 1e-3);
    }

    #[test]
    fn opacity_clamps() {
        assert_eq!(
            GpuPrimitiveInstance::pack_z_opacity(0, 4.0) & 0xffff,
            0xffff
        );
        assert_eq!(GpuPrimitiveInstance::pack_z_opacity(0, -1.0) & 0xffff, 0);
    }

    #[test]
    fn animation_bit_does_not_alias_the_index() {
        let mut p = GpuPrimitiveInstance {
            transform_idx: 1234,
            ..Default::default()
        };
        assert!(!p.is_animated());
        assert_eq!(p.transform_index(), 1234);
        p.transform_idx |= TRANSFORM_ANIMATED_BIT;
        assert!(p.is_animated());
        assert_eq!(p.transform_index(), 1234);
    }

    #[test]
    fn radii_and_border_round_trip_through_f16() {
        let mut p = GpuPrimitiveInstance::default();
        p.set_radii([4.0, 8.0, 12.0, 16.0]);
        assert_eq!(p.radii(), [4.0, 8.0, 12.0, 16.0]);
        p.set_border_width(1.0);
        assert_eq!(p.border_width(), 1.0);
        p.set_shadow_blur(24.0);
        assert_eq!(p.shadow_blur(), 24.0);
    }

    #[test]
    fn clip_node_is_16_byte_aligned_and_64_bytes() {
        assert_eq!(size_of::<GpuClipNode>(), 64);
        assert_eq!(align_of::<GpuClipNode>(), 16);
        let c = GpuClipNode::from_aabb([0.0, 0.0, 10.0, 10.0]);
        assert!(!c.is_empty());
        assert!(GpuClipNode::from_aabb([10.0, 0.0, 0.0, 10.0]).is_empty());
    }

    /// `ClipNode` 布局必须与 `common.glsl` 中的 `struct ClipNode` 严格一致。
    ///
    /// 两侧通过物理设备地址绑定，无反射检查机制。std430 强制 `vec4` 16 字节对齐，
    /// 故宽成员须前置，避免着色器步长膨胀至 80 字节而 Rust 侧仍为 64 字节。
    #[test]
    fn clip_node_layout_matches_the_shader_struct() {
        assert_eq!(offset_of!(GpuClipNode, aabb_clip), 0);
        assert_eq!(offset_of!(GpuClipNode, rounded_clip_0), 16);
        assert_eq!(offset_of!(GpuClipNode, rounded_clip_1), 32);
        assert_eq!(offset_of!(GpuClipNode, radii_0), 48);
        assert_eq!(offset_of!(GpuClipNode, radii_1), 56);
        // 圆角半径数组等价于两个打包 f16 的 `u32`，与着色器中的 `uvec2` 成员完全对应。
        assert_eq!(size_of::<[u16; 4]>(), 8);
    }

    /// 着色器 `ClipNode` 的成员顺序必须与 Rust 结构体完全镜像。
    ///
    /// 两侧通过物理设备地址绑定，任一侧重排字段都会静默破坏索引 > 0 的裁剪节点。
    /// 本测试解析 GLSL 源码并比对成员顺序与 Rust 字段顺序。
    #[test]
    fn shader_clip_node_member_order_matches_the_rust_struct() {
        let glsl = include_str!("../shaders/common.glsl");
        let start = glsl
            .find("struct ClipNode {")
            .expect("common.glsl must declare `struct ClipNode`");
        let body = &glsl[start..];
        let body = &body[body.find('{').expect("`struct ClipNode` must open") + 1..];
        let end = body.find('}').expect("`struct ClipNode` must be closed");
        let body = &body[..end];

        // 按声明顺序收集成员类型。
        let members: Vec<&str> = body
            .lines()
            .filter_map(|line| {
                let line = line.split("//").next().unwrap_or("").trim();
                let mut parts = line.split_whitespace();
                let ty = parts.next()?;
                // 忽略成员名称，仅类型决定内存布局。
                parts.next()?;
                Some(ty)
            })
            .collect();
        assert_eq!(
            members,
            vec!["vec4", "vec4", "vec4", "uvec2", "uvec2"],
            "the shader's ClipNode member order changed; mirror it in `GpuClipNode`"
        );

        // std430 规则：`vec4` 为 16 字节对齐，`uvec2` 为 8 字节对齐。
        let mut offset = 0usize;
        let mut offsets = Vec::new();
        for ty in &members {
            let (align, size) = match *ty {
                "vec4" => (16usize, 16usize),
                "uvec2" => (8, 8),
                other => panic!("unhandled GLSL type `{other}` in ClipNode"),
            };
            offset = (offset + align - 1) & !(align - 1);
            offsets.push(offset);
            offset += size;
        }
        assert_eq!(offsets, vec![0, 16, 32, 48, 56]);
        assert_eq!(offset, 64, "the shader's ClipNode must stay 64 bytes");
    }

    #[test]
    fn transform_is_32_bytes_and_round_trips() {
        assert_eq!(size_of::<GpuTransform>(), 32);
        assert_eq!(align_of::<GpuTransform>(), 16);
        let m = unbound_math::Affine2::around(
            unbound_math::Vec2::new(4.0, 4.0),
            unbound_math::Vec2::new(2.0, 2.0),
            0.5,
            unbound_math::Vec2::new(1.0, 0.0),
        );
        let t = GpuTransform::from_affine(m);
        assert_eq!(t.to_affine(), m);
        assert!(GpuTransform::default().is_identity());
    }

    #[test]
    fn payload_is_48_bytes() {
        assert_eq!(size_of::<GpuPayload>(), 48);
        assert_eq!(align_of::<GpuPayload>(), 16);
    }

    #[test]
    fn draw_command_matches_vulkan_layout() {
        assert_eq!(size_of::<DrawIndirectCommand>(), 16);
        let c = DrawIndirectCommand::new(7, 42);
        assert_eq!(c.vertex_count, 4);
        assert_eq!(c.instance_count, 7);
        assert_eq!(c.first_instance, 42);
    }

    #[test]
    fn pass_layer_and_prim_type_normalise_unknowns() {
        assert_eq!(PrimType::from_u8(99), PrimType::RoundedRect);
        assert_eq!(BlendMode::from_u8(9), BlendMode::Normal);
        assert_eq!(PassLayer::from_u8(77), PassLayer::Overlay);
        assert_eq!(PassLayer::portal(0), PassLayer::from_u8(2));
        assert_eq!(PassLayer::portal(253), PassLayer::from_u8(255));
        assert_eq!(PassLayer::portal(254), PassLayer::Debug);
        assert!(PrimType::Glyph.is_text());
        assert!(!PrimType::Image.is_text());
    }

    #[test]
    fn push_constants_are_exactly_128_bytes() {
        assert_eq!(size_of::<PushConstants>(), 128);
        assert_eq!(PushConstants::SIZE, 128);
        assert_eq!(offset_of!(PushConstants, frame), 0);
        assert_eq!(offset_of!(PushConstants, prim_count), 72);
        assert_eq!(offset_of!(PushConstants, digit), 76);
        assert_eq!(offset_of!(PushConstants, block_count), 80);
        assert_eq!(offset_of!(PushConstants, aux), 84);
        assert_eq!(offset_of!(PushConstants, scratch_bda), 88);
        assert_eq!(offset_of!(PushConstants, scratch_bda5), 120);
    }

    #[test]
    fn prim_flags_pack_sub_element_and_markers() {
        let f = PrimFlags::with_sub(SubElement::Border, PrimFlags::DARK_BG);
        assert_eq!(f.sub_element(), SubElement::Border as u8);
        assert!(f.contains(PrimFlags::DARK_BG));
        assert!(!f.contains(PrimFlags::MSDF));
    }

    #[test]
    fn gradient_flags_do_not_collide_with_text_markers() {
        for flag in [
            PrimFlags::GRADIENT_LINEAR,
            PrimFlags::GRADIENT_RADIAL,
            PrimFlags::INNER_SHADOW,
        ] {
            assert_eq!(flag.bits().count_ones(), 1);
            assert_eq!(flag.bits() & PrimFlags::SUB_ELEMENT_MASK.bits(), 0);
            assert_eq!(flag.bits() & PrimFlags::ELLIPSIS.bits(), 0);
            assert_eq!(flag.bits() & PrimFlags::SUBPIXEL.bits(), 0);
        }
        assert_eq!(
            PrimFlags::GRADIENT_ANY.bits(),
            PrimFlags::GRADIENT_LINEAR.bits() | PrimFlags::GRADIENT_RADIAL.bits()
        );
        assert!(PrimFlags::GRADIENT_ANY.contains(PrimFlags::GRADIENT_RADIAL));
    }

    #[test]
    fn gradient_payload_round_trips_its_stop_tokens() {
        let p = GpuPayload::gradient_linear([0.0, 0.0], [1.0, 1.0], 7, 42, 1.6);
        assert_eq!(p.gradient_tokens(), (7, 42));
        assert!((p.gradient_exponent() - 1.6).abs() < 1e-6);
        assert_eq!(p.p0, [0.0, 0.0]);
        assert_eq!(p.p1, [1.0, 1.0]);
        assert_eq!(size_of::<GpuPayload>(), 48);

        let r = GpuPayload::gradient_radial([0.5, 0.5], [0.5, 0.25], 3, 4, 1.0);
        assert_eq!(r.gradient_tokens(), (3, 4));
        assert_eq!(r.p1, [0.5, 0.25]);
    }

    #[test]
    fn blur_push_constants_are_32_bytes() {
        assert_eq!(size_of::<BlurPush>(), 32);
        assert_eq!(align_of::<BlurPush>(), 4);
        assert_eq!(BlurPush::SIZE, 32);
    }

    #[test]
    fn backdrop_composite_push_constants_are_32_bytes() {
        assert_eq!(size_of::<BackdropCompositePush>(), 32);
        assert_eq!(BackdropCompositePush::SIZE, 32);
    }

    #[test]
    fn scatter_patch_matches_the_shader_layout() {
        assert_eq!(size_of::<GpuScatterPatch>(), 16);
        let p = GpuScatterPatch {
            dst_offset: 1,
            src_offset: 2,
            len: 64,
            _pad: 0,
        };
        assert_eq!(bytemuck::bytes_of(&p).len(), 16);
    }

    #[test]
    fn sorted_entry_is_16_bytes() {
        assert_eq!(size_of::<SortedEntry>(), 16);
    }

    #[test]
    fn bounds_include_shadow_bleed() {
        let mut p = GpuPrimitiveInstance {
            local_pos: [10.0, 20.0],
            size: [100.0, 50.0],
            ..Default::default()
        };
        p.set_shadow_blur(4.0);
        let (min, max) = p.bounds_with_shadow();
        assert_eq!(min, [-2.0, 8.0]);
        assert_eq!(max, [122.0, 82.0]);
    }
}

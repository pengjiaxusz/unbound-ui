//! 单图元程序化 SDF 网格：次/主两级解析抗锯齿网格线。
//!
//! 网格线由片元着色器在单个 `PrimType::ProceduralGrid` 图元与单个 48B [`GpuPayload`] 内解析求值，
//! 图元数量恒为 `O(1)`，不随视口尺寸或缩放级别膨胀。

use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::instance;
use crate::WidgetOutput;

/// 程序化 SDF 网格描述符。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProceduralGridDesc {
    /// 网格绘制包围盒。
    pub rect: Rect,
    /// 网格原点相对包围盒左上角的局部偏移（逻辑像素）。
    pub origin_offset: Vec2,
    /// 次网格线间距（逻辑像素；`<= 0.0` 表示禁用次网格）。
    pub minor_spacing: f32,
    /// 主网格线间距（逻辑像素；`<= 0.0` 表示禁用主网格）。
    pub major_spacing: f32,
    /// 次网格线宽（逻辑像素）。
    pub minor_line_width: f32,
    /// 主网格线宽（逻辑像素）。
    pub major_line_width: f32,
    /// 次网格线 sRGB RGBA8 颜色。
    pub minor_rgba: [u8; 4],
    /// 主网格线 sRGB RGBA8 颜色。
    pub major_rgba: [u8; 4],
    /// 四角独立圆角半径：左上、右上、右下、左下。
    pub radius: [f32; 4],
    /// 整体不透明度（范围 `0.0..=1.0`）。
    pub opacity: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl Default for ProceduralGridDesc {
    fn default() -> Self {
        Self::new(Rect::ZERO)
    }
}

impl ProceduralGridDesc {
    /// 以默认网格间距与配色创建程序化网格描述符。
    #[must_use]
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            origin_offset: Vec2::ZERO,
            minor_spacing: 16.0,
            major_spacing: 64.0,
            minor_line_width: 1.0,
            major_line_width: 1.0,
            minor_rgba: [128, 128, 128, 48],
            major_rgba: [128, 128, 128, 112],
            radius: [0.0; 4],
            opacity: 1.0,
            z: 100,
        }
    }

    /// 设置网格原点相对包围盒左上角的局部偏移。
    #[must_use]
    pub const fn with_origin_offset(mut self, offset: Vec2) -> Self {
        self.origin_offset = offset;
        self
    }

    /// 设置网格原点局部偏移（无前缀别名）。
    #[must_use]
    pub const fn origin_offset(self, offset: Vec2) -> Self {
        self.with_origin_offset(offset)
    }

    /// 设置次网格线间距（逻辑像素）。
    #[must_use]
    pub const fn with_minor_spacing(mut self, spacing: f32) -> Self {
        self.minor_spacing = spacing;
        self
    }

    /// 设置次网格线间距（无前缀别名）。
    #[must_use]
    pub const fn minor_spacing(self, spacing: f32) -> Self {
        self.with_minor_spacing(spacing)
    }

    /// 设置主网格线间距（逻辑像素）。
    #[must_use]
    pub const fn with_major_spacing(mut self, spacing: f32) -> Self {
        self.major_spacing = spacing;
        self
    }

    /// 设置主网格线间距（无前缀别名）。
    #[must_use]
    pub const fn major_spacing(self, spacing: f32) -> Self {
        self.with_major_spacing(spacing)
    }

    /// 设置次网格线宽（逻辑像素）。
    #[must_use]
    pub const fn with_minor_line_width(mut self, width: f32) -> Self {
        self.minor_line_width = width;
        self
    }

    /// 设置次网格线宽（无前缀别名）。
    #[must_use]
    pub const fn minor_line_width(self, width: f32) -> Self {
        self.with_minor_line_width(width)
    }

    /// 设置主网格线宽（逻辑像素）。
    #[must_use]
    pub const fn with_major_line_width(mut self, width: f32) -> Self {
        self.major_line_width = width;
        self
    }

    /// 设置主网格线宽（无前缀别名）。
    #[must_use]
    pub const fn major_line_width(self, width: f32) -> Self {
        self.with_major_line_width(width)
    }

    /// 设置次网格线 sRGB RGBA8 颜色。
    #[must_use]
    pub const fn with_minor_rgba(mut self, rgba: [u8; 4]) -> Self {
        self.minor_rgba = rgba;
        self
    }

    /// 设置次网格线颜色（无前缀别名）。
    #[must_use]
    pub const fn minor_rgba(self, rgba: [u8; 4]) -> Self {
        self.with_minor_rgba(rgba)
    }

    /// 设置主网格线 sRGB RGBA8 颜色。
    #[must_use]
    pub const fn with_major_rgba(mut self, rgba: [u8; 4]) -> Self {
        self.major_rgba = rgba;
        self
    }

    /// 设置主网格线颜色（无前缀别名）。
    #[must_use]
    pub const fn major_rgba(self, rgba: [u8; 4]) -> Self {
        self.with_major_rgba(rgba)
    }

    /// 设置四角统一圆角半径。
    #[must_use]
    pub const fn with_radius(mut self, radius: f32) -> Self {
        self.radius = [radius; 4];
        self
    }

    /// 设置四角独立圆角半径。
    #[must_use]
    pub const fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radius = radii;
        self
    }

    /// 设置整体不透明度。
    #[must_use]
    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// 设置整体不透明度（无前缀别名）。
    #[must_use]
    pub const fn opacity(self, opacity: f32) -> Self {
        self.with_opacity(opacity)
    }

    /// 设置绘制顺序键。
    #[must_use]
    pub const fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 设置绘制顺序键（无前缀别名）。
    #[must_use]
    pub const fn z(self, z: u16) -> Self {
        self.with_z(z)
    }
}

/// 将程序化 SDF 网格发射为单个 `PrimType::ProceduralGrid` 图元与单个 [`GpuPayload`]。
#[must_use]
pub fn emit_procedural_grid(desc: &ProceduralGridDesc) -> WidgetOutput {
    let mut prim = instance(
        desc.rect,
        desc.radius,
        Token::NONE,
        Token::NONE,
        0.0,
        0.0,
        PrimType::ProceduralGrid,
        SubElement::Background,
        desc.z,
    );
    prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(desc.z, desc.opacity.clamp(0.0, 1.0));
    prim.extra_payload_idx = 0;

    let payload = GpuPayload::procedural_grid(
        [desc.origin_offset.x, desc.origin_offset.y],
        desc.minor_spacing,
        desc.major_spacing,
        desc.minor_line_width,
        desc.major_line_width,
        desc.minor_rgba,
        desc.major_rgba,
    );

    WidgetOutput {
        primitives: vec![prim],
        payloads: vec![payload],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_gpu::data::pack_rgba8;

    #[test]
    fn emit_procedural_grid_produces_one_primitive_and_one_payload() {
        let desc = ProceduralGridDesc::new(Rect::new(10.0, 20.0, 400.0, 300.0))
            .with_origin_offset(Vec2::new(6.0, -4.0))
            .with_minor_spacing(20.0)
            .with_major_spacing(100.0)
            .with_minor_line_width(1.0)
            .with_major_line_width(2.0)
            .with_minor_rgba([10, 20, 30, 40])
            .with_major_rgba([50, 60, 70, 200])
            .with_opacity(0.75)
            .with_z(250);

        let out = emit_procedural_grid(&desc);
        assert_eq!(out.primitives.len(), 1);
        assert_eq!(out.payloads.len(), 1);

        let prim = &out.primitives[0];
        assert_eq!(prim.prim_type(), PrimType::ProceduralGrid);
        assert_eq!(prim.local_pos, [10.0, 20.0]);
        assert_eq!(prim.size, [400.0, 300.0]);
        assert_eq!(prim.depth_u16(), 250);
        assert!((prim.opacity() - 0.75).abs() < 1e-3);
        assert_eq!(prim.extra_payload_idx, 0);
        assert!(WidgetOutput::primitive_uses_payload(prim));

        let pl = &out.payloads[0];
        assert_eq!(pl.p0, [6.0, -4.0]);
        assert_eq!(pl.p1, [20.0, 100.0]);
        assert_eq!(pl.p2, [1.0, 2.0]);
        assert_eq!(pl.extra, pack_rgba8([10, 20, 30, 40]));
        assert_eq!(pl.flags, pack_rgba8([50, 60, 70, 200]));
    }
}

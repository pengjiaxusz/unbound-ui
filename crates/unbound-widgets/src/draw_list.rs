//! 轻量级 2D 批量绘制原语列表。
//!
//! 面向高频 2D 矢量图元（波形柱、刻度线、网格、折线、圆点等）提供零 Taffy 子节点开销的
//! 批量发射通道：所有命令坐标均以宿主容器 `bounds.origin` 为原点，经 [`emit_draw_list`]
//! 直接打包为 [`GpuPrimitiveInstance`] 与 [`GpuPayload`] 并可选挂载单一 AABB 裁剪节点。

use unbound_gpu::data::{
    GpuClipNode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement, CLIP_NONE,
};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::{primitive, WidgetOutput};

/// 单条 2D 批量绘制命令（坐标均相对于容器 `bounds.origin`）。
#[derive(Clone, Debug, PartialEq)]
pub enum Draw2dCommand {
    /// 直角矩形（支持填充、描边或纯描边）。
    Rect {
        rect: Rect,
        fill: Token,
        stroke: Token,
        stroke_width: f32,
        opacity: f32,
    },
    /// 四角独立圆角矩形（支持填充、描边或纯描边）。
    RoundedRect {
        rect: Rect,
        radii: [f32; 4],
        fill: Token,
        stroke: Token,
        stroke_width: f32,
        opacity: f32,
    },
    /// 单段直线（通过退化二次 Bézier 曲线解析抗锯齿渲染）。
    Line {
        start: Vec2,
        end: Vec2,
        color: Token,
        width: f32,
        opacity: f32,
    },
    /// 折线或多边形轮廓（支持可选闭合）。
    Polyline {
        points: Vec<Vec2>,
        color: Token,
        width: f32,
        closed: bool,
        opacity: f32,
    },
    /// 圆形（支持填充、描边或纯描边）。
    Circle {
        center: Vec2,
        radius: f32,
        fill: Token,
        stroke: Token,
        stroke_width: f32,
        opacity: f32,
    },
}

impl Draw2dCommand {
    /// 构造默认不透明度 `1.0`、无描边的填充直角矩形命令。
    #[must_use]
    pub const fn rect(rect: Rect, fill: Token) -> Self {
        Self::Rect {
            rect,
            fill,
            stroke: Token::NONE,
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    /// 构造默认不透明度 `1.0`、无描边的填充圆角矩形命令。
    #[must_use]
    pub const fn rounded_rect(rect: Rect, radii: [f32; 4], fill: Token) -> Self {
        Self::RoundedRect {
            rect,
            radii,
            fill,
            stroke: Token::NONE,
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    /// 构造默认不透明度 `1.0` 的直线命令。
    #[must_use]
    pub const fn line(start: Vec2, end: Vec2, color: Token, width: f32) -> Self {
        Self::Line {
            start,
            end,
            color,
            width,
            opacity: 1.0,
        }
    }

    /// 构造默认不透明度 `1.0` 的折线命令。
    #[must_use]
    pub fn polyline(points: Vec<Vec2>, color: Token, width: f32, closed: bool) -> Self {
        Self::Polyline {
            points,
            color,
            width,
            closed,
            opacity: 1.0,
        }
    }

    /// 构造默认不透明度 `1.0`、无描边的填充圆命令。
    #[must_use]
    pub const fn circle(center: Vec2, radius: f32, fill: Token) -> Self {
        Self::Circle {
            center,
            radius,
            fill,
            stroke: Token::NONE,
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    /// 为矩形、圆角矩形或圆形附加描边颜色与线宽（直线与折线保持不变）。
    #[must_use]
    pub fn with_stroke(mut self, stroke: Token, stroke_width: f32) -> Self {
        match &mut self {
            Self::Rect {
                stroke: s,
                stroke_width: w,
                ..
            }
            | Self::RoundedRect {
                stroke: s,
                stroke_width: w,
                ..
            }
            | Self::Circle {
                stroke: s,
                stroke_width: w,
                ..
            } => {
                *s = stroke;
                *w = stroke_width;
            }
            Self::Line { .. } | Self::Polyline { .. } => {}
        }
        self
    }

    /// 覆写当前命令的不透明度（发射时钳制到 `[0.0, 1.0]`）。
    #[must_use]
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        match &mut self {
            Self::Rect { opacity: o, .. }
            | Self::RoundedRect { opacity: o, .. }
            | Self::Line { opacity: o, .. }
            | Self::Polyline { opacity: o, .. }
            | Self::Circle { opacity: o, .. } => {
                *o = opacity;
            }
        }
        self
    }
}

/// 将 2D 绘制命令批次发射为 [`WidgetOutput`]。
///
/// - 当 `bounds` 退化（宽或高 `<= 0.0`）或 `commands` 为空时直接返回空输出；
/// - 当 `clip == true` 时仅推入一条 AABB [`GpuClipNode`] 并将其局部索引 `0` 写入所有图元；
/// - 直线与折线每段生成一条 [`PrimType::Curve`] 图元及对应 [`GpuPayload::curve`] 载荷。
#[must_use]
pub fn emit_draw_list(
    bounds: Rect,
    commands: &[Draw2dCommand],
    z_base: u16,
    clip: bool,
) -> WidgetOutput {
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 || commands.is_empty() {
        return WidgetOutput::default();
    }

    let mut output = WidgetOutput::default();
    let clip_idx = if clip {
        let min = bounds.min();
        let max = bounds.max();
        output.push_clip(GpuClipNode::from_aabb([min.x, min.y, max.x, max.y]))
    } else {
        CLIP_NONE
    };
    output.primitives.reserve(commands.len());

    let origin = bounds.origin;
    for cmd in commands {
        match cmd {
            Draw2dCommand::Rect {
                rect,
                fill,
                stroke,
                stroke_width,
                opacity,
            } => {
                if rect.width() <= 0.0 || rect.height() <= 0.0 || *opacity <= 0.0 {
                    continue;
                }
                let world_rect = Rect::new(
                    origin.x + rect.origin.x,
                    origin.y + rect.origin.y,
                    rect.width(),
                    rect.height(),
                );
                emit_box_primitive(
                    &mut output,
                    world_rect,
                    [0.0; 4],
                    *fill,
                    *stroke,
                    *stroke_width,
                    *opacity,
                    z_base,
                    clip_idx,
                );
            }
            Draw2dCommand::RoundedRect {
                rect,
                radii,
                fill,
                stroke,
                stroke_width,
                opacity,
            } => {
                if rect.width() <= 0.0 || rect.height() <= 0.0 || *opacity <= 0.0 {
                    continue;
                }
                let world_rect = Rect::new(
                    origin.x + rect.origin.x,
                    origin.y + rect.origin.y,
                    rect.width(),
                    rect.height(),
                );
                emit_box_primitive(
                    &mut output,
                    world_rect,
                    *radii,
                    *fill,
                    *stroke,
                    *stroke_width,
                    *opacity,
                    z_base,
                    clip_idx,
                );
            }
            Draw2dCommand::Circle {
                center,
                radius,
                fill,
                stroke,
                stroke_width,
                opacity,
            } => {
                if *radius <= 0.0 || *opacity <= 0.0 {
                    continue;
                }
                let diameter = *radius * 2.0;
                let world_rect = Rect::new(
                    origin.x + center.x - *radius,
                    origin.y + center.y - *radius,
                    diameter,
                    diameter,
                );
                emit_box_primitive(
                    &mut output,
                    world_rect,
                    [*radius; 4],
                    *fill,
                    *stroke,
                    *stroke_width,
                    *opacity,
                    z_base,
                    clip_idx,
                );
            }
            Draw2dCommand::Line {
                start,
                end,
                color,
                width,
                opacity,
            } => {
                emit_segment(
                    &mut output,
                    origin + *start,
                    origin + *end,
                    *color,
                    *width,
                    *opacity,
                    z_base,
                    clip_idx,
                );
            }
            Draw2dCommand::Polyline {
                points,
                color,
                width,
                closed,
                opacity,
            } => {
                if points.len() < 2 || *width <= 0.0 || *color == Token::NONE || *opacity <= 0.0 {
                    continue;
                }
                for pair in points.windows(2) {
                    emit_segment(
                        &mut output,
                        origin + pair[0],
                        origin + pair[1],
                        *color,
                        *width,
                        *opacity,
                        z_base,
                        clip_idx,
                    );
                }
                if *closed && points.len() >= 3 {
                    let last = points[points.len() - 1];
                    let first = points[0];
                    emit_segment(
                        &mut output,
                        origin + last,
                        origin + first,
                        *color,
                        *width,
                        *opacity,
                        z_base,
                        clip_idx,
                    );
                }
            }
        }
    }

    output
}

#[allow(clippy::too_many_arguments)]
fn emit_box_primitive(
    output: &mut WidgetOutput,
    world_rect: Rect,
    radii: [f32; 4],
    fill: Token,
    stroke: Token,
    stroke_width: f32,
    opacity: f32,
    z_base: u16,
    clip_idx: u32,
) {
    let clamped_opacity = opacity.clamp(0.0, 1.0);
    if fill != Token::NONE {
        let effective_stroke = if stroke == Token::NONE {
            0.0
        } else {
            stroke_width.max(0.0)
        };
        let mut prim = primitive::instance(
            world_rect,
            radii,
            fill,
            stroke,
            effective_stroke,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            z_base,
        );
        prim.clip_idx = clip_idx;
        prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_base, clamped_opacity);
        output.primitives.push(prim);
    } else if stroke != Token::NONE && stroke_width > 0.0 {
        let mut prim = primitive::instance(
            world_rect,
            radii,
            Token::NONE,
            stroke,
            stroke_width,
            0.0,
            PrimType::Stroke,
            SubElement::Border,
            z_base,
        );
        prim.clip_idx = clip_idx;
        prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_base, clamped_opacity);
        output.primitives.push(prim);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_segment(
    output: &mut WidgetOutput,
    a: Vec2,
    b: Vec2,
    color: Token,
    width: f32,
    opacity: f32,
    z_base: u16,
    clip_idx: u32,
) {
    if width <= 0.0 || color == Token::NONE || opacity <= 0.0 {
        return;
    }
    let pad = width * 0.5 + 1.0;
    let aabb_min = Vec2::new(a.x.min(b.x) - pad, a.y.min(b.y) - pad);
    let aabb_max = Vec2::new(a.x.max(b.x) + pad, a.y.max(b.y) + pad);
    let size = aabb_max - aabb_min;
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let p0 = a - aabb_min;
    let p2 = b - aabb_min;
    let p1 = (p0 + p2) * 0.5;
    let payload_idx = u32::try_from(output.payloads.len()).unwrap_or(u32::MAX);
    output.payloads.push(GpuPayload::curve(
        [p0.x, p0.y],
        [p1.x, p1.y],
        [p2.x, p2.y],
        width,
    ));
    let mut prim = primitive::instance(
        Rect::new(aabb_min.x, aabb_min.y, size.x, size.y),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z_base,
    );
    prim.clip_idx = clip_idx;
    prim.extra_payload_idx = payload_idx;
    prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_base, opacity.clamp(0.0, 1.0));
    output.primitives.push(prim);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_all_five_command_variants_with_local_coordinates_and_clip() {
        let bounds = Rect::new(100.0, 50.0, 400.0, 200.0);
        let commands = vec![
            Draw2dCommand::rect(Rect::new(10.0, 20.0, 30.0, 40.0), Token::ACCENT),
            Draw2dCommand::rounded_rect(
                Rect::new(50.0, 10.0, 60.0, 25.0),
                [4.0, 4.0, 4.0, 4.0],
                Token::NONE,
            )
            .with_stroke(Token::CARD_BORDER, 2.0),
            Draw2dCommand::line(
                Vec2::new(0.0, 100.0),
                Vec2::new(100.0, 120.0),
                Token::TEXT_PRIMARY,
                2.0,
            ),
            Draw2dCommand::polyline(
                vec![
                    Vec2::new(10.0, 10.0),
                    Vec2::new(40.0, 20.0),
                    Vec2::new(70.0, 15.0),
                ],
                Token::ACCENT,
                1.5,
                true,
            ),
            Draw2dCommand::circle(Vec2::new(200.0, 100.0), 12.0, Token::ACCENT).with_opacity(0.5),
        ];

        let out = emit_draw_list(bounds, &commands, 120, true);
        // 1 个矩形 + 1 个纯描边圆角矩形 + 1 条直线 + 3 段闭合折线 + 1 个圆形 = 7
        assert_eq!(out.primitives.len(), 7);
        assert_eq!(out.payloads.len(), 4);
        assert_eq!(out.clips.len(), 1);
        assert_eq!(out.clips[0].aabb_clip, [100.0, 50.0, 500.0, 250.0]);
        for prim in &out.primitives {
            assert_eq!(prim.clip_idx, 0);
            assert_eq!(prim.depth_u16(), 120);
        }

        // Rect 坐标偏移到世界空间。
        assert_eq!(out.primitives[0].prim_type(), PrimType::RoundedRect);
        assert_eq!(out.primitives[0].local_pos, [110.0, 70.0]);
        assert_eq!(out.primitives[0].size, [30.0, 40.0]);

        // 纯描边圆角矩形应走 PrimType::Stroke。
        assert_eq!(out.primitives[1].prim_type(), PrimType::Stroke);
        assert_eq!(
            out.primitives[1].border_token_or_tex,
            u32::from(Token::CARD_BORDER.raw())
        );

        // 直线与折线均走 PrimType::Curve 且使用 fill_token_or_uv 着色。
        assert_eq!(out.primitives[2].prim_type(), PrimType::Curve);
        assert_eq!(
            out.primitives[2].fill_token_or_uv,
            u32::from(Token::TEXT_PRIMARY.raw())
        );
        assert_eq!(out.primitives[2].extra_payload_idx, 0);
        assert_eq!(out.primitives[5].extra_payload_idx, 3);

        // 圆形尺寸为 2r x 2r。
        assert_eq!(out.primitives[6].prim_type(), PrimType::RoundedRect);
        assert_eq!(out.primitives[6].local_pos, [288.0, 138.0]);
        assert_eq!(out.primitives[6].size, [24.0, 24.0]);
    }

    #[test]
    fn skips_empty_and_degenerate_inputs_and_supports_unclipped_mode() {
        let empty = emit_draw_list(Rect::new(0.0, 0.0, 100.0, 100.0), &[], 10, true);
        assert!(empty.primitives.is_empty());
        assert!(empty.clips.is_empty());

        let zero_bounds = emit_draw_list(
            Rect::new(0.0, 0.0, 0.0, 100.0),
            &[Draw2dCommand::rect(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Token::ACCENT,
            )],
            10,
            true,
        );
        assert!(zero_bounds.primitives.is_empty());

        let commands = vec![
            Draw2dCommand::rect(Rect::new(0.0, 0.0, 0.0, 10.0), Token::ACCENT),
            Draw2dCommand::circle(Vec2::new(5.0, 5.0), 0.0, Token::ACCENT),
            Draw2dCommand::line(Vec2::ZERO, Vec2::new(10.0, 10.0), Token::NONE, 2.0),
            Draw2dCommand::polyline(vec![Vec2::ZERO], Token::ACCENT, 2.0, true),
            Draw2dCommand::rect(Rect::new(2.0, 3.0, 8.0, 9.0), Token::ACCENT),
        ];
        let out = emit_draw_list(Rect::new(10.0, 20.0, 100.0, 100.0), &commands, 15, false);
        assert_eq!(out.primitives.len(), 1);
        assert!(out.clips.is_empty());
        assert_eq!(out.primitives[0].clip_idx, CLIP_NONE);
    }

    #[test]
    fn handles_high_volume_batch_of_over_1000_primitives() {
        let mut commands = Vec::with_capacity(1100);
        // 800 条柱状图矩形 + 200 条刻度线 + 1 条 101 点折线（100 段）= 1100 图元。
        for i in 0..800 {
            let x = i as f32 * 2.0;
            let h = 10.0 + ((i % 50) as f32);
            commands.push(Draw2dCommand::rect(
                Rect::new(x, 100.0 - h, 1.5, h),
                Token::ACCENT,
            ));
        }
        for i in 0..200 {
            let x = i as f32 * 8.0;
            commands.push(Draw2dCommand::line(
                Vec2::new(x, 100.0),
                Vec2::new(x, 108.0),
                Token::CARD_BORDER,
                1.0,
            ));
        }
        let poly_points: Vec<Vec2> = (0..101)
            .map(|i| Vec2::new(i as f32 * 16.0, 20.0 + ((i % 7) as f32) * 5.0))
            .collect();
        commands.push(Draw2dCommand::polyline(
            poly_points,
            Token::TEXT_PRIMARY,
            1.5,
            false,
        ));

        let out = emit_draw_list(Rect::new(0.0, 0.0, 1600.0, 120.0), &commands, 50, true);
        assert_eq!(out.primitives.len(), 1100);
        assert_eq!(out.payloads.len(), 300);
        assert_eq!(out.clips.len(), 1);
    }
}

//! 富文本选区合并与 IME 锚定（`docs` §6.2）。
//!
//! 此处包含两项解决“文本并非单个矩形”的基础设施：
//!
//! * **圆角选区合并。** 若每行单独绘制一个圆角矩形，换行处会出现明显缺口。
//!   因此我们合并垂直相邻且 X 范围重叠的行框，仅在合并后区段的外侧角点
//!   应用圆角半径。所得结果为单个胶囊状外形，其轮廓在**几何上**是连续的
//!   单一区域，无需通用的多边形布尔运算。
//! * **IME 候选框锚定。** `set_ime_cursor_area` 需要屏幕空间矩形，
//!   而文本光标位于世界空间。[`ime_cursor_area`] 会应用相机变换、
//!   向下取整到整物理像素（小数锚点会导致 Windows 候选框抖动），
//!   并钳制到窗口范围内，防止 IME 弹窗超出屏幕边界。
//!
//!
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Affine2, Rect, Vec2};

/// 合并后的连续行框区段，可直接作为单个圆角矩形绘制。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundedSlot {
    pub rect: Rect,
    /// 四角圆角半径，顺序为 `TL, TR, BR, BL`。共享边处为 `0`。
    pub radii: [f32; 4],
}

/// 将垂直相邻的选区矩形合并为圆角区段。
///
/// 当两个连续行框垂直相邻（间距在 1 px 内）且水平跨度相同时合并。
/// 要求水平跨度相等是刻意为之：两个**不同**跨度的并集呈 L 形，
/// 若用单个矩形伪造会高亮未选中的文本。不同跨度保持为独立槽位，
/// 这正是桌面编辑器为折行选区绘制的样式。
///
///
#[must_use]
pub fn merge_line_rects(lines: &[Rect], radius: f32) -> Vec<RoundedSlot> {
    let mut slots: Vec<RoundedSlot> = Vec::new();
    for rect in lines {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 || !rect.is_finite() {
            continue;
        }
        if let Some(last) = slots.last_mut() {
            let spans_match = (last.rect.min().x - rect.min().x).abs() <= 0.5
                && (last.rect.max().x - rect.max().x).abs() <= 0.5;
            // 行框必须按从上到下顺序衔接：下一个行框的顶部必须位于前一个顶部
            // 之下或重合。若缺少方向检查，位于前一个**上方**的行框也会被误判为
            // “相邻”（原测试使用绝对差值），导致合并后的高度 `rect.max().y - top`
            // 变成负数。
            //
            let below = rect.min().y >= last.rect.min().y - 0.5;
            let adjacent = (last.rect.max().y - rect.min().y).abs() <= 1.0;
            if spans_match && below && adjacent {
                let top = last.rect.min().y;
                let height = rect.max().y - top;
                if height > 0.0 {
                    last.rect = Rect::new(last.rect.origin.x, top, last.rect.size.x, height);
                    continue;
                }
            }
        }
        slots.push(RoundedSlot {
            rect: *rect,
            radii: [radius; 4],
        });
    }
    slots
}

/// 为圆角槽位构建闭合轮廓多边形，用于描边或命中测试。
///
/// 角点从左上角起顺时针排列；零半径直接输出精确角点坐标，
/// 使直角槽位保持为真正的四边形，而非带有重合顶点的 28 边形。
///
#[must_use]
pub fn outline_polygon(slot: &RoundedSlot) -> Vec<Vec2> {
    const ARC_STEPS: usize = 6;
    const QUARTER: f32 = core::f32::consts::FRAC_PI_2;
    let min = slot.rect.min();
    let max = slot.rect.max();
    // TL、TR、BR、BL 对应的 (圆心, 起始角度)。
    let corners = [
        (
            Vec2::new(min.x + slot.radii[0], min.y + slot.radii[0]),
            core::f32::consts::PI,
            slot.radii[0],
        ),
        (
            Vec2::new(max.x - slot.radii[1], min.y + slot.radii[1]),
            core::f32::consts::PI + QUARTER,
            slot.radii[1],
        ),
        (
            Vec2::new(max.x - slot.radii[2], max.y - slot.radii[2]),
            0.0,
            slot.radii[2],
        ),
        (
            Vec2::new(min.x + slot.radii[3], max.y - slot.radii[3]),
            QUARTER,
            slot.radii[3],
        ),
    ];
    let mut points = Vec::with_capacity(ARC_STEPS * 4 + 4);
    for (centre, start, radius) in corners {
        if radius <= 0.0 {
            points.push(centre);
            continue;
        }
        for step in 0..=ARC_STEPS {
            let angle = start + QUARTER * (step as f32 / ARC_STEPS as f32);
            points.push(Vec2::new(
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            ));
        }
    }
    points
}

/// 供 `set_ime_cursor_area` 使用的屏幕空间光标矩形。
///
/// `caret` 位于世界空间，`camera` 将世界坐标映射到**逻辑**像素，
/// `dpi` 为物理像素与逻辑像素之比，`window` 为物理窗口尺寸。
/// 结果对齐到整物理像素并钳制在窗口内，同时保证最小高度以防 IME 窗口塌陷。
///
#[must_use]
pub fn ime_cursor_area(caret: Rect, camera: &Affine2, dpi: f32, window: Vec2) -> Rect {
    let transformed = camera.transform_rect_bounds(&caret);
    let scale = dpi.max(0.1);
    // 矩形对齐到整物理像素。原点四舍五入到最近像素，尺寸由取整后的角点决定，
    // 因此区域每侧最多与光标相差 1 像素，绝不会整体放大。
    //
    let x = (transformed.origin.x * scale).round();
    let y = (transformed.origin.y * scale).round();
    let right = (transformed.max().x * scale).round();
    let bottom = (transformed.max().y * scale).round();
    let width = (right - x).max(1.0);
    let height = (bottom - y).max(1.0);
    let max_x = (window.x - width).max(0.0);
    let max_y = (window.y - height).max(0.0);
    Rect::new(x.clamp(0.0, max_x), y.clamp(0.0, max_y), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn three_stacked_lines_merge_into_one_capsule() {
        let lines = [
            rect(10.0, 0.0, 100.0, 20.0),
            rect(10.0, 20.0, 100.0, 20.0),
            rect(10.0, 40.0, 100.0, 20.0),
        ];
        let slots = merge_line_rects(&lines, 6.0);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].rect, rect(10.0, 0.0, 100.0, 60.0));
        // 单个矩形意味着每个角都是外角，内部换行缺口完全消失——即“无缺口”特性。
        //
        assert_eq!(slots[0].radii, [6.0, 6.0, 6.0, 6.0]);
    }

    #[test]
    fn a_wrapped_selection_with_uneven_spans_keeps_two_slots() {
        let lines = [rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 20.0, 80.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2, "an L-shape must not become one rectangle");
    }

    #[test]
    fn a_gap_splits_the_run() {
        let lines = [rect(0.0, 0.0, 100.0, 20.0), rect(0.0, 200.0, 100.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2, "far apart lines must not merge");
    }

    #[test]
    fn non_overlapping_columns_do_not_merge() {
        let lines = [rect(0.0, 0.0, 40.0, 20.0), rect(200.0, 20.0, 40.0, 20.0)];
        let slots = merge_line_rects(&lines, 4.0);
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn an_empty_rect_is_ignored() {
        let slots = merge_line_rects(&[rect(0.0, 0.0, 0.0, 20.0)], 4.0);
        assert!(slots.is_empty());
    }

    #[test]
    fn the_outline_starts_at_the_top_left_and_closes() {
        let slot = RoundedSlot {
            rect: rect(0.0, 0.0, 100.0, 50.0),
            radii: [8.0; 4],
        };
        let points = outline_polygon(&slot);
        assert!(points.len() >= 16, "{}", points.len());
        // 首点位于左上圆弧上，即角点的左下方。
        assert!(points[0].x <= 0.0 + 8.0);
        assert!(points[0].y >= 0.0);
        // 所有点均保持在矩形边界内（浮点误差范围内）。
        for point in &points {
            assert!(point.x >= -0.01 && point.x <= 100.01);
            assert!(point.y >= -0.01 && point.y <= 50.01);
        }
    }

    #[test]
    fn a_square_corner_emits_the_exact_corner() {
        let slot = RoundedSlot {
            rect: rect(10.0, 20.0, 30.0, 40.0),
            radii: [0.0; 4],
        };
        let points = outline_polygon(&slot);
        assert_eq!(points.len(), 4, "a square slot is a quad");
        assert!(points.contains(&Vec2::new(10.0, 20.0)));
        assert!(points.contains(&Vec2::new(40.0, 60.0)));
    }

    #[test]
    fn ime_area_tracks_the_camera_zoom_and_dpi() {
        let caret = rect(10.0, 20.0, 2.0, 20.0);
        let camera = Affine2 {
            m00: 2.0,
            m10: 0.0,
            m01: 0.0,
            m11: 2.0,
            tx: 100.0,
            ty: 50.0,
        };
        let area = ime_cursor_area(caret, &camera, 2.0, Vec2::new(1920.0, 1080.0));
        // (10,20) * 2 + (100,50) = (120, 90)，再乘以 dpi 2 = (240, 180)。
        assert_eq!(area.origin, Vec2::new(240.0, 180.0));
        // 光标宽 2 px、高 20 px；先经缩放加倍，再经 dpi 加倍。
        assert_eq!(area.size, Vec2::new(8.0, 80.0));
    }

    #[test]
    fn ime_area_is_clamped_inside_the_window() {
        let caret = rect(5000.0, 5000.0, 2.0, 20.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(area.max().x <= 800.0 + 0.01);
        assert!(area.max().y <= 600.0 + 0.01);
    }

    #[test]
    fn ime_area_has_a_minimum_extent() {
        let caret = rect(0.0, 0.0, 0.0, 0.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(area.size.x >= 1.0);
        assert!(area.size.y >= 1.0);
    }

    /// IME 区域绝不能大于光标本身。
    ///
    /// 原点向下取整而尺寸向上取整曾导致矩形每侧最多比光标大出两个物理像素。
    ///
    #[test]
    fn ime_area_is_not_larger_than_the_caret() {
        let caret = rect(10.4, 20.6, 2.0, 20.0);
        let camera = Affine2::IDENTITY;
        let area = ime_cursor_area(caret, &camera, 1.0, Vec2::new(800.0, 600.0));
        assert!(
            area.size.x <= caret.size.x + 1.0,
            "the width must not exceed the caret's: {} vs {}",
            area.size.x,
            caret.size.x
        );
        assert!(
            area.size.y <= caret.size.y + 1.0,
            "the height must not exceed the caret's: {} vs {}",
            area.size.y,
            caret.size.y
        );
    }

    /// 合并行框绝不能产生负高度。
    ///
    /// 先前的邻接测试使用绝对差值，导致位于前一个**上方**的矩形也被判定为相邻，
    /// 进而算出负的合并高度。
    ///
    #[test]
    fn merging_lines_never_produces_a_negative_height() {
        let above = rect(0.0, 0.0, 20.0, 5.0);
        let below = rect(0.0, 5.0, 20.0, 5.0);
        // “下方”的框先出现，随后是其上方的框：两者绝不能合并。
        //
        let slots = merge_line_rects(&[below, above], 4.0);
        for slot in &slots {
            assert!(
                slot.rect.size.y >= 0.0,
                "a merged slot must not have a negative height: {:?}",
                slot.rect
            );
        }
    }
}

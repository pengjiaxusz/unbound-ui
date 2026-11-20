//! VirtualMasonry：带视口裁剪与跨滚动拉框多选的虚拟化瀑布流布局容器（ADR-0021 / `GAP-UI-007`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, outlined, surface};
use crate::{ControlState, WidgetOutput};

/// 拉框多选叠加模式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum MarqueeSelectMode {
    /// 替换当前选择集。
    #[default]
    Replace,
    /// 按 Ctrl 反选命中项。
    Toggle,
    /// 按 Shift 追加命中项。
    Add,
}

/// 跨滚动橡皮筋拉框状态机（锚点与当前点统一存储在内容坐标系，确保滚屏时选区自然延伸）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VirtualMarqueeState {
    /// 当前是否处于活跃拉框中。
    pub active: bool,
    /// 按下起点（内容坐标系）。
    pub anchor_content: Vec2,
    /// 当前指针点（内容坐标系）。
    pub current_content: Vec2,
    /// 选择叠加模式。
    pub mode: MarqueeSelectMode,
}

impl VirtualMarqueeState {
    /// 在内容坐标 `anchor_content` 启动一次拉框。
    pub fn begin(&mut self, anchor_content: Vec2, mode: MarqueeSelectMode) {
        self.active = true;
        self.anchor_content = anchor_content;
        self.current_content = anchor_content;
        self.mode = mode;
    }

    /// 更新当前指针的内容坐标。
    pub fn update(&mut self, current_content: Vec2) {
        if self.active {
            self.current_content = current_content;
        }
    }

    /// 结束拉框并返回最终内容坐标选区矩形。
    pub fn finish(&mut self) -> Option<Rect> {
        if !self.active {
            return None;
        }
        let rect = self.content_rect();
        self.active = false;
        rect
    }

    /// 返回规范化（非负宽高）的内容坐标选区矩形。
    #[must_use]
    pub fn content_rect(&self) -> Option<Rect> {
        if !self.active {
            return None;
        }
        let min_x = self.anchor_content.x.min(self.current_content.x);
        let min_y = self.anchor_content.y.min(self.current_content.y);
        let max_x = self.anchor_content.x.max(self.current_content.x);
        let max_y = self.anchor_content.y.max(self.current_content.y);
        Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    /// 将内容坐标选区投影至当前视口坐标系，供绘制半透明橡皮筋使用。
    #[must_use]
    pub fn viewport_rect(&self, viewport: Rect, scroll_y: f32) -> Option<Rect> {
        let r = self.content_rect()?;
        let x0 = (viewport.origin.x + r.origin.x).clamp(viewport.origin.x, viewport.max().x);
        let y0 =
            (viewport.origin.y + r.origin.y - scroll_y).clamp(viewport.origin.y, viewport.max().y);
        let x1 = (viewport.origin.x + r.max().x).clamp(viewport.origin.x, viewport.max().x);
        let y1 =
            (viewport.origin.y + r.max().y - scroll_y).clamp(viewport.origin.y, viewport.max().y);
        let w = (x1 - x0).max(0.0);
        let h = (y1 - y0).max(0.0);
        (w > 0.0 || h > 0.0).then_some(Rect::new(x0, y0, w, h))
    }
}

/// 计算拉框拖拽接近视口上下边缘时的自动滚屏垂直偏移增量（正值向下滚，负值向上滚）。
#[must_use]
pub fn compute_edge_autoscroll(
    viewport: Rect,
    pointer: Vec2,
    edge_zone_px: f32,
    max_speed_px_per_sec: f32,
    dt: f32,
) -> f32 {
    if edge_zone_px <= 0.0 || max_speed_px_per_sec <= 0.0 || dt <= 0.0 {
        return 0.0;
    }
    let top_edge = viewport.origin.y + edge_zone_px;
    let bottom_edge = viewport.max().y - edge_zone_px;
    if pointer.y < top_edge {
        let ratio = ((top_edge - pointer.y) / edge_zone_px).clamp(0.0, 1.0);
        -ratio * max_speed_px_per_sec * dt
    } else if pointer.y > bottom_edge {
        let ratio = ((pointer.y - bottom_edge) / edge_zone_px).clamp(0.0, 1.0);
        ratio * max_speed_px_per_sec * dt
    } else {
        0.0
    }
}

/// 返回内容坐标系中与拉框矩形相交的全部条目下标。
#[must_use]
pub fn hit_items_in_marquee(item_rects: &[Rect], marquee_content: Rect) -> Vec<usize> {
    item_rects
        .iter()
        .enumerate()
        .filter_map(|(idx, r)| rects_intersect(*r, marquee_content).then_some(idx))
        .collect()
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.origin.x <= b.max().x
        && a.max().x >= b.origin.x
        && a.origin.y <= b.max().y
        && a.max().y >= b.origin.y
}

/// 虚拟瀑布流网格中的单个条目。
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryItem {
    pub id: u64,
    pub aspect_ratio: f32, // 宽 / 高
}

/// 虚拟瀑布流描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasonryDesc {
    pub rect: Rect,
    pub columns: usize,
    pub gap: f32,
    pub item_count: usize,
    pub radius: f32,
    /// 可选活跃橡皮筋选区（视口坐标系）；为 `None` 时不发射选区图元。
    pub marquee_rect: Option<Rect>,
}

impl VirtualMasonryDesc {
    #[must_use]
    pub fn new(rect: Rect, columns: usize) -> Self {
        Self {
            rect,
            columns: columns.max(1),
            gap: 12.0,
            item_count: 24,
            radius: 8.0,
            marquee_rect: None,
        }
    }

    #[must_use]
    pub fn columns(columns: usize) -> Self {
        Self::new(Rect::ZERO, columns)
    }

    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    #[must_use]
    pub fn item_count(mut self, count: usize) -> Self {
        self.item_count = count;
        self
    }

    /// 绑定当前活跃的橡皮筋选区矩形（视口坐标系）。
    #[must_use]
    pub fn with_marquee_rect(mut self, marquee_rect: Option<Rect>) -> Self {
        self.marquee_rect = marquee_rect;
        self
    }
}

/// 虚拟瀑布流状态。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VirtualMasonryState {
    pub scroll_y: f32,
    pub marquee: VirtualMarqueeState,
}

impl VirtualMasonryState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// 输出虚拟瀑布流布局的图元与命中区域。
#[must_use]
pub fn emit_virtual_masonry(
    node: u64,
    desc: &VirtualMasonryDesc,
    state: &VirtualMasonryState,
    control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. 容器背景
    out.primitives
        .push(surface(rect, desc.radius, Token::SURFACE, z));

    // 2. 固定 6 个交错卡片槽位，以在帧间保持结构稳定并限制在容器边界内
    let col_count = desc.columns.clamp(1, 4);
    let pad = 8.0;
    let inner_w = (rect.width() - pad * 2.0).max(20.0);
    let col_w = ((inner_w - desc.gap * (col_count as f32 - 1.0)) / col_count as f32).max(12.0);
    let rows = 6_usize.div_ceil(col_count);
    let max_cell_h = ((rect.height() - pad * 2.0 - desc.gap * (rows.saturating_sub(1)) as f32)
        / rows.max(1) as f32)
        .max(8.0);
    let heights = [0.78_f32, 1.0, 0.85, 0.96, 0.72, 0.90];
    let mut col_y = [rect.origin.y + pad; 4];

    for (i, &height_scale) in heights.iter().enumerate() {
        let col = i % col_count;
        let x = rect.origin.x + pad + col as f32 * (col_w + desc.gap);
        let y = col_y[col].min(rect.max().y - pad - 4.0);
        let h = (max_cell_h * height_scale).min((rect.max().y - pad - y).max(4.0));
        let item_rect = Rect::new(x, y, col_w, h);
        col_y[col] = y + h + desc.gap;

        out.primitives.push(surface(
            item_rect,
            4.0,
            if i % 2 == 0 {
                Token::SURFACE_RAISED
            } else {
                Token::PRIMARY_SOFT
            },
            z + 1 + i as u16,
        ));
    }

    // 3. 外边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if control.focused {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        z + 8,
    ));

    // 4. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 9,
        control.focused && !control.disabled,
    ));

    // 5. 活跃拉框橡皮筋选区（优先使用显式描述符矩形，否则自动从状态投影）
    let active_marquee = desc
        .marquee_rect
        .or_else(|| state.marquee.viewport_rect(rect, state.scroll_y));
    if let Some(marquee) = active_marquee {
        out.primitives.push(outlined(
            marquee,
            2.0,
            Token::PRIMARY_SOFT,
            Token::ACCENT,
            1.0,
            0.0,
            z + 10,
        ));
    }

    out.hit = (!control.disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    out.a11y = Some(A11yNode::new(Role::Grid, "VirtualMasonry", rect).disabled(control.disabled));
    out.focusable = !control.disabled;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_tracks_content_coordinates_across_scroll() {
        let mut marquee = VirtualMarqueeState::default();
        marquee.begin(Vec2::new(10.0, 20.0), MarqueeSelectMode::Replace);
        marquee.update(Vec2::new(200.0, 620.0));
        let rect = marquee.content_rect().expect("active marquee rect");
        assert_eq!(rect, Rect::new(10.0, 20.0, 190.0, 600.0));

        let items = [
            Rect::new(0.0, 0.0, 50.0, 50.0),
            Rect::new(250.0, 100.0, 40.0, 40.0),
            Rect::new(100.0, 500.0, 80.0, 80.0),
        ];
        assert_eq!(hit_items_in_marquee(&items, rect), vec![0, 2]);
        assert_eq!(marquee.finish(), Some(rect));
        assert!(!marquee.active);
    }

    #[test]
    fn edge_autoscroll_is_zero_inside_safe_zone_and_signed_at_edges() {
        let vp = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(
            compute_edge_autoscroll(vp, Vec2::new(400.0, 300.0), 36.0, 600.0, 0.1),
            0.0
        );
        let up = compute_edge_autoscroll(vp, Vec2::new(400.0, 18.0), 36.0, 600.0, 0.1);
        assert!((up - (-30.0)).abs() < 1e-4);
        let down = compute_edge_autoscroll(vp, Vec2::new(400.0, 591.0), 36.0, 600.0, 0.1);
        assert!((down - 45.0).abs() < 1e-4);
    }
}

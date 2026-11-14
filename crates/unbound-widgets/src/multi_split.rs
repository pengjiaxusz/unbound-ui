//! 多重分割容器（MultiSplit）：连续 N 路水平或垂直分割容器。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, outlined, surface};
use crate::scroll::ScrollAxis;
use crate::{ControlState, WidgetOutput};

pub const MULTI_SPLIT_GUTTER_SIZE: f32 = 6.0;

/// 多重分割容器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct MultiSplitDesc {
    pub rect: Rect,
    pub axis: ScrollAxis,
    pub count: usize,
    pub radius: f32,
}

impl MultiSplitDesc {
    #[must_use]
    pub fn new(rect: Rect, count: usize) -> Self {
        Self {
            rect,
            axis: ScrollAxis::Horizontal,
            count: count.max(2),
            radius: 6.0,
        }
    }

    #[must_use]
    pub fn count(count: usize) -> Self {
        Self::new(Rect::ZERO, count)
    }

    #[must_use]
    pub fn axis(mut self, axis: ScrollAxis) -> Self {
        self.axis = axis;
        self
    }
}

/// 存储各窗格比例的多重分割容器状态。
#[derive(Clone, Debug, PartialEq)]
pub struct MultiSplitState {
    pub ratios: Vec<f32>,
    pub dragging_gutter: Option<usize>,
}

impl Default for MultiSplitState {
    fn default() -> Self {
        Self::new(3)
    }
}

impl MultiSplitState {
    #[must_use]
    pub fn new(count: usize) -> Self {
        let count = count.max(2);
        let equal = 1.0 / count as f32;
        Self {
            ratios: vec![equal; count],
            dragging_gutter: None,
        }
    }
}

/// 发射多重分割窗格的图元与命中区域。
#[must_use]
pub fn emit_multi_split(
    node: u64,
    desc: &MultiSplitDesc,
    state: &MultiSplitState,
    control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. 容器背景
    out.primitives
        .push(surface(rect, desc.radius, Token::SURFACE, z));

    // 2. 固定 2 条分隔槽以维持固定形状（按轴向与 state.ratios 定位）
    let mut accum = 0.0_f32;
    let total_ratio: f32 = state.ratios.iter().sum::<f32>().max(1e-5);
    for g_idx in 0..2 {
        let r = state.ratios.get(g_idx).copied().unwrap_or(1.0 / 3.0) / total_ratio;
        accum += r;
        let gutter_rect = match desc.axis {
            // 分隔槽与 ratios 均为一维，双轴滚动时沿用水平分栏布局。
            ScrollAxis::Horizontal | ScrollAxis::Both => {
                let gx = rect.origin.x + accum * rect.width() - MULTI_SPLIT_GUTTER_SIZE * 0.5;
                Rect::new(gx, rect.origin.y, MULTI_SPLIT_GUTTER_SIZE, rect.height())
            }
            ScrollAxis::Vertical => {
                let gy = rect.origin.y + accum * rect.height() - MULTI_SPLIT_GUTTER_SIZE * 0.5;
                Rect::new(rect.origin.x, gy, rect.width(), MULTI_SPLIT_GUTTER_SIZE)
            }
        };
        let is_dragging = !control.disabled && state.dragging_gutter == Some(g_idx);
        let gutter_bg = if control.disabled {
            Token::DISABLED
        } else if is_dragging {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        };

        out.primitives
            .push(surface(gutter_rect, 2.0, gutter_bg, z + 1 + g_idx as u16));
    }

    // 3. 容器轮廓线
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
        z + 3,
    ));

    // 4. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    let cursor = match desc.axis {
        ScrollAxis::Horizontal | ScrollAxis::Both => CursorKind::ResizeEastWest,
        ScrollAxis::Vertical => CursorKind::ResizeNorthSouth,
    };
    out.hit = (!control.disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(cursor)
    });

    out.a11y = Some(A11yNode::new(Role::Group, "MultiSplit", rect).disabled(control.disabled));
    out.focusable = !control.disabled;
    out
}

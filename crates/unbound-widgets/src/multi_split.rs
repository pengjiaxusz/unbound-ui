//! 多重分割容器（MultiSplit）：连续 N 路水平或垂直分割容器。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
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

    // 2. 固定 3 条分隔槽以维持固定形状
    let total_w = rect.width();
    let col_w = total_w / 3.0;
    for g_idx in 0..2 {
        let gx = rect.origin.x + (g_idx + 1) as f32 * col_w - MULTI_SPLIT_GUTTER_SIZE * 0.5;
        let gutter_rect = Rect::new(gx, rect.origin.y, MULTI_SPLIT_GUTTER_SIZE, rect.height());
        let is_dragging = state.dragging_gutter == Some(g_idx);
        let gutter_bg = if is_dragging {
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

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::ResizeEastWest),
    );

    out.a11y = Some(A11yNode::new(Role::Group, "MultiSplit", rect));
    out
}

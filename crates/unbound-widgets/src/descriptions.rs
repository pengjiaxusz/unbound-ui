//! 描述列表（Descriptions）：结构化多列只读键值对列表。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
use crate::{ControlState, WidgetOutput};

pub const DESCRIPTION_ROW_HEIGHT: f32 = 32.0;

/// 单条描述项。
#[derive(Clone, Debug, PartialEq)]
pub struct DescriptionItem {
    pub label: String,
    pub value: String,
}

impl DescriptionItem {
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// 描述展示列表描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DescriptionsDesc {
    pub rect: Rect,
    pub title: String,
    pub items: Vec<DescriptionItem>,
    pub columns: usize,
    pub radius: f32,
}

impl DescriptionsDesc {
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            items: Vec::new(),
            columns: 2,
            radius: 6.0,
        }
    }

    #[must_use]
    pub fn title(title: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, title)
    }

    #[must_use]
    pub fn columns(mut self, cols: usize) -> Self {
        self.columns = cols.max(1);
        self
    }

    #[must_use]
    pub fn item(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.items.push(DescriptionItem::new(label, value));
        self
    }
}

/// 描述列表状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DescriptionsState;

impl DescriptionsState {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// 发射描述列表块的图元与命中区域。
#[must_use]
pub fn emit_descriptions(
    node: u64,
    desc: &DescriptionsDesc,
    _state: &DescriptionsState,
    _control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. 外层背景
    out.primitives
        .push(surface(rect, desc.radius, Token::SURFACE, z));

    // 2. 固定 3 行分隔线以维持固定形状
    for r_idx in 0..3 {
        let y = rect.origin.y + (r_idx + 1) as f32 * DESCRIPTION_ROW_HEIGHT;
        let line_rect = Rect::new(rect.origin.x, y, rect.width(), 1.0);
        out.primitives.push(surface(
            line_rect,
            0.0,
            Token::OUTLINE,
            z + 1 + r_idx as u16,
        ));
    }

    // 3. 外层边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        Token::OUTLINE,
        1.0,
        0.0,
        z + 4,
    ));

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer),
    );

    out.a11y = Some(A11yNode::new(
        Role::DescriptionList,
        desc.title.clone(),
        rect,
    ));
    out
}

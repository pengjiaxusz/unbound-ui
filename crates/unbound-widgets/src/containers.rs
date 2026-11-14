//! 三种结构容器：卡片（Card）、换行行（WrapRow）与堆叠（Stack）。
//!
//! 三者均不持有状态。Card 是带头部和底部槽位的矩形，WrapRow 是纯换行计算函数，
//! Stack 是一组共享同一矩形且深度键递增的子节点列表。保持无状态意味着容器可
//! 直接根据布局重新计算，无需经过事件循环往返。

use unbound_abi::desc::NodeFlags;
use unbound_gpu::data::{GpuPayload, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{instance, outlined, surface};
use crate::WidgetOutput;

/// 卡片可使用的海拔层级。
pub const CARD_ELEVATIONS: [f32; 5] = [0.0, 2.0, 6.0, 12.0, 20.0];

/// 卡片默认圆角半径。
pub const CARD_RADIUS: f32 = 10.0;

/// 将行转换为自动换行行的节点标志。
pub const WRAP_FLAG: NodeFlags = NodeFlags::WRAP;

/// 卡片的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CardDesc {
    /// 卡片矩形。
    pub rect: Rect,
    /// 圆角半径。
    pub radius: f32,
    /// 海拔层级，`0..=4`。
    pub elevation: u8,
    /// 头部高度（若卡片包含头部）。
    pub header_height: Option<f32>,
    /// 底部高度（若卡片包含底部）。
    pub footer_height: Option<f32>,
    /// 绘制顺序键。
    pub z: u16,
}

impl Default for CardDesc {
    fn default() -> Self {
        Self::new(Rect::ZERO)
    }
}

impl CardDesc {
    /// 创建无阴影扁平卡片。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radius: CARD_RADIUS,
            elevation: 0,
            header_height: None,
            footer_height: None,
            z: 100,
        }
    }

    /// 设置卡片海拔阴影。
    #[must_use]
    pub fn elevated(mut self, elevation: u8) -> Self {
        self.elevation = elevation.min(CARD_ELEVATIONS.len() as u8 - 1);
        self
    }

    /// 附加头部条带。
    #[must_use]
    pub fn with_header(mut self, height: f32) -> Self {
        self.header_height = Some(height.max(0.0));
        self
    }

    /// 附加底部条带。
    #[must_use]
    pub fn with_footer(mut self, height: f32) -> Self {
        self.footer_height = Some(height.max(0.0));
        self
    }

    /// 当前海拔层级对应的阴影模糊半径。
    #[must_use]
    pub fn shadow(&self) -> f32 {
        CARD_ELEVATIONS[self.elevation.min(4) as usize]
    }

    /// 头部条带矩形（若存在）。
    #[must_use]
    pub fn header_rect(&self) -> Option<Rect> {
        self.header_height.map(|height| {
            Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                self.rect.size.x,
                height.min(self.rect.size.y),
            )
        })
    }

    /// 底部条带矩形（若存在）。
    #[must_use]
    pub fn footer_rect(&self) -> Option<Rect> {
        self.footer_height.map(|height| {
            let height = height.min(self.rect.size.y);
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - height,
                self.rect.size.x,
                height,
            )
        })
    }

    /// 头部与底部之间的内容区域矩形。
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        let top = self.header_height.unwrap_or(0.0).min(self.rect.size.y);
        let bottom = self.footer_height.unwrap_or(0.0);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + top,
            self.rect.size.x,
            (self.rect.size.y - top - bottom).max(0.0),
        )
    }
}

/// 卡片绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardTokens {
    /// 卡片表面背景色。
    pub surface: Token,
    /// 卡片边框色。
    pub border: Token,
    /// 头部条带背景色。
    pub header: Token,
    /// 底部条带背景色。
    pub footer: Token,
    /// 主体与条带之间的细分隔线颜色。
    pub divider: Token,
}

impl CardTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            surface: Token::CARD_BG,
            border: Token::CARD_BORDER,
            header: Token::SURFACE_RAISED,
            footer: Token::SURFACE_RAISED,
            divider: Token::DIVIDER,
        }
    }
}

/// 发射卡片表面、边框、条带与分隔线。
#[must_use]
pub fn emit_card(desc: &CardDesc, tokens: CardTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5);
    primitives.push(outlined(
        desc.rect,
        desc.radius,
        tokens.surface,
        tokens.border,
        1.0,
        desc.shadow(),
        desc.z,
    ));
    if let Some(header) = desc.header_rect() {
        primitives.push(outlined(
            header,
            desc.radius,
            tokens.header,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        ));
        primitives.push(hairline(
            Rect::new(header.origin.x, header.max().y - 1.0, header.size.x, 1.0),
            tokens.divider,
            desc.z,
        ));
    }
    if let Some(footer) = desc.footer_rect() {
        primitives.push(outlined(
            footer,
            desc.radius,
            tokens.footer,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        ));
        primitives.push(hairline(
            Rect::new(footer.origin.x, footer.origin.y, footer.size.x, 1.0),
            tokens.divider,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::<GpuPayload>::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 自动换行行的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct WrapDesc {
    /// 行矩形。
    pub rect: Rect,
    /// 元素之间的水平间距。
    pub gap_x: f32,
    /// 行与行之间的垂直间距。
    pub gap_y: f32,
    /// 单行高度。
    pub line_height: f32,
    /// 调用方应应用的节点标志；恒为 [`WRAP_FLAG`]。
    pub flags: NodeFlags,
}

impl WrapDesc {
    /// 使用默认间距创建自动换行行。
    #[must_use]
    pub fn new(rect: Rect, line_height: f32) -> Self {
        Self {
            rect,
            gap_x: 8.0,
            gap_y: 8.0,
            line_height: line_height.max(1.0),
            flags: WRAP_FLAG,
        }
    }

    /// 覆盖间距设置。
    #[must_use]
    pub fn with_gaps(mut self, x: f32, y: f32) -> Self {
        self.gap_x = x.max(0.0);
        self.gap_y = y.max(0.0);
        self
    }

    /// 将一组元素尺寸拆分为多行。
    ///
    /// 宽于整行的元素会独占一行并向右侧溢出，这是唯一不会背着调用方擅自修改元素尺寸的策略。
    #[must_use]
    pub fn layout(&self, sizes: &[Vec2]) -> Vec<Rect> {
        let mut out = Vec::with_capacity(sizes.len());
        let mut x = self.rect.origin.x;
        let mut y = self.rect.origin.y;
        let mut line_height = self.line_height;
        for size in sizes {
            if x > self.rect.origin.x && x + size.x > self.rect.max().x {
                x = self.rect.origin.x;
                y += line_height + self.gap_y;
                line_height = self.line_height;
            }
            out.push(Rect::new(x, y, size.x, size.y.max(self.line_height)));
            x += size.x + self.gap_x;
            line_height = line_height.max(size.y);
        }
        out
    }

    /// 给定尺寸列表产生的行数。
    #[must_use]
    pub fn line_count(&self, sizes: &[Vec2]) -> usize {
        let mut lines = usize::from(!sizes.is_empty());
        let mut x = 0.0;
        for size in sizes {
            if x > 0.0 && x + size.x > self.rect.size.x {
                lines += 1;
                x = 0.0;
            }
            x += size.x + self.gap_x;
        }
        lines
    }

    /// 换行后所需的总高度。
    #[must_use]
    pub fn content_height(&self, sizes: &[Vec2]) -> f32 {
        let lines = self.line_count(sizes);
        if lines == 0 {
            0.0
        } else {
            lines as f32 * self.line_height + (lines - 1) as f32 * self.gap_y
        }
    }
}

/// 堆叠容器的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct StackDesc {
    /// 所有子元素共同填充的共享矩形。
    pub rect: Rect,
    /// 最底层子元素的绘制顺序键。
    pub z: u16,
    /// 每层的深度增量。
    pub z_step: u16,
}

impl StackDesc {
    /// 在指定矩形上创建堆叠容器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            z: 100,
            z_step: 1,
        }
    }

    /// 单层矩形：每一层共享堆叠容器的包围盒。
    #[must_use]
    pub fn layer_rect(&self, _index: usize) -> Rect {
        self.rect
    }

    /// 单层的绘制顺序键。
    #[must_use]
    pub fn layer_z(&self, index: usize) -> u16 {
        self.z.saturating_add(
            self.z_step
                .saturating_mul(index.min(u16::MAX as usize) as u16),
        )
    }

    /// 返回所有层的矩形，最底层在前。
    #[must_use]
    pub fn layout(&self, count: usize) -> Vec<Rect> {
        (0..count).map(|index| self.layer_rect(index)).collect()
    }
}

/// 发射堆叠容器的可选背景。
#[must_use]
pub fn emit_stack(desc: &StackDesc, fill: Token) -> WidgetOutput {
    WidgetOutput {
        primitives: if fill == Token::NONE {
            Vec::new()
        } else {
            vec![surface(desc.rect, 0.0, fill, desc.z)]
        },
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 1 像素分隔线。
fn hairline(rect: Rect, color: Token, z: u16) -> unbound_gpu::data::GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_without_bands_fills_with_its_body() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0));
        assert_eq!(desc.body_rect(), desc.rect);
        assert!(desc.header_rect().is_none());
        assert!(desc.footer_rect().is_none());
    }

    #[test]
    fn the_body_sits_between_the_bands() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0))
            .with_header(40.0)
            .with_footer(30.0);
        assert_eq!(desc.header_rect().expect("header").size.y, 40.0);
        assert_eq!(desc.footer_rect().expect("footer").origin.y, 170.0);
        let body = desc.body_rect();
        assert_eq!(body.origin.y, 40.0);
        assert_eq!(body.size.y, 130.0);
    }

    #[test]
    fn elevation_maps_to_shadow_blur() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 10.0, 10.0)).elevated(2);
        assert_eq!(desc.shadow(), CARD_ELEVATIONS[2]);
        assert_eq!(desc.clone().elevated(9).shadow(), CARD_ELEVATIONS[4]);
    }

    #[test]
    fn a_card_emits_its_bands_and_separators() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0))
            .with_header(40.0)
            .with_footer(30.0);
        let output = emit_card(&desc, CardTokens::standard());
        assert_eq!(output.primitives.len(), 5);
        assert!(output.hit.is_none());
    }

    #[test]
    fn wrap_breaks_a_row_at_the_right_edge() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        let rects = desc.layout(&[
            Vec2::new(40.0, 20.0),
            Vec2::new(40.0, 20.0),
            Vec2::new(40.0, 20.0),
        ]);
        assert_eq!(rects[0].origin, Vec2::new(0.0, 0.0));
        assert_eq!(rects[1].origin, Vec2::new(48.0, 0.0));
        assert_eq!(rects[2].origin, Vec2::new(0.0, 28.0));
        assert_eq!(desc.line_count(&[Vec2::new(40.0, 20.0); 3]), 2);
        assert_eq!(desc.content_height(&[Vec2::new(40.0, 20.0); 3]), 48.0);
    }

    #[test]
    fn an_oversized_item_gets_its_own_line() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        let rects = desc.layout(&[Vec2::new(200.0, 20.0), Vec2::new(30.0, 20.0)]);
        assert_eq!(rects[0].origin, Vec2::new(0.0, 0.0));
        assert_eq!(rects[1].origin, Vec2::new(0.0, 28.0));
    }

    #[test]
    fn wrap_carries_the_layout_flag() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        assert_eq!(desc.flags, NodeFlags::WRAP);
    }

    #[test]
    fn a_stack_shares_one_rectangle_and_raises_each_layer() {
        let desc = StackDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let rects = desc.layout(3);
        assert_eq!(rects.len(), 3);
        assert!(rects.iter().all(|rect| *rect == desc.rect));
        assert_eq!(desc.layer_z(0), 100);
        assert_eq!(desc.layer_z(1), 101);
        assert_eq!(desc.layer_z(2), 102);
    }

    #[test]
    fn an_empty_stack_paints_nothing() {
        let desc = StackDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let output = emit_stack(&desc, Token::NONE);
        assert!(output.primitives.is_empty());
    }
}

//! 分隔线（Divider）：分隔两个区域的细线，支持可选居中标签。
//!
//! 分隔线属于装饰性元素，因此没有命中区域也不获取焦点。它唯一强制遵守的规则是
//! 描边必须对齐物理像素网格：跨两个像素的 1 像素细线会显得模糊成 2 像素。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::{snap_screen, Token};

use crate::primitive::{glyphs, instance};
use crate::WidgetOutput;

/// 分隔线的方向。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DividerOrientation {
    /// 跨越宽度的水平线。
    #[default]
    Horizontal,
    /// 跨越高度的垂直线。
    Vertical,
}

/// 分隔线的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DividerDesc {
    /// 分隔线所在的区域。
    pub rect: Rect,
    /// 线条是水平还是垂直。
    pub orientation: DividerOrientation,
    /// 可选的居中标签。
    pub label: Option<String>,
    /// 线条首尾两端的内缩距离。
    pub inset: f32,
    /// 线条与标签之间的间距。
    pub label_gap: f32,
    /// 细线厚度。
    pub thickness: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl DividerDesc {
    /// 横跨给定矩形的水平分隔线。
    #[must_use]
    pub fn horizontal(rect: Rect) -> Self {
        Self {
            rect,
            orientation: DividerOrientation::Horizontal,
            label: None,
            inset: 0.0,
            label_gap: 12.0,
            thickness: 1.0,
            z: 300,
        }
    }

    /// 贯穿给定矩形的垂直分隔线。
    #[must_use]
    pub fn vertical(rect: Rect) -> Self {
        Self {
            orientation: DividerOrientation::Vertical,
            ..Self::horizontal(rect)
        }
    }

    /// 附加居中标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// 设置首尾内缩距离。
    #[must_use]
    pub fn with_inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self
    }

    /// 对齐像素网格后的描边矩形。
    #[must_use]
    pub fn rule_rect(&self) -> Rect {
        let thickness = self.thickness.max(1.0);
        match self.orientation {
            DividerOrientation::Horizontal => {
                let y = snap_screen(self.rect.origin.y + (self.rect.size.y - thickness) * 0.5);
                Rect::new(
                    snap_screen(self.rect.origin.x + self.inset),
                    y,
                    (self.rect.size.x - self.inset * 2.0).max(0.0),
                    snap_screen(thickness),
                )
            }
            DividerOrientation::Vertical => {
                let x = snap_screen(self.rect.origin.x + (self.rect.size.x - thickness) * 0.5);
                Rect::new(
                    x,
                    snap_screen(self.rect.origin.y + self.inset),
                    snap_screen(thickness),
                    (self.rect.size.y - self.inset * 2.0).max(0.0),
                )
            }
        }
    }

    /// 为标签留出空间的两段半线。
    ///
    /// 返回前段、后段线条以及标签矩形。
    #[must_use]
    pub fn labelled_segments(&self, label_width: f32) -> Option<(Rect, Rect, Rect)> {
        let rule = self.rule_rect();
        if label_width <= 0.0 {
            return None;
        }
        let horizontal = self.orientation == DividerOrientation::Horizontal;
        let span = if horizontal { rule.size.x } else { rule.size.y };
        if label_width + self.label_gap * 2.0 >= span {
            return None;
        }
        let start = if horizontal {
            rule.origin.x
        } else {
            rule.origin.y
        };
        let leading = (span - label_width) * 0.5 - self.label_gap;
        let trailing = span - leading - label_width - self.label_gap * 2.0;
        if horizontal {
            let band = Rect::new(
                rule.origin.x,
                rule.origin.y - self.rect.size.y * 0.25,
                label_width.max(1.0),
                (self.rect.size.y * 0.5).max(rule.size.y),
            );
            let first = Rect::new(start, rule.origin.y, leading.max(0.0), rule.size.y);
            let second = Rect::new(
                start + leading + self.label_gap * 2.0 + label_width,
                rule.origin.y,
                trailing.max(0.0),
                rule.size.y,
            );
            Some((first, second, band))
        } else {
            // 带标签的垂直分隔线较少见；标签布局为横跨线条的水平条带。
            let band = Rect::new(
                rule.origin.x - self.rect.size.x * 0.25,
                rule.origin.y,
                (self.rect.size.x * 0.5).max(rule.size.x),
                label_width.max(1.0),
            );
            let first = Rect::new(rule.origin.x, rule.origin.y, rule.size.x, leading.max(0.0));
            let second = Rect::new(
                rule.origin.x,
                rule.origin.y + leading + self.label_gap * 2.0 + label_width,
                rule.size.x,
                trailing.max(0.0),
            );
            Some((first, second, band))
        }
    }
}

/// 分隔线绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DividerTokens {
    /// 线条颜色。
    pub rule: Token,
    /// 标签文字颜色。
    pub label: Token,
}

impl DividerTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            rule: Token::DIVIDER,
            label: Token::TEXT_SECONDARY,
        }
    }
}

/// 发射分隔线，若调用方排版了标签则一并发射。
///
/// `label_width` 为 `label` 的测量宽度；传入 `0.0` 时即使设置了 [`DividerDesc::label`] 也绘制完整连续线条。
#[must_use]
pub fn emit_divider(
    desc: &DividerDesc,
    label: &[GlyphQuad],
    label_width: f32,
    tokens: DividerTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    match desc.labelled_segments(label_width) {
        Some((first, second, _band)) => {
            primitives.push(line(first, tokens.rule, desc.z));
            primitives.push(line(second, tokens.rule, desc.z));
            glyphs(label, tokens.label, desc.z, &mut primitives, &mut payloads);
        }
        None => primitives.push(line(desc.rule_rect(), tokens.rule, desc.z)),
    }

    let a11y = desc
        .label
        .as_ref()
        .map(|text| A11yNode::new(Role::GenericContainer, text.clone(), desc.rect));

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y,
        focusable: false,
        ..Default::default()
    }
}

/// 实心线段图元。
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
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
    fn a_horizontal_rule_sits_on_the_pixel_grid() {
        let desc = DividerDesc::horizontal(Rect::new(10.4, 20.6, 200.0, 8.0));
        let rule = desc.rule_rect();
        assert_eq!(rule.origin.y, rule.origin.y.round());
        assert_eq!(rule.size.y, 1.0);
        assert_eq!(rule.origin.x, 10.0);
        assert_eq!(rule.size.x, 200.0);
    }

    #[test]
    fn the_inset_shortens_both_ends() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 100.0, 8.0)).with_inset(12.0);
        let rule = desc.rule_rect();
        assert_eq!(rule.origin.x, 12.0);
        assert_eq!(rule.size.x, 76.0);
    }

    #[test]
    fn a_vertical_rule_is_one_pixel_wide() {
        let desc = DividerDesc::vertical(Rect::new(0.0, 0.0, 8.0, 100.0));
        let rule = desc.rule_rect();
        assert_eq!(rule.size.x, 1.0);
        assert_eq!(rule.size.y, 100.0);
    }

    #[test]
    fn a_label_splits_the_rule_into_two_segments() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 20.0)).with_label("OR");
        let (first, second, band) = desc.labelled_segments(24.0).expect("room for the label");
        assert!(first.size.x > 0.0);
        assert!(second.size.x > 0.0);
        assert_eq!(band.size.x, 24.0);
        assert_eq!(band.size.y, 10.0);
        assert!(first.max().x < second.origin.x);
    }

    #[test]
    fn a_label_that_does_not_fit_leaves_the_rule_whole() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 20.0, 20.0)).with_label("OR");
        assert!(desc.labelled_segments(200.0).is_none());
    }

    #[test]
    fn the_divider_is_decorative_and_takes_no_input() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 8.0));
        let output = emit_divider(&desc, &[], 0.0, DividerTokens::standard());
        assert!(output.hit.is_none());
        assert!(output.a11y.is_none());
        assert!(!output.focusable);
        assert_eq!(output.primitives.len(), 1);
    }

    #[test]
    fn a_labelled_divider_exposes_its_text() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 20.0)).with_label("OR");
        let output = emit_divider(&desc, &[], 24.0, DividerTokens::standard());
        let a11y = output.a11y.expect("a label is reachable");
        assert_eq!(a11y.label, "OR");
        assert_eq!(output.primitives.len(), 2);
    }

    #[test]
    fn the_rule_uses_the_divider_token() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 8.0));
        let tokens = DividerTokens::standard();
        let output = emit_divider(&desc, &[], 0.0, tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.rule.raw() as u32
        );
        assert_eq!(output.primitives[0].prim_type(), PrimType::Line);
    }
}

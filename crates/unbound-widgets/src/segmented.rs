//! 分段控件：一排槽位，其中一项处于选中状态。
//!
//! 每个槽位都是独立控件——拥有各自的标识、命中区域与语义节点——
//! 但它们共享同一个描述符，因此整排的几何形状不会与其槽位发生偏移。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 槽位在其单元格内的内边距。
pub const SEGMENT_INSET: f32 = 2.0;

/// 分段控件绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentedTokens {
    /// 槽位背后的外框。
    pub frame: Token,
    /// 已选中的槽位。
    pub selected: Token,
    /// 已选中槽位的边框。
    pub selected_border: Token,
    /// 悬停状态的槽位。
    pub hover: Token,
    /// 已选中槽位的标签。
    pub label_selected: Token,
    /// 未选中槽位的标签。
    pub label: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl SegmentedTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            frame: Token::CONTROL_TRACK,
            selected: Token::PRIMARY_SOFT,
            selected_border: Token::PRIMARY,
            hover: Token::CONTROL_HOVER,
            label_selected: Token::PRIMARY,
            label: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 分段控件的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentedDesc {
    /// 外框矩形。
    pub rect: Rect,
    /// 槽位标签列表。
    pub labels: Vec<String>,
    /// 当前选中槽位的索引。
    pub selected: usize,
    /// 滑块胶囊的动画位置（以槽位为单位）。
    ///
    /// `None` 表示将胶囊绘制在 [`Self::selected`] 槽位内（逐帧稳态结果）。
    /// `Some(t)` 表示在槽位之间的 `t` 处绘制单个浮动胶囊，使切换选中项时呈现滑动而非跳变。
    pub thumb: Option<f32>,
    /// 各槽位的自定义宽度（当整行不按等宽列划分时）。
    ///
    /// 较短的列表对其余缺失槽位回退到等宽划分。
    pub item_widths: Option<Vec<f32>>,
    /// 每个槽位占一位的禁用掩码：置位表示该槽位拒绝交互。
    pub disabled_mask: u64,
    /// 绘制顺序键。
    pub z: u16,
}

impl SegmentedDesc {
    /// 创建带有指定标签的分段控件。
    #[must_use]
    pub fn new(rect: Rect, labels: &[&str]) -> Self {
        Self {
            rect,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            selected: 0,
            thumb: None,
            item_widths: None,
            disabled_mask: 0,
            z: 700,
        }
    }

    /// 切换选中的槽位索引。
    #[must_use]
    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// 在插值后的槽位位置绘制胶囊，而非直接吸附到选中槽位。
    #[must_use]
    pub fn with_thumb(mut self, thumb: f32) -> Self {
        self.thumb = Some(thumb);
        self
    }

    /// 为槽位指定显式宽度，替代等宽划分。
    #[must_use]
    pub fn with_item_widths(mut self, widths: Vec<f32>) -> Self {
        self.item_widths = Some(widths);
        self
    }

    /// 禁用 `mask` 中置位对应的槽位。
    #[must_use]
    pub fn with_disabled_mask(mut self, mask: u64) -> Self {
        self.disabled_mask = mask;
        self
    }

    /// 禁用单个槽位，保留掩码其余位。
    #[must_use]
    pub fn with_disabled(mut self, index: usize, disabled: bool) -> Self {
        if index < u64::BITS as usize {
            let bit = 1u64 << index;
            if disabled {
                self.disabled_mask |= bit;
            } else {
                self.disabled_mask &= !bit;
            }
        }
        self
    }

    /// 查询某个槽位是否禁用交互。
    #[must_use]
    pub fn is_disabled(&self, index: usize) -> bool {
        index < u64::BITS as usize && (self.disabled_mask >> index) & 1 == 1
    }

    /// 计算单个槽位的 x 偏移与宽度（遵循显式宽度设置）。
    fn slot_geometry(&self, index: usize) -> (f32, f32) {
        if let Some(widths) = &self.item_widths {
            if let Some(width) = widths.get(index) {
                let offset: f32 = widths[..index].iter().sum();
                return (self.rect.origin.x + offset, *width);
            }
        }
        let count = self.count();
        let width = self.rect.size.x / count as f32;
        (self.rect.origin.x + index as f32 * width, width)
    }

    /// 胶囊矩形；当设置了 [`Self::thumb`] 时返回插值位置。
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        let count = self.count();
        let position = self
            .thumb
            .unwrap_or(self.selected as f32)
            .clamp(0.0, (count - 1) as f32);
        let lower = position.floor() as usize;
        let frac = position - lower as f32;
        let (x0, w0) = self.slot_geometry(lower);
        let (x1, w1) = self.slot_geometry((lower + 1).min(count - 1));
        Rect::new(
            x0 + (x1 - x0) * frac + SEGMENT_INSET,
            self.rect.origin.y + SEGMENT_INSET,
            (w0 + (w1 - w0) * frac - SEGMENT_INSET * 2.0).max(8.0),
            (self.rect.size.y - SEGMENT_INSET * 2.0).max(8.0),
        )
    }

    /// 控件包含的槽位数量。
    #[must_use]
    pub fn count(&self) -> usize {
        self.labels.len().max(1)
    }

    /// 单个槽位的单元格矩形：未设置 [`Self::item_widths`] 时为等宽列。
    #[must_use]
    pub fn cell_rect(&self, index: usize) -> Rect {
        let (x, width) = self.slot_geometry(index);
        Rect::new(x, self.rect.origin.y, width, self.rect.size.y)
    }

    /// 单个槽位的实际绘制矩形（由单元格向内缩进）。
    #[must_use]
    pub fn segment_rect(&self, index: usize) -> Rect {
        let cell = self.cell_rect(index);
        Rect::new(
            cell.origin.x + SEGMENT_INSET,
            cell.origin.y + SEGMENT_INSET,
            (cell.size.x - SEGMENT_INSET * 2.0).max(8.0),
            (cell.size.y - SEGMENT_INSET * 2.0).max(8.0),
        )
    }
}

/// 可移动分段选区的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentedKey {
    /// 移动到上一个可用槽位。
    ArrowLeft,
    /// 移动到下一个可用槽位。
    ArrowRight,
}

/// 将选中项移动一个可用槽位。
///
/// 整行构成一个单选组，因此方向键会循环回绕：在首个可用槽位按左键会落到最后一个可用槽位。
/// 双向都会跳过禁用槽位；若所有槽位均被禁用，则不做任何移动并原样返回 `current`。
#[must_use]
pub fn reduce_segmented_key(desc: &SegmentedDesc, current: usize, key: SegmentedKey) -> usize {
    let count = desc.count();
    let step: isize = match key {
        SegmentedKey::ArrowLeft => -1,
        SegmentedKey::ArrowRight => 1,
    };
    let mut index = current.min(count - 1) as isize;
    for _ in 0..count {
        index = (index + step).rem_euclid(count as isize);
        if !desc.is_disabled(index as usize) {
            return index as usize;
        }
    }
    current
}

/// 输出承载槽位的外框。
///
/// 外框不携带命中区域：仅槽位本身可拾取。
#[must_use]
pub fn emit_segmented_frame(desc: &SegmentedDesc, tokens: SegmentedTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2);
    primitives.push(surface(desc.rect, 7.0, tokens.frame, desc.z));
    // 动画胶囊驻留在外框中而非槽位中：它必须是单个可移动的图元，
    // 否则填充色会在槽位间瞬移，“动画”就会变成闪烁。
    //
    if desc.thumb.is_some() {
        primitives.push(outlined(
            desc.thumb_rect(),
            5.0,
            tokens.selected,
            tokens.selected_border,
            1.0,
            0.0,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 输出分段控件的单个槽位。
///
/// `label` 携带调用方已在槽位内定位好的字形。
#[must_use]
pub fn emit_segmented_segment(
    node: u64,
    desc: &SegmentedDesc,
    index: usize,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SegmentedTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let segment = desc.segment_rect(index);
    let active = index == desc.selected;
    let disabled = state.disabled || desc.is_disabled(index);

    // 始终仅保留一个胶囊图元。激活槽位将其绘制为带边框的凸起表面；
    // 悬停槽位绘制悬停填充且无边框；空闲槽位则完全不绘制。
    // 由于该图元在所有状态下都存在，悬停槽位只需修补单个实例而无需重新编号图元池。
    //
    //
    let (fill, border, border_width) = if desc.thumb.is_some() {
        // 外框持有主胶囊；槽位仅绘制自身的悬停填充。
        if state.phase == PointerPhase::Hovered && !active && !disabled {
            (tokens.hover, Token::NONE, 0.0)
        } else {
            (Token::NONE, Token::NONE, 0.0)
        }
    } else if active {
        (tokens.selected, tokens.selected_border, 1.0)
    } else if state.phase == PointerPhase::Hovered && !disabled {
        (tokens.hover, Token::NONE, 0.0)
    } else {
        (Token::NONE, Token::NONE, 0.0)
    };
    primitives.push(outlined(segment, 5.0, fill, border, border_width, 0.0, z));

    let label_color = if disabled {
        Token::TEXT_DISABLED
    } else if active {
        tokens.label_selected
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // 焦点环始终输出：焦点变化仅改变其不透明度而非图元池结构，因此聚焦只是绘制变更而非重传。
    //
    primitives.push(focus_ring(
        segment.inflate(2.0),
        7.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), segment)
            .with_shape(Shape::Rounded { radii: [5.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let label_text = desc.labels.get(index).cloned().unwrap_or_default();
    let mut a11y = A11yNode::new(Role::Tab, label_text, segment)
        .clickable()
        .disabled(disabled);
    a11y.selected = active;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SegmentedDesc {
        SegmentedDesc::new(
            Rect::new(0.0, 0.0, 300.0, 32.0),
            &["Light", "Dark", "System"],
        )
    }

    #[test]
    fn the_slots_split_the_frame_into_equal_columns() {
        let desc = desc();
        assert_eq!(desc.count(), 3);
        assert_eq!(desc.cell_rect(0).size.x, 100.0);
        assert_eq!(desc.cell_rect(1).origin.x, 100.0);
        assert_eq!(desc.cell_rect(2).origin.x, 200.0);
    }

    #[test]
    fn the_slot_is_inset_from_its_cell() {
        let desc = desc();
        let segment = desc.segment_rect(0);
        assert_eq!(segment.origin.x, 2.0);
        assert_eq!(segment.size.x, 96.0);
    }

    #[test]
    fn the_selected_slot_is_filled_and_bordered() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            1,
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.selected.raw() as u32
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.selected_border.raw() as u32
        );
    }

    #[test]
    fn an_unselected_slot_paints_nothing_on_its_own() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            0,
            ControlState::default(),
            &[],
            tokens,
        );
        // 空闲槽位不绘制可见内容，但仍输出两个图元——不可见胶囊与不可见焦点环——
        // 以便悬停时只需修补字段而无需插入图元。
        //
        assert_eq!(output.primitives.len(), 2);
        assert!(
            output
                .primitives
                .iter()
                .all(|primitive| primitive.fill_token_or_uv == Token::NONE.raw() as u32),
            "an idle slot must not paint a fill"
        );
    }

    #[test]
    fn a_hovered_unselected_slot_paints_the_hover_fill() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            0,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.hover.raw() as u32
        );
    }

    #[test]
    fn the_frame_carries_no_hit_region() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_frame(&desc(), tokens);
        assert!(output.hit.is_none());
        assert!(output.a11y.is_none());
        assert_eq!(output.primitives.len(), 1);
    }

    #[test]
    fn every_slot_is_its_own_control() {
        let tokens = SegmentedTokens::standard();
        let desc = desc();
        for index in 0..desc.count() {
            let output = emit_segmented_segment(
                10 + index as u64,
                &desc,
                index,
                ControlState::default(),
                &[],
                tokens,
            );
            let hit = output.hit.expect("每个槽位都可点击");
            assert_eq!(hit.node, 10 + index as u64);
            let a11y = output.a11y.expect("每个槽位都有语义节点");
            assert_eq!(a11y.role, Role::Tab);
        }
    }

    #[test]
    fn the_frame_paints_the_thumb_at_its_interpolated_position() {
        let tokens = SegmentedTokens::standard();
        let desc = desc().with_selected(2).with_thumb(1.5);
        // 300 px / 3 个槽位 = 每槽位 100 px；位于槽位 1 与 2 正中间。
        assert!((desc.thumb_rect().origin.x - 152.0).abs() < 0.01);
        let output = emit_segmented_frame(&desc, tokens);
        assert_eq!(output.primitives.len(), 2);
        assert_eq!(
            output.primitives[1].fill_token_or_uv,
            tokens.selected.raw() as u32
        );
        assert_eq!(
            output.primitives[1].border_token_or_tex,
            tokens.selected_border.raw() as u32
        );
    }

    #[test]
    fn a_thumb_frame_leaves_the_slot_fills_empty() {
        let tokens = SegmentedTokens::standard();
        let desc = desc().with_selected(1).with_thumb(1.0);
        let output = emit_segmented_segment(1, &desc, 1, ControlState::default(), &[], tokens);
        // 外框持有主胶囊；激活槽位不得再绘制第二个胶囊。
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            Token::NONE.raw() as u32
        );
    }

    #[test]
    fn the_thumb_is_clamped_to_the_row() {
        let desc = desc();
        assert!((desc.clone().with_thumb(-1.0).thumb_rect().origin.x - 2.0).abs() < 0.01);
        assert!((desc.clone().with_thumb(9.0).thumb_rect().origin.x - 202.0).abs() < 0.01);
    }
}

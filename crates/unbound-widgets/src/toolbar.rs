//! 工具栏与状态栏：密集排列的小型控件行。
//!
//! 空间不足时，工具栏会将尾部折叠进一个“»”溢出按钮，而非将每个按钮缩小到无法点击。
//! 状态栏采用相同的行布局逻辑但无交互：仅含左对齐或右对齐的标签与分隔符。
//!
//!

use accesskit::Role;
use rustc_hash::FxHashSet;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 工具栏中按钮的默认宽度。
pub const TOOLBAR_BUTTON: f32 = 30.0;

/// 分隔符宽度。
pub const TOOLBAR_SEPARATOR: f32 = 9.0;

/// 工具栏项的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarItemKind {
    /// 瞬时触发操作按钮。
    Button,
    /// 双态切换按钮。
    Toggle,
    /// 垂直分隔线。
    Separator,
    /// 弹性空白占位符。
    Spacer,
    /// 纯文本标签。
    Label,
}

/// 单个工具栏项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarItem {
    /// 稳定标识；分隔符与占位符为 0。
    pub id: u64,
    /// 项类型。
    pub kind: ToolbarItemKind,
    /// 标签或工具提示文本。
    pub label: String,
}

impl ToolbarItem {
    /// 创建瞬时操作按钮。
    #[must_use]
    pub fn button(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            kind: ToolbarItemKind::Button,
            label: label.into(),
        }
    }

    /// 创建切换按钮。
    #[must_use]
    pub fn toggle(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            kind: ToolbarItemKind::Toggle,
            label: label.into(),
        }
    }

    /// 创建垂直分隔线。
    #[must_use]
    pub fn separator() -> Self {
        Self {
            id: 0,
            kind: ToolbarItemKind::Separator,
            label: String::new(),
        }
    }

    /// 创建弹性空白占位符。
    #[must_use]
    pub fn spacer() -> Self {
        Self {
            id: 0,
            kind: ToolbarItemKind::Spacer,
            label: String::new(),
        }
    }

    /// 创建非交互标签。
    #[must_use]
    pub fn label(text: impl Into<String>) -> Self {
        Self {
            id: 0,
            kind: ToolbarItemKind::Label,
            label: text.into(),
        }
    }

    /// 该项是否可点击。
    #[must_use]
    pub fn interactive(&self) -> bool {
        matches!(self.kind, ToolbarItemKind::Button | ToolbarItemKind::Toggle)
    }
}

/// 工具栏的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarDesc {
    /// 工具栏矩形。
    pub rect: Rect,
    /// 从左到右排列的项列表。
    pub items: Vec<ToolbarItem>,
    /// 按钮尺寸。
    pub button_size: Vec2,
    /// 项之间的间距。
    pub gap: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl ToolbarDesc {
    /// 使用默认尺寸创建的工具栏。
    #[must_use]
    pub fn new(rect: Rect, items: Vec<ToolbarItem>) -> Self {
        Self {
            rect,
            items,
            button_size: Vec2::new(TOOLBAR_BUTTON, rect.size.y.min(28.0)),
            gap: 4.0,
            z: 680,
        }
    }

    /// 单个项的自然宽度。
    #[must_use]
    pub fn item_width(&self, item: &ToolbarItem) -> f32 {
        match item.kind {
            ToolbarItemKind::Button | ToolbarItemKind::Toggle => self.button_size.x,
            ToolbarItemKind::Separator => TOOLBAR_SEPARATOR,
            ToolbarItemKind::Spacer => 0.0,
            ToolbarItemKind::Label => item.label.chars().count() as f32 * 8.0 + 8.0,
        }
    }

    /// 布局所有项，并将超出部分折叠进溢出按钮。
    ///
    /// 返回的向量与 [`Self::items`] 对齐；被隐藏的项没有矩形。
    /// 溢出点之前的弹性空白会吸收剩余宽度，因此带有一个弹性空白的工具栏外观与以往一致。
    #[must_use]
    pub fn layout(&self) -> ToolbarLayout {
        let available = self.rect.size.x;
        let count = self.items.len();
        let mut widths: Vec<f32> = self
            .items
            .iter()
            .map(|item| self.item_width(item))
            .collect();
        let spacers = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == ToolbarItemKind::Spacer)
            .count();
        let gaps = count.saturating_sub(1) as f32 * self.gap;
        let fixed: f32 = widths.iter().sum::<f32>() + gaps;

        let overflow = fixed > available && count > 0;
        let visible_count = if overflow {
            let mut used = self.button_size.x + self.gap;
            let mut visible = 0;
            for width in &widths {
                if used + width + self.gap > available {
                    break;
                }
                used += *width + self.gap;
                visible += 1;
            }
            visible
        } else {
            count
        };

        if spacers > 0 && visible_count > 0 {
            let visible_gaps = visible_count.saturating_sub(1) as f32 * self.gap;
            let visible_fixed: f32 = widths.iter().take(visible_count).sum::<f32>() + visible_gaps;
            let overflow_width = if overflow {
                self.button_size.x + self.gap
            } else {
                0.0
            };
            let leftover = (available - visible_fixed - overflow_width).max(0.0);
            let visible_spacers = self
                .items
                .iter()
                .take(visible_count)
                .filter(|item| item.kind == ToolbarItemKind::Spacer)
                .count();
            if visible_spacers > 0 {
                let each = leftover / visible_spacers as f32;
                for (index, item) in self.items.iter().take(visible_count).enumerate() {
                    if item.kind == ToolbarItemKind::Spacer {
                        widths[index] = each;
                    }
                }
            }
        }

        let mut rects = vec![None; count];
        let mut x = self.rect.origin.x;
        for index in 0..visible_count.min(count) {
            let width = widths[index];
            rects[index] = Some(Rect::new(x, self.rect.origin.y, width, self.rect.size.y));
            x += width + self.gap;
        }
        let overflow = overflow.then(|| {
            Rect::new(
                self.rect.max().x - self.button_size.x,
                self.rect.origin.y,
                self.button_size.x,
                self.rect.size.y,
            )
        });
        let hidden = (visible_count..count).collect();
        ToolbarLayout {
            rects,
            overflow,
            hidden,
        }
    }
}

/// 工具栏布局结果。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarLayout {
    /// 每个项对应的可选矩形。
    pub rects: Vec<Option<Rect>>,
    /// 当尾部放不下时出现的溢出按钮矩形。
    pub overflow: Option<Rect>,
    /// 被隐藏的项索引列表。
    pub hidden: Vec<usize>,
}

/// 工具栏的实时状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolbarState {
    /// 当前处于开启状态的切换按钮集合。
    pub toggles: FxHashSet<u64>,
    /// 溢出菜单是否处于打开状态。
    pub overflow_open: bool,
}

impl ToolbarState {
    /// 创建所有切换按钮均关闭的工具栏状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 查询切换按钮是否开启。
    #[must_use]
    pub fn is_on(&self, id: u64) -> bool {
        self.toggles.contains(&id)
    }

    /// 翻转切换按钮状态并返回新状态。
    pub fn toggle(&mut self, id: u64) -> bool {
        if self.toggles.contains(&id) {
            self.toggles.remove(&id);
            false
        } else {
            self.toggles.insert(id);
            true
        }
    }

    /// 切换溢出菜单开关状态。
    pub fn toggle_overflow(&mut self) -> bool {
        self.overflow_open = !self.overflow_open;
        self.overflow_open
    }
}

/// 工具栏绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolbarTokens {
    /// 工具栏背景。
    pub bar: Token,
    /// 底部细分隔线。
    pub border: Token,
    /// 空闲按钮。
    pub item: Token,
    /// 悬停按钮。
    pub item_hover: Token,
    /// 按下的切换按钮。
    pub item_on: Token,
    /// 图标与标签。
    pub icon: Token,
    /// 按下状态切换按钮的图标。
    pub icon_on: Token,
    /// 分隔符。
    pub separator: Token,
    /// 标签文本。
    pub text: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl ToolbarTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            bar: Token::HEADER_BG,
            border: Token::DIVIDER,
            item: Token::NONE,
            item_hover: Token::CONTROL_HOVER,
            item_on: Token::PRIMARY_SOFT,
            icon: Token::TEXT_SECONDARY,
            icon_on: Token::PRIMARY,
            separator: Token::DIVIDER,
            text: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 输出工具栏背景及其底部细分隔线。
#[must_use]
pub fn emit_toolbar(desc: &ToolbarDesc, tokens: ToolbarTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2);
    primitives.push(crate::primitive::surface(
        desc.rect, 0.0, tokens.bar, desc.z,
    ));
    let line = Rect::new(
        desc.rect.origin.x,
        desc.rect.max().y - 1.0,
        desc.rect.size.x,
        1.0,
    );
    primitives.push(instance(
        line,
        [0.0; 4],
        tokens.border,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        desc.z,
    ));
    WidgetOutput {
        primitives,
        ..WidgetOutput::default()
    }
}

/// 输出单个工具栏项。
///
/// 分隔符、空白占位符与标签仅输出装饰图元；按钮与切换按钮还会输出命中区域与语义节点。
#[must_use]
pub fn emit_toolbar_item(
    desc: &ToolbarDesc,
    state: &ToolbarState,
    index: usize,
    rect: Rect,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: ToolbarTokens,
) -> WidgetOutput {
    let Some(item) = desc.items.get(index) else {
        return WidgetOutput::default();
    };
    match item.kind {
        ToolbarItemKind::Separator => {
            let x = rect.origin.x + rect.size.x * 0.5;
            let inset = rect.size.y * 0.18;
            let rule = Rect::new(x, rect.origin.y + inset, 1.0, rect.size.y - inset * 2.0);
            WidgetOutput {
                primitives: vec![instance(
                    rule,
                    [0.0; 4],
                    tokens.separator,
                    Token::NONE,
                    0.0,
                    0.0,
                    PrimType::Line,
                    SubElement::Content,
                    desc.z,
                )],
                ..WidgetOutput::default()
            }
        }
        ToolbarItemKind::Spacer => WidgetOutput::default(),
        ToolbarItemKind::Label => {
            let mut primitives = Vec::with_capacity(label.len());
            let mut payloads = Vec::with_capacity(label.len());
            glyphs(label, tokens.text, desc.z, &mut primitives, &mut payloads);
            WidgetOutput {
                primitives,
                payloads,
                hit: None,
                a11y: Some(A11yNode::new(Role::Label, item.label.clone(), rect)),
                focusable: false,
                ..Default::default()
            }
        }
        ToolbarItemKind::Button | ToolbarItemKind::Toggle => {
            let on = item.kind == ToolbarItemKind::Toggle && state.is_on(item.id);
            let disabled = control.disabled;
            let mut primitives = Vec::with_capacity(3 + label.len());
            let mut payloads = Vec::with_capacity(label.len());
            let fill = if on {
                tokens.item_on
            } else if control.phase == PointerPhase::Hovered {
                tokens.item_hover
            } else {
                tokens.item
            };
            primitives.push(outlined(rect, 6.0, fill, Token::NONE, 0.0, 0.0, desc.z));
            glyphs(
                label,
                if on { tokens.icon_on } else { tokens.icon },
                desc.z,
                &mut primitives,
                &mut payloads,
            );
            primitives.push(focus_ring(
                rect.inflate(1.0),
                7.0,
                tokens.focus_ring,
                2.0,
                desc.z,
                control.focused,
            ));

            let hit = (!disabled).then(|| {
                HitRegion::new(item.id, CONTROL_LAYER, u32::from(desc.z), rect)
                    .with_shape(Shape::Rounded { radii: [6.0; 4] })
                    .with_cursor(CursorKind::Pointer)
            });
            let mut a11y = A11yNode::new(Role::Button, item.label.clone(), rect)
                .clickable()
                .disabled(disabled);
            a11y.selected = on;

            WidgetOutput {
                primitives,
                payloads,
                hit,
                a11y: Some(a11y),
                focusable: !disabled,
                ..Default::default()
            }
        }
    }
}

/// 输出“»”溢出按钮。
#[must_use]
pub fn emit_toolbar_overflow(
    node: u64,
    rect: Rect,
    control: ControlState,
    tokens: ToolbarTokens,
    z: u16,
) -> WidgetOutput {
    let disabled = control.disabled;
    let fill = if control.phase == PointerPhase::Hovered {
        tokens.item_hover
    } else {
        tokens.item
    };
    let mut primitives = Vec::with_capacity(4);
    primitives.push(outlined(rect, 6.0, fill, Token::NONE, 0.0, 0.0, z));
    let cx = rect.origin.x + rect.size.x * 0.5;
    let cy = rect.origin.y + rect.size.y * 0.5;
    let arm = rect.size.y * 0.16;
    for (start, end) in [
        ([cx - arm, cy - arm], [cx + arm * 0.4, cy - arm]),
        ([cx - arm, cy + arm], [cx + arm * 0.4, cy + arm]),
        ([cx - arm, cy - arm], [cx - arm, cy + arm]),
    ] {
        primitives.push(instance(
            Rect::new(
                start[0].min(end[0]),
                start[1].min(end[1]),
                (start[0] - end[0]).abs().max(1.5),
                (start[1] - end[1]).abs().max(1.5),
            ),
            [0.0; 4],
            tokens.icon,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            z,
        ));
    }
    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii: [6.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, "More tools", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 状态栏分段贴靠的边缘。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusAlign {
    /// 左边缘。
    #[default]
    Left,
    /// 右边缘。
    Right,
}

/// 单个状态栏分段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSegment {
    /// 稳定标识，同时用作节点 ID。
    pub id: u64,
    /// 可见文本。
    pub label: String,
    /// 分段贴靠的边缘。
    pub align: StatusAlign,
}

impl StatusSegment {
    /// 创建左对齐分段。
    #[must_use]
    pub fn left(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            align: StatusAlign::Left,
        }
    }

    /// 创建右对齐分段。
    #[must_use]
    pub fn right(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            align: StatusAlign::Right,
        }
    }
}

/// 状态栏的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct StatusBarDesc {
    /// 状态栏矩形。
    pub rect: Rect,
    /// 分段列表。
    pub segments: Vec<StatusSegment>,
    /// 单个分段内部的水平内边距。
    pub padding: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl StatusBarDesc {
    /// 使用指定分段列表创建状态栏。
    #[must_use]
    pub fn new(rect: Rect, segments: Vec<StatusSegment>) -> Self {
        Self {
            rect,
            segments,
            padding: 10.0,
            z: 690,
        }
    }

    /// 每个分段的测量或估算宽度。
    #[must_use]
    pub fn widths(&self) -> Vec<f32> {
        self.segments
            .iter()
            .map(|segment| segment.label.chars().count() as f32 * 7.0 + self.padding * 2.0)
            .collect()
    }

    /// 布局所有分段：左对齐分段从左向右排布，右对齐分段从右向左排布。
    /// 两组永不重叠；若发生冲突，则丢弃靠后的分段。
    #[must_use]
    pub fn layout(&self, widths: &[f32]) -> Vec<Option<Rect>> {
        let mut rects = vec![None; self.segments.len()];
        let mut left = self.rect.origin.x;
        let right_edge = self.rect.max().x;
        let mut right = right_edge;
        for (index, segment) in self.segments.iter().enumerate() {
            let width = widths.get(index).copied().unwrap_or(0.0);
            match segment.align {
                StatusAlign::Left => {
                    if left + width <= right {
                        rects[index] =
                            Some(Rect::new(left, self.rect.origin.y, width, self.rect.size.y));
                        left += width;
                    }
                }
                StatusAlign::Right => {
                    if right - width >= left {
                        right -= width;
                        rects[index] = Some(Rect::new(
                            right,
                            self.rect.origin.y,
                            width,
                            self.rect.size.y,
                        ));
                    }
                }
            }
        }
        rects
    }
}

/// 输出状态栏背景及其顶部细分隔线。
#[must_use]
pub fn emit_status_bar(desc: &StatusBarDesc, tokens: ToolbarTokens) -> WidgetOutput {
    let primitives = vec![
        crate::primitive::surface(desc.rect, 0.0, tokens.bar, desc.z),
        instance(
            Rect::new(
                desc.rect.origin.x,
                desc.rect.origin.y,
                desc.rect.size.x,
                1.0,
            ),
            [0.0; 4],
            tokens.border,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Background,
            desc.z,
        ),
    ];
    WidgetOutput {
        primitives,
        ..WidgetOutput::default()
    }
}

/// 输出单个状态分段。
#[must_use]
pub fn emit_status_segment(
    desc: &StatusBarDesc,
    rect: Rect,
    segment: &StatusSegment,
    label: &[GlyphQuad],
    tokens: ToolbarTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(label.len());
    let mut payloads = Vec::with_capacity(label.len());
    glyphs(label, tokens.text, desc.z, &mut primitives, &mut payloads);
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(A11yNode::new(Role::Label, segment.label.clone(), rect)),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ToolbarDesc {
        ToolbarDesc::new(
            Rect::new(0.0, 0.0, 300.0, 32.0),
            vec![
                ToolbarItem::button(1, "Bold"),
                ToolbarItem::toggle(2, "Italic"),
                ToolbarItem::separator(),
                ToolbarItem::button(3, "Link"),
            ],
        )
    }

    #[test]
    fn items_are_laid_out_left_to_right() {
        let desc = desc();
        let layout = desc.layout();
        assert!(layout.overflow.is_none());
        assert_eq!(layout.rects[0].expect("button").origin.x, 0.0);
        assert_eq!(layout.rects[1].expect("toggle").origin.x, 34.0);
        assert_eq!(layout.hidden.len(), 0);
    }

    #[test]
    fn a_narrow_toolbar_folds_its_tail_into_overflow() {
        let mut desc = desc();
        desc.rect.size.x = 100.0;
        let layout = desc.layout();
        assert!(layout.overflow.is_some());
        assert!(!layout.hidden.is_empty());
        let overflow = layout.overflow.expect("overflow");
        assert_eq!(overflow.max().x, 100.0);
        // 任何可见项均不与溢出按钮重叠。
        for rect in layout.rects.iter().flatten() {
            assert!(rect.max().x <= overflow.origin.x);
        }
    }

    #[test]
    fn a_spacer_absorbs_the_leftover_width() {
        let mut desc = desc();
        desc.items = vec![ToolbarItem::button(1, "A"), ToolbarItem::spacer()];
        let layout = desc.layout();
        let spacer = layout.rects[1].expect("spacer");
        assert!((spacer.size.x - (300.0 - 34.0)).abs() < 1e-3);
    }

    #[test]
    fn toggles_flip_and_are_remembered() {
        let mut state = ToolbarState::new();
        assert!(!state.is_on(2));
        assert!(state.toggle(2));
        assert!(state.is_on(2));
        assert!(!state.toggle(2));
        assert!(!state.is_on(2));
    }

    #[test]
    fn a_button_is_pickable_and_a_separator_is_not() {
        let desc = desc();
        let state = ToolbarState::new();
        let layout = desc.layout();
        let tokens = ToolbarTokens::standard();
        let button = emit_toolbar_item(
            &desc,
            &state,
            0,
            layout.rects[0].expect("button"),
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(button.hit.expect("pickable").node, 1);
        assert_eq!(button.a11y.expect("reachable").role, Role::Button);

        let separator = emit_toolbar_item(
            &desc,
            &state,
            2,
            layout.rects[2].expect("separator"),
            ControlState::default(),
            &[],
            tokens,
        );
        assert!(separator.hit.is_none());
        assert!(separator.a11y.is_none());
    }

    #[test]
    fn a_pressed_toggle_uses_the_on_tokens() {
        let desc = desc();
        let mut state = ToolbarState::new();
        state.toggle(2);
        let layout = desc.layout();
        let tokens = ToolbarTokens::standard();
        let output = emit_toolbar_item(
            &desc,
            &state,
            1,
            layout.rects[1].expect("toggle"),
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.item_on.raw() as u32
        );
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn the_overflow_button_is_pickable() {
        let output = emit_toolbar_overflow(
            99,
            Rect::new(0.0, 0.0, 30.0, 30.0),
            ControlState::default(),
            ToolbarTokens::standard(),
            10,
        );
        assert_eq!(output.hit.expect("pickable").node, 99);
        assert_eq!(output.a11y.expect("reachable").role, Role::Button);
    }

    #[test]
    fn status_segments_hug_their_edges() {
        let desc = StatusBarDesc::new(
            Rect::new(0.0, 0.0, 400.0, 22.0),
            vec![
                StatusSegment::left(1, "Ready"),
                StatusSegment::right(2, "UTF-8"),
                StatusSegment::right(3, "Ln 1"),
            ],
        );
        let widths = desc.widths();
        let rects = desc.layout(&widths);
        assert_eq!(rects[0].expect("left").origin.x, 0.0);
        // 右对齐分段从右边缘向内排布，因此第一个右分段紧贴边缘，下一个位于其左侧。
        //
        //
        assert_eq!(rects[1].expect("right").max().x, 400.0);
        assert_eq!(
            rects[2].expect("right").max().x,
            rects[1].expect("right").origin.x
        );
    }

    #[test]
    fn status_segments_never_overlap() {
        let mut desc = StatusBarDesc::new(
            Rect::new(0.0, 0.0, 60.0, 22.0),
            vec![
                StatusSegment::left(1, "Left side is long"),
                StatusSegment::right(2, "Right side is long too"),
            ],
        );
        desc.padding = 0.0;
        let rects = desc.layout(&desc.widths());
        assert!(
            rects.iter().any(Option::is_none)
                || rects[0].expect("left").max().x <= rects[1].expect("right").origin.x
        );
    }

    #[test]
    fn a_status_segment_is_a_label() {
        let desc = StatusBarDesc::new(Rect::new(0.0, 0.0, 400.0, 22.0), Vec::new());
        let segment = StatusSegment::left(1, "Ready");
        let rect = Rect::new(0.0, 0.0, 50.0, 22.0);
        let output = emit_status_segment(&desc, rect, &segment, &[], ToolbarTokens::standard());
        assert_eq!(output.a11y.expect("reachable").role, Role::Label);
        assert!(output.hit.is_none());
    }
}

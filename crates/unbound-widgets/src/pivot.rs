//! Win11 Pivot（页签条）：纯文字二级导航，选中项用强调色下划线标记。
//!
//! Pivot 与 [`crate::tabs`] 是两种不同语义：文档标签页切换的是「同一区域的内容」，
//! 而 Pivot 切换的是「页面内的视图」，因此没有关闭按钮、拖拽排序和标签背景，
//! 只有文字、悬停亮度与 3px 强调色下划线——这是 Win11 设置页、任务管理器等
//! 系统应用里最常见的二级导航形态。
//!
//! 几何约定（对齐 WinUI `PivotHeaderItem`）：
//! - 高度 [`PIVOT_HEIGHT`]（48px，`Height = 48`）；
//! - 标签间距 [`PIVOT_GAP`]（24px，`PivotHeaderItemMargin = 12,0,12,0`）；
//! - 指示器厚度 [`PIVOT_INDICATOR_HEIGHT`]（3px，`SelectedPipe` 的 `Height = 3`），
//!   距底边 2px（`Margin = 0,0,0,2`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::typography::{font_size, geometry, weight};
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{AlignX, AlignY, ControlState, TextCtx, UiWidget, WidgetInputEvent, WidgetOutput};

/// Pivot 高度。对齐 WinUI `PivotHeaderItem` 的 `Height = 48`。
pub const PIVOT_HEIGHT: f32 = 48.0;
/// 标签之间的水平间距。WinUI `PivotHeaderItemMargin = 12,0,12,0`（两侧各 12）。
pub const PIVOT_GAP: f32 = 24.0;
/// 选中指示器厚度。WinUI `SelectedPipe` 的 `Height = 3`。
pub const PIVOT_INDICATOR_HEIGHT: f32 = 3.0;
/// 选中指示器距控件底边的留白（避开 `item.inflate(-2.0)` 的 2px 焦点环底边）。
const PIVOT_INDICATOR_INSET: f32 = 5.0;
/// 选中指示器左右内缩量（与 `item.inflate(-2.0)` 焦点环内侧对齐，避免左右外凸）。
const PIVOT_INDICATOR_INSET_X: f32 = 4.0;

/// Pivot 的单个标签。
#[derive(Clone, Debug, PartialEq)]
pub struct PivotItem {
    /// 标签文字。
    pub label: String,
    /// 标签槽位宽度；`0.0` 表示与同排其它标签等分可用宽度。
    pub width: f32,
}

impl PivotItem {
    /// 用等分宽度的标签。
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            width: 0.0,
        }
    }

    /// 指定标签槽位宽度。
    #[must_use]
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }
}

/// Pivot 描述符：一排标签，其中一项处于选中状态。
#[derive(Clone, Debug, PartialEq)]
pub struct PivotDesc {
    /// 控件在屏幕空间的矩形。
    pub rect: Rect,
    /// 标签列表。
    pub items: Vec<PivotItem>,
    /// 当前选中项下标。
    pub selected: usize,
    /// 标签之间的水平间距。
    pub gap: f32,
    /// 选中指示器厚度。
    pub indicator_height: f32,
    /// 绘制层级。
    pub z: u16,
}

impl PivotDesc {
    /// 用标签文字构造等宽 Pivot。
    #[must_use]
    pub fn new(rect: Rect, labels: &[&str]) -> Self {
        Self {
            rect,
            items: labels.iter().map(|label| PivotItem::new(*label)).collect(),
            selected: 0,
            gap: PIVOT_GAP,
            indicator_height: PIVOT_INDICATOR_HEIGHT,
            z: 0,
        }
    }

    /// 用显式槽位宽度构造 Pivot（长度不足以 `items` 为准）。
    #[must_use]
    pub fn with_item_widths(mut self, widths: &[f32]) -> Self {
        for (item, width) in self.items.iter_mut().zip(widths.iter().copied()) {
            item.width = width.max(0.0);
        }
        self
    }

    /// 指定选中项。
    #[must_use]
    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected.min(self.items.len().saturating_sub(1));
        self
    }

    /// 指定标签间距。
    #[must_use]
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// 指定绘制层级。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 标签数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否没有任何标签。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 第 `index` 个标签的命中 / 绘制矩形。
    ///
    /// 所有标签都给出显式宽度时按显式宽度排布；否则等分可用宽度。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Rect {
        let count = self.items.len();
        if count == 0 || index >= count {
            return Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                0.0,
                self.rect.height(),
            );
        }
        let total_gap = self.gap * (count.saturating_sub(1)) as f32;
        let available = (self.rect.width() - total_gap).max(0.0);
        let fixed: f32 = self.items.iter().map(|item| item.width).sum();
        let auto_count = self.items.iter().filter(|item| item.width <= 0.0).count();
        let fallback = if auto_count == 0 {
            0.0
        } else {
            (available - fixed).max(0.0) / auto_count as f32
        };
        let mut x = self.rect.origin.x;
        for item in self.items.iter().take(index) {
            x += if item.width > 0.0 {
                item.width
            } else {
                fallback
            } + self.gap;
        }
        let width = if self.items[index].width > 0.0 {
            self.items[index].width
        } else {
            fallback
        };
        Rect::new(x, self.rect.origin.y, width, self.rect.height())
    }

    /// 第 `index` 个标签的选中指示器矩形（内缩至焦点环内侧，避免盖住白框底边与左右外凸）。
    #[must_use]
    pub fn indicator_rect(&self, index: usize) -> Rect {
        let item = self.item_rect(index);
        let inset_x = PIVOT_INDICATOR_INSET_X.min(item.width() * 0.5);
        Rect::new(
            item.origin.x + inset_x,
            item.max().y - PIVOT_INDICATOR_INSET - self.indicator_height,
            (item.width() - inset_x * 2.0).max(0.0),
            self.indicator_height,
        )
    }
}

/// Pivot 的持久化状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PivotState {
    /// 当前选中项下标。
    pub selected: usize,
}

impl PivotState {
    /// 以指定选中项构造状态。
    #[must_use]
    pub fn new(selected: usize) -> Self {
        Self { selected }
    }
}

/// 输出 Pivot 的单个标签（命中区域与语义节点都属于该标签）。
///
/// `label` 携带调用方已在标签槽位内定位好的字形。
#[must_use]
pub fn emit_pivot_item(
    node: u64,
    desc: &PivotDesc,
    index: usize,
    state: ControlState,
    label: &[GlyphQuad],
) -> WidgetOutput {
    let item = desc.item_rect(index);
    let active = index == desc.selected;
    let disabled = state.disabled;

    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    // 始终输出固定数量的图元：背景占位、文字、焦点环、指示器。
    // 选中态只改变指示器与文字的颜色，不改变图元池结构。
    //
    // 背景占位保持透明：整排 Pivot 在声明式层只有一个命中区域，
    // `ControlState` 无法区分「指针悬停在哪一个标签上」。需要逐标签悬停反馈时，
    // 调用方可直接使用 `emit_pivot_item` 的独立命中区域自行维护每项状态。
    primitives.push(outlined(
        item,
        geometry::CONTROL_RADIUS,
        Token::NONE,
        Token::NONE,
        0.0,
        0.0,
        z,
    ));

    // `PivotHeaderItemForegroundSelected` = TextFillColorPrimary，
    // 强调色只用于下划线（`PivotHeaderItemSelectedPipeFill`）。
    let label_color = if disabled {
        Token::TEXT_DISABLED
    } else if active {
        Token::TEXT_PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        item.inflate(-2.0),
        geometry::CONTROL_RADIUS,
        Token::FOCUS_RING,
        2.0,
        z,
        state.focused && active,
    ));

    // 指示器在图元池中始终存在：未选中时填充透明令牌。
    let indicator = desc.indicator_rect(index);
    primitives.push(outlined(
        indicator,
        // `SelectedPipe` 的圆角取控件自身 CornerRadius（`ControlCornerRadius = 4`）。
        geometry::CONTROL_RADIUS,
        if active { Token::PRIMARY } else { Token::NONE },
        Token::NONE,
        0.0,
        0.0,
        z,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), item)
            .with_shape(Shape::Rounded {
                radii: [geometry::CONTROL_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let label_text = desc
        .items
        .get(index)
        .map(|i| i.label.clone())
        .unwrap_or_default();
    let mut a11y = A11yNode::new(Role::Tab, label_text, item)
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

impl UiWidget for PivotDesc {
    type State = PivotState;

    fn initial_state(&self) -> Self::State {
        PivotState::new(self.selected)
    }

    fn sync_state(&self, state: &mut Self::State) {
        // 受控控件：描述符的 `selected` 是唯一真源。
        state.selected = self.selected;
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let mut width = self.gap * (self.items.len().saturating_sub(1)) as f32;
        for item in &self.items {
            width += if item.width > 0.0 {
                item.width
            } else {
                let measured = text_ctx.measure_line(&item.label, font_size::BODY, weight::REGULAR);
                measured.x + 2.0 * geometry::PIVOT_LABEL_PADDING
            };
        }
        Vec2::new(
            width.max(self.rect.width()),
            self.rect.height().max(PIVOT_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let mut laid = self.clone();
                laid.rect = rect;
                for index in 0..laid.items.len() {
                    if laid.item_rect(index).contains_point(*pos) {
                        if state.selected != index {
                            state.selected = index;
                            return true;
                        }
                        return false;
                    }
                }
                false
            }
            WidgetInputEvent::KeyDown(chord) => {
                let last = self.items.len().saturating_sub(1);
                let next = match chord.key {
                    KeyName::Named("ArrowLeft" | "ArrowUp") => state.selected.saturating_sub(1),
                    KeyName::Named("ArrowRight" | "ArrowDown") => (state.selected + 1).min(last),
                    KeyName::Named("Home") => 0,
                    KeyName::Named("End") => last,
                    _ => return false,
                };
                if next != state.selected {
                    state.selected = next;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.selected = state.selected.min(desc.items.len().saturating_sub(1));

        let mut out = WidgetOutput::default();
        for index in 0..desc.items.len() {
            let item = desc.item_rect(index);
            let weight = if index == desc.selected {
                weight::SEMIBOLD
            } else {
                weight::REGULAR
            };
            let label = text_ctx.shape_line(
                &desc.items[index].label,
                item,
                font_size::BODY,
                weight,
                AlignX::Center,
                AlignY::Middle,
            );
            let mut item_out =
                emit_pivot_item(node + 1 + index as u64, &desc, index, control, &label);
            // 整排只暴露一个命中区域与一个可聚焦控件；子标签只保留语义节点。
            item_out.hit = None;
            out.primitives.append(&mut item_out.primitives);
            out.payloads.append(&mut item_out.payloads);
            if let Some(a11y) = item_out.a11y.take() {
                out.sub_a11y.push((node + 1 + index as u64, a11y));
            }
        }

        out.hit = (!control.disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rounded {
                    radii: [geometry::CONTROL_RADIUS; 4],
                })
                .with_cursor(CursorKind::Pointer)
        });
        out.a11y = Some(A11yNode::new(Role::TabList, "Pivot", desc.rect));
        out.focusable = !control.disabled;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PivotDesc {
        PivotDesc::new(
            Rect::new(10.0, 20.0, 300.0, PIVOT_HEIGHT),
            &["Overview", "Activity", "Settings"],
        )
    }

    #[test]
    fn labels_share_the_available_width_evenly() {
        let desc = desc();
        // 300 - 2 * 24 = 252，三等分即每项 84。
        for index in 0..3 {
            let rect = desc.item_rect(index);
            assert!((rect.width() - 84.0).abs() < 0.01);
            assert!((rect.height() - PIVOT_HEIGHT).abs() < 0.01);
        }
        assert!((desc.item_rect(1).origin.x - (10.0 + 84.0 + PIVOT_GAP)).abs() < 0.01);
        assert!((desc.item_rect(2).origin.x - (10.0 + 2.0 * (84.0 + PIVOT_GAP))).abs() < 0.01);
    }

    #[test]
    fn explicit_widths_are_respected() {
        let desc = desc().with_item_widths(&[96.0, 120.0, 64.0]);
        assert!((desc.item_rect(0).width() - 96.0).abs() < 0.01);
        assert!((desc.item_rect(1).width() - 120.0).abs() < 0.01);
        assert!(
            (desc.item_rect(2).origin.x - (10.0 + 96.0 + 120.0 + 2.0 * PIVOT_GAP)).abs() < 0.01
        );
    }

    #[test]
    fn indicator_hugs_the_bottom_edge() {
        let desc = desc();
        let item = desc.item_rect(1);
        let indicator = desc.indicator_rect(1);
        let ring = item.inflate(-2.0);
        assert!((indicator.height() - PIVOT_INDICATOR_HEIGHT).abs() < 0.01);
        assert!((indicator.max().y - (desc.rect.max().y - PIVOT_INDICATOR_INSET)).abs() < 0.01);
        assert!((indicator.width() - (item.width() - PIVOT_INDICATOR_INSET_X * 2.0)).abs() < 0.01);
        assert!(ring.contains_rect(&indicator));
    }

    #[test]
    fn emit_marks_the_selected_tab() {
        let desc = desc().with_selected(1);
        let glyphs = Vec::new();
        let selected = emit_pivot_item(1, &desc, 1, ControlState::default(), &glyphs);
        let idle = emit_pivot_item(2, &desc, 0, ControlState::default(), &glyphs);
        assert!(selected.a11y.as_ref().expect("semantic node").selected);
        assert!(!idle.a11y.as_ref().expect("semantic node").selected);
        assert_eq!(
            selected.a11y.as_ref().expect("semantic node").role,
            Role::Tab
        );
        // 选中态只改变指示器颜色，不改变图元数量。
        assert_eq!(selected.primitives.len(), idle.primitives.len());
    }

    #[test]
    fn click_and_keyboard_move_the_selection() {
        let desc = desc();
        let mut state = PivotState::new(0);
        let position = Vec2::new(desc.item_rect(2).origin.x + 4.0, desc.rect.origin.y + 8.0);
        assert!(desc.reduce(
            &mut state,
            desc.rect,
            &WidgetInputEvent::PointerDown {
                pos: position,
                click_count: 1,
            }
        ));
        assert_eq!(state.selected, 2);
        assert!(desc.reduce(
            &mut state,
            desc.rect,
            &WidgetInputEvent::KeyDown(unbound_input::KeyChord::named("Home"))
        ));
        assert_eq!(state.selected, 0);
        assert!(!desc.reduce(
            &mut state,
            desc.rect,
            &WidgetInputEvent::KeyDown(unbound_input::KeyChord::named("Home"))
        ));

        // 描述符矩形为空时，reduce 必须使用布局后的 rect 判定命中。
        let zero_desc = PivotDesc::new(Rect::ZERO, &["Overview", "Activity", "Settings"]);
        let laid_rect = Rect::new(40.0, 80.0, 300.0, PIVOT_HEIGHT);
        let mut laid = zero_desc.clone();
        laid.rect = laid_rect;
        let second_pos = Vec2::new(laid.item_rect(1).origin.x + 6.0, laid_rect.origin.y + 10.0);
        assert!(zero_desc.reduce(
            &mut state,
            laid_rect,
            &WidgetInputEvent::PointerDown {
                pos: second_pos,
                click_count: 1,
            }
        ));
        assert_eq!(state.selected, 1);
    }
}

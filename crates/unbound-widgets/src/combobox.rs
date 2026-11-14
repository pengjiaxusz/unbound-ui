//! 组合框（ComboBox）：文本触发器加可过滤弹出列表。
//!
//! 输入会过滤列表而非直接替换文本，这使组合框适用于长选项集；
//! 若当前高亮行在过滤后仍存在则予以保留，否则回退到首条匹配项。
//! `Enter` 确认提交，`Escape` 关闭，弹出列表使用与其他浮层相同的锚定定位助手，因此绝不会超出视口。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::overlay::{place_anchored_popup, PlacedPopup, Placement};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::TextEditState;
use crate::{ControlState, WidgetOutput};

/// 单个选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComboItem {
    /// 回传给应用程序的稳定值。
    pub value: String,
    /// 列表与触发器中显示的文本。
    pub label: String,
    /// 该选项是否不可选择。
    pub disabled: bool,
}

impl ComboItem {
    /// 创建可选择的选项。
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    /// 将选项标记为禁用。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 弹出列表单行高度。
pub const COMBO_ROW: f32 = 28.0;
/// 弹出列表垂直内边距。
pub const COMBO_PADDING: f32 = 6.0;
/// 触发器圆角半径。
pub const COMBO_RADIUS: f32 = 8.0;

/// 组合框的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ComboboxDesc {
    /// 触发器矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 未选中任何选项时显示的占位文本。
    pub placeholder: String,
    /// 按模型顺序排列的全部选项。
    pub items: Vec<ComboItem>,
    /// 弹出列表在滚动前最多显示的行数。
    pub max_visible: usize,
    /// 触发器是否禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl ComboboxDesc {
    /// 基于 `items` 创建组合框。
    #[must_use]
    pub fn new(rect: Rect, items: Vec<ComboItem>) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            items,
            max_visible: 8,
            disabled: false,
            z: 740,
        }
    }

    /// 当前选项数量对应的弹出列表尺寸。
    #[must_use]
    pub fn popup_size(&self) -> Vec2 {
        let rows = self.items.len().min(self.max_visible.max(1)) as f32;
        Vec2::new(self.rect.size.x, rows * COMBO_ROW + COMBO_PADDING * 2.0)
    }

    /// 将弹出列表置于触发器下方，并约束在 `viewport` 内。
    #[must_use]
    pub fn popup_rect(&self, viewport: Rect) -> Rect {
        let placed: PlacedPopup = place_anchored_popup(
            self.rect,
            self.popup_size(),
            viewport,
            Placement::BottomStart,
        );
        Rect::from(placed)
    }
}

/// 组合框消费的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboboxKey {
    Down,
    Up,
    Enter,
    Escape,
    Tab,
}

/// 按键处理结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComboboxOutcome {
    /// 按键已被消费。
    pub handled: bool,
    /// 应用程序应采用的选中选项索引。
    pub committed: Option<usize>,
    /// 弹出列表应关闭。
    pub closed: bool,
}

/// 组合框的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct ComboboxState {
    query: TextEditState,
    open: bool,
    filtered: Vec<usize>,
    highlighted: Option<usize>,
    selected: Option<usize>,
}

impl ComboboxState {
    /// 创建空组合框，所有选项初始均可见。
    #[must_use]
    pub fn new(desc: &ComboboxDesc) -> Self {
        Self {
            query: TextEditState::new(),
            open: false,
            filtered: (0..desc.items.len()).collect(),
            highlighted: None,
            selected: None,
        }
    }

    /// 弹出列表是否展开。
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 当前过滤文本。
    #[must_use]
    pub fn query(&self) -> &str {
        self.query.text()
    }

    /// 行内过滤编辑器。
    #[must_use]
    pub fn editor(&self) -> &TextEditState {
        &self.query
    }

    /// 通过当前过滤的选项（模型索引列表）。
    #[must_use]
    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    /// 当前高亮选项（模型索引）。
    #[must_use]
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// 已提交选项（模型索引）。
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 已提交的值。
    #[must_use]
    pub fn selected_value<'a>(&self, desc: &'a ComboboxDesc) -> Option<&'a str> {
        self.selected
            .and_then(|index| desc.items.get(index))
            .map(|item| item.value.as_str())
    }

    /// 触发器显示的文本。
    #[must_use]
    pub fn display_text(&self, desc: &ComboboxDesc) -> String {
        self.selected
            .and_then(|index| desc.items.get(index))
            .map(|item| item.label.clone())
            .unwrap_or_else(|| self.query.text().to_owned())
    }

    /// 切换弹出列表展开/关闭，不改变过滤文本。
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// 打开弹出列表并定位到已提交选项。
    pub fn open(&mut self, desc: &ComboboxDesc) {
        self.open = true;
        self.filtered = (0..desc.items.len()).collect();
        self.highlighted = self.selected.or(if self.filtered.is_empty() {
            None
        } else {
            Some(self.filtered[0])
        });
    }

    /// 关闭弹出列表。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 替换过滤文本并重新过滤。
    pub fn set_query(&mut self, desc: &ComboboxDesc, text: impl Into<String>) {
        self.query = TextEditState::with_text(text);
        self.refilter_with(desc);
    }

    fn refilter_with(&mut self, desc: &ComboboxDesc) {
        let needle = self.query.text().to_lowercase();
        let mut matches: Vec<usize> = desc
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| needle.is_empty() || item.label.to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect();
        // 前缀匹配排在前面，随后保持模型顺序。对组合框而言列表规模有限，稳定分区已足够。
        matches.sort_by_key(|index| {
            let label = desc.items[*index].label.to_lowercase();
            (!label.starts_with(&needle), *index)
        });
        self.filtered = matches;
        self.highlighted = self
            .highlighted
            .filter(|index| self.filtered.contains(index))
            .or_else(|| self.filtered.first().copied());
    }

    /// 按整行移动高亮，跳过禁用选项并在首尾循环。
    pub fn move_highlight(&mut self, desc: &ComboboxDesc, delta: isize) -> bool {
        if self.filtered.is_empty() {
            return false;
        }
        let current = self
            .highlighted
            .and_then(|index| {
                self.filtered
                    .iter()
                    .position(|candidate| *candidate == index)
            })
            .unwrap_or(0) as isize;
        let count = self.filtered.len() as isize;
        for step in 1..=count {
            let position = (current + delta * step).rem_euclid(count) as usize;
            let index = self.filtered[position];
            if !desc.items.get(index).is_some_and(|item| item.disabled) {
                self.highlighted = Some(index);
                return true;
            }
        }
        false
    }

    /// 提交当前高亮的选项。
    pub fn accept(&mut self, desc: &ComboboxDesc) -> Option<usize> {
        let index = self.highlighted?;
        if desc.items.get(index).is_some_and(|item| item.disabled) {
            return None;
        }
        self.selected = Some(index);
        self.query = TextEditState::with_text(desc.items[index].label.clone());
        self.open = false;
        Some(index)
    }

    /// 归约导航按键。
    pub fn key(&mut self, desc: &ComboboxDesc, key: ComboboxKey) -> ComboboxOutcome {
        match key {
            ComboboxKey::Down => {
                if !self.open {
                    self.open(desc);
                    return ComboboxOutcome {
                        handled: true,
                        committed: None,
                        closed: false,
                    };
                }
                ComboboxOutcome {
                    handled: self.move_highlight(desc, 1),
                    committed: None,
                    closed: false,
                }
            }
            ComboboxKey::Up => ComboboxOutcome {
                handled: self.move_highlight(desc, -1),
                committed: None,
                closed: false,
            },
            ComboboxKey::Enter => {
                let committed = self.accept(desc);
                ComboboxOutcome {
                    handled: committed.is_some(),
                    committed,
                    closed: committed.is_some(),
                }
            }
            ComboboxKey::Escape => {
                let closed = self.open;
                self.open = false;
                ComboboxOutcome {
                    handled: closed,
                    committed: None,
                    closed,
                }
            }
            ComboboxKey::Tab => {
                let closed = self.open;
                self.open = false;
                ComboboxOutcome {
                    handled: false,
                    committed: None,
                    closed,
                }
            }
        }
    }
}

/// 发射触发器。
#[must_use]
pub fn emit_combobox_trigger(
    node: u64,
    desc: &ComboboxDesc,
    state: &ComboboxState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(8 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::INPUT_BG
    };
    primitives.push(surface(rect, COMBO_RADIUS, fill, desc.z));
    primitives.push(outlined(
        rect,
        COMBO_RADIUS,
        Token::NONE,
        if control.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if control.focused || state.open {
            Token::OUTLINE_STRONG
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        desc.z,
    ));

    // 使用两条描边绘制展开箭头，因此框架无需依赖字形。
    let cx = rect.max().x - 14.0;
    let cy = rect.center().y;
    for (dx, dy) in [(-4.0, -1.0), (4.0, 1.0)] {
        primitives.push(instance(
            Rect::new(cx + dx - 3.0, cy + dy - 0.75, 6.0, 1.5),
            [0.0; 4],
            Token::TEXT_SECONDARY,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else if state.selected.is_none() && state.query.text().is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        COMBO_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [COMBO_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::ComboBox, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.display_text(desc);
    a11y.expanded = Some(state.open);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 发射弹出列表。浮层条目由调用方持有；此处仅负责绘制。
#[must_use]
pub fn emit_combobox_popup(
    desc: &ComboboxDesc,
    state: &ComboboxState,
    popup: Rect,
    hovered: Option<usize>,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + state.filtered.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    primitives.push(surface(popup, 8.0, Token::OVERLAY_BG, desc.z));
    primitives.push(outlined(
        popup,
        8.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        desc.z,
    ));
    for (row, index) in state.filtered.iter().take(desc.max_visible).enumerate() {
        let item = &desc.items[*index];
        let rect = Rect::new(
            popup.origin.x + 4.0,
            popup.origin.y + COMBO_PADDING + row as f32 * COMBO_ROW,
            popup.size.x - 8.0,
            COMBO_ROW,
        );
        let fill = if state.highlighted == Some(*index) {
            Token::NAV_SELECTED
        } else if hovered == Some(*index) {
            Token::NAV_HOVER
        } else {
            Token::NONE
        };
        primitives.push(surface(rect, 6.0, fill, desc.z));
        if item.disabled {
            primitives.push(surface(
                Rect::new(
                    rect.origin.x,
                    rect.origin.y + rect.size.y - 1.0,
                    rect.size.x,
                    1.0,
                ),
                0.0,
                Token::DIVIDER,
                desc.z,
            ));
        }
    }
    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        desc.z,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(Role::ListBox, desc.label.clone(), popup);
    a11y.value = state.display_text(desc);
    a11y.expanded = Some(state.open);
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ComboboxDesc {
        let items = vec![
            ComboItem::new("apple", "Apple"),
            ComboItem::new("banana", "Banana"),
            ComboItem::new("berry", "Berry"),
            ComboItem::new("cherry", "Cherry"),
        ];
        ComboboxDesc::new(Rect::new(10.0, 10.0, 200.0, 34.0), items)
    }

    #[test]
    fn typing_filters_and_ranks_prefix_matches_first() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        state.set_query(&desc, "b");
        assert_eq!(state.filtered(), &[1, 2]);
        state.set_query(&desc, "be");
        assert_eq!(state.filtered(), &[2]);
        state.set_query(&desc, "");
        assert_eq!(state.filtered().len(), 4);
    }

    #[test]
    fn filtering_is_case_insensitive() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        state.set_query(&desc, "APP");
        assert_eq!(state.filtered(), &[0]);
    }

    #[test]
    fn highlight_wraps_and_skips_disabled_options() {
        let mut desc = desc();
        desc.items[1].disabled = true;
        let mut state = ComboboxState::new(&desc);
        state.open(&desc);
        assert_eq!(state.highlighted(), Some(0));
        assert!(state.move_highlight(&desc, 1));
        assert_eq!(state.highlighted(), Some(2), "banana is skipped");
        state.highlighted = Some(3);
        assert!(state.move_highlight(&desc, 1));
        assert_eq!(state.highlighted(), Some(0), "wraps to the top");
    }

    #[test]
    fn accepting_commits_the_highlight_and_closes() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        state.open(&desc);
        state.move_highlight(&desc, 2);
        assert_eq!(state.accept(&desc), Some(2));
        assert_eq!(state.selected_value(&desc), Some("berry"));
        assert!(!state.is_open());
        assert_eq!(state.display_text(&desc), "Berry");
    }

    #[test]
    fn keyboard_navigation_opens_and_closes() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        let down = state.key(&desc, ComboboxKey::Down);
        assert!(down.handled && state.is_open());
        assert!(state.key(&desc, ComboboxKey::Down).handled);
        let enter = state.key(&desc, ComboboxKey::Enter);
        assert_eq!(enter.committed, Some(1));
        assert!(enter.closed);

        state.open(&desc);
        let escape = state.key(&desc, ComboboxKey::Escape);
        assert!(escape.closed);
        assert!(!state.is_open());
    }

    #[test]
    fn the_popup_stays_inside_the_viewport() {
        let mut desc = desc();
        desc.rect = Rect::new(600.0, 500.0, 180.0, 34.0);
        let viewport = Rect::new(0.0, 0.0, 680.0, 520.0);
        let popup = desc.popup_rect(viewport);
        assert!(popup.max().y <= viewport.max().y);
        assert!(popup.max().x <= viewport.max().x);
        assert_eq!(popup.size.x, desc.rect.size.x);
    }

    #[test]
    fn the_trigger_reports_an_expanded_combo_box() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        state.open(&desc);
        let output = emit_combobox_trigger(3, &desc, &state, ControlState::default(), &mut []);
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::ComboBox);
        assert_eq!(a11y.expanded, Some(true));
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
    }

    #[test]
    fn the_popup_paints_one_row_per_visible_match() {
        let desc = desc();
        let mut state = ComboboxState::new(&desc);
        state.open(&desc);
        let popup = Rect::new(10.0, 50.0, 200.0, 4.0 * COMBO_ROW + COMBO_PADDING * 2.0);
        let output = emit_combobox_popup(&desc, &state, popup, Some(1), &mut []);
        assert_eq!(
            output.primitives.len(),
            2 + state.filtered().len(),
            "chrome plus one row surface each"
        );
        assert_eq!(output.a11y.expect("reachable").role, Role::ListBox);
    }
}

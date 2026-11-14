//! 双栏穿梭框（Transfer），支持过滤与源/目标列表间的批量移动。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// [`TransferDesc`] 中的单个条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferItem {
    /// 稳定条目标识键。
    pub key: u64,
    /// 显示标签。
    pub label: String,
    /// 该条目当前是否位于目标（右侧）列表。
    pub in_target: bool,
    /// 该条目是否已勾选待批量移动。
    pub checked: bool,
}

/// [`TransferDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferState {
    /// 跨越两侧列表的所有条目。
    pub items: Vec<TransferItem>,
}

impl TransferState {
    /// 将所有已勾选项移动到 `to_target`（`true` = 右侧，`false` = 左侧）并取消勾选。
    pub fn move_checked(&mut self, to_target: bool) -> usize {
        let mut moved = 0;
        for item in &mut self.items {
            if item.checked && item.in_target != to_target {
                item.in_target = to_target;
                item.checked = false;
                moved += 1;
            }
        }
        moved
    }
}

/// “移动到目标（`>`）”按钮的确定性子节点键。
#[must_use]
pub const fn move_right_node(base: u64) -> u64 {
    base ^ 0x5452_4652_0001
}

/// “移动到源（`<`）”按钮的确定性子节点键。
#[must_use]
pub const fn move_left_node(base: u64) -> u64 {
    base ^ 0x5452_4652_0002
}

/// 单个 [`TransferItem`] 的确定性子节点键。
#[must_use]
pub const fn transfer_item_node(base: u64, item_key: u64) -> u64 {
    base ^ (item_key.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// 双列表 [`Transfer`] 穿梭框的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TransferDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 左侧列表条目数。
    pub source_count: usize,
    /// 右侧列表条目数。
    pub target_count: usize,
    /// 两侧面板的全部条目。
    pub items: Vec<TransferItem>,
    /// 左侧搜索过滤词。
    pub left_query: String,
    /// 右侧搜索过滤词。
    pub right_query: String,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl TransferDesc {
    /// 创建穿梭框描述符。
    #[must_use]
    pub fn new(rect: Rect, source_count: usize, target_count: usize) -> Self {
        Self {
            rect,
            source_count,
            target_count,
            items: Vec::new(),
            left_query: String::new(),
            right_query: String::new(),
            label: "Transfer".to_owned(),
            z: 600,
        }
    }

    /// 为穿梭框描述符附加条目列表。
    #[must_use]
    pub fn with_items(mut self, items: Vec<TransferItem>) -> Self {
        self.source_count = items.iter().filter(|i| !i.in_target).count();
        self.target_count = items.iter().filter(|i| i.in_target).count();
        self.items = items;
        self
    }

    /// 附加左右搜索过滤词。
    #[must_use]
    pub fn with_queries(mut self, left: impl Into<String>, right: impl Into<String>) -> Self {
        self.left_query = left.into();
        self.right_query = right.into();
        self
    }

    /// `>`（右移）与 `<`（左移）按钮的矩形。
    #[must_use]
    pub fn action_button_rects(&self) -> (Rect, Rect) {
        let pad = 6.0;
        let mid_w = 28.0;
        let pane_w = ((self.rect.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
        let left_max_x = self.rect.origin.x + pad + pane_w;
        let mid_x = left_max_x + pad;
        let cy = self.rect.origin.y + self.rect.height() * 0.5;
        let right_btn = Rect::new(mid_x + 2.0, cy - 16.0, mid_w - 4.0, 20.0);
        let left_btn = Rect::new(mid_x + 2.0, cy + 4.0, mid_w - 4.0, 20.0);
        (right_btn, left_btn)
    }
}

/// 输出结构稳定的双栏穿梭框，带有独立的 `>`/`<` 与条目子命中区域。
#[must_use]
pub fn emit_transfer(
    node: u64,
    desc: &TransferDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        Token::SURFACE,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pad = 6.0;
    let mid_w = 28.0;
    let pane_w = ((desc.rect.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
    let pane_h = (desc.rect.height() - pad * 2.0).max(24.0);

    let left_r = Rect::new(
        desc.rect.origin.x + pad,
        desc.rect.origin.y + pad,
        pane_w,
        pane_h,
    );
    let right_r = Rect::new(
        left_r.max().x + pad * 2.0 + mid_w,
        desc.rect.origin.y + pad,
        pane_w,
        pane_h,
    );
    for pane in [left_r, right_r] {
        primitives.push(outlined(
            pane,
            6.0,
            Token::SURFACE_SUNKEN,
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    let (right_btn, left_btn) = desc.action_button_rects();
    for btn_r in [right_btn, left_btn] {
        primitives.push(outlined(
            btn_r,
            4.0,
            if hovered {
                Token::PRIMARY_SOFT
            } else {
                Token::SURFACE_RAISED
            },
            Token::PRIMARY,
            1.0,
            0.0,
            desc.z + 2,
        ));
    }

    primitives.push(focus_ring(
        desc.rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let r_key = move_right_node(node);
        out.with_sub_control(
            r_key,
            HitRegion::new(r_key, CONTROL_LAYER, u32::from(desc.z) + 2, right_btn)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Move to target", right_btn).clickable(),
            true,
        );
        let l_key = move_left_node(node);
        out.with_sub_control(
            l_key,
            HitRegion::new(l_key, CONTROL_LAYER, u32::from(desc.z) + 2, left_btn)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Move to source", left_btn).clickable(),
            true,
        );

        let row_h = 24.0;
        let mut left_idx = 0usize;
        let mut right_idx = 0usize;
        for item in &desc.items {
            let (pane, idx, query) = if item.in_target {
                (&right_r, &mut right_idx, &desc.right_query)
            } else {
                (&left_r, &mut left_idx, &desc.left_query)
            };
            if !query.is_empty() && !item.label.to_lowercase().contains(&query.to_lowercase()) {
                continue;
            }
            let y = pane.origin.y + 4.0 + (*idx as f32) * row_h;
            *idx += 1;
            if y + row_h > pane.max().y {
                continue;
            }
            let item_r = Rect::new(
                pane.origin.x + 4.0,
                y,
                (pane.width() - 8.0).max(8.0),
                row_h - 2.0,
            );
            let item_key = transfer_item_node(node, item.key);
            let mut item_a11y =
                A11yNode::new(Role::CheckBox, item.label.clone(), item_r).clickable();
            item_a11y.selected = item.checked;
            out.with_sub_control(
                item_key,
                HitRegion::new(item_key, CONTROL_LAYER, u32::from(desc.z) + 2, item_r)
                    .with_cursor(CursorKind::Pointer),
                item_a11y,
                false,
            );
        }
    }

    out
}

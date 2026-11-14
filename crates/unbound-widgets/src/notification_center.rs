//! NotificationCenter 抽屉，聚合 Toast 历史、严重级别过滤与已读状态。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_ITEMS: usize = 4;

/// 单条已归档的通知项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationItem {
    /// 稳定 ID。
    pub id: u64,
    /// 标题文本。
    pub title: String,
    /// 严重级别。
    pub severity: Severity,
    /// 是否已读。
    pub read: bool,
}

/// [`NotificationCenterDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationCenterState {
    /// 已归档的通知列表。
    pub items: Vec<NotificationItem>,
    /// 可选严重级别过滤器。
    pub filter: Option<Severity>,
}

impl NotificationCenterState {
    /// 将所有通知标记为已读。
    pub fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.read = true;
        }
    }

    /// 清空所有通知。
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 统计未读项数量。
    #[must_use]
    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|i| !i.read).count()
    }
}

/// [`NotificationCenter`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationCenterDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 可见项列表。
    pub items: Vec<NotificationItem>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl NotificationCenterDesc {
    /// 创建通知中心描述符。
    #[must_use]
    pub fn new(rect: Rect, items: Vec<NotificationItem>) -> Self {
        Self {
            rect,
            items,
            label: "Notification Center".to_owned(),
            z: 600,
        }
    }
}

/// 通知卡片项的确定性子节点 ID（`0..4`）。
#[must_use]
pub const fn notification_item_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xC00 + index as u64)
}

/// 输出结构稳定的通知中心（1 个背景 + 4 张项卡片 + 4 个未读圆点 + 1 个焦点环 = 10 个图元）。
#[must_use]
pub fn emit_notification_center(
    node: u64,
    desc: &NotificationCenterDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(10 + label_glyphs.len());
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
    let row_h = ((desc.rect.height() - pad * 5.0) / MAX_ITEMS as f32).max(12.0);
    for i in 0..MAX_ITEMS {
        let item = desc.items.get(i);
        let r = Rect::new(
            desc.rect.origin.x + pad,
            desc.rect.origin.y + pad + i as f32 * (row_h + pad),
            (desc.rect.width() - pad * 2.0).max(16.0),
            row_h,
        );
        let mut card = outlined(
            r,
            5.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE_SUNKEN
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        );
        if item.is_none() {
            card.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(card.depth_u16(), 0.0);
        }
        primitives.push(card);

        let dot_r = Rect::new(r.max().x - 14.0, r.origin.y + (row_h - 6.0) * 0.5, 6.0, 6.0);
        let unread = item.is_some_and(|it| !it.read);
        let dot_tok = match item.map(|it| it.severity).unwrap_or(Severity::Info) {
            Severity::Error => Token::ERROR,
            Severity::Warning => Token::WARNING,
            Severity::Success => Token::SUCCESS,
            Severity::Info => Token::PRIMARY,
        };
        let mut dot = surface(dot_r, 3.0, dot_tok, desc.z + 2);
        if !unread {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
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
    let a11y = A11yNode::new(Role::Region, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (i, item) in desc.items.iter().take(MAX_ITEMS).enumerate() {
            let r = Rect::new(
                desc.rect.origin.x + pad,
                desc.rect.origin.y + pad + i as f32 * (row_h + pad),
                (desc.rect.width() - pad * 2.0).max(16.0),
                row_h,
            );
            let key = notification_item_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::ListItem, item.title.clone(), r).clickable(),
                true,
            );
        }
    }

    out
}

//! 自动补全 / 提及输入框（AutoComplete / MentionInput），支持前缀过滤与建议浮层指示。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// [`AutoCompleteDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoCompleteState {
    /// 当前查询文本。
    pub query: String,
    /// 当前高亮的建议项索引。
    pub highlighted: usize,
    /// 建议列表是否展开。
    pub open: bool,
}

impl AutoCompleteState {
    /// 按 `query` 对 `candidates` 进行大小写不敏感过滤。
    #[must_use]
    pub fn filter<'a>(&self, candidates: &'a [String]) -> Vec<&'a str> {
        let q = self.query.to_ascii_lowercase();
        candidates
            .iter()
            .filter(|c| c.to_ascii_lowercase().contains(&q))
            .map(String::as_str)
            .collect()
    }
}

/// 自动补全建议项的确定性子节点 ID。
#[must_use]
pub const fn autocomplete_option_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xB00 + index as u64)
}

/// [`AutoComplete`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AutoCompleteDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 当前输入值。
    pub value: String,
    /// 匹配的建议数量。
    pub suggestion_count: usize,
    /// `open` 为 true 时渲染的可选建议标签。
    pub suggestions: Vec<String>,
    /// 建议浮层是否展开。
    pub open: bool,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl AutoCompleteDesc {
    /// 创建自动补全描述符。
    #[must_use]
    pub fn new(rect: Rect, value: impl Into<String>, suggestion_count: usize) -> Self {
        Self {
            rect,
            value: value.into(),
            suggestion_count,
            suggestions: Vec::new(),
            open: false,
            label: "AutoComplete".to_owned(),
            z: 600,
        }
    }

    /// 附加建议标签与展开状态。
    #[must_use]
    pub fn with_suggestions(mut self, suggestions: Vec<String>, open: bool) -> Self {
        self.suggestion_count = suggestions.len();
        self.suggestions = suggestions;
        self.open = open;
        self
    }
}

/// 发射固定形状的自动补全输入框（1 输入框 + 1 建议徽标 + 1 焦点环 = 3 个图元）。
#[must_use]
pub fn emit_autocomplete(
    node: u64,
    desc: &AutoCompleteDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE
        },
        if state.disabled {
            Token::DISABLED
        } else if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let badge_r = Rect::new(
        desc.rect.max().x - 18.0,
        desc.rect.origin.y + (desc.rect.height() - 10.0) * 0.5,
        10.0,
        10.0,
    );
    let mut badge = surface(badge_r, 5.0, Token::PRIMARY, desc.z + 1);
    if desc.suggestion_count == 0 {
        badge.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(badge.depth_u16(), 0.0);
    }
    primitives.push(badge);

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Text)
    });
    let mut a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = desc.value.clone();

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if desc.open && !state.disabled {
        let count = desc.suggestions.len().min(6);
        let row_h = 26.0;
        if count > 0 {
            let popup_r = Rect::new(
                desc.rect.origin.x,
                desc.rect.max().y + 4.0,
                desc.rect.width(),
                count as f32 * row_h + 8.0,
            );
            out.primitives.push(outlined(
                popup_r,
                6.0,
                Token::SURFACE_RAISED,
                Token::CARD_BORDER,
                1.0,
                6.0,
                desc.z + 4,
            ));
            for (idx, label) in desc.suggestions.iter().take(count).enumerate() {
                let row_r = Rect::new(
                    popup_r.origin.x + 4.0,
                    popup_r.origin.y + 4.0 + idx as f32 * row_h,
                    (popup_r.width() - 8.0).max(16.0),
                    row_h - 2.0,
                );
                out.primitives
                    .push(surface(row_r, 4.0, Token::SURFACE, desc.z + 5));
                let key = autocomplete_option_node(node, idx);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 5, row_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::ListItem, label.clone(), row_r).clickable(),
                    true,
                );
            }
        }
    }

    out
}

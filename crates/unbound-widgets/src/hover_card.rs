//! 富文本悬停卡片（HoverCard）与空状态（EmptyState）。
//!
//! 提示条（Tooltip）仅展示信息且穿透指针事件；悬停卡片则是其可交互的兄弟组件：
//! 指针可以移*入*卡片内部并操作其中内容。这改变了计时模型——离开触发器不再立即隐藏卡片，
//! 而是启动宽限期，仅当指针同时离开触发器与卡片后才到期隐藏。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::overlay::{place_anchored_popup, OverlayEntry, PlacedPopup, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

/// 指针必须在触发器上停留的时长。
pub const HOVER_CARD_DELAY_MS: f32 = 500.0;
/// 指针同时离开两个区域后卡片继续保留的时长。
pub const HOVER_CARD_GRACE_MS: f32 = 150.0;
/// 卡片圆角半径。
pub const HOVER_CARD_RADIUS: f32 = 10.0;
const HOVER_CARD_SHADOW: f32 = 22.0;

/// 悬停卡片描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct HoverCardDesc {
    /// 触发器矩形。
    pub anchor: Rect,
    /// 卡片需保持在内的视口。
    pub viewport: Rect,
    /// 测量后的卡片尺寸。
    pub size: Vec2,
    /// 卡片首选停靠方向。
    pub placement: Placement,
    /// 显示前的延迟时长。
    pub delay_ms: f32,
    /// 指针离开后的宽限期。
    pub grace_ms: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl HoverCardDesc {
    /// 创建位于触发器下方的悬停卡片。
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::BottomStart,
            delay_ms: HOVER_CARD_DELAY_MS,
            grace_ms: HOVER_CARD_GRACE_MS,
            radius: HOVER_CARD_RADIUS,
            z: 935,
        }
    }

    /// 将卡片放置到另一侧。
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// 放置后的卡片矩形。
    #[must_use]
    pub fn rect(&self) -> Rect {
        let placed: PlacedPopup =
            place_anchored_popup(self.anchor, self.size, self.viewport, self.placement);
        placed.rect
    }
}

/// 单个悬停卡片的打开/关闭计时状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverCardState {
    visible: bool,
    anchor: Option<u64>,
    now_ms: f32,
    hovered_since: Option<f32>,
    hide_at: Option<f32>,
}

impl HoverCardState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 卡片当前是否显示。
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 推进一帧。
    ///
    /// `hovered` 为指针下的触发器节点，`over_card` 表示指针是否在卡片内。返回可见性是否发生变化。
    pub fn update(
        &mut self,
        desc: &HoverCardDesc,
        hovered: Option<u64>,
        over_card: bool,
        dt_ms: f32,
    ) -> bool {
        let was = self.visible;
        let previous = self.now_ms;
        self.now_ms += dt_ms.max(0.0);

        match hovered {
            Some(node) => {
                if self.anchor != Some(node) {
                    // 切换到不同触发器会重新开始延迟并隐藏前一张卡片：其内容已不再适用。
                    self.anchor = Some(node);
                    self.visible = false;
                    self.hide_at = None;
                    self.hovered_since = Some(previous);
                }
            }
            None => {
                if self.anchor.is_some() {
                    self.anchor = None;
                    self.hovered_since = None;
                }
            }
        }

        if !self.visible {
            if let Some(since) = self.hovered_since {
                if self.anchor.is_some() && self.now_ms - since >= desc.delay_ms.max(0.0) {
                    self.visible = true;
                }
            }
        }

        if self.visible {
            if hovered.is_some() || over_card {
                self.hide_at = None;
            } else {
                let hide_at = *self
                    .hide_at
                    .get_or_insert(previous + desc.grace_ms.max(0.0));
                if self.now_ms >= hide_at {
                    self.visible = false;
                    self.hide_at = None;
                    self.hovered_since = None;
                    self.anchor = None;
                }
            }
        }

        self.visible != was
    }

    /// 立即隐藏。
    pub fn hide(&mut self) {
        self.visible = false;
        self.anchor = None;
        self.hovered_since = None;
        self.hide_at = None;
    }
}

/// 发射悬停卡片。输出**确实**携带命中区域：卡片可交互，这正是它与提示条的区别。
#[must_use]
pub fn emit_hover_card(
    node: u64,
    anchor_node: u64,
    desc: &HoverCardDesc,
    card_glyphs: &[GlyphQuad],
    state: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect();
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            [desc.radius; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            HOVER_CARD_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::OVERLAY_BG, desc.z),
        instance(
            rect,
            [desc.radius; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads = Vec::with_capacity(card_glyphs.len());
    crate::primitive::glyphs(
        card_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );

    let a11y = A11yNode::new(Role::Group, String::new(), rect).disabled(state.disabled);

    (
        WidgetOutput {
            primitives,
            payloads,
            hit: Some(
                HitRegion::new(node, PORTAL_LAYER, u32::from(desc.z), rect)
                    .with_cursor(CursorKind::Default),
            ),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        OverlayEntry::popover(node, anchor_node, rect),
    )
}

/// 空状态的图标直径。
pub const EMPTY_ICON: f32 = 48.0;
/// 操作按钮尺寸。
pub const EMPTY_ACTION_SIZE: (f32, f32) = (120.0, 32.0);

/// 居中空列表占位符。
#[derive(Clone, Debug, PartialEq)]
pub struct EmptyStateDesc {
    /// 占位符在其中居中的区域。
    pub rect: Rect,
    /// 主标题。
    pub title: String,
    /// 辅助说明文案。
    pub message: String,
    /// 可选行动号召按钮文案。
    pub action_label: Option<String>,
    /// 绘制顺序键。
    pub z: u16,
}

impl EmptyStateDesc {
    /// 创建带标题的空状态。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            message: String::new(),
            action_label: None,
            z: 520,
        }
    }

    /// 附加辅助说明文案。
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// 附加行动号召按钮。
    #[must_use]
    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action_label = Some(label.into());
        self
    }

    /// 用于居中计算的内容块高度。
    fn content_height(&self) -> f32 {
        let mut height = EMPTY_ICON + 12.0 + 24.0;
        if !self.message.is_empty() {
            height += 6.0 + 20.0;
        }
        if self.action_label.is_some() {
            height += 16.0 + EMPTY_ACTION_SIZE.1;
        }
        height
    }

    /// 居中内容块的顶部坐标。
    fn content_top(&self) -> f32 {
        self.rect.origin.y + (self.rect.size.y - self.content_height()).max(0.0) * 0.5
    }

    /// 图标正方形区域。
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        Rect::new(
            self.rect.center().x - EMPTY_ICON * 0.5,
            self.content_top(),
            EMPTY_ICON,
            EMPTY_ICON,
        )
    }

    /// 主标题区域。
    #[must_use]
    pub fn title_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.icon_rect().max().y + 12.0,
            self.rect.size.x,
            24.0,
        )
    }

    /// 辅助说明文案区域。
    #[must_use]
    pub fn message_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.title_rect().max().y + 6.0,
            self.rect.size.x,
            20.0,
        )
    }

    /// 行动号召按钮区域（若存在）。
    #[must_use]
    pub fn action_rect(&self) -> Option<Rect> {
        self.action_label.as_ref().map(|_| {
            let top = if self.message.is_empty() {
                self.title_rect().max().y + 16.0
            } else {
                self.message_rect().max().y + 16.0
            };
            Rect::new(
                self.rect.center().x - EMPTY_ACTION_SIZE.0 * 0.5,
                top,
                EMPTY_ACTION_SIZE.0,
                EMPTY_ACTION_SIZE.1,
            )
        })
    }
}

/// 单个空状态的发射结果。
#[derive(Clone, Debug)]
pub struct EmptyStateOutput {
    /// 图标、字形与操作按钮表面。
    pub output: WidgetOutput,
    /// 操作按钮命中区域（若存在）。
    pub action: Option<HitRegion>,
}

/// 发射居中空状态。
#[must_use]
pub fn emit_empty_state(
    node: u64,
    desc: &EmptyStateDesc,
    empty_glyphs: &[GlyphQuad],
    control: ControlState,
) -> EmptyStateOutput {
    let icon = desc.icon_rect();
    let mut primitives = Vec::with_capacity(5 + empty_glyphs.len());
    primitives.push(surface(
        icon,
        icon.size.x * 0.5,
        Token::SURFACE_SUNKEN,
        desc.z,
    ));
    primitives.push(surface(
        Rect::new(
            icon.origin.x + 12.0,
            icon.origin.y + 16.0,
            icon.size.x - 24.0,
            icon.size.y - 28.0,
        ),
        3.0,
        Token::PRIMARY_SOFT,
        desc.z + 1,
    ));
    let mut payloads = Vec::with_capacity(empty_glyphs.len());
    crate::primitive::glyphs(
        empty_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );

    let action = desc.action_rect();
    if let Some(action) = action {
        primitives.push(surface(action, 8.0, Token::PRIMARY_SOFT, desc.z + 1));
        primitives.push(crate::primitive::focus_ring(
            action,
            8.0,
            Token::FOCUS_RING,
            2.0,
            desc.z + 2,
            control.focused,
        ));
    }

    let mut a11y = A11yNode::new(Role::Group, desc.title.clone(), desc.rect);
    a11y.description = desc.message.clone();

    EmptyStateOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        action: action.map(|action| {
            HitRegion::new(node, PORTAL_LAYER, u32::from(desc.z), action)
                .with_cursor(CursorKind::Pointer)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    fn card() -> HoverCardDesc {
        HoverCardDesc::new(
            Rect::new(200.0, 200.0, 120.0, 32.0),
            viewport(),
            Vec2::new(240.0, 160.0),
        )
    }

    #[test]
    fn the_card_waits_for_the_delay_and_shows_once() {
        let desc = card();
        let mut state = HoverCardState::new();
        assert!(!state.update(&desc, Some(7), false, 499.0));
        assert!(!state.visible());
        assert!(state.update(&desc, Some(7), false, 2.0));
        assert!(state.visible());
        assert!(!state.update(&desc, Some(7), false, 16.0));
    }

    #[test]
    fn leaving_the_trigger_before_the_delay_cancels_it() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 300.0);
        state.update(&desc, None, false, 16.0);
        assert!(!state.update(&desc, None, false, 500.0));
        assert!(!state.visible());
    }

    #[test]
    fn moving_into_the_card_keeps_it_open() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 500.0);
        assert!(state.visible());
        // 指针离开触发器并进入卡片内部。
        state.update(&desc, None, true, 16.0);
        assert!(!state.update(&desc, None, true, 500.0));
        assert!(state.visible());
    }

    #[test]
    fn leaving_both_waits_for_the_grace_period() {
        let desc = card();
        let mut state = HoverCardState::new();
        state.update(&desc, Some(7), false, 500.0);
        state.update(&desc, None, false, 100.0);
        assert!(state.visible());
        assert!(state.update(&desc, None, false, 60.0));
        assert!(!state.visible());
    }

    #[test]
    fn the_card_stays_inside_the_viewport() {
        let mut desc = card();
        desc.anchor = Rect::new(1200.0, 690.0, 60.0, 24.0);
        let rect = desc.rect();
        assert!(viewport().contains_rect(&rect));
    }

    #[test]
    fn the_card_is_interactive_unlike_a_tooltip() {
        let desc = card();
        let (output, entry) = emit_hover_card(9, 4, &desc, &[], ControlState::default());
        let hit = output.hit.expect("interactive hit region");
        assert_eq!(hit.node, 9);
        assert_eq!(hit.cursor, Some(CursorKind::Default));
        assert_eq!(entry.anchor, Some(4));
        assert_eq!(output.primitives.len(), 3);
    }

    fn empty() -> EmptyStateDesc {
        EmptyStateDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0), "Nothing here yet")
            .with_message("Create your first item to begin.")
            .with_action("Create")
    }

    #[test]
    fn the_empty_state_is_vertically_centred() {
        let desc = empty();
        let icon = desc.icon_rect();
        assert_eq!(icon.center().x, desc.rect.center().x);
        let top_gap = icon.origin.y - desc.rect.origin.y;
        let bottom_gap = desc.rect.max().y - desc.action_rect().unwrap().max().y;
        assert!(
            (top_gap - bottom_gap).abs() < 0.5,
            "{top_gap} vs {bottom_gap}"
        );
    }

    #[test]
    fn the_empty_state_without_an_action_has_no_hit_region() {
        let desc = EmptyStateDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0), "Empty");
        let output = emit_empty_state(3, &desc, &[], ControlState::default());
        assert!(output.action.is_none());
        assert!(output.output.hit.is_none());
        assert_eq!(output.output.a11y.map(|node| node.role), Some(Role::Group));
    }

    #[test]
    fn the_empty_state_action_is_clickable() {
        let desc = empty();
        let output = emit_empty_state(3, &desc, &[], ControlState::default());
        let action = output.action.expect("action");
        assert_eq!(action.bounds, desc.action_rect().unwrap());
        assert_eq!(action.cursor, Some(CursorKind::Pointer));
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = empty();
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_empty_state(3, &desc, &[], control);
                        counts.push(output.output.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

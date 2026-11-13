//! The tab strip: a horizontally scrolling row of tabs with an animated
//! selection indicator.
//!
//! The strip owns identity and geometry, never content. Each tab is emitted
//! by its own call so that the close affordance can be a separate hit region,
//! and so that a caller can put any label quads it likes inside the tab.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_anim::Spring;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default width of one tab, in logical pixels.
pub const TAB_WIDTH: f32 = 140.0;

/// The default strip height.
pub const TAB_STRIP_HEIGHT: f32 = 36.0;

/// The width of the close affordance.
pub const TAB_CLOSE: f32 = 16.0;

/// One tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabItem {
    /// Stable identity, also used as the tab's node.
    pub id: u64,
    /// The visible title.
    pub title: String,
    /// Whether this tab has a close button.
    pub closable: bool,
}

impl TabItem {
    /// A closable tab.
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            closable: true,
        }
    }

    /// The same tab without a close button.
    #[must_use]
    pub fn fixed(mut self) -> Self {
        self.closable = false;
        self
    }
}

/// The pure descriptor of a tab strip.
#[derive(Clone, Debug, PartialEq)]
pub struct TabsDesc {
    /// The strip rectangle.
    pub rect: Rect,
    /// The tabs, left to right.
    pub items: Vec<TabItem>,
    /// Whether every tab gets a close button.
    pub closable: bool,
    /// Whether the trailing "+" button is shown.
    pub addable: bool,
    /// The width of one tab.
    pub tab_width: f32,
    /// The draw order key.
    pub z: u16,
}

impl TabsDesc {
    /// A strip with the given titles.
    #[must_use]
    pub fn new(rect: Rect, titles: &[&str]) -> Self {
        Self {
            rect,
            items: titles
                .iter()
                .enumerate()
                .map(|(index, title)| TabItem::new(index as u64 + 1, *title))
                .collect(),
            closable: true,
            addable: false,
            tab_width: TAB_WIDTH,
            z: 620,
        }
    }

    /// Use explicit tab identities and titles.
    #[must_use]
    pub fn with_items(mut self, items: Vec<TabItem>) -> Self {
        self.items = items;
        self
    }

    /// Show or hide the trailing "+" button.
    #[must_use]
    pub fn with_addable(mut self, addable: bool) -> Self {
        self.addable = addable;
        self
    }

    /// Override the tab width.
    #[must_use]
    pub fn with_tab_width(mut self, width: f32) -> Self {
        self.tab_width = width.max(24.0);
        self
    }

    /// How many tabs the strip has.
    #[must_use]
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// The width available to the tabs, the add button excluded.
    #[must_use]
    pub fn strip_width(&self) -> f32 {
        let add = if self.addable { TAB_STRIP_HEIGHT } else { 0.0 };
        (self.rect.size.x - add).max(0.0)
    }

    /// The furthest the strip can scroll before the last tab is flush right.
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.count() as f32 * self.tab_width - self.strip_width()).max(0.0)
    }

    /// The unclipped rectangle of one tab at a given scroll offset.
    #[must_use]
    pub fn raw_tab_rect(&self, index: usize, scroll: f32) -> Rect {
        Rect::new(
            self.rect.origin.x + index as f32 * self.tab_width - scroll,
            self.rect.origin.y,
            self.tab_width,
            self.rect.size.y,
        )
    }

    /// The visible rectangle of one tab, clipped to the strip.
    #[must_use]
    pub fn tab_rect(&self, index: usize, scroll: f32) -> Rect {
        let raw = self.raw_tab_rect(index, scroll);
        let strip = Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.strip_width(),
            self.rect.size.y,
        );
        raw.intersect(&strip)
    }

    /// The close affordance of one tab, if it has one.
    #[must_use]
    pub fn close_rect(&self, index: usize, scroll: f32) -> Option<Rect> {
        let tab = self.items.get(index)?;
        if !(self.closable && tab.closable) {
            return None;
        }
        let visible = self.tab_rect(index, scroll);
        if visible.size.x < TAB_CLOSE * 2.0 {
            return None;
        }
        Some(Rect::new(
            visible.max().x - TAB_CLOSE - 6.0,
            visible.origin.y + (visible.size.y - TAB_CLOSE) * 0.5,
            TAB_CLOSE,
            TAB_CLOSE,
        ))
    }

    /// The trailing "+" button.
    #[must_use]
    pub fn add_rect(&self) -> Option<Rect> {
        self.addable.then(|| {
            Rect::new(
                self.rect.max().x - TAB_STRIP_HEIGHT,
                self.rect.origin.y,
                TAB_STRIP_HEIGHT,
                self.rect.size.y,
            )
        })
    }
}

/// The live state of a tab strip.
#[derive(Clone, Debug, PartialEq)]
pub struct TabsState {
    /// The selected tab.
    pub active: usize,
    /// The animated left edge of the indicator.
    pub indicator_x: Spring,
    /// The animated width of the indicator.
    pub indicator_w: Spring,
    /// The horizontal scroll offset of the strip.
    pub scroll_x: f32,
    /// Whether the springs have been placed at their resting target.
    initialised: bool,
}

impl Default for TabsState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TabsState {
    /// A strip with the given tab selected.
    #[must_use]
    pub fn new(active: usize) -> Self {
        Self {
            active,
            indicator_x: Spring::new(unbound_anim::SpringPreset::Precise, 0.0),
            indicator_w: Spring::new(unbound_anim::SpringPreset::Precise, 0.0),
            scroll_x: 0.0,
            initialised: false,
        }
    }

    /// The active tab, clamped to the current tab count.
    #[must_use]
    pub fn active_index(&self, desc: &TabsDesc) -> usize {
        self.active.min(desc.count().saturating_sub(1))
    }

    /// The resting indicator geometry for the active tab.
    #[must_use]
    pub fn indicator_target(&self, desc: &TabsDesc) -> (f32, f32) {
        let index = self.active_index(desc);
        let rect = desc.tab_rect(index, self.scroll_x);
        (rect.origin.x, rect.size.x)
    }

    /// Place the indicator at rest, with no animation.
    pub fn snap(&mut self, desc: &TabsDesc) {
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.snap_to(x);
        self.indicator_w.snap_to(width);
        self.initialised = true;
    }

    /// Aim the springs at the active tab.
    ///
    /// Call once per frame before [`Self::advance`]; the caller reads
    /// [`Self::is_animating`] to decide whether it needs another frame.
    pub fn sync(&mut self, desc: &TabsDesc) {
        if !self.initialised {
            self.snap(desc);
            return;
        }
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.set_target(x);
        self.indicator_w.set_target(width);
    }

    /// Step the springs by `dt` seconds. Returns whether they still move.
    pub fn advance(&mut self, dt: f32) -> bool {
        let moving_x = self.indicator_x.step(dt);
        let moving_w = self.indicator_w.step(dt);
        moving_x || moving_w
    }

    /// Whether the indicator is still moving.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.indicator_x.is_settled() || !self.indicator_w.is_settled()
    }

    /// The animated indicator rectangle.
    #[must_use]
    pub fn indicator_rect(&self, desc: &TabsDesc) -> Rect {
        Rect::new(
            self.indicator_x.value,
            desc.rect.max().y - 2.0,
            self.indicator_w.value.max(0.0),
            2.0,
        )
    }

    /// Activate a tab and reveal it. Returns whether the selection changed.
    pub fn activate(&mut self, desc: &TabsDesc, index: usize) -> bool {
        if index >= desc.count() || index == self.active_index(desc) {
            return false;
        }
        self.active = index;
        self.reveal(desc, index);
        true
    }

    /// Scroll so one tab is fully visible.
    pub fn reveal(&mut self, desc: &TabsDesc, index: usize) {
        let left = index as f32 * desc.tab_width;
        let right = left + desc.tab_width;
        let width = desc.strip_width();
        if left < self.scroll_x {
            self.scroll_x = left;
        } else if right > self.scroll_x + width {
            self.scroll_x = right - width;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, desc.max_scroll());
    }

    /// Scroll by a wheel delta.
    pub fn scroll_by(&mut self, desc: &TabsDesc, delta: f32) -> bool {
        let next = (self.scroll_x + delta).clamp(0.0, desc.max_scroll());
        let changed = (next - self.scroll_x).abs() > 1e-4;
        self.scroll_x = next;
        changed
    }
}

/// The keys a focused tab strip understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsKey {
    /// Select the previous tab, wrapping.
    Left,
    /// Select the next tab, wrapping.
    Right,
    /// Select the first tab.
    Home,
    /// Select the last tab.
    End,
}

/// Reduce one key press into the next active index.
#[must_use]
pub fn reduce_tabs_key(desc: &TabsDesc, active: usize, key: TabsKey) -> usize {
    let count = desc.count();
    if count == 0 {
        return 0;
    }
    match key {
        TabsKey::Left => (active + count - 1) % count,
        TabsKey::Right => (active + 1) % count,
        TabsKey::Home => 0,
        TabsKey::End => count - 1,
    }
}

/// The colour slots a tab strip paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabsTokens {
    /// The strip background.
    pub strip: Token,
    /// The strip hairline.
    pub border: Token,
    /// An idle tab.
    pub tab: Token,
    /// A hovered tab.
    pub tab_hover: Token,
    /// The active tab.
    pub tab_active: Token,
    /// The text of an idle tab.
    pub tab_text: Token,
    /// The text of the active tab.
    pub tab_text_active: Token,
    /// The selection indicator.
    pub indicator: Token,
    /// The close glyph.
    pub close: Token,
    /// The close glyph while hovered.
    pub close_hover: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl TabsTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            strip: Token::SURFACE,
            border: Token::OUTLINE,
            tab: Token::NONE,
            tab_hover: Token::CONTROL_HOVER,
            tab_active: Token::CARD_BG,
            tab_text: Token::TEXT_SECONDARY,
            tab_text_active: Token::TEXT_PRIMARY,
            indicator: Token::PRIMARY,
            close: Token::TEXT_SECONDARY,
            close_hover: Token::TEXT_PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit the strip background, its hairline and the animated indicator.
#[must_use]
pub fn emit_tabs_strip(desc: &TabsDesc, state: &TabsState, tokens: TabsTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    primitives.push(surface(desc.rect, 0.0, tokens.strip, desc.z));
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
    if desc.count() > 0 {
        primitives.push(surface(
            state.indicator_rect(desc),
            1.0,
            tokens.indicator,
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

/// Emit one tab.
#[must_use]
pub fn emit_tab(
    desc: &TabsDesc,
    state: &TabsState,
    index: usize,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(item) = desc.items.get(index) else {
        return WidgetOutput::default();
    };
    let rect = desc.tab_rect(index, state.scroll_x);
    if rect.size.x <= 0.0 {
        return WidgetOutput::default();
    }
    let active = index == state.active_index(desc);
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    let fill = if active {
        tokens.tab_active
    } else if control.phase == PointerPhase::Hovered {
        tokens.tab_hover
    } else {
        tokens.tab
    };
    primitives.push(outlined(rect, 0.0, fill, Token::NONE, 0.0, 0.0, desc.z));
    glyphs(
        label,
        if active {
            tokens.tab_text_active
        } else {
            tokens.tab_text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        2.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(item.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [0.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Tab, item.title.clone(), rect)
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

/// Emit the close affordance of one tab.
///
/// The close button is its own node so that a click on "×" cannot be
/// mistaken for a click on the tab, and so that assistive technology can
/// announce it separately.
#[must_use]
pub fn emit_tab_close(
    node: u64,
    desc: &TabsDesc,
    state: &TabsState,
    index: usize,
    control: ControlState,
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(rect) = desc.close_rect(index, state.scroll_x) else {
        return WidgetOutput::default();
    };
    let disabled = control.disabled;
    let color = if control.phase == PointerPhase::Hovered {
        tokens.close_hover
    } else {
        tokens.close
    };
    let mut primitives = Vec::with_capacity(3);
    let inset = rect.size.x * 0.3;
    let thickness = 1.5;
    for (start, end) in [
        (
            [rect.origin.x + inset, rect.origin.y + inset],
            [rect.max().x - inset, rect.max().y - inset],
        ),
        (
            [rect.max().x - inset, rect.origin.y + inset],
            [rect.origin.x + inset, rect.max().y - inset],
        ),
    ] {
        primitives.push(stroke_between(start, end, thickness, color, desc.z));
    }
    primitives.push(focus_ring(
        rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        1.5,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let title = desc
        .items
        .get(index)
        .map(|item| format!("Close {}", item.title))
        .unwrap_or_else(|| "Close".to_owned());
    WidgetOutput {
        primitives,
        payloads: Vec::<GpuPayload>::new(),
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, title, rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// Emit the trailing "+" button.
#[must_use]
pub fn emit_tabs_add(
    node: u64,
    desc: &TabsDesc,
    control: ControlState,
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(rect) = desc.add_rect() else {
        return WidgetOutput::default();
    };
    let disabled = control.disabled;
    let fill = if control.phase == PointerPhase::Hovered {
        tokens.tab_hover
    } else {
        tokens.tab
    };
    let color = if disabled {
        Token::TEXT_DISABLED
    } else {
        tokens.tab_text
    };
    let mut primitives = Vec::with_capacity(4);
    primitives.push(outlined(rect, 0.0, fill, Token::NONE, 0.0, 0.0, desc.z));
    let centre = rect.size.x * 0.5;
    let arm = rect.size.x * 0.18;
    let cx = rect.origin.x + centre;
    let cy = rect.origin.y + rect.size.y * 0.5;
    primitives.push(stroke_between(
        [cx - arm, cy],
        [cx + arm, cy],
        1.5,
        color,
        desc.z,
    ));
    primitives.push(stroke_between(
        [cx, cy - arm],
        [cx, cy + arm],
        1.5,
        color,
        desc.z,
    ));
    primitives.push(focus_ring(
        rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        1.5,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, "New tab", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// A thin stroke between two points, packed as a line primitive.
fn stroke_between(
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: Token,
    z: u16,
) -> GpuPrimitiveInstance {
    let min_x = start[0].min(end[0]);
    let min_y = start[1].min(end[1]);
    let width = (start[0] - end[0]).abs().max(thickness);
    let height = (start[1] - end[1]).abs().max(thickness);
    instance(
        Rect::new(min_x, min_y, width, height),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TabsDesc {
        TabsDesc::new(Rect::new(0.0, 0.0, 300.0, 36.0), &["One", "Two", "Three"])
    }

    #[test]
    fn tabs_are_laid_out_left_to_right() {
        let desc = desc();
        assert_eq!(desc.tab_rect(0, 0.0).origin.x, 0.0);
        assert_eq!(desc.tab_rect(1, 0.0).origin.x, TAB_WIDTH);
        assert_eq!(desc.tab_rect(2, 0.0).origin.x, TAB_WIDTH * 2.0);
    }

    #[test]
    fn a_tab_past_the_strip_is_clipped_away() {
        let desc = desc();
        // 300 px wide, 140 px tabs: tab 2 starts at 280 and is 20 px visible.
        let third = desc.tab_rect(2, 0.0);
        assert_eq!(third.origin.x, 280.0);
        assert_eq!(third.size.x, 20.0);
    }

    #[test]
    fn scrolling_reveals_the_last_tab() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        assert_eq!(desc.max_scroll(), 120.0);
        assert!(state.scroll_by(&desc, 500.0));
        assert_eq!(state.scroll_x, 120.0);
        let third = desc.tab_rect(2, state.scroll_x);
        assert_eq!(third.origin.x, 160.0);
        assert_eq!(third.size.x, 140.0);
    }

    #[test]
    fn activating_a_tab_reveals_it() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        assert!(state.activate(&desc, 2));
        assert_eq!(state.scroll_x, 120.0);
        assert!(!state.activate(&desc, 2));
    }

    #[test]
    fn the_indicator_snaps_and_then_animates() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        let rest = state.indicator_rect(&desc);
        assert_eq!(rest.origin.x, 0.0);
        assert_eq!(rest.size.x, TAB_WIDTH);

        state.activate(&desc, 1);
        state.sync(&desc);
        assert!(state.is_animating());
        for _ in 0..120 {
            state.advance(1.0 / 60.0);
        }
        let moved = state.indicator_rect(&desc);
        assert!((moved.origin.x - TAB_WIDTH).abs() < 1.0);
        assert!((moved.size.x - TAB_WIDTH).abs() < 1.0);
    }

    #[test]
    fn the_indicator_clamps_when_the_active_tab_leaves_the_strip() {
        let desc = desc();
        let mut state = TabsState::new(2);
        state.snap(&desc);
        let indicator = state.indicator_rect(&desc);
        assert_eq!(indicator.origin.x, 280.0);
        assert_eq!(indicator.size.x, 20.0);
    }

    #[test]
    fn the_close_affordance_only_exists_when_the_tab_is_wide_enough() {
        let desc = desc();
        assert!(desc.close_rect(0, 0.0).is_some());
        assert!(desc.close_rect(2, 0.0).is_none(), "20 px is too narrow");
        let wide = desc.clone().with_tab_width(80.0);
        assert!(wide.close_rect(2, 0.0).is_some());
    }

    #[test]
    fn the_add_button_sits_at_the_trailing_edge() {
        let desc = desc().with_addable(true);
        let add = desc.add_rect().expect("addable");
        assert_eq!(add.origin.x, 264.0);
        assert_eq!(desc.strip_width(), 264.0);
    }

    #[test]
    fn arrow_keys_wrap_and_home_end_jump() {
        let desc = desc();
        assert_eq!(reduce_tabs_key(&desc, 0, TabsKey::Left), 2);
        assert_eq!(reduce_tabs_key(&desc, 2, TabsKey::Right), 0);
        assert_eq!(reduce_tabs_key(&desc, 1, TabsKey::Home), 0);
        assert_eq!(reduce_tabs_key(&desc, 1, TabsKey::End), 2);
    }

    #[test]
    fn a_tab_is_pickable_and_the_active_tab_is_selected() {
        let desc = desc();
        let state = TabsState::new(1);
        let tokens = TabsTokens::standard();
        let output = emit_tab(&desc, &state, 1, ControlState::default(), &[], tokens);
        let hit = output.hit.expect("a tab is pickable");
        assert_eq!(hit.node, 2);
        let a11y = output.a11y.expect("a tab is reachable");
        assert_eq!(a11y.role, Role::Tab);
        assert!(a11y.selected);
        assert_eq!(a11y.label, "Two");
    }

    #[test]
    fn a_hovered_tab_paints_the_hover_fill() {
        let desc = desc();
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let output = emit_tab(
            &desc,
            &state,
            1,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.tab_hover.raw() as u32
        );
    }

    #[test]
    fn the_close_button_is_its_own_control() {
        let desc = desc();
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let output = emit_tab_close(77, &desc, &state, 0, ControlState::default(), tokens);
        let hit = output.hit.expect("the close button is pickable");
        assert_eq!(hit.node, 77);
        let a11y = output.a11y.expect("the close button is reachable");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(a11y.label, "Close One");
    }

    #[test]
    fn the_strip_paints_the_indicator() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        let output = emit_tabs_strip(&desc, &state, TabsTokens::standard());
        assert_eq!(output.primitives.len(), 3);
    }
}

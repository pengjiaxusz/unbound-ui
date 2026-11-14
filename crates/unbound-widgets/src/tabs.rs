//! 标签页栏：水平滚动的标签页行，带动画选中指示器。
//!
//!
//! 标签页栏仅持有标识与几何信息，从不持有内容。每个标签页由单独调用输出，
//! 以便关闭按钮拥有独立的命中区域，且调用方可在标签页内放置任意标签字形四边形。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_anim::Spring;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 单个标签页的默认宽度（逻辑像素）。
pub const TAB_WIDTH: f32 = 140.0;

/// 标签页栏默认高度。
pub const TAB_STRIP_HEIGHT: f32 = 36.0;

/// 关闭按钮的宽度。
pub const TAB_CLOSE: f32 = 16.0;

/// 单个标签页。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabItem {
    /// 稳定标识，同时用作该标签页的节点 ID。
    pub id: u64,
    /// 可见标题。
    pub title: String,
    /// 该标签页是否带有关闭按钮。
    pub closable: bool,
}

impl TabItem {
    /// 创建可关闭的标签页。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            closable: true,
        }
    }

    /// 设为不带关闭按钮。
    #[must_use]
    pub fn fixed(mut self) -> Self {
        self.closable = false;
        self
    }
}

/// 标签页栏的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TabsDesc {
    /// 标签页栏矩形。
    pub rect: Rect,
    /// 从左到右排列的标签页列表。
    pub items: Vec<TabItem>,
    /// 是否为每个标签页显示关闭按钮。
    pub closable: bool,
    /// 是否显示末尾的“+”按钮。
    pub addable: bool,
    /// 单个标签页的宽度。
    pub tab_width: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl TabsDesc {
    /// 使用指定标题列表创建标签页栏。
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

    /// 使用显式标签页标识与标题。
    #[must_use]
    pub fn with_items(mut self, items: Vec<TabItem>) -> Self {
        self.items = items;
        self
    }

    /// 显示或隐藏末尾的“+”按钮。
    #[must_use]
    pub fn with_addable(mut self, addable: bool) -> Self {
        self.addable = addable;
        self
    }

    /// 覆盖标签页宽度。
    #[must_use]
    pub fn with_tab_width(mut self, width: f32) -> Self {
        self.tab_width = width.max(24.0);
        self
    }

    /// 标签页数量。
    #[must_use]
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 不含新增按钮的标签页可用宽度。
    #[must_use]
    pub fn strip_width(&self) -> f32 {
        let add = if self.addable { TAB_STRIP_HEIGHT } else { 0.0 };
        (self.rect.size.x - add).max(0.0)
    }

    /// 最后一个标签页右边缘齐平前可滚动的最大距离。
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.count() as f32 * self.tab_width - self.strip_width()).max(0.0)
    }

    /// 给定滚动偏移下单个标签页的未裁剪矩形。
    #[must_use]
    pub fn raw_tab_rect(&self, index: usize, scroll: f32) -> Rect {
        Rect::new(
            self.rect.origin.x + index as f32 * self.tab_width - scroll,
            self.rect.origin.y,
            self.tab_width,
            self.rect.size.y,
        )
    }

    /// 裁剪至标签页栏范围内的单个标签页可见矩形。
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

    /// 单个标签页的关闭按钮矩形（若有）。
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

    /// 末尾的“+”按钮矩形。
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

/// 标签页栏的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct TabsState {
    /// 当前选中的标签页索引。
    pub active: usize,
    /// 指示器左边缘的动画位置。
    pub indicator_x: Spring,
    /// 指示器宽度的动画值。
    pub indicator_w: Spring,
    /// 标签页栏的水平滚动偏移。
    pub scroll_x: f32,
    /// 弹簧是否已定位到静止目标。
    initialised: bool,
}

impl Default for TabsState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TabsState {
    /// 创建选中指定标签页的状态。
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

    /// 夹紧到当前标签页数量的激活索引。
    #[must_use]
    pub fn active_index(&self, desc: &TabsDesc) -> usize {
        self.active.min(desc.count().saturating_sub(1))
    }

    /// 激活标签页对应的静止指示器几何信息。
    #[must_use]
    pub fn indicator_target(&self, desc: &TabsDesc) -> (f32, f32) {
        let index = self.active_index(desc);
        let rect = desc.tab_rect(index, self.scroll_x);
        (rect.origin.x, rect.size.x)
    }

    /// 无动画直接将指示器放置到静止位置。
    pub fn snap(&mut self, desc: &TabsDesc) {
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.snap_to(x);
        self.indicator_w.snap_to(width);
        self.initialised = true;
    }

    /// 将弹簧目标对准当前激活标签页。
    ///
    /// 每帧在 [`Self::advance`] 之前调用一次；调用方读取
    /// [`Self::is_animating`] 以决定是否需要下一帧。
    pub fn sync(&mut self, desc: &TabsDesc) {
        if !self.initialised {
            self.snap(desc);
            return;
        }
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.set_target(x);
        self.indicator_w.set_target(width);
    }

    /// 推进弹簧物理步进 `dt` 秒，返回是否仍在运动。
    pub fn advance(&mut self, dt: f32) -> bool {
        let moving_x = self.indicator_x.step(dt);
        let moving_w = self.indicator_w.step(dt);
        moving_x || moving_w
    }

    /// 指示器是否仍在运动。
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.indicator_x.is_settled() || !self.indicator_w.is_settled()
    }

    /// 动画中的指示器矩形。
    #[must_use]
    pub fn indicator_rect(&self, desc: &TabsDesc) -> Rect {
        Rect::new(
            self.indicator_x.value,
            desc.rect.max().y - 2.0,
            self.indicator_w.value.max(0.0),
            2.0,
        )
    }

    /// 激活标签页并滚动使其可见，返回选中项是否变化。
    pub fn activate(&mut self, desc: &TabsDesc, index: usize) -> bool {
        if index >= desc.count() || index == self.active_index(desc) {
            return false;
        }
        self.active = index;
        self.reveal(desc, index);
        true
    }

    /// 滚动使指定标签页完全可见。
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

    /// 按滚轮增量滚动。
    pub fn scroll_by(&mut self, desc: &TabsDesc, delta: f32) -> bool {
        let next = (self.scroll_x + delta).clamp(0.0, desc.max_scroll());
        let changed = (next - self.scroll_x).abs() > 1e-4;
        self.scroll_x = next;
        changed
    }
}

/// 聚焦后的标签页栏可识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsKey {
    /// 选择上一个标签页（循环）。
    Left,
    /// 选择下一个标签页（循环）。
    Right,
    /// 选择第一个标签页。
    Home,
    /// 选择最后一个标签页。
    End,
}

/// 将单次按键归约为下一个激活索引。
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

/// 标签页栏绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabsTokens {
    /// 标签页栏背景。
    pub strip: Token,
    /// 标签页栏细分隔线。
    pub border: Token,
    /// 空闲标签页。
    pub tab: Token,
    /// 悬停标签页。
    pub tab_hover: Token,
    /// 激活标签页。
    pub tab_active: Token,
    /// 空闲标签页文本。
    pub tab_text: Token,
    /// 激活标签页文本。
    pub tab_text_active: Token,
    /// 选中指示器。
    pub indicator: Token,
    /// 关闭图标。
    pub close: Token,
    /// 悬停状态的关闭图标。
    pub close_hover: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl TabsTokens {
    /// 标准配色集。
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

/// 输出标签页栏背景、细分隔线与动画指示器。
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

/// 输出单个标签页。
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

/// 输出单个标签页的关闭按钮。
///
/// 关闭按钮拥有独立节点，使点击“×”不会被误判为点击标签页，
/// 且辅助技术可以单独播报。
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
    let mut payloads = Vec::with_capacity(2);
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
        curve_stroke(
            start,
            end,
            thickness,
            color,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
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

/// 输出末尾的“+”按钮。
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

/// 两点之间的细描边，打包为线段图元。
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
        // 300 px 宽，140 px 标签页：标签页 2 从 280 开始，可见 20 px。
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

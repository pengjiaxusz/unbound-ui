//! 手风琴组件（Accordion）：由一组标题栏堆叠而成，各自展开对应内容区。
//!
//! 内容区高度由弹簧驱动而非布尔值。折叠时会动画过渡至零，一旦动画高度
//! 低于 1 像素，调用方即可跳过构建子节点，因此关闭的分区不产生绘制与布局开销。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_anim::{Spring, SpringPreset};
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认标题栏高度。
pub const ACCORDION_HEADER: f32 = 40.0;

/// 低于此高度的内容区视为完全折叠。
pub const ACCORDION_COLLAPSE_EPSILON: f32 = 0.5;

/// 手风琴中的单个分区。
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionSection {
    /// 稳定标识，同时用作标题栏节点。
    pub id: u64,
    /// 标题文本。
    pub title: String,
    /// 完全展开时内容区占用的高度。
    pub content_height: f32,
}

impl AccordionSection {
    /// 创建具有指定标题和展开高度的分区。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, content_height: f32) -> Self {
        Self {
            id,
            title: title.into(),
            content_height: content_height.max(0.0),
        }
    }
}

/// 手风琴的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionDesc {
    /// 整体堆叠容器。
    pub rect: Rect,
    /// 从上到下排列的分区列表。
    pub sections: Vec<AccordionSection>,
    /// 是否允许同时展开多个分区。
    pub allow_multiple: bool,
    /// 单个标题栏的高度。
    pub header_height: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl AccordionDesc {
    /// 使用指定分区创建手风琴。
    #[must_use]
    pub fn new(rect: Rect, sections: Vec<AccordionSection>) -> Self {
        Self {
            rect,
            sections,
            allow_multiple: true,
            header_height: ACCORDION_HEADER,
            z: 500,
        }
    }

    /// 限制同一时间仅允许展开一个分区。
    #[must_use]
    pub fn exclusive(mut self) -> Self {
        self.allow_multiple = false;
        self
    }

    /// 指定分区的标题栏矩形。
    #[must_use]
    pub fn header_rect(&self, state: &AccordionState, index: usize) -> Rect {
        let y = self.section_origin(state, index);
        Rect::new(
            self.rect.origin.x,
            y,
            self.rect.size.x,
            self.header_height.min((self.rect.max().y - y).max(0.0)),
        )
    }

    /// 指定分区的可见内容区矩形。
    #[must_use]
    pub fn body_rect(&self, state: &AccordionState, index: usize) -> Rect {
        let header = self.header_rect(state, index);
        let height = state
            .body_height(index)
            .min((self.rect.max().y - header.max().y).max(0.0));
        Rect::new(header.origin.x, header.max().y, header.size.x, height)
    }

    /// 指定分区的顶部边缘位置（位于所有前置内容区高度之后）。
    #[must_use]
    pub fn section_origin(&self, state: &AccordionState, index: usize) -> f32 {
        let mut y = self.rect.origin.y;
        for position in 0..index.min(self.sections.len()) {
            y += self.header_height + state.body_height(position);
        }
        y
    }

    /// 包含标题栏与动画内容区在内的总布局高度。
    #[must_use]
    pub fn content_height(&self, state: &AccordionState) -> f32 {
        let headers = self.sections.len() as f32 * self.header_height;
        let bodies: f32 = (0..self.sections.len())
            .map(|index| state.body_height(index))
            .sum();
        headers + bodies
    }
}

/// 手风琴的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionState {
    /// 每个分区的展开状态位。
    pub open_mask: u64,
    heights: Vec<Spring>,
}

impl AccordionState {
    /// 创建包含 `count` 个分区的状态，初始全部关闭。
    #[must_use]
    pub fn new(count: usize) -> Self {
        Self {
            open_mask: 0,
            heights: (0..count)
                .map(|_| Spring::new(SpringPreset::Gentle, 0.0))
                .collect(),
        }
    }

    /// 判断指定分区是否处于展开（或正在展开）状态。
    #[must_use]
    pub fn is_open(&self, index: usize) -> bool {
        index < u64::BITS as usize && (self.open_mask >> index) & 1 == 1
    }

    /// 判断指定分区的内容区是否已完全折叠，可跳过构建。
    #[must_use]
    pub fn is_collapsed(&self, index: usize) -> bool {
        self.body_height(index) < ACCORDION_COLLAPSE_EPSILON
    }

    /// 指定分区的当前动画内容区高度。
    #[must_use]
    pub fn body_height(&self, index: usize) -> f32 {
        self.heights
            .get(index)
            .map_or(0.0, |spring| spring.value.max(0.0))
    }

    /// 指定分区的动画展开度，`0.0..=1.0`。
    ///
    /// 用于旋转展开箭头：与内容区共用同一弹簧，因此箭头与面板进度始终一致。
    #[must_use]
    pub fn progress(&self, desc: &AccordionDesc, index: usize) -> f32 {
        let full = desc
            .sections
            .get(index)
            .map_or(0.0, |section| section.content_height);
        if full <= 0.0 {
            return if self.is_open(index) { 1.0 } else { 0.0 };
        }
        (self.body_height(index) / full).clamp(0.0, 1.0)
    }

    /// 展开指定分区。
    ///
    /// 在互斥模式下会关闭其余所有分区；调用方仍需运行
    /// [`Self::advance`]，使关闭中的内容区以动画收起而非直接消失。
    pub fn open(&mut self, desc: &AccordionDesc, index: usize) -> bool {
        if index >= desc.sections.len() {
            return false;
        }
        if !desc.allow_multiple {
            self.open_mask = 0;
        }
        let was_open = self.is_open(index);
        self.open_mask |= 1u64 << index;
        self.sync_targets(desc);
        !was_open
    }

    /// 关闭指定分区。
    pub fn close(&mut self, desc: &AccordionDesc, index: usize) -> bool {
        if !self.is_open(index) {
            return false;
        }
        self.open_mask &= !(1u64 << index);
        self.sync_targets(desc);
        true
    }

    /// 切换指定分区的展开状态。
    pub fn toggle(&mut self, desc: &AccordionDesc, index: usize) -> bool {
        if self.is_open(index) {
            self.close(desc, index)
        } else {
            self.open(desc, index)
        }
    }

    /// 将所有弹簧目标设为各自对应的展开或关闭高度。
    pub fn sync_targets(&mut self, desc: &AccordionDesc) {
        if self.heights.len() < desc.sections.len() {
            self.heights.resize_with(desc.sections.len(), || {
                Spring::new(SpringPreset::Gentle, 0.0)
            });
        }
        for (index, section) in desc.sections.iter().enumerate() {
            let target = if self.is_open(index) {
                section.content_height
            } else {
                0.0
            };
            self.heights[index].set_target(target);
        }
    }

    /// 步进所有弹簧。返回是否仍有内容区在运动。
    pub fn advance(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for spring in &mut self.heights {
            moving |= spring.step(dt);
        }
        moving
    }

    /// 是否仍有内容区正在播放动画。
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.heights.iter().any(|spring| !spring.is_settled())
    }

    /// 直接将所有内容区置于目标位置，无动画过渡。
    pub fn snap(&mut self, desc: &AccordionDesc) {
        self.sync_targets(desc);
        for spring in &mut self.heights {
            spring.snap_to(spring.target);
        }
    }
}

/// 手风琴绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccordionTokens {
    /// 内容区背景色。
    pub body: Token,
    /// 闲置标题栏背景色。
    pub header: Token,
    /// 悬停标题栏背景色。
    pub header_hover: Token,
    /// 展开状态标题栏背景色。
    pub header_open: Token,
    /// 标题文字颜色。
    pub text: Token,
    /// 展开状态下的标题文字颜色。
    pub text_open: Token,
    /// 折叠指示箭头颜色。
    pub chevron: Token,
    /// 标题栏下方细分隔线颜色。
    pub divider: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl AccordionTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            body: Token::SURFACE,
            header: Token::SURFACE_RAISED,
            header_hover: Token::CONTROL_HOVER,
            header_open: Token::SURFACE,
            text: Token::TEXT_PRIMARY,
            text_open: Token::PRIMARY,
            chevron: Token::TEXT_SECONDARY,
            divider: Token::DIVIDER,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 发射单个标题栏：包含背景、标题、旋转箭头及命中区域。
#[must_use]
pub fn emit_accordion_header(
    desc: &AccordionDesc,
    state: &AccordionState,
    index: usize,
    control: ControlState,
    title: &[GlyphQuad],
    tokens: AccordionTokens,
) -> WidgetOutput {
    let Some(section) = desc.sections.get(index) else {
        return WidgetOutput::default();
    };
    let rect = desc.header_rect(state, index);
    if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    let disabled = control.disabled;
    let open = state.is_open(index);
    let mut primitives = Vec::with_capacity(6 + title.len());
    let mut payloads = Vec::with_capacity(title.len());

    let fill = if open {
        tokens.header_open
    } else if control.phase == PointerPhase::Hovered {
        tokens.header_hover
    } else {
        tokens.header
    };
    primitives.push(surface(rect, 0.0, fill, desc.z));
    glyphs(
        title,
        if open { tokens.text_open } else { tokens.text },
        desc.z,
        &mut primitives,
        &mut payloads,
    );

    // 箭头随驱动内容区高度的同一弹簧从 ">" 旋转为 "v"。
    let progress = state.progress(desc, index);
    let angle = progress * std::f32::consts::FRAC_PI_2;
    let cx = rect.origin.x + 16.0;
    let cy = rect.origin.y + rect.size.y * 0.5;
    let arm = (rect.size.y * 0.16).clamp(3.0, 6.0);
    let base_top = [cx - arm * 0.6, cy - arm];
    let base_bottom = [cx - arm * 0.6, cy + arm];
    let tip = [cx + arm * 0.6, cy];
    for point in [base_top, base_bottom] {
        let (sin, cos) = angle.sin_cos();
        let dx = point[0] - cx;
        let dy = point[1] - cy;
        let rotated = [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos];
        let (tsin, tcos) = angle.sin_cos();
        let tdx = tip[0] - cx;
        let tdy = tip[1] - cy;
        let rotated_tip = [cx + tdx * tcos - tdy * tsin, cy + tdx * tsin + tdy * tcos];
        curve_stroke(
            rotated,
            rotated_tip,
            1.5,
            tokens.chevron,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
    }

    let divider = Rect::new(rect.origin.x, rect.max().y - 1.0, rect.size.x, 1.0);
    primitives.push(instance(
        divider,
        [0.0; 4],
        tokens.divider,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        desc.z,
    ));
    primitives.push(focus_ring(
        rect.inflate(1.0),
        3.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(section.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [3.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, section.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.expanded = Some(open);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 发射单个分区的内容区表面。
///
/// 调用方将子节点裁剪到此矩形内；发射的表面使半展开状态
/// 的内容区呈现为完整面板，而非空白缺口。
#[must_use]
pub fn emit_accordion_body(
    desc: &AccordionDesc,
    state: &AccordionState,
    index: usize,
    tokens: AccordionTokens,
) -> WidgetOutput {
    let rect = desc.body_rect(state, index);
    if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    WidgetOutput {
        primitives: vec![outlined(
            rect,
            0.0,
            tokens.body,
            tokens.divider,
            1.0,
            0.0,
            desc.z,
        )],
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> AccordionDesc {
        AccordionDesc::new(
            Rect::new(0.0, 0.0, 300.0, 400.0),
            vec![
                AccordionSection::new(1, "General", 100.0),
                AccordionSection::new(2, "Advanced", 80.0),
                AccordionSection::new(3, "About", 60.0),
            ],
        )
    }

    #[test]
    fn closed_sections_stack_only_their_headers() {
        let desc = desc();
        let state = AccordionState::new(3);
        assert_eq!(desc.header_rect(&state, 0).origin.y, 0.0);
        assert_eq!(desc.header_rect(&state, 1).origin.y, ACCORDION_HEADER);
        assert_eq!(desc.header_rect(&state, 2).origin.y, 80.0);
        assert_eq!(desc.content_height(&state), 120.0);
    }

    #[test]
    fn opening_a_section_pushes_the_following_headers_down() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        state.open(&desc, 0);
        state.snap(&desc);
        assert_eq!(desc.body_rect(&state, 0).size.y, 100.0);
        assert_eq!(desc.header_rect(&state, 1).origin.y, 140.0);
        assert_eq!(desc.header_rect(&state, 2).origin.y, 180.0);
        assert_eq!(desc.content_height(&state), 220.0);
    }

    #[test]
    fn a_body_closed_below_a_pixel_reports_collapsed() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        assert!(state.is_collapsed(0));
        state.open(&desc, 0);
        for _ in 0..120 {
            state.advance(1.0 / 60.0);
        }
        assert!(!state.is_collapsed(0));
        assert!((state.body_height(0) - 100.0).abs() < 0.5);
    }

    #[test]
    fn exclusive_mode_closes_the_previous_section() {
        let desc = desc().exclusive();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        state.open(&desc, 0);
        state.snap(&desc);
        assert!(state.is_open(0));
        state.open(&desc, 2);
        state.snap(&desc);
        assert!(!state.is_open(0));
        assert!(state.is_open(2));
        assert_eq!(state.body_height(0), 0.0);
    }

    #[test]
    fn a_multi_open_accordion_keeps_both_bodies() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        state.open(&desc, 0);
        state.open(&desc, 2);
        state.snap(&desc);
        assert!(state.is_open(0));
        assert!(state.is_open(2));
        assert!((state.body_height(0) - 100.0).abs() < 0.5);
        assert!((state.body_height(2) - 60.0).abs() < 0.5);
    }

    #[test]
    fn toggling_returns_and_closes() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        assert!(state.toggle(&desc, 1));
        assert!(state.is_open(1));
        assert!(state.toggle(&desc, 1));
        assert!(!state.is_open(1));
        assert!(!state.toggle(&desc, 9), "out of range is inert");
    }

    #[test]
    fn the_chevron_rotation_tracks_the_body() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        assert_eq!(state.progress(&desc, 0), 0.0);
        state.open(&desc, 0);
        state.snap(&desc);
        assert!((state.progress(&desc, 0) - 1.0).abs() < 1e-3);
        state.close(&desc, 0);
        state.snap(&desc);
        assert_eq!(state.progress(&desc, 0), 0.0);
    }

    #[test]
    fn a_header_is_pickable_and_reports_expansion() {
        let desc = desc();
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        state.open(&desc, 1);
        state.snap(&desc);
        let output = emit_accordion_header(
            &desc,
            &state,
            1,
            ControlState::default(),
            &[],
            AccordionTokens::standard(),
        );
        let hit = output.hit.expect("the header is pickable");
        assert_eq!(hit.node, 2);
        let a11y = output.a11y.expect("the header is reachable");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(a11y.expanded, Some(true));
    }

    #[test]
    fn the_body_is_clipped_to_the_remaining_space() {
        let mut desc = desc();
        desc.sections[0].content_height = 500.0;
        let mut state = AccordionState::new(3);
        state.snap(&desc);
        state.open(&desc, 0);
        state.snap(&desc);
        let body = desc.body_rect(&state, 0);
        assert_eq!(body.size.y, 360.0, "the stack rect clips the body");
    }

    #[test]
    fn a_disabled_header_is_not_pickable() {
        let desc = desc();
        let state = AccordionState::new(3);
        let output = emit_accordion_header(
            &desc,
            &state,
            0,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            AccordionTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

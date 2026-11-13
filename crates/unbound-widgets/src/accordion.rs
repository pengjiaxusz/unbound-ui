//! The accordion: a stack of headers, each revealing a body.
//!
//! The body height is a spring, not a boolean. Collapsing animates to zero and
//! the caller can skip building the children as soon as the animated height
//! drops below a pixel, so a closed section costs neither paint nor layout.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_anim::{Spring, SpringPreset};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default header height.
pub const ACCORDION_HEADER: f32 = 40.0;

/// A body shorter than this is treated as fully collapsed.
pub const ACCORDION_COLLAPSE_EPSILON: f32 = 0.5;

/// One section of an accordion.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionSection {
    /// Stable identity, also used as the header's node.
    pub id: u64,
    /// The header title.
    pub title: String,
    /// The height the body occupies when fully open.
    pub content_height: f32,
}

impl AccordionSection {
    /// A section with the given title and open height.
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, content_height: f32) -> Self {
        Self {
            id,
            title: title.into(),
            content_height: content_height.max(0.0),
        }
    }
}

/// The pure descriptor of an accordion.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionDesc {
    /// The whole stack.
    pub rect: Rect,
    /// The sections, top to bottom.
    pub sections: Vec<AccordionSection>,
    /// Whether more than one section may be open at a time.
    pub allow_multiple: bool,
    /// The height of one header.
    pub header_height: f32,
    /// The draw order key.
    pub z: u16,
}

impl AccordionDesc {
    /// An accordion with the given sections.
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

    /// Allow only one open section at a time.
    #[must_use]
    pub fn exclusive(mut self) -> Self {
        self.allow_multiple = false;
        self
    }

    /// The header of one section.
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

    /// The visible body of one section.
    #[must_use]
    pub fn body_rect(&self, state: &AccordionState, index: usize) -> Rect {
        let header = self.header_rect(state, index);
        let height = state
            .body_height(index)
            .min((self.rect.max().y - header.max().y).max(0.0));
        Rect::new(header.origin.x, header.max().y, header.size.x, height)
    }

    /// The top edge of a section, after every preceding body height.
    #[must_use]
    pub fn section_origin(&self, state: &AccordionState, index: usize) -> f32 {
        let mut y = self.rect.origin.y;
        for position in 0..index.min(self.sections.len()) {
            y += self.header_height + state.body_height(position);
        }
        y
    }

    /// The total laid-out height, headers and animated bodies included.
    #[must_use]
    pub fn content_height(&self, state: &AccordionState) -> f32 {
        let headers = self.sections.len() as f32 * self.header_height;
        let bodies: f32 = (0..self.sections.len())
            .map(|index| state.body_height(index))
            .sum();
        headers + bodies
    }
}

/// The live state of an accordion.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionState {
    /// One bit per open section.
    pub open_mask: u64,
    heights: Vec<Spring>,
}

impl AccordionState {
    /// A state of `count` sections, all closed.
    #[must_use]
    pub fn new(count: usize) -> Self {
        Self {
            open_mask: 0,
            heights: (0..count)
                .map(|_| Spring::new(SpringPreset::Gentle, 0.0))
                .collect(),
        }
    }

    /// Whether a section is open (or opening).
    #[must_use]
    pub fn is_open(&self, index: usize) -> bool {
        index < u64::BITS as usize && (self.open_mask >> index) & 1 == 1
    }

    /// Whether the section's body is fully collapsed and can be skipped.
    #[must_use]
    pub fn is_collapsed(&self, index: usize) -> bool {
        self.body_height(index) < ACCORDION_COLLAPSE_EPSILON
    }

    /// The animated body height of one section.
    #[must_use]
    pub fn body_height(&self, index: usize) -> f32 {
        self.heights
            .get(index)
            .map_or(0.0, |spring| spring.value.max(0.0))
    }

    /// The animated openness of one section, `0.0..=1.0`.
    ///
    /// This is what rotates the chevron: it uses the same spring as the body,
    /// so the arrow and the panel can never disagree about progress.
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

    /// Open a section.
    ///
    /// In exclusive mode every other section closes; the caller still runs
    /// [`Self::advance`] so the closing body animates instead of vanishing.
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

    /// Close a section.
    pub fn close(&mut self, desc: &AccordionDesc, index: usize) -> bool {
        if !self.is_open(index) {
            return false;
        }
        self.open_mask &= !(1u64 << index);
        self.sync_targets(desc);
        true
    }

    /// Toggle a section.
    pub fn toggle(&mut self, desc: &AccordionDesc, index: usize) -> bool {
        if self.is_open(index) {
            self.close(desc, index)
        } else {
            self.open(desc, index)
        }
    }

    /// Aim every spring at its open or closed height.
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

    /// Step every spring. Returns whether any body still moves.
    pub fn advance(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for spring in &mut self.heights {
            moving |= spring.step(dt);
        }
        moving
    }

    /// Whether any body is still animating.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.heights.iter().any(|spring| !spring.is_settled())
    }

    /// Place every body at its target with no animation.
    pub fn snap(&mut self, desc: &AccordionDesc) {
        self.sync_targets(desc);
        for spring in &mut self.heights {
            spring.snap_to(spring.target);
        }
    }
}

/// The colour slots an accordion paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccordionTokens {
    /// The body background.
    pub body: Token,
    /// An idle header.
    pub header: Token,
    /// A hovered header.
    pub header_hover: Token,
    /// An open header.
    pub header_open: Token,
    /// The header title.
    pub text: Token,
    /// The header title while open.
    pub text_open: Token,
    /// The chevron.
    pub chevron: Token,
    /// The hairline under a header.
    pub divider: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl AccordionTokens {
    /// The standard set.
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

/// Emit one header: background, title, rotating chevron and hit region.
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

    // The chevron rotates from ">" to "v" over the same spring that drives
    // the body height.
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
        primitives.push(stroke(rotated, rotated_tip, 1.5, tokens.chevron, desc.z));
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

/// Emit the body surface of one section.
///
/// The caller clips the children to this rectangle; the emitted surface is
/// what makes a partly-open body read as a panel rather than as a gap.
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

/// A thin stroke between two points.
fn stroke(
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: Token,
    z: u16,
) -> GpuPrimitiveInstance {
    instance(
        Rect::new(
            start[0].min(end[0]),
            start[1].min(end[1]),
            (start[0] - end[0]).abs().max(thickness),
            (start[1] - end[1]).abs().max(thickness),
        ),
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

//! The floating canvas toolbar: tool selection, zoom and fit.
//!
//! The toolbar is deliberately one *row* of mutually exclusive tools plus a
//! zoom readout. A tool palette that scrolls or nests costs a second look
//! every time; a row can be learned once. The bar requests a blur pass behind
//! itself so it stays legible over arbitrary artwork.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::overlay::BackdropRequest;
use crate::primitive::{instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The bar height.
pub const TOOLBAR_HEIGHT: f32 = 44.0;
/// One tool button.
pub const TOOL_BUTTON: f32 = 32.0;
/// The gap between buttons.
pub const TOOL_GAP: f32 = 4.0;
/// The zoom readout width.
pub const ZOOM_WIDTH: f32 = 72.0;
/// The default blur behind the bar.
pub const TOOLBAR_BLUR: f32 = 16.0;
/// The zoom limits.
pub const ZOOM_RANGE: (f32, f32) = (0.05, 64.0);

/// A canvas tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasTool {
    /// Pick and transform.
    #[default]
    Select,
    /// Pan the viewport.
    Hand,
    /// Draw a frame.
    Frame,
    /// Draw a path.
    Pen,
    /// Connect two ports.
    Connect,
    /// Place text.
    Text,
}

impl CanvasTool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 6] = [
        Self::Select,
        Self::Hand,
        Self::Frame,
        Self::Pen,
        Self::Connect,
        Self::Text,
    ];

    /// The tool name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "选择",
            Self::Hand => "抓手",
            Self::Frame => "框架",
            Self::Pen => "钢笔",
            Self::Connect => "连接",
            Self::Text => "文本",
        }
    }

    /// The keyboard shortcut.
    #[must_use]
    pub const fn shortcut(self) -> &'static str {
        match self {
            Self::Select => "V",
            Self::Hand => "H",
            Self::Frame => "F",
            Self::Pen => "P",
            Self::Connect => "C",
            Self::Text => "T",
        }
    }
}

/// The toolbar descriptor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasToolbarDesc {
    /// The viewport the bar floats over.
    pub viewport: Rect,
    /// The distance from the bottom edge.
    pub bottom_margin: f32,
    /// The backdrop blur; `0.0` disables the pass.
    pub backdrop_blur: f32,
    /// The draw order key.
    pub z: u16,
}

impl CanvasToolbarDesc {
    #[must_use]
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            bottom_margin: 20.0,
            backdrop_blur: TOOLBAR_BLUR,
            z: 890,
        }
    }

    /// The total bar width.
    #[must_use]
    pub fn width(&self) -> f32 {
        let tools = CanvasTool::ALL.len() as f32;
        let separator = TOOL_GAP * 2.0 + 1.0;
        8.0 * 2.0 + tools * TOOL_BUTTON + (tools - 1.0) * TOOL_GAP + separator + ZOOM_WIDTH
    }

    /// The bar rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.viewport.center().x - self.width() * 0.5,
            self.viewport.max().y - self.bottom_margin - TOOLBAR_HEIGHT,
            self.width(),
            TOOLBAR_HEIGHT,
        )
    }

    /// The button rectangle for one tool.
    #[must_use]
    pub fn tool_rect(&self, tool: CanvasTool) -> Rect {
        let index = CanvasTool::ALL
            .iter()
            .position(|item| *item == tool)
            .unwrap_or_default() as f32;
        Rect::new(
            self.rect().origin.x + 8.0 + index * (TOOL_BUTTON + TOOL_GAP),
            self.rect().origin.y + (TOOLBAR_HEIGHT - TOOL_BUTTON) * 0.5,
            TOOL_BUTTON,
            TOOL_BUTTON,
        )
    }

    /// The zoom readout, right of the separator.
    #[must_use]
    pub fn zoom_rect(&self) -> Rect {
        Rect::new(
            self.rect().max().x - 8.0 - ZOOM_WIDTH,
            self.rect().origin.y + (TOOLBAR_HEIGHT - TOOL_BUTTON) * 0.5,
            ZOOM_WIDTH,
            TOOL_BUTTON,
        )
    }

    /// The fit button, inside the zoom readout's left half.
    #[must_use]
    pub fn fit_rect(&self) -> Rect {
        let zoom = self.zoom_rect();
        Rect::new(zoom.origin.x, zoom.origin.y, 24.0, zoom.size.y)
    }

    /// The separator rule.
    #[must_use]
    pub fn separator_rect(&self) -> Rect {
        let last = self.tool_rect(CanvasTool::ALL[CanvasTool::ALL.len() - 1]);
        Rect::new(
            last.max().x + TOOL_GAP + 4.0,
            self.rect().origin.y + 10.0,
            1.0,
            TOOLBAR_HEIGHT - 20.0,
        )
    }
}

/// The toolbar state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasToolbarState {
    /// The active tool.
    pub active: CanvasTool,
    /// The current zoom.
    pub zoom: f32,
}

impl CanvasToolbarState {
    #[must_use]
    pub fn new(zoom: f32) -> Self {
        Self {
            active: CanvasTool::Select,
            zoom: zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1),
        }
    }

    /// Switch tools.
    pub fn set_tool(&mut self, tool: CanvasTool) {
        self.active = tool;
    }

    /// Multiply the zoom, clamped to [`ZOOM_RANGE`].
    pub fn zoom_by(&mut self, factor: f32) -> f32 {
        self.zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        self.zoom
    }

    /// The zoom readout text.
    #[must_use]
    pub fn zoom_label(&self) -> String {
        format!("{:.0}%", self.zoom * 100.0)
    }
}

/// What one toolbar emits.
#[derive(Clone, Debug)]
pub struct CanvasToolbarOutput {
    /// The bar, buttons and glyphs.
    pub output: WidgetOutput,
    /// One hit region per tool.
    pub tools: Vec<(CanvasTool, HitRegion)>,
    /// The zoom readout.
    pub zoom: HitRegion,
    /// The fit button.
    pub fit: HitRegion,
    /// The blur request, when enabled.
    pub backdrop: Option<BackdropRequest>,
    /// The bar rectangle.
    pub rect: Rect,
}

/// Emit the floating toolbar.
#[must_use]
pub fn emit_canvas_toolbar(
    node: u64,
    desc: &CanvasToolbarDesc,
    state: &CanvasToolbarState,
    glyphs: &[GlyphQuad],
    control: ControlState,
) -> CanvasToolbarOutput {
    let rect = desc.rect();
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            [10.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            18.0,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        outlined(rect, 10.0, Token::HUD_BG, Token::OUTLINE, 1.0, 0.0, desc.z),
    ];

    let mut tools = Vec::with_capacity(CanvasTool::ALL.len());
    for tool in CanvasTool::ALL {
        let button = desc.tool_rect(tool);
        let active = tool == state.active;
        if active {
            primitives.push(surface(button, 7.0, Token::PRIMARY, desc.z + 1));
        } else if control.phase == crate::PointerPhase::Hovered {
            primitives.push(surface(button, 7.0, Token::CONTROL_HOVER, desc.z + 1));
        }
        tools.push((
            tool,
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), button)
                .with_cursor(CursorKind::Pointer),
        ));
    }

    primitives.push(surface(
        desc.separator_rect(),
        0.0,
        Token::OUTLINE,
        desc.z + 1,
    ));
    primitives.push(surface(
        desc.zoom_rect(),
        7.0,
        Token::CONTROL_HOVER,
        desc.z + 1,
    ));

    let mut payloads = Vec::new();
    crate::primitive::glyphs(
        glyphs,
        Token::HUD_TEXT,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(Role::Toolbar, "画布工具", rect);
    a11y.value = format!("{} · {}", state.active.label(), state.zoom_label());

    CanvasToolbarOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
        },
        tools,
        zoom: HitRegion::new(node ^ 1, CONTROL_LAYER, u32::from(desc.z), desc.zoom_rect())
            .with_cursor(CursorKind::Pointer),
        fit: HitRegion::new(node ^ 2, CONTROL_LAYER, u32::from(desc.z), desc.fit_rect())
            .with_cursor(CursorKind::Pointer),
        backdrop: BackdropRequest::new(node, rect, desc.backdrop_blur),
        rect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CanvasToolbarDesc {
        CanvasToolbarDesc::new(Rect::new(0.0, 0.0, 1280.0, 720.0))
    }

    #[test]
    fn the_bar_is_centred_above_the_bottom_edge() {
        let rect = desc().rect();
        assert!((rect.center().x - 640.0).abs() < 0.001);
        assert_eq!(rect.max().y, 720.0 - 20.0);
        assert_eq!(rect.size.y, TOOLBAR_HEIGHT);
    }

    #[test]
    fn tool_buttons_do_not_overlap_and_stay_inside() {
        let desc = desc();
        let mut previous: Option<Rect> = None;
        for tool in CanvasTool::ALL {
            let rect = desc.tool_rect(tool);
            assert!(desc.rect().contains_rect(&rect));
            if let Some(previous) = previous {
                assert!(rect.origin.x >= previous.max().x);
            }
            previous = Some(rect);
        }
    }

    #[test]
    fn the_zoom_readout_is_left_of_the_right_edge() {
        let desc = desc();
        let zoom = desc.zoom_rect();
        assert_eq!(zoom.max().x, desc.rect().max().x - 8.0);
        assert!(desc.separator_rect().origin.x < zoom.origin.x);
        assert!(desc.fit_rect().origin.x >= zoom.origin.x);
    }

    #[test]
    fn tools_and_shortcuts_are_unique() {
        let mut labels: Vec<&str> = CanvasTool::ALL.iter().map(|tool| tool.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), CanvasTool::ALL.len());
        let mut keys: Vec<&str> = CanvasTool::ALL.iter().map(|tool| tool.shortcut()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CanvasTool::ALL.len());
    }

    #[test]
    fn zoom_is_clamped_and_labelled() {
        let mut state = CanvasToolbarState::new(1.0);
        assert_eq!(state.zoom_by(2.0), 2.0);
        assert_eq!(state.zoom_label(), "200%");
        state.zoom_by(1000.0);
        assert_eq!(state.zoom, ZOOM_RANGE.1);
        state.zoom_by(0.0001);
        assert_eq!(state.zoom, ZOOM_RANGE.0);
    }

    #[test]
    fn the_emitter_reports_every_button_and_the_blur() {
        let desc = desc();
        let state = CanvasToolbarState::new(1.0);
        let output = emit_canvas_toolbar(6, &desc, &state, &[], ControlState::default());
        assert_eq!(output.tools.len(), CanvasTool::ALL.len());
        assert_eq!(output.rect, desc.rect());
        assert_eq!(
            output.backdrop.map(|request| request.blur),
            Some(TOOLBAR_BLUR)
        );
        let a11y = output.output.a11y.expect("toolbar");
        assert_eq!(a11y.role, Role::Toolbar);
        assert!(a11y.value.contains("选择"));
    }

    #[test]
    fn switching_tools_updates_the_state() {
        let mut state = CanvasToolbarState::new(1.0);
        state.set_tool(CanvasTool::Pen);
        assert_eq!(state.active, CanvasTool::Pen);
    }
}

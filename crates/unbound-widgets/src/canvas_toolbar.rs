//! 浮动画布工具栏（CanvasToolbar）：工具选择、缩放与自适应画布。
//!
//! 工具栏刻意设计为单*行*互斥工具加缩放读数。可滚动或嵌套的工具面板每次都需二次查找；
//! 单行布局只需记忆一次。工具栏会在自身背后请求模糊通道，使其在任意画面上都清晰可读。

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

/// 工具栏高度。
pub const TOOLBAR_HEIGHT: f32 = 44.0;
/// 单个工具按钮尺寸。
pub const TOOL_BUTTON: f32 = 32.0;
/// 按钮间距。
pub const TOOL_GAP: f32 = 4.0;
/// 缩放读数区域宽度。
pub const ZOOM_WIDTH: f32 = 72.0;
/// 工具栏背后的默认模糊半径。
pub const TOOLBAR_BLUR: f32 = 16.0;
/// 缩放比例上下限。
pub const ZOOM_RANGE: (f32, f32) = (0.05, 64.0);

/// 画布工具类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasTool {
    /// 拾取与变换。
    #[default]
    Select,
    /// 平移视口。
    Hand,
    /// 绘制框架。
    Frame,
    /// 绘制路径。
    Pen,
    /// 连接两个端口。
    Connect,
    /// 放置文本。
    Text,
}

impl CanvasTool {
    /// 按工具栏顺序列出的所有工具。
    pub const ALL: [Self; 6] = [
        Self::Select,
        Self::Hand,
        Self::Frame,
        Self::Pen,
        Self::Connect,
        Self::Text,
    ];

    /// 工具名称。
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

    /// 键盘快捷键。
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

/// 工具栏描述符。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasToolbarDesc {
    /// 工具栏浮动所在的视口。
    pub viewport: Rect,
    /// 距底部边缘的距离。
    pub bottom_margin: f32,
    /// 背景模糊半径；`0.0` 表示禁用。
    pub backdrop_blur: f32,
    /// 绘制顺序键。
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

    /// 工具栏总宽度。
    #[must_use]
    pub fn width(&self) -> f32 {
        let tools = CanvasTool::ALL.len() as f32;
        let separator = TOOL_GAP * 2.0 + 1.0;
        8.0 * 2.0 + tools * TOOL_BUTTON + (tools - 1.0) * TOOL_GAP + separator + ZOOM_WIDTH
    }

    /// 工具栏矩形区域。
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.viewport.center().x - self.width() * 0.5,
            self.viewport.max().y - self.bottom_margin - TOOLBAR_HEIGHT,
            self.width(),
            TOOLBAR_HEIGHT,
        )
    }

    /// 单个工具的按钮矩形。
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

    /// 位于分隔符右侧的缩放读数区域。
    #[must_use]
    pub fn zoom_rect(&self) -> Rect {
        Rect::new(
            self.rect().max().x - 8.0 - ZOOM_WIDTH,
            self.rect().origin.y + (TOOLBAR_HEIGHT - TOOL_BUTTON) * 0.5,
            ZOOM_WIDTH,
            TOOL_BUTTON,
        )
    }

    /// 位于缩放读数左半部分的自适应按钮。
    #[must_use]
    pub fn fit_rect(&self) -> Rect {
        let zoom = self.zoom_rect();
        Rect::new(zoom.origin.x, zoom.origin.y, 24.0, zoom.size.y)
    }

    /// 分隔线。
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

/// 工具栏状态。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasToolbarState {
    /// 当前激活的工具。
    pub active: CanvasTool,
    /// 当前缩放倍率。
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

    /// 切换工具。
    pub fn set_tool(&mut self, tool: CanvasTool) {
        self.active = tool;
    }

    /// 按倍率缩放，并限制在 [`ZOOM_RANGE`] 内。
    pub fn zoom_by(&mut self, factor: f32) -> f32 {
        self.zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        self.zoom
    }

    /// 缩放读数文本。
    #[must_use]
    pub fn zoom_label(&self) -> String {
        format!("{:.0}%", self.zoom * 100.0)
    }
}

/// 单个工具栏的发射结果。
#[derive(Clone, Debug)]
pub struct CanvasToolbarOutput {
    /// 工具栏、按钮与字形。
    pub output: WidgetOutput,
    /// 每个工具对应的命中区域。
    pub tools: Vec<(CanvasTool, HitRegion)>,
    /// 缩放读数。
    pub zoom: HitRegion,
    /// 自适应按钮。
    pub fit: HitRegion,
    /// 启用模糊时的模糊请求。
    pub backdrop: Option<BackdropRequest>,
    /// 工具栏矩形。
    pub rect: Rect,
}

/// 发射浮动工具栏。
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
    let hovered = control.phase == crate::PointerPhase::Hovered;
    for tool in CanvasTool::ALL {
        let button = desc.tool_rect(tool);
        let active = tool == state.active;
        // 每个工具在池中始终占有一个槽位：激活工具绘制主填充色，
        // 其余工具保留悬停底色，悬停仅改变其不透明度，因此图元序列永不重排。
        let slot = if active {
            surface(button, 7.0, Token::PRIMARY, desc.z + 1)
        } else {
            let mut wash = surface(button, 7.0, Token::CONTROL_HOVER, desc.z + 1);
            wash.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
                wash.depth_u16(),
                if hovered { 1.0 } else { 0.0 },
            );
            wash
        };
        primitives.push(slot);
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
            ..Default::default()
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

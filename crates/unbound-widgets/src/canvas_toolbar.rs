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

/// 工具栏高度。对齐 WinUI `AppBarThemeCompactHeight = 48`（`CommandBar_themeresources.xaml`）。
pub const TOOLBAR_HEIGHT: f32 = 48.0;
/// 单个工具按钮尺寸。对齐 WinUI `AppBarButton` 32 × 32。
pub const TOOL_BUTTON: f32 = 32.0;
/// 按钮间距。对齐 WinUI CommandBar 分组间距 4。
pub const TOOL_GAP: f32 = 4.0;
/// 缩放读数区域宽度。
pub const ZOOM_WIDTH: f32 = 72.0;
/// 工具栏背后的默认模糊半径。
pub const TOOLBAR_BLUR: f32 = 16.0;
/// 缩放比例上下限。
pub const ZOOM_RANGE: (f32, f32) = (0.05, 64.0);

/// 浮动工具栏圆角。对齐 WinUI `OverlayCornerRadius = 8`（浮层容器）。
const TOOLBAR_RADIUS: f32 = 8.0;
/// 按钮与缩放读数圆角。对齐 WinUI `ControlCornerRadius = 4`。
const BUTTON_RADIUS: f32 = 4.0;
/// 分隔线上下留白。对齐 WinUI `AppBarSeparatorMargin = 2,8,2,8`。
const SEPARATOR_MARGIN: f32 = 8.0;

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
            Self::Select => "Select",
            Self::Hand => "Hand",
            Self::Frame => "Frame",
            Self::Pen => "Pen",
            Self::Connect => "Connect",
            Self::Text => "Text",
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

    /// 工具栏矩形区域（视口窄于工具栏总宽时自动钳制宽度，防止左右越界）。
    #[must_use]
    pub fn rect(&self) -> Rect {
        let clamped_w = self.width().min(self.viewport.width().max(0.0));
        Rect::new(
            self.viewport.center().x - clamped_w * 0.5,
            self.viewport.max().y - self.bottom_margin - TOOLBAR_HEIGHT,
            clamped_w,
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
    ///
    /// 宽度对齐 WinUI `AppBarButton` 的 32 × 32；高度随缩放读数撑满工具栏内高。
    #[must_use]
    pub fn fit_rect(&self) -> Rect {
        let zoom = self.zoom_rect();
        Rect::new(zoom.origin.x, zoom.origin.y, TOOL_BUTTON, zoom.size.y)
    }

    /// 分隔线。
    ///
    /// WinUI `AppBarSeparatorMargin = 2,8,2,8`：上下各留 8，宽度 1。
    #[must_use]
    pub fn separator_rect(&self) -> Rect {
        let last = self.tool_rect(CanvasTool::ALL[CanvasTool::ALL.len() - 1]);
        Rect::new(
            last.max().x + TOOL_GAP,
            self.rect().origin.y + SEPARATOR_MARGIN,
            1.0,
            (TOOLBAR_HEIGHT - SEPARATOR_MARGIN * 2.0).max(0.0),
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
            [TOOLBAR_RADIUS; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            18.0,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        // 浮层圆角 `OverlayCornerRadius = 8`，1px 描边对齐浮动工具条表面。
        outlined(
            rect,
            TOOLBAR_RADIUS,
            Token::HUD_BG,
            Token::OUTLINE,
            1.0,
            0.0,
            desc.z,
        ),
    ];

    let zoom_r = desc.zoom_rect();
    let zoom_fits = zoom_r.origin.x >= rect.origin.x + 8.0 && zoom_r.max().x <= rect.max().x - 8.0;
    let tool_right_limit = if zoom_fits {
        zoom_r.origin.x - TOOL_GAP * 2.0 - 1.0
    } else {
        rect.max().x - 8.0
    };

    let mut tools = Vec::with_capacity(CanvasTool::ALL.len());
    let hovered = control.phase == crate::PointerPhase::Hovered;
    for tool in CanvasTool::ALL {
        let button = desc.tool_rect(tool);
        if button.origin.x < rect.origin.x || button.max().x > tool_right_limit {
            break;
        }
        let active = tool == state.active;
        // 每个可见工具在池中始终占有一个槽位：激活工具绘制主填充色，
        // 其余工具保留悬停底色，悬停仅改变其不透明度，因此图元序列永不重排。
        let slot = if active {
            surface(button, BUTTON_RADIUS, Token::PRIMARY, desc.z + 1)
        } else {
            // 悬停底色对齐 WinUI `AppBarButtonBackgroundPointerOver`
            // （`SubtleFillColorSecondary`，即 HOVER 令牌）。
            let mut wash = surface(button, BUTTON_RADIUS, Token::HOVER, desc.z + 1);
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

    if zoom_fits {
        let sep_r = Rect::new(
            zoom_r.origin.x - TOOL_GAP - 1.0,
            rect.origin.y + SEPARATOR_MARGIN,
            1.0,
            (TOOLBAR_HEIGHT - SEPARATOR_MARGIN * 2.0).max(0.0),
        );
        if !tools.is_empty() && sep_r.origin.x >= rect.origin.x + 8.0 {
            primitives.push(surface(sep_r, 0.0, Token::OUTLINE, desc.z + 1));
        }
        // 缩放读数是一枚控件表面：`ControlFillColorDefault` + `ControlCornerRadius = 4`。
        primitives.push(surface(
            zoom_r,
            BUTTON_RADIUS,
            Token::CONTROL_FILL,
            desc.z + 1,
        ));
    }

    let mut payloads = Vec::new();
    let visible_glyphs: Vec<GlyphQuad> = glyphs
        .iter()
        .copied()
        .filter(|q| q.rect.origin.x >= rect.origin.x && q.rect.max().x <= rect.max().x)
        .collect();
    crate::primitive::glyphs(
        &visible_glyphs,
        Token::HUD_TEXT,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(Role::Toolbar, "Canvas tools", rect).disabled(control.disabled);
    a11y.value = format!("{} · {}", state.active.label(), state.zoom_label());

    let sub_hits = if control.disabled {
        Vec::new()
    } else {
        tools.iter().map(|(_, hit)| hit.clone()).collect()
    };

    CanvasToolbarOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: (!control.disabled).then(|| {
                HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
                    .with_cursor(CursorKind::Pointer)
            }),
            sub_hits: sub_hits.into(),
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
        assert!(a11y.value.contains("Select"));
    }

    #[test]
    fn switching_tools_updates_the_state() {
        let mut state = CanvasToolbarState::new(1.0);
        state.set_tool(CanvasTool::Pen);
        assert_eq!(state.active, CanvasTool::Pen);
    }
}

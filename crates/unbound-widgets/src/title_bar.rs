//! 无边框窗口默认标题栏描述符与约定节点 ID。
//!
//! 所有基于 `unbound-app` 无边框窗口运行的应用与示例均可使用 [`TitleBarDesc`]
//! 提供一致的窗口标题展示、拖拽区域以及最小化 / 最大化 / 关闭按钮。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::instance;
use crate::{ControlState, PointerPhase, TextCtx, UiWidget, WidgetOutput};

/// 默认标题栏高度（逻辑像素）。
pub const TITLE_BAR_HEIGHT: f32 = 38.0;

/// 窗口控制按钮（最小化 / 最大化 / 关闭）的默认宽度（逻辑像素，与 Showcase 一致）。
pub const TITLE_BAR_CAPTION_WIDTH: f32 = 46.0;

/// 标题栏拖拽区域（背景装饰控件）的保留节点键。
///
/// 该键挂在覆盖整条标题栏的 [`TitleBarChrome`] 控件上，因此
/// `AppRuntime::widget_rect` 能直接读出标题栏包围盒，供窗口运行器把
/// 拖拽高度写入 `FrameHitMetrics`；纯布局容器不在挂载控件表中，无法这样读取。
pub const TITLE_BAR_DRAG_ID: u64 = 0xFFFF_FF00;

/// 默认标题栏的背景表面与底部分隔线。
///
/// 与 showcase 的 `draw_top_bar` 保持一致的视觉约定：`HEADER_BG` 铺满整条
/// 标题栏，底边以 1px `DIVIDER` 与内容区分隔。它是绝对定位的纯装饰控件，
/// 不参与 flex 排布、不发射命中区域，因此标题栏空白处的拖拽仍由
/// 自定义边框子类的 `HTCAPTION` 承担。
#[derive(Clone, Copy, Debug, Default)]
pub struct TitleBarChrome;

impl UiWidget for TitleBarChrome {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        // 真实尺寸由 `ViewBuilder` 以绝对定位的显式宽高给出，此处仅占位。
        Vec2::new(1.0, 1.0)
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut output = WidgetOutput::default();
        output.primitives.push(instance(
            rect,
            [0.0; 4],
            Token::HEADER_BG,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            0,
        ));
        let rule = Rect::new(rect.origin.x, rect.max().y - 1.0, rect.size.x, 1.0);
        output.primitives.push(instance(
            rule,
            [0.0; 4],
            Token::DIVIDER,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Background,
            1,
        ));
        output
    }
}

/// 窗口控制按钮类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCaptionKind {
    /// 最小化窗口。
    Minimize,
    /// 最大化或还原窗口。
    Maximize { maximized: bool },
    /// 关闭窗口（悬停/按压时呈红色警示背景）。
    Close,
}

/// 与 Showcase 完全一致的无边框窗口控制按钮控件（46px 宽、零圆角贴边、几何图元图标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowCaptionButton {
    pub kind: WindowCaptionKind,
    pub height: f32,
}

impl WindowCaptionButton {
    #[must_use]
    pub fn new(kind: WindowCaptionKind, height: f32) -> Self {
        Self {
            kind,
            height: height.max(24.0),
        }
    }
}

impl UiWidget for WindowCaptionButton {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(TITLE_BAR_CAPTION_WIDTH, self.height)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut output = WidgetOutput::default();
        let active = matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed);
        let destructive = matches!(self.kind, WindowCaptionKind::Close);
        let bg_token = if destructive {
            Token::ERROR
        } else {
            Token::CONTROL_HOVER
        };
        let mut bg = instance(
            rect,
            [0.0; 4],
            bg_token,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            0,
        );
        bg.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(0, if active { 1.0 } else { 0.0 });
        output.primitives.push(bg);

        let fg = if destructive && active {
            Token::ON_ERROR
        } else {
            Token::TEXT_PRIMARY
        };
        let middle = rect.center();
        match self.kind {
            WindowCaptionKind::Minimize => {
                output.primitives.push(instance(
                    Rect::new(middle.x - 5.0, (middle.y - 0.5).round(), 10.0, 1.0),
                    [0.0; 4],
                    fg,
                    Token::NONE,
                    0.0,
                    0.0,
                    PrimType::RoundedRect,
                    SubElement::Content,
                    1,
                ));
            }
            WindowCaptionKind::Maximize { maximized } => {
                if maximized {
                    output.primitives.push(instance(
                        Rect::new(middle.x - 3.0, middle.y - 5.0, 8.0, 8.0),
                        [0.0; 4],
                        Token::NONE,
                        fg,
                        1.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        1,
                    ));
                    let occluder = if active {
                        Token::CONTROL_HOVER
                    } else {
                        Token::HEADER_BG
                    };
                    output.primitives.push(instance(
                        Rect::new(middle.x - 5.0, middle.y - 3.0, 8.0, 8.0),
                        [0.0; 4],
                        occluder,
                        fg,
                        1.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        2,
                    ));
                } else {
                    let square = Rect::new(middle.x - 5.0, middle.y - 5.0, 10.0, 10.0);
                    let mut pad = instance(
                        square,
                        [0.0; 4],
                        Token::NONE,
                        Token::NONE,
                        0.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        1,
                    );
                    pad.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(1, 0.0);
                    output.primitives.push(pad);
                    output.primitives.push(instance(
                        square,
                        [0.0; 4],
                        Token::NONE,
                        fg,
                        1.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        2,
                    ));
                }
            }
            WindowCaptionKind::Close => {
                let half = 4.5;
                let strokes = [
                    (
                        (middle.x - half, middle.y - half),
                        (middle.x + half, middle.y + half),
                    ),
                    (
                        (middle.x + half, middle.y - half),
                        (middle.x - half, middle.y + half),
                    ),
                ];
                for (from, to) in strokes {
                    let origin = Rect::new(
                        from.0.min(to.0),
                        from.1.min(to.1),
                        (from.0 - to.0).abs().max(1.0),
                        (from.1 - to.1).abs().max(1.0),
                    );
                    let mut prim = instance(
                        origin,
                        [0.0; 4],
                        fg,
                        Token::NONE,
                        0.0,
                        0.0,
                        PrimType::Curve,
                        SubElement::Content,
                        1,
                    );
                    let payload = GpuPayload::curve(
                        [from.0 - origin.origin.x, from.1 - origin.origin.y],
                        [
                            (from.0 + to.0) * 0.5 - origin.origin.x,
                            (from.1 + to.1) * 0.5 - origin.origin.y,
                        ],
                        [to.0 - origin.origin.x, to.1 - origin.origin.y],
                        1.25,
                    );
                    prim.extra_payload_idx = output.payloads.len() as u32;
                    output.payloads.push(payload);
                    output.primitives.push(prim);
                }
            }
        }
        let label = match self.kind {
            WindowCaptionKind::Minimize => "Minimize window",
            WindowCaptionKind::Maximize { maximized: true } => "Restore window",
            WindowCaptionKind::Maximize { maximized: false } => "Maximize window",
            WindowCaptionKind::Close => "Close window",
        };
        output.hit = Some(HitRegion::new(node, 0, 0, rect));
        output.a11y = Some(A11yNode::new(Role::Button, label.to_owned(), rect));
        output
    }
}

/// 默认标题栏“最小化”按钮的保留节点键。
pub const TITLE_BAR_MINIMIZE_ID: u64 = 0xFFFF_FF01;

/// 默认标题栏“最大化 / 还原”按钮的保留节点键。
pub const TITLE_BAR_MAXIMIZE_ID: u64 = 0xFFFF_FF02;

/// 默认标题栏“关闭”按钮的保留节点键。
pub const TITLE_BAR_CLOSE_ID: u64 = 0xFFFF_FF03;

/// 无边框窗口默认标题栏配置。
#[derive(Clone, Debug, PartialEq)]
pub struct TitleBarDesc {
    /// 显示在标题栏左侧的窗口标题。
    pub title: String,
    /// 标题栏高度（逻辑像素）。
    pub height: f32,
    /// 窗口当前是否处于最大化状态（用于切换最大化/还原图标）。
    pub is_maximized: bool,
    /// 是否显示最小化按钮。
    pub show_minimize: bool,
    /// 是否显示最大化/还原按钮。
    pub show_maximize: bool,
    /// 是否显示关闭按钮。
    pub show_close: bool,
    /// 最小化按钮节点键。
    pub minimize_id: u64,
    /// 最大化按钮节点键。
    pub maximize_id: u64,
    /// 关闭按钮节点键。
    pub close_id: u64,
    /// 是否显示默认标题徽章（使用自定义 `left` 插槽时可关闭）。
    pub show_title: bool,
}

impl Default for TitleBarDesc {
    fn default() -> Self {
        Self::new("Unbound UI")
    }
}

impl TitleBarDesc {
    /// 使用窗口标题构造默认标题栏配置。
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            height: TITLE_BAR_HEIGHT,
            is_maximized: false,
            show_minimize: true,
            show_maximize: true,
            show_close: true,
            minimize_id: TITLE_BAR_MINIMIZE_ID,
            maximize_id: TITLE_BAR_MAXIMIZE_ID,
            close_id: TITLE_BAR_CLOSE_ID,
            show_title: true,
        }
    }

    /// 设置标题栏高度（逻辑像素）。
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(24.0);
        self
    }

    /// 设置窗口最大化状态（影响按钮显示符号）。
    #[must_use]
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.is_maximized = maximized;
        self
    }

    /// 设置是否显示最小化按钮。
    #[must_use]
    pub fn show_minimize(mut self, show: bool) -> Self {
        self.show_minimize = show;
        self
    }

    /// 设置是否显示最大化按钮。
    #[must_use]
    pub fn show_maximize(mut self, show: bool) -> Self {
        self.show_maximize = show;
        self
    }

    /// 设置是否显示关闭按钮。
    #[must_use]
    pub fn show_close(mut self, show: bool) -> Self {
        self.show_close = show;
        self
    }

    /// 自定义最小化按钮节点键。
    #[must_use]
    pub fn minimize_id(mut self, id: u64) -> Self {
        self.minimize_id = id;
        self
    }

    /// 自定义最大化按钮节点键。
    #[must_use]
    pub fn maximize_id(mut self, id: u64) -> Self {
        self.maximize_id = id;
        self
    }

    /// 自定义关闭按钮节点键。
    #[must_use]
    pub fn close_id(mut self, id: u64) -> Self {
        self.close_id = id;
        self
    }

    /// 设置是否显示默认标题。
    #[must_use]
    pub fn show_title(mut self, show: bool) -> Self {
        self.show_title = show;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_bar_desc_defaults_and_builders() {
        let desc = TitleBarDesc::new("My App")
            .height(42.0)
            .maximized(true)
            .show_minimize(false);
        assert_eq!(desc.title, "My App");
        assert_eq!(desc.height, 42.0);
        assert!(desc.is_maximized);
        assert!(!desc.show_minimize);
        assert!(desc.show_maximize);
        assert!(desc.show_close);
        assert_eq!(desc.close_id, TITLE_BAR_CLOSE_ID);
        assert!(desc.show_title);
    }

    #[test]
    fn window_caption_button_emits_showcase_geometry() {
        // 几何图元按钮不排版文字，但仍需提供合法的文本上下文给 `UiWidget` 调用。
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = crate::TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
        };
        let rect = Rect::new(0.0, 0.0, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_HEIGHT);
        let min_out = WindowCaptionButton::new(WindowCaptionKind::Minimize, TITLE_BAR_HEIGHT)
            .emit_laid_out(1, rect, ControlState::default(), &(), &mut font);
        assert_eq!(min_out.primitives.len(), 2);
        assert!(min_out.hit.is_some());

        let max_out = WindowCaptionButton::new(
            WindowCaptionKind::Maximize { maximized: false },
            TITLE_BAR_HEIGHT,
        )
        .emit_laid_out(2, rect, ControlState::default(), &(), &mut font);
        let restore_out = WindowCaptionButton::new(
            WindowCaptionKind::Maximize { maximized: true },
            TITLE_BAR_HEIGHT,
        )
        .emit_laid_out(2, rect, ControlState::default(), &(), &mut font);
        assert_eq!(max_out.primitives.len(), restore_out.primitives.len());

        let close_out = WindowCaptionButton::new(WindowCaptionKind::Close, TITLE_BAR_HEIGHT)
            .emit_laid_out(3, rect, ControlState::default(), &(), &mut font);
        assert_eq!(close_out.primitives.len(), 3);
        assert_eq!(close_out.payloads.len(), 2);
    }
}

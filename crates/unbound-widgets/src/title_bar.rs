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
use crate::{ControlState, PointerPhase, TextCtx, UiWidget, WidgetInputEvent, WidgetOutput};

/// 默认标题栏高度（逻辑像素）。
///
/// WinUI：`TitleBarCompactHeight` = 32，`AppWindowTitleBar` 的标题栏高度同样是 32；
/// 取紧凑高度后，窗口控制按钮为 46×32 的贴边矩形。
pub const TITLE_BAR_HEIGHT: f32 = 32.0;

/// 壳层融合标题栏高度（逻辑像素）。
///
/// 对齐 WinUI `TabView` 融合标题栏高度：`TabViewItemMinHeight = 32` + 顶部 `8px` 留白 = 40。
pub const TITLE_BAR_FUSED_HEIGHT: f32 = 40.0;

/// 窗口控制按钮（最小化 / 最大化 / 关闭）的默认宽度（逻辑像素，与 Showcase 一致）。
///
/// Win11 的标题栏按钮固定 46 宽（`AppWindowTitleBar` 的 caption button 度量），
/// 三枚按钮合计 138；[`TitleBarChrome`] 的拖拽命中区据此让出右侧这一段。
pub const TITLE_BAR_CAPTION_WIDTH: f32 = 46.0;

/// 默认标题栏三枚窗口控制按钮占用的横向空间（46 × 3 = 138）。
///
/// Win11 的窗口控制按钮属于 non-client 区域，DWM 会把它们从拖拽区里挖掉；
/// 这里同样让拖拽命中区止于按钮列的左沿，避免与按钮命中区重叠。
const TITLE_BAR_CAPTION_RESERVE: f32 = TITLE_BAR_CAPTION_WIDTH * 3.0;

/// 拖拽区至少保留的宽度（WinUI `TitleBarMinDragRegionWidth` = 48）。
///
/// 窗口窄到按钮列几乎占满标题栏时，优先保住这段可拖拽区域而不是让它归零。
pub const TITLE_BAR_MIN_DRAG_WIDTH: f32 = 48.0;

/// 标题栏背景表面与底部分隔线的图元深度。
///
/// 窗口根背景固定落在 `80`、普通容器表面按树深度递增（上限 `480`），
/// 因此标题栏 chrome 必须落在它们之上、页面交互控件之下。
pub const TITLE_BAR_CHROME_Z: u16 = 100;

/// 标题栏底部 1px 分隔线的图元深度（紧贴背景之上）。
pub const TITLE_BAR_CHROME_RULE_Z: u16 = 101;

/// 窗口控制按钮底色与命中区域的深度。
///
/// 与 `Button` 默认深度（`703`）同带并高一级，保证标题栏 chrome 永远压住页面表面，
/// 同时不再与任何节点键耦合。
pub const TITLE_BAR_CAPTION_Z: u16 = 704;

/// 窗口控制按钮几何图元（最小化横线 / 还原方框 / 关闭叉）的深度。
pub const TITLE_BAR_CAPTION_GLYPH_Z: u16 = 706;

/// 标题栏拖拽区域（背景装饰控件）的保留节点键。
///
/// 该键是默认标题栏的根容器键，[`TitleBarChrome`] 使用它的派生键挂载，
/// 因此 `AppRuntime::widget_rect` 能直接读出标题栏包围盒；纯布局容器不在挂载控件表中，
/// 无法这样读取。
pub const TITLE_BAR_DRAG_ID: u64 = 0xFFFF_FF00;

/// 默认标题栏的背景表面与底部分隔线。
///
/// 与 showcase 的 `draw_top_bar` 保持一致的视觉约定：`HEADER_BG` 铺满整条
/// 标题栏，底边以 1px `DIVIDER` 与内容区分隔（融合态下可通过
/// `show_bottom_divider = false` 隐藏底边线）。它是绝对定位的纯装饰控件，
/// 不参与 flex 排布；只发射 `layer = 0` 的最低优先级命中区，作为标题栏空白处的
/// 冒泡起点（拖动窗口 / 双击最大化由标题栏根容器的冒泡兜底接管）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TitleBarChrome {
    /// 当前窗口是否处于激活聚焦状态（失焦时降低标题文本与图标对比度）。
    pub focused: bool,
    /// 是否显示底部 1px 分隔线（壳层融合态设为 `false`，由 `TabView` 统一绘制）。
    pub show_bottom_divider: bool,
    /// 左侧固定非拖拽内容槽预留宽度（逻辑像素）。
    pub left_reserved_width: f32,
    /// 右侧自定义全局操作槽预留宽度（不含三枚系统窗口控制按钮，逻辑像素）。
    pub right_reserved_width: f32,
    /// 右侧系统窗口控制按钮区预留宽度（默认 `46 × 3 = 138`）。
    pub caption_reserved_width: f32,
}

impl Default for TitleBarChrome {
    fn default() -> Self {
        Self::new(true)
    }
}

impl TitleBarChrome {
    /// 创建默认标题栏背景装饰控件（保留 138px 窗口控制按钮区与底部分隔线）。
    #[must_use]
    pub const fn new(focused: bool) -> Self {
        Self {
            focused,
            show_bottom_divider: true,
            left_reserved_width: 0.0,
            right_reserved_width: 0.0,
            caption_reserved_width: TITLE_BAR_CAPTION_RESERVE,
        }
    }

    /// 从 [`TitleBarDesc`] 构造同步配置的背景装饰控件。
    #[must_use]
    pub fn from_desc(desc: &TitleBarDesc, focused: bool) -> Self {
        let caption_count = usize::from(desc.show_minimize)
            + usize::from(desc.show_maximize)
            + usize::from(desc.show_close);
        Self {
            focused,
            show_bottom_divider: desc.show_bottom_divider,
            left_reserved_width: desc.left_reserved_width.max(0.0),
            right_reserved_width: desc.right_reserved_width.max(0.0),
            caption_reserved_width: caption_count as f32 * TITLE_BAR_CAPTION_WIDTH,
        }
    }

    /// 设置是否显示底部 1px 分隔线。
    #[must_use]
    pub const fn show_bottom_divider(mut self, show: bool) -> Self {
        self.show_bottom_divider = show;
        self
    }

    /// 设置左侧与右侧自定义内容槽的预留宽度（逻辑像素）。
    #[must_use]
    pub fn with_reserved_widths(mut self, left: f32, right: f32) -> Self {
        self.left_reserved_width = left.max(0.0);
        self.right_reserved_width = right.max(0.0);
        self
    }

    /// 覆盖右侧窗口控制按钮区预留宽度（逻辑像素）。
    #[must_use]
    pub fn with_caption_reserved_width(mut self, width: f32) -> Self {
        self.caption_reserved_width = width.max(0.0);
        self
    }

    /// 计算扣除窗口控制按钮与左右预留槽后的拖拽区矩形。
    ///
    /// 始终保障在窗口宽度足够时，拖拽区宽度不少于 [`TITLE_BAR_MIN_DRAG_WIDTH`]（48px）。
    #[must_use]
    pub fn drag_rect(&self, rect: Rect) -> Rect {
        let total_w = rect.size.x.max(0.0);
        let min_drag = TITLE_BAR_MIN_DRAG_WIDTH.min(total_w);
        let budget = (total_w - min_drag).max(0.0);
        let total_right = self.caption_reserved_width.max(0.0) + self.right_reserved_width.max(0.0);
        let right_reserve = total_right.min(budget);
        let left_reserve = self
            .left_reserved_width
            .max(0.0)
            .min((budget - right_reserve).max(0.0));
        Rect::new(
            rect.origin.x + left_reserve,
            rect.origin.y,
            (total_w - left_reserve - right_reserve).max(0.0),
            rect.size.y,
        )
    }
}

impl UiWidget for TitleBarChrome {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        // 真实尺寸由 `ViewBuilder` 以绝对定位的显式宽高给出，此处仅占位。
        Vec2::new(1.0, 1.0)
    }

    fn emit_laid_out(
        &self,
        node: u64,
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
            TITLE_BAR_CHROME_Z,
        ));
        let rule = Rect::new(rect.origin.x, rect.max().y - 1.0, rect.size.x, 1.0);
        let mut rule_prim = instance(
            rule,
            [0.0; 4],
            Token::DIVIDER,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Background,
            TITLE_BAR_CHROME_RULE_Z,
        );
        rule_prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
            TITLE_BAR_CHROME_RULE_Z,
            if self.show_bottom_divider { 1.0 } else { 0.0 },
        );
        output.primitives.push(rule_prim);
        // 发射 layer=0、层内 z=0 的最低优先级命中区：标题栏空白处的按压由它作为冒泡起点。
        // 命中区自动扣除右侧窗口控制按钮列与自定义操作槽，并保证拖拽区宽度至少保留 48px。
        let drag = self.drag_rect(rect);
        output.hit = Some(HitRegion::new(node, 0, 0, drag));
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
    pub focused: bool,
}

impl WindowCaptionButton {
    #[must_use]
    pub fn new(kind: WindowCaptionKind, height: f32) -> Self {
        Self {
            kind,
            height: height.max(24.0),
            focused: true,
        }
    }

    /// 设置窗口是否处于激活聚焦状态（失焦时降低图标fg对比度）。
    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

impl UiWidget for WindowCaptionButton {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(TITLE_BAR_CAPTION_WIDTH, self.height)
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        // 消费 PointerDown 以阻止按压控制按钮时事件继续冒泡到标题栏根容器触发窗口拖拽。
        matches!(event, WidgetInputEvent::PointerDown { .. })
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
        let pressed = matches!(control.phase, PointerPhase::Pressed);
        let destructive = matches!(self.kind, WindowCaptionKind::Close);
        // WinUI 的窗口控制按钮是「透明底 + 状态薄纱」：悬停 `SubtleFillColorSecondary`、
        // 按压 `SubtleFillColorTertiary`（即既有 `HOVER` / `PRESSED` 令牌）。
        // 关闭按钮例外：悬停/按压铺 `WindowCaptionCloseButtonBackgroundPointerOver` =
        // #C42B1C，本仓库 `Token::ERROR` 在 Win11 浅色调色板中正是该值。
        let bg_token = if destructive {
            Token::ERROR
        } else if pressed {
            Token::PRESSED
        } else {
            Token::HOVER
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
            TITLE_BAR_CAPTION_Z,
        );
        bg.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
            TITLE_BAR_CAPTION_Z,
            if active { 1.0 } else { 0.0 },
        );
        output.primitives.push(bg);

        let fg = if destructive && active {
            // 关闭按钮悬停/按压时图标反白：WinUI `WindowCaptionCloseButtonForegroundPointerOver`
            // 取 `TextOnAccentFillColorPrimary`（浅色 #FFFFFF）。
            Token::ON_ERROR
        } else if self.focused || active {
            Token::TEXT_PRIMARY
        } else {
            // 窗口失焦：WinUI `TitleBarDeactivatedForegroundBrush` = `TextFillColorTertiaryBrush`，
            // 并叠加 `TitleBarDeactivatedOpacity` 0.5；这里用同量级的既有 TEXT_DISABLED。
            Token::TEXT_DISABLED
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
                    TITLE_BAR_CAPTION_GLYPH_Z,
                ));
            }
            WindowCaptionKind::Maximize { maximized } => {
                if maximized {
                    // 前方 8x8 完整镂空方框 + 后方 L 形外露两条边，避免实心遮挡块在透明/Mica 背景下露馅。
                    output.primitives.push(instance(
                        Rect::new(middle.x - 5.0, middle.y - 3.0, 8.0, 8.0),
                        [0.0; 4],
                        Token::NONE,
                        fg,
                        1.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        TITLE_BAR_CAPTION_GLYPH_Z,
                    ));
                    // 后方方框上沿横边 (x: -3..+5, y: -5)
                    output.primitives.push(instance(
                        Rect::new(middle.x - 3.0, middle.y - 5.0, 8.0, 1.0),
                        [0.0; 4],
                        fg,
                        Token::NONE,
                        0.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        TITLE_BAR_CAPTION_GLYPH_Z,
                    ));
                    // 后方方框右沿竖边 (x: +4, y: -4..+3)
                    output.primitives.push(instance(
                        Rect::new(middle.x + 4.0, middle.y - 4.0, 1.0, 7.0),
                        [0.0; 4],
                        fg,
                        Token::NONE,
                        0.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        TITLE_BAR_CAPTION_GLYPH_Z,
                    ));
                } else {
                    let square = Rect::new(middle.x - 5.0, middle.y - 5.0, 10.0, 10.0);
                    output.primitives.push(instance(
                        square,
                        [0.0; 4],
                        Token::NONE,
                        fg,
                        1.0,
                        0.0,
                        PrimType::RoundedRect,
                        SubElement::Content,
                        TITLE_BAR_CAPTION_GLYPH_Z,
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
                        TITLE_BAR_CAPTION_GLYPH_Z,
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
        output.hit = Some(HitRegion::new(
            node,
            crate::button::BUTTON_LAYER,
            u32::from(TITLE_BAR_CAPTION_Z),
            rect,
        ));
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
    /// 标题栏根容器节点键（默认 [`TITLE_BAR_DRAG_ID`]，多标题栏场景可自定义避免键冲突）。
    pub id: u64,
    /// 显示在标题栏左侧的窗口标题。
    pub title: String,
    /// 标题栏高度（逻辑像素）。
    pub height: f32,
    /// 窗口当前是否处于最大化状态（用于切换最大化/还原图标）。
    pub is_maximized: bool,
    /// 显式覆盖最大化状态（`None` 时自动跟随 `ViewContext::window_maximized`）。
    pub maximized_override: Option<bool>,
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
    /// 是否显示底部 1px 分隔线（壳层融合态可设为 `false`）。
    pub show_bottom_divider: bool,
    /// 左侧非拖拽内容槽预留宽度（逻辑像素）。
    pub left_reserved_width: f32,
    /// 右侧自定义全局操作槽预留宽度（逻辑像素，不含三枚窗口按钮）。
    pub right_reserved_width: f32,
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
            id: TITLE_BAR_DRAG_ID,
            title: title.into(),
            height: TITLE_BAR_HEIGHT,
            is_maximized: false,
            maximized_override: None,
            show_minimize: true,
            show_maximize: true,
            show_close: true,
            minimize_id: TITLE_BAR_MINIMIZE_ID,
            maximize_id: TITLE_BAR_MAXIMIZE_ID,
            close_id: TITLE_BAR_CLOSE_ID,
            show_title: true,
            show_bottom_divider: true,
            left_reserved_width: 0.0,
            right_reserved_width: 0.0,
        }
    }

    /// 设置标题栏高度（逻辑像素）。
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(24.0);
        self
    }

    /// 设置壳层融合态标题栏高度（常取 [`TITLE_BAR_FUSED_HEIGHT`] = `40.0`）。
    #[must_use]
    pub fn with_fused_height(mut self, height: f32) -> Self {
        self.height = height.max(24.0);
        self
    }

    /// 设置是否显示底部 1px 分隔线。
    #[must_use]
    pub fn show_bottom_divider(mut self, show: bool) -> Self {
        self.show_bottom_divider = show;
        self
    }

    /// 设置左侧与右侧自定义内容槽预留宽度（逻辑像素）。
    #[must_use]
    pub fn with_reserved_widths(mut self, left: f32, right: f32) -> Self {
        self.left_reserved_width = left.max(0.0);
        self.right_reserved_width = right.max(0.0);
        self
    }

    /// 自定义标题栏根容器节点键（多标题栏场景用于隔离键空间）。
    #[must_use]
    pub fn id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// 显式设置窗口最大化状态（影响按钮显示符号）。
    #[must_use]
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.is_maximized = maximized;
        self.maximized_override = Some(maximized);
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

    fn text_ctx<'a>(
        engine: &'a mut unbound_text::TextEngine,
        cache: &'a mut unbound_text::GlyphCache,
    ) -> crate::TextCtx<'a> {
        crate::TextCtx {
            engine,
            cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        }
    }

    #[test]
    fn title_bar_desc_defaults_and_builders() {
        let desc = TitleBarDesc::new("My App")
            .height(42.0)
            .maximized(true)
            .show_minimize(false);
        assert_eq!(desc.title, "My App");
        assert_eq!(desc.height, 42.0);
        assert!(desc.is_maximized);
        assert_eq!(desc.maximized_override, Some(true));
        assert!(!desc.show_minimize);
        assert!(desc.show_maximize);
        assert!(desc.show_close);
        assert_eq!(desc.id, TITLE_BAR_DRAG_ID);
        assert_eq!(desc.close_id, TITLE_BAR_CLOSE_ID);
        assert!(desc.show_title);
        assert!(desc.show_bottom_divider);
        assert_eq!(desc.left_reserved_width, 0.0);
        assert_eq!(desc.right_reserved_width, 0.0);
    }

    #[test]
    fn title_bar_desc_fused_builders_configure_height_divider_and_reserves() {
        let desc = TitleBarDesc::new("Explorer")
            .with_fused_height(TITLE_BAR_FUSED_HEIGHT)
            .show_bottom_divider(false)
            .with_reserved_widths(120.0, 64.0)
            .show_title(false);
        assert_eq!(desc.height, 40.0);
        assert!(!desc.show_bottom_divider);
        assert_eq!(desc.left_reserved_width, 120.0);
        assert_eq!(desc.right_reserved_width, 64.0);
        assert!(!desc.show_title);
    }

    #[test]
    fn title_bar_desc_id_and_maximized_override_default_to_follow_context() {
        // 未显式调用 `.maximized()` 时保持 `None`，由 `ViewContext::window_maximized` 决定。
        let desc = TitleBarDesc::new("Multi");
        assert_eq!(desc.maximized_override, None);
        // 多标题栏场景必须能用 `.id()` 隔离根容器键空间。
        let custom = desc.clone().id(0x1234_5678);
        assert_eq!(custom.id, 0x1234_5678);
        assert_ne!(custom.id, TITLE_BAR_DRAG_ID);
    }

    #[test]
    fn title_bar_chrome_emits_background_band_and_lowest_priority_hit() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);
        let rect = Rect::new(0.0, 0.0, 1000.0, TITLE_BAR_HEIGHT);
        let out = TitleBarChrome::new(true).emit_laid_out(
            1,
            rect,
            ControlState::default(),
            &(),
            &mut font,
        );

        // 背景铺满整条标题栏 + 底部 1px 分隔线，且深度落在窗口根背景（80）之上。
        assert_eq!(out.primitives.len(), 2);
        assert_eq!(out.primitives[0].depth_u16(), TITLE_BAR_CHROME_Z);
        const { assert!(TITLE_BAR_CHROME_Z > 80) };
        assert_eq!(out.primitives[1].depth_u16(), TITLE_BAR_CHROME_RULE_Z);
        const { assert!(TITLE_BAR_CHROME_RULE_Z > TITLE_BAR_CHROME_Z) };
        assert_eq!(
            out.primitives[1].size[1], 1.0,
            "divider must be exactly 1px"
        );
        assert_eq!(out.primitives[1].opacity(), 1.0);

        // 命中区必须是最低优先级：layer 0（画布层）且层内 z 为 0，
        // 任何 layer >= 1 的交互控件与浮层都天然覆盖其上。
        let hit = out
            .hit
            .as_ref()
            .expect("the title bar background must emit a hit region as the bubbling origin");
        assert_eq!((hit.layer, hit.z), (0, 0));
        // 拖拽区止于三枚窗口控制按钮（46 × 3）的左沿，与按钮命中区互不重叠。
        assert_eq!(hit.bounds.origin, rect.origin);
        assert_eq!(hit.bounds.size.y, rect.size.y);
        assert_eq!(
            hit.bounds.size.x,
            rect.size.x - TITLE_BAR_CAPTION_WIDTH * 3.0
        );
    }

    #[test]
    fn fused_title_bar_chrome_hides_divider_and_preserves_minimum_drag_width() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);

        let desc = TitleBarDesc::new("Fused")
            .with_fused_height(TITLE_BAR_FUSED_HEIGHT)
            .show_bottom_divider(false)
            .with_reserved_widths(200.0, 64.0);
        let chrome = TitleBarChrome::from_desc(&desc, true);

        // 宽窗口：扣除左侧 200px、右侧操作槽 64px 与三枚窗口按钮 138px。
        let wide_rect = Rect::new(0.0, 0.0, 1000.0, TITLE_BAR_FUSED_HEIGHT);
        let wide_out = chrome.emit_laid_out(1, wide_rect, ControlState::default(), &(), &mut font);
        assert_eq!(wide_out.primitives.len(), 2);
        assert_eq!(
            wide_out.primitives[1].opacity(),
            0.0,
            "fused title bar must hide bottom divider via zero opacity"
        );
        let wide_hit = wide_out.hit.expect("drag region");
        assert_eq!(wide_hit.bounds.origin.x, 200.0);
        assert_eq!(
            wide_hit.bounds.size.x,
            1000.0 - 200.0 - 64.0 - TITLE_BAR_CAPTION_WIDTH * 3.0
        );

        // 窄窗口：即使预留宽度之和超出窗口宽度，仍严格保留至少 48px 拖拽区。
        for width in [48.0f32, 120.0, 180.0, 260.0, 400.0] {
            let narrow_rect = Rect::new(0.0, 0.0, width, TITLE_BAR_FUSED_HEIGHT);
            let drag = chrome.drag_rect(narrow_rect);
            assert!(
                drag.size.x >= TITLE_BAR_MIN_DRAG_WIDTH,
                "width={width}: drag width {} must be >= {}",
                drag.size.x,
                TITLE_BAR_MIN_DRAG_WIDTH
            );
            assert!(drag.max().x <= narrow_rect.max().x + 1e-4);
        }
    }

    #[test]
    fn window_caption_button_consumes_pointer_down_only() {
        let button = WindowCaptionButton::new(WindowCaptionKind::Close, TITLE_BAR_HEIGHT);
        let mut state = ();
        let rect = Rect::new(0.0, 0.0, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_HEIGHT);
        assert!(button.reduce(
            &mut state,
            rect,
            &WidgetInputEvent::PointerDown {
                pos: rect.center(),
                click_count: 1,
            }
        ));
        // 释放不得吞掉，`Router` 合成的 `Click` 还要继续冒泡给应用层做拦截判定。
        assert!(!button.reduce(
            &mut state,
            rect,
            &WidgetInputEvent::PointerUp { pos: rect.center() }
        ));
    }

    #[test]
    fn window_caption_button_hover_highlight_and_geometry() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);
        let rect = Rect::new(0.0, 0.0, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_HEIGHT);

        // 空闲态：底色图元存在但不透明度为 0；悬停态：不透明度为 1。
        let idle = WindowCaptionButton::new(WindowCaptionKind::Minimize, TITLE_BAR_HEIGHT)
            .emit_laid_out(1, rect, ControlState::default(), &(), &mut font);
        assert_eq!(idle.primitives.len(), 2);
        assert_eq!(idle.primitives[0].depth_u16(), TITLE_BAR_CAPTION_Z);
        assert_eq!(idle.primitives[0].opacity(), 0.0);
        assert_eq!(idle.primitives[1].depth_u16(), TITLE_BAR_CAPTION_GLYPH_Z);

        let hovered = WindowCaptionButton::new(WindowCaptionKind::Minimize, TITLE_BAR_HEIGHT)
            .emit_laid_out(
                1,
                rect,
                ControlState {
                    phase: PointerPhase::Hovered,
                    ..ControlState::default()
                },
                &(),
                &mut font,
            );
        assert_eq!(hovered.primitives[0].opacity(), 1.0);
        assert_eq!(hovered.primitives[0].size, [rect.size.x, rect.size.y]);

        // 命中区深度与控制按钮底色同带，高于页面按钮默认深度。
        let hit = hovered
            .hit
            .as_ref()
            .expect("caption buttons must emit a hit region");
        assert_eq!(hit.layer, crate::button::BUTTON_LAYER);
        assert_eq!(hit.z, u32::from(TITLE_BAR_CAPTION_Z));

        // 还原图标必须是 4 个图元：底色 + front_square + 后方两条外露边。
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
        assert_eq!(max_out.primitives.len(), 2);
        assert_eq!(restore_out.primitives.len(), 4);

        let close_out = WindowCaptionButton::new(WindowCaptionKind::Close, TITLE_BAR_HEIGHT)
            .emit_laid_out(3, rect, ControlState::default(), &(), &mut font);
        assert_eq!(close_out.primitives.len(), 3);
        assert_eq!(close_out.payloads.len(), 2);
    }

    #[test]
    fn restore_icon_is_hollow_outline_without_solid_occluder() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);
        let rect = Rect::new(0.0, 0.0, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_HEIGHT);
        let out = WindowCaptionButton::new(
            WindowCaptionKind::Maximize { maximized: true },
            TITLE_BAR_HEIGHT,
        )
        .emit_laid_out(2, rect, ControlState::default(), &(), &mut font);

        // 旧实现用 `HEADER_BG` / `CONTROL_HOVER` 实心块遮挡后方方框；新实现里除了
        // 前景描边色与 `Token::NONE`，任何图元都不得再引用背景类令牌。
        let fg = u32::from(Token::TEXT_PRIMARY.raw());
        for prim in &out.primitives[1..] {
            assert!(
                prim.fill_token_or_uv == fg
                    || prim.fill_token_or_uv == u32::from(Token::NONE.raw()),
                "restore icon fills may only be the foreground color or none; never mask the back square with a background block"
            );
            assert!(
                prim.border_token_or_tex == fg
                    || prim.border_token_or_tex == u32::from(Token::NONE.raw()),
                "restore icon borders may only be the foreground color or none"
            );
            assert_eq!(prim.depth_u16(), TITLE_BAR_CAPTION_GLYPH_Z);
        }

        // front_square必须是真的镂空：无填充 + 1px 前景描边。
        let front_square = &out.primitives[1];
        assert_eq!(front_square.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(front_square.border_token_or_tex, fg);

        // 后方方框只保留「上沿横边 + 右沿竖边」两条 1px 前景线，其余边绝不出现。
        assert_eq!(out.primitives[2].size, [8.0, 1.0]);
        assert_eq!(out.primitives[3].size, [1.0, 7.0]);
    }

    #[test]
    fn unfocused_caption_button_lowers_foreground_contrast() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);
        let rect = Rect::new(0.0, 0.0, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_HEIGHT);

        let focused = WindowCaptionButton::new(WindowCaptionKind::Minimize, TITLE_BAR_HEIGHT)
            .focused(true)
            .emit_laid_out(1, rect, ControlState::default(), &(), &mut font);
        let unfocused = WindowCaptionButton::new(WindowCaptionKind::Minimize, TITLE_BAR_HEIGHT)
            .focused(false)
            .emit_laid_out(1, rect, ControlState::default(), &(), &mut font);

        assert_eq!(
            focused.primitives[1].fill_token_or_uv,
            u32::from(Token::TEXT_PRIMARY.raw())
        );
        assert_eq!(
            unfocused.primitives[1].fill_token_or_uv,
            u32::from(Token::TEXT_DISABLED.raw()),
            "an unfocused window must lower the icon contrast"
        );
    }
}

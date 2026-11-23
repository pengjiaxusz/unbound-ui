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
/// 对齐 Win11 Fluent 融合标题栏高度（`48px`）：`38px` 标签卡片 + 顶部 `10px` 留白。
pub const TITLE_BAR_FUSED_HEIGHT: f32 = 48.0;

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
        let caption = desc.caption_buttons(focused);
        Self {
            focused,
            show_bottom_divider: desc.show_bottom_divider,
            left_reserved_width: desc.left_reserved_width.max(0.0),
            right_reserved_width: desc.right_reserved_width.max(0.0),
            caption_reserved_width: caption.total_width(),
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

/// 独立窗口控制按钮组描述符（最小化 / 最大化还原 / 关闭，ADR-0029）。
///
/// 默认严格遵循 Win11 Fluent 规范：单钮宽 [`TITLE_BAR_CAPTION_WIDTH`]（`46.0px`）、
/// 默认高 [`TITLE_BAR_HEIGHT`]（`32.0px`）、零圆角贴边、标准悬停薄纱与关闭红底反白，
/// 并随窗口最大化状态自动在最大化/还原图标间切换。支持自定义高度以及
/// `show_minimize` / `show_maximize` / `show_close` 开关以适配自定义顶栏、对话框与工具面板。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowCaptionButtons {
    /// 按钮高度（逻辑像素，默认 `32.0`，下限 `24.0`）。
    pub height: f32,
    /// 单枚按钮宽度（逻辑像素，默认 `46.0`，下限 `24.0`）。
    pub button_width: f32,
    /// 显式覆盖窗口最大化状态（`None` 时自动跟随宿主上下文 `window_maximized`）。
    pub maximized_override: Option<bool>,
    /// 窗口是否处于激活聚焦态（失焦时降低图标前景对比度）。
    pub focused: bool,
    /// 是否显示最小化按钮。
    pub show_minimize: bool,
    /// 是否显示最大化/还原按钮。
    pub show_maximize: bool,
    /// 是否显示关闭按钮。
    pub show_close: bool,
    /// 最小化按钮节点键。
    pub minimize_id: u64,
    /// 最大化/还原按钮节点键。
    pub maximize_id: u64,
    /// 关闭按钮节点键。
    pub close_id: u64,
}

impl Default for WindowCaptionButtons {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowCaptionButtons {
    /// 创建默认遵循 Win11 规范（`46.0×32.0` 三联按钮）的窗口控制按钮组。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            height: TITLE_BAR_HEIGHT,
            button_width: TITLE_BAR_CAPTION_WIDTH,
            maximized_override: None,
            focused: true,
            show_minimize: true,
            show_maximize: true,
            show_close: true,
            minimize_id: TITLE_BAR_MINIMIZE_ID,
            maximize_id: TITLE_BAR_MAXIMIZE_ID,
            close_id: TITLE_BAR_CLOSE_ID,
        }
    }

    /// 设置按钮高度（逻辑像素，下限 `24.0`）。
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(24.0);
        self
    }

    /// 设置单枚按钮宽度（逻辑像素，下限 `24.0`）。
    #[must_use]
    pub fn button_width(mut self, width: f32) -> Self {
        self.button_width = width.max(24.0);
        self
    }

    /// 显式覆盖窗口最大化状态（传 `Some` 后忽略宿主 `window_maximized`）。
    #[must_use]
    pub const fn maximized(mut self, maximized: bool) -> Self {
        self.maximized_override = Some(maximized);
        self
    }

    /// 设置窗口是否处于激活聚焦态。
    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// 设置是否显示最小化按钮。
    #[must_use]
    pub const fn show_minimize(mut self, show: bool) -> Self {
        self.show_minimize = show;
        self
    }

    /// 设置是否显示最大化/还原按钮。
    #[must_use]
    pub const fn show_maximize(mut self, show: bool) -> Self {
        self.show_maximize = show;
        self
    }

    /// 设置是否显示关闭按钮。
    #[must_use]
    pub const fn show_close(mut self, show: bool) -> Self {
        self.show_close = show;
        self
    }

    /// 自定义最小化按钮节点键。
    #[must_use]
    pub const fn minimize_id(mut self, id: u64) -> Self {
        self.minimize_id = id;
        self
    }

    /// 自定义最大化/还原按钮节点键。
    #[must_use]
    pub const fn maximize_id(mut self, id: u64) -> Self {
        self.maximize_id = id;
        self
    }

    /// 自定义关闭按钮节点键。
    #[must_use]
    pub const fn close_id(mut self, id: u64) -> Self {
        self.close_id = id;
        self
    }

    /// 解析当前生效的窗口最大化状态。
    #[must_use]
    pub const fn resolve_maximized(&self, window_maximized: bool) -> bool {
        match self.maximized_override {
            Some(value) => value,
            None => window_maximized,
        }
    }

    /// 当前可见按钮数量。
    #[must_use]
    pub const fn visible_count(&self) -> usize {
        (self.show_minimize as usize) + (self.show_maximize as usize) + (self.show_close as usize)
    }

    /// 按钮组整体占用的逻辑宽度。
    #[must_use]
    pub fn total_width(&self) -> f32 {
        self.visible_count() as f32 * self.button_width
    }

    /// 按钮组固有尺寸（总宽度 × 高度）。
    #[must_use]
    pub fn intrinsic_size(&self) -> Vec2 {
        Vec2::new(self.total_width(), self.height)
    }

    /// 生成按自左向右顺序排列的可见单钮描述符列表。
    #[must_use]
    pub fn buttons(&self, window_maximized: bool) -> Vec<(u64, WindowCaptionButton)> {
        let maximized = self.resolve_maximized(window_maximized);
        let mut list = Vec::with_capacity(self.visible_count());
        if self.show_minimize {
            list.push((
                self.minimize_id,
                WindowCaptionButton::new(WindowCaptionKind::Minimize, self.height)
                    .focused(self.focused),
            ));
        }
        if self.show_maximize {
            list.push((
                self.maximize_id,
                WindowCaptionButton::new(WindowCaptionKind::Maximize { maximized }, self.height)
                    .focused(self.focused),
            ));
        }
        if self.show_close {
            list.push((
                self.close_id,
                WindowCaptionButton::new(WindowCaptionKind::Close, self.height)
                    .focused(self.focused),
            ));
        }
        list
    }

    /// 在给定容器矩形右沿对齐排布可见按钮，返回精确几何布局。
    ///
    /// 当 `container` 为整条顶栏矩形时，按钮组自动贴齐右沿；
    /// 当 `container` 为按钮组自身矩形时，起点与 `container.origin` 重合。
    #[must_use]
    pub fn layout(&self, container: Rect) -> WindowCaptionButtonsLayout {
        let total_w = self.total_width();
        let height = if container.size.y > 0.0 {
            container.size.y
        } else {
            self.height
        };
        if total_w <= 0.0 {
            return WindowCaptionButtonsLayout {
                bounds: Rect::new(container.max().x, container.origin.y, 0.0, height),
                minimize_rect: None,
                maximize_rect: None,
                close_rect: None,
            };
        }
        let start_x = (container.max().x - total_w).max(container.origin.x);
        let mut cursor_x = start_x;
        let minimize_rect = self.show_minimize.then(|| {
            let r = Rect::new(cursor_x, container.origin.y, self.button_width, height);
            cursor_x += self.button_width;
            r
        });
        let maximize_rect = self.show_maximize.then(|| {
            let r = Rect::new(cursor_x, container.origin.y, self.button_width, height);
            cursor_x += self.button_width;
            r
        });
        let close_rect = self.show_close.then(|| {
            let r = Rect::new(cursor_x, container.origin.y, self.button_width, height);
            cursor_x += self.button_width;
            r
        });
        WindowCaptionButtonsLayout {
            bounds: Rect::new(start_x, container.origin.y, cursor_x - start_x, height),
            minimize_rect,
            maximize_rect,
            close_rect,
        }
    }

    /// 直接计算与 `FrameHitMetrics` 对齐的非客户区热区契约。
    #[must_use]
    pub fn frame_hit_contract(&self, container: Rect) -> WindowCaptionFrameHitContract {
        self.layout(container).frame_hit_contract()
    }
}

/// 窗口控制按钮组的几何布局结果。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowCaptionButtonsLayout {
    /// 整个按钮组占用的包围盒。
    pub bounds: Rect,
    /// 最小化按钮矩形（隐藏时为 `None`）。
    pub minimize_rect: Option<Rect>,
    /// 最大化/还原按钮矩形（隐藏时为 `None`）。
    pub maximize_rect: Option<Rect>,
    /// 关闭按钮矩形（隐藏时为 `None`）。
    pub close_rect: Option<Rect>,
}

impl WindowCaptionButtonsLayout {
    /// 导出统一 `FrameHitMetrics` 热区契约（ADR-0029）。
    ///
    /// 仅暴露最大化按钮矩形用于 Win11 Snap Layout（`HTMAXBUTTON`），
    /// 最小化与关闭矩形固定为 [`Rect::ZERO`]，确保点击始终经框架事件层分发。
    #[must_use]
    pub const fn frame_hit_contract(&self) -> WindowCaptionFrameHitContract {
        WindowCaptionFrameHitContract::from_maximize_rect(self.maximize_rect)
    }
}

/// 统一 Win32 `FrameHitMetrics` 非客户区热区契约（ADR-0029）。
///
/// 与 `unbound-app::window_runner` 保持完全一致：
/// - `title_bar_height` 固定为 `0.0`（拖拽与双击最大化由框架 UI 事件冒泡接管）；
/// - 仅将 `maximize_button_rect` 暴露给系统 `HTMAXBUTTON` 以触发原生 Win11 Snap Layout；
/// - `minimize_button_rect` 与 `close_button_rect` 恒为 [`Rect::ZERO`]，确保最小化与关闭
///   点击始终留给框架事件层以便拦截关闭确认与维持跨平台一致。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowCaptionFrameHitContract {
    /// 写入 `FrameHitMetrics::set_title_bar_height` 的高度（恒为 `0.0`）。
    pub title_bar_height: f32,
    /// 写入 `FrameHitMetrics::set_maximize_button_rect` 的矩形（无最大化按钮时为 `Rect::ZERO`）。
    pub maximize_button_rect: Rect,
    /// 写入 `FrameHitMetrics::set_minimize_button_rect` 的矩形（恒为 `Rect::ZERO`）。
    pub minimize_button_rect: Rect,
    /// 写入 `FrameHitMetrics::set_close_button_rect` 的矩形（恒为 `Rect::ZERO`）。
    pub close_button_rect: Rect,
}

impl WindowCaptionFrameHitContract {
    /// 根据可选的最大化按钮矩形构造统一热区契约。
    #[must_use]
    pub const fn from_maximize_rect(maximize_rect: Option<Rect>) -> Self {
        Self {
            title_bar_height: 0.0,
            maximize_button_rect: match maximize_rect {
                Some(rect) => rect,
                None => Rect::ZERO,
            },
            minimize_button_rect: Rect::ZERO,
            close_button_rect: Rect::ZERO,
        }
    }
}

/// 窗口控制按钮组各按钮的交互控制态集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowCaptionButtonControlStates {
    /// 最小化按钮状态。
    pub minimize: ControlState,
    /// 最大化/还原按钮状态。
    pub maximize: ControlState,
    /// 关闭按钮状态。
    pub close: ControlState,
}

/// 单枚已发射的窗口控制按钮条目。
#[derive(Clone, Debug)]
pub struct WindowCaptionButtonEntry {
    /// 按钮节点键。
    pub node: u64,
    /// 按钮类型（包含解析后的最大化状态）。
    pub kind: WindowCaptionKind,
    /// 按钮布局矩形。
    pub rect: Rect,
    /// 按钮渲染图元、命中区与无障碍节点输出。
    pub output: WidgetOutput,
}

/// 窗口控制按钮组发射输出。
#[derive(Clone, Debug, Default)]
pub struct WindowCaptionButtonsOutput {
    /// 按钮组几何布局。
    pub layout: WindowCaptionButtonsLayout,
    /// 统一 `FrameHitMetrics` 热区契约（仅含最大化按钮矩形）。
    pub frame_hit_contract: WindowCaptionFrameHitContract,
    /// 按自左向右顺序排列的可见按钮输出列表。
    pub entries: Vec<WindowCaptionButtonEntry>,
}

/// 发射独立窗口控制按钮组（`WindowCaptionButtons`）。
///
/// 返回每个可见按钮的独立渲染/命中/无障碍输出，以及可直接同步至平台层的
/// [`WindowCaptionFrameHitContract`]。
#[must_use]
pub fn emit_window_caption_buttons(
    desc: &WindowCaptionButtons,
    container: Rect,
    window_maximized: bool,
    states: WindowCaptionButtonControlStates,
    text_ctx: &mut TextCtx<'_>,
) -> WindowCaptionButtonsOutput {
    let layout = desc.layout(container);
    let frame_hit_contract = layout.frame_hit_contract();
    let maximized = desc.resolve_maximized(window_maximized);
    let mut entries = Vec::with_capacity(desc.visible_count());

    if let Some(rect) = layout.minimize_rect {
        let kind = WindowCaptionKind::Minimize;
        let widget = WindowCaptionButton::new(kind, rect.size.y).focused(desc.focused);
        let output = widget.emit_laid_out(desc.minimize_id, rect, states.minimize, &(), text_ctx);
        entries.push(WindowCaptionButtonEntry {
            node: desc.minimize_id,
            kind,
            rect,
            output,
        });
    }
    if let Some(rect) = layout.maximize_rect {
        let kind = WindowCaptionKind::Maximize { maximized };
        let widget = WindowCaptionButton::new(kind, rect.size.y).focused(desc.focused);
        let output = widget.emit_laid_out(desc.maximize_id, rect, states.maximize, &(), text_ctx);
        entries.push(WindowCaptionButtonEntry {
            node: desc.maximize_id,
            kind,
            rect,
            output,
        });
    }
    if let Some(rect) = layout.close_rect {
        let kind = WindowCaptionKind::Close;
        let widget = WindowCaptionButton::new(kind, rect.size.y).focused(desc.focused);
        let output = widget.emit_laid_out(desc.close_id, rect, states.close, &(), text_ctx);
        entries.push(WindowCaptionButtonEntry {
            node: desc.close_id,
            kind,
            rect,
            output,
        });
    }

    WindowCaptionButtonsOutput {
        layout,
        frame_hit_contract,
        entries,
    }
}

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

    /// 设置壳层融合态标题栏高度（常取 [`TITLE_BAR_FUSED_HEIGHT`] = `48.0`）。
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

    /// 导出组合复用的 [`WindowCaptionButtons`] 描述符。
    ///
    /// `TitleBarDesc` 内部通过该组合视图驱动窗口控制按钮渲染与拖拽区预留计算，
    /// 确保标准标题栏与独立自定义顶栏共享同一套按钮规格与热区契约。
    #[must_use]
    pub fn caption_buttons(&self, focused: bool) -> WindowCaptionButtons {
        let mut buttons = WindowCaptionButtons::new()
            .height(self.height)
            .focused(focused)
            .show_minimize(self.show_minimize)
            .show_maximize(self.show_maximize)
            .show_close(self.show_close)
            .minimize_id(self.minimize_id)
            .maximize_id(self.maximize_id)
            .close_id(self.close_id);
        if let Some(maximized) = self.maximized_override {
            buttons = buttons.maximized(maximized);
        }
        buttons
    }

    /// 使用独立的 [`WindowCaptionButtons`] 配置同步标题栏控制按钮组参数。
    #[must_use]
    pub fn with_caption_buttons(mut self, buttons: WindowCaptionButtons) -> Self {
        self.height = buttons.height;
        self.maximized_override = buttons.maximized_override;
        if let Some(maximized) = buttons.maximized_override {
            self.is_maximized = maximized;
        }
        self.show_minimize = buttons.show_minimize;
        self.show_maximize = buttons.show_maximize;
        self.show_close = buttons.show_close;
        self.minimize_id = buttons.minimize_id;
        self.maximize_id = buttons.maximize_id;
        self.close_id = buttons.close_id;
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
        assert_eq!(desc.height, 48.0);
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

    #[test]
    fn caption_buttons_default_win11_layout_and_custom_height() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);

        let desc = WindowCaptionButtons::new();
        assert_eq!(desc.intrinsic_size(), Vec2::new(138.0, 32.0));

        // 传入整条 1000px 顶栏矩形时，三枚 46×32 按钮应精确贴齐右沿且无缝相邻。
        let bar = Rect::new(0.0, 0.0, 1000.0, TITLE_BAR_HEIGHT);
        let out = emit_window_caption_buttons(
            &desc,
            bar,
            false,
            WindowCaptionButtonControlStates::default(),
            &mut font,
        );
        assert_eq!(out.entries.len(), 3);
        assert_eq!(out.layout.bounds, Rect::new(862.0, 0.0, 138.0, 32.0));
        assert_eq!(
            out.layout.minimize_rect,
            Some(Rect::new(862.0, 0.0, 46.0, 32.0))
        );
        assert_eq!(
            out.layout.maximize_rect,
            Some(Rect::new(908.0, 0.0, 46.0, 32.0))
        );
        assert_eq!(
            out.layout.close_rect,
            Some(Rect::new(954.0, 0.0, 46.0, 32.0))
        );

        // 自定义高度适配融合顶栏（40px），且低于下限时自动钳位到 24px。
        let tall = WindowCaptionButtons::new().height(40.0);
        assert_eq!(tall.intrinsic_size(), Vec2::new(138.0, 40.0));
        let clamped = WindowCaptionButtons::new().height(12.0);
        assert_eq!(clamped.height, 24.0);
    }

    #[test]
    fn caption_buttons_visibility_toggles_and_hit_contract() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);

        // 标准三钮：热区契约仅暴露最大化按钮矩形，最小化/关闭恒为 Rect::ZERO，title_bar_height 恒为 0。
        let bar = Rect::new(0.0, 0.0, 800.0, 36.0);
        let full = emit_window_caption_buttons(
            &WindowCaptionButtons::new().height(36.0),
            bar,
            false,
            WindowCaptionButtonControlStates::default(),
            &mut font,
        );
        assert_eq!(full.frame_hit_contract.title_bar_height, 0.0);
        assert_eq!(
            full.frame_hit_contract.maximize_button_rect,
            Rect::new(708.0, 0.0, 46.0, 36.0)
        );
        assert_eq!(full.frame_hit_contract.minimize_button_rect, Rect::ZERO);
        assert_eq!(full.frame_hit_contract.close_button_rect, Rect::ZERO);
        // 最小化与关闭按钮仍必须向框架 UI 层发射完整客户区 HitRegion，确保点击走框架事件分发。
        assert_eq!(
            full.entries[0].output.hit.as_ref().map(|h| h.bounds),
            full.layout.minimize_rect
        );
        assert_eq!(
            full.entries[2].output.hit.as_ref().map(|h| h.bounds),
            full.layout.close_rect
        );

        // 对话框场景（隐藏最小化与最大化）：仅渲染关闭按钮，且热区契约中最大化矩形归零。
        let dialog = WindowCaptionButtons::new()
            .show_minimize(false)
            .show_maximize(false);
        let dialog_out = emit_window_caption_buttons(
            &dialog,
            bar,
            false,
            WindowCaptionButtonControlStates::default(),
            &mut font,
        );
        assert_eq!(dialog_out.entries.len(), 1);
        assert_eq!(dialog_out.entries[0].kind, WindowCaptionKind::Close);
        assert_eq!(dialog_out.layout.minimize_rect, None);
        assert_eq!(dialog_out.layout.maximize_rect, None);
        assert_eq!(
            dialog_out.layout.close_rect,
            Some(Rect::new(754.0, 0.0, 46.0, 36.0))
        );
        assert_eq!(
            dialog_out.frame_hit_contract.maximize_button_rect,
            Rect::ZERO
        );
        assert_eq!(
            dialog_out.frame_hit_contract.minimize_button_rect,
            Rect::ZERO
        );
        assert_eq!(dialog_out.frame_hit_contract.close_button_rect, Rect::ZERO);
    }

    #[test]
    fn caption_buttons_sync_maximize_icon_and_close_hover_tokens() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut font = text_ctx(&mut engine, &mut cache);

        let desc = WindowCaptionButtons::new();
        let bar = Rect::new(0.0, 0.0, 600.0, TITLE_BAR_HEIGHT);

        // 自动跟随窗口最大化状态：false -> 最大化单框（2 图元）；true -> 还原双框（4 图元）。
        let normal = emit_window_caption_buttons(
            &desc,
            bar,
            false,
            WindowCaptionButtonControlStates::default(),
            &mut font,
        );
        let maximized = emit_window_caption_buttons(
            &desc,
            bar,
            true,
            WindowCaptionButtonControlStates {
                close: ControlState {
                    phase: PointerPhase::Hovered,
                    ..ControlState::default()
                },
                ..WindowCaptionButtonControlStates::default()
            },
            &mut font,
        );
        assert_eq!(
            normal.entries[1].kind,
            WindowCaptionKind::Maximize { maximized: false }
        );
        assert_eq!(normal.entries[1].output.primitives.len(), 2);
        assert_eq!(
            normal.entries[1]
                .output
                .a11y
                .as_ref()
                .map(|a| a.label.as_str()),
            Some("Maximize window")
        );

        assert_eq!(
            maximized.entries[1].kind,
            WindowCaptionKind::Maximize { maximized: true }
        );
        assert_eq!(maximized.entries[1].output.primitives.len(), 4);
        assert_eq!(
            maximized.entries[1]
                .output
                .a11y
                .as_ref()
                .map(|a| a.label.as_str()),
            Some("Restore window")
        );

        // 关闭按钮悬停时必须呈 Win11 红底（Token::ERROR）+ 白色反白叉号（Token::ON_ERROR）。
        let close_prims = &maximized.entries[2].output.primitives;
        assert_eq!(
            close_prims[0].fill_token_or_uv,
            u32::from(Token::ERROR.raw())
        );
        assert_eq!(close_prims[0].opacity(), 1.0);
        assert_eq!(
            close_prims[1].fill_token_or_uv,
            u32::from(Token::ON_ERROR.raw())
        );

        // 显式 `.maximized(false)` 覆盖时不再跟随宿主 `window_maximized = true`。
        let overridden = emit_window_caption_buttons(
            &desc.maximized(false),
            bar,
            true,
            WindowCaptionButtonControlStates::default(),
            &mut font,
        );
        assert_eq!(
            overridden.entries[1].kind,
            WindowCaptionKind::Maximize { maximized: false }
        );
    }

    #[test]
    fn title_bar_desc_composes_window_caption_buttons_consistently() {
        let caption = WindowCaptionButtons::new()
            .height(40.0)
            .show_minimize(false)
            .maximize_id(0xBEEF_0002)
            .close_id(0xBEEF_0003)
            .maximized(true);
        let desc = TitleBarDesc::new("Dialog")
            .with_caption_buttons(caption)
            .with_reserved_widths(40.0, 20.0);

        let exported = desc.caption_buttons(false);
        assert_eq!(exported.height, 40.0);
        assert!(!exported.show_minimize);
        assert!(exported.show_maximize);
        assert!(exported.show_close);
        assert_eq!(exported.maximize_id, 0xBEEF_0002);
        assert_eq!(exported.close_id, 0xBEEF_0003);
        assert!(!exported.focused);
        assert!(exported.resolve_maximized(false));

        // TitleBarChrome 预留的按钮宽度必须与 WindowCaptionButtons::total_width 精确一致。
        let chrome = TitleBarChrome::from_desc(&desc, false);
        assert_eq!(chrome.caption_reserved_width, exported.total_width());
        assert_eq!(chrome.caption_reserved_width, TITLE_BAR_CAPTION_WIDTH * 2.0);
    }

    #[test]
    fn fused_title_bar_height_matches_win11_fluent_scale() {
        assert!((TITLE_BAR_FUSED_HEIGHT - 48.0).abs() < f32::EPSILON);
    }
}

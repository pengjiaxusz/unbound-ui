//! 带延迟、鼠标穿透与窗口边界防溢出自动定位的悬停工具提示（Tooltip）。
//!
//! 工具提示是唯一**绝不能**接收输入的浮层：它出现在指针下方，
//! 若设置命中区域反而会窃取维持其显示的悬停事件。因此 [`emit_tooltip`] 返回的
//! `WidgetOutput` 中 `hit: None` 且 `focusable: false`，而 [`TooltipState`] 持有延迟计时，
//! 使指针扫过工具栏时不会为每个按钮都闪烁气泡。
//!
//! 默认视觉对齐 Win11 Fluent ToolTip 与 `parity` 基线：
//! - `4.0px` 圆角（[`TOOLTIP_RADIUS`]）
//! - `Token::OVERLAY_BG` 填充与 `1.0px Token::CARD_BORDER` 描边（[`TOOLTIP_BORDER_WIDTH`]）
//! - 轻量阴影（[`TOOLTIP_SHADOW`]）
//! - `12.0px` 左对齐自动换行（[`TOOLTIP_FONT_SIZE`] / [`TOOLTIP_MAX_WIDTH`]）
//! - WinUI 3 非对称内边距 `9, 6, 9, 8`（左/上/右/下，[`TOOLTIP_PADDING`]）
//!
//! 除 [`TooltipDesc::new`] / [`TooltipDesc::labeled`] 纯文本构造外，
//! 本模块遵循 ADR-0030 提供纯展示富内容容器排版（[`TooltipDesc::rich`]、
//! [`TooltipDesc::layout`]）与组合发射（[`TooltipDesc::render_with_content`]），
//! 便于在气泡内自由组合图标、标题、正文、`Kbd` 快捷键徽章等任意展示元素。
//!
//! 位置求解统一经由 [`place_tooltip_in_window`]：
//! - 优先采用 `placement` 指定方向；
//! - 当靠近窗口边缘导致主轴空间不足时，自动翻转至对向（`Top <-> Bottom`、`Left <-> Right`）或正交轴；
//! - 最终严格钳制在窗口视口（扣除 `viewport_margin` 安全边距）之内，确保工具提示在任意角落触发时均不出窗口。

use accesskit::Role;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::overlay::{place_tooltip_in_window, OverlayEntry, PlacedPopup, Placement, POPUP_GAP};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 工具提示出现前指针必须停留的时长（毫秒）。
pub const TOOLTIP_DELAY_MS: f32 = 400.0;
/// 自动换行后工具提示的最大宽度。对齐 WinUI `ToolTipMaxWidth = 320`。
pub const TOOLTIP_MAX_WIDTH: f32 = 320.0;
/// 工具提示轻量阴影模糊半径（逻辑像素），轻于 `Popover`（24px）与 `HoverCard`（22px）。
pub const TOOLTIP_SHADOW: f32 = 8.0;
/// 气泡圆角半径。对齐 WinUI `ControlCornerRadius = 4` 与 `parity` 基线。
pub const TOOLTIP_RADIUS: f32 = 4.0;
/// 气泡外描边宽度（逻辑像素）。对齐 WinUI 3 `SurfaceStroke`（1px 微描边）。
pub const TOOLTIP_BORDER_WIDTH: f32 = 1.0;
/// 工具提示默认字号（逻辑像素）。对齐 WinUI 3 `ToolTip` 12px 正文基线。
pub const TOOLTIP_FONT_SIZE: f32 = 12.0;
/// 工具提示默认单行行高（逻辑像素）。对齐 `parity` 基线的 16px 行框。
pub const TOOLTIP_LINE_HEIGHT: f32 = 16.0;
/// WinUI 3 默认非对称内边距（左 9、上 6、右 9、下 8）。
pub const TOOLTIP_PADDING: TooltipPadding = TooltipPadding::WINUI_DEFAULT;

/// 工具提示四边内边距（逻辑像素，顺序为左/上/右/下）。
///
/// 对齐 WinUI 3 `ToolTipBorderPadding = 9, 6, 9, 8` 非对称基线，
/// 同时实现 `From<Vec2>` 以兼容既有对称内边距调用方。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TooltipPadding {
    /// 左内边距（逻辑像素）。
    pub left: f32,
    /// 上内边距（逻辑像素）。
    pub top: f32,
    /// 右内边距（逻辑像素）。
    pub right: f32,
    /// 下内边距（逻辑像素）。
    pub bottom: f32,
}

impl TooltipPadding {
    /// WinUI 3 官方默认内边距：`9, 6, 9, 8`（左/上/右/下）。
    pub const WINUI_DEFAULT: Self = Self {
        left: 9.0,
        top: 6.0,
        right: 9.0,
        bottom: 8.0,
    };

    /// 按左/上/右/下四边构造内边距。
    #[must_use]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 构造水平/垂直对称内边距。
    #[must_use]
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }

    /// 水平方向总内边距（`left + right`）。
    #[must_use]
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    /// 垂直方向总内边距（`top + bottom`）。
    #[must_use]
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    /// 将四边负值钳制为 `0.0`。
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            left: self.left.max(0.0),
            top: self.top.max(0.0),
            right: self.right.max(0.0),
            bottom: self.bottom.max(0.0),
        }
    }
}

impl Default for TooltipPadding {
    fn default() -> Self {
        Self::WINUI_DEFAULT
    }
}

impl From<Vec2> for TooltipPadding {
    fn from(v: Vec2) -> Self {
        Self::symmetric(v.x, v.y).sanitized()
    }
}

impl From<[f32; 4]> for TooltipPadding {
    fn from(edges: [f32; 4]) -> Self {
        Self::new(edges[0], edges[1], edges[2], edges[3]).sanitized()
    }
}

/// 富内容工具提示气泡的完整布局结果（供 `Canvas` 或自定义绘制使用）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TooltipLayout {
    /// 气泡外框的最终窗口矩形（已自动翻转并钳制在视口安全区内）。
    pub outer_rect: Rect,
    /// 扣除四边内边距后的内容区域矩形（供内部组合图标、标题、正文、`Kbd` 等元素）。
    pub content_rect: Rect,
    /// 实际选定的放置方向（发生防溢出翻转后可能与首选方向不同）。
    pub chosen_placement: Placement,
    /// 气泡朝向锚点的箭头尖端坐标。
    pub arrow_tip: Vec2,
}

/// 绝对悬停截止时间戳（毫秒）。
///
/// 本 crate 尚无定时器队列，因此延迟以调用方注册（`TooltipState::schedule`）
/// 并用帧时钟轮询（`TooltipState::poll`）的截止时间表示。待 `TimerQueue` 落地后，
/// 本结构体即可直接存入其中：它携带调度器所需的一切，且不依赖任何控件树。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TooltipDelay {
    /// 悬停开始时的时钟值。
    pub started_ms: f32,
    /// 指针需要停留的时长。
    pub delay_ms: f32,
}

impl TooltipDelay {
    /// 注册自 `now_ms` 起延迟 `delay_ms` 的截止时间。
    #[must_use]
    pub fn start(now_ms: f32, delay_ms: f32) -> Self {
        Self {
            started_ms: now_ms,
            delay_ms: delay_ms.max(0.0),
        }
    }

    /// 工具提示到期显示的时钟值。
    #[must_use]
    pub fn deadline_ms(&self) -> f32 {
        self.started_ms + self.delay_ms
    }

    /// 在 `now_ms` 时刻截止时间是否已过。
    #[must_use]
    pub fn is_due(&self, now_ms: f32) -> bool {
        now_ms >= self.deadline_ms()
    }

    /// 距离截止时间还剩多久（永不为负）。
    #[must_use]
    pub fn remaining(&self, now_ms: f32) -> f32 {
        (self.deadline_ms() - now_ms).max(0.0)
    }
}

/// 按 `max_width` 与每字符估算步进进行贪心自动换行。
///
/// 超过单行宽度的单词会被硬拆，确保返回行不超过 `max_chars = floor(max_width / char_width)`。
/// 这是一个测量辅助函数：调用方传入其文本引擎的平均步进，使控件 crate 独立于具体字体栈。
#[must_use]
pub fn wrap_text(text: &str, max_width: f32, char_width: f32) -> Vec<String> {
    let max_width = if max_width > 0.0 {
        max_width
    } else {
        TOOLTIP_MAX_WIDTH
    };
    let char_width = if char_width > 0.0 { char_width } else { 1.0 };
    let max_chars = (max_width / char_width).floor().max(1.0) as usize;

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut remaining = word;
        while remaining.chars().count() > max_chars {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let chunk: String = remaining.chars().take(max_chars).collect();
            remaining = &remaining[chunk.len()..];
            lines.push(chunk);
        }
        if remaining.is_empty() {
            continue;
        }
        let used = current.chars().count();
        let candidate = if used == 0 {
            remaining.chars().count()
        } else {
            used + 1 + remaining.chars().count()
        };
        if candidate > max_chars && used > 0 {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(remaining);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// 工具提示描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipDesc {
    /// 触发器矩形。
    pub anchor: Rect,
    /// 气泡必须保持在内的窗口视口矩形。
    pub viewport: Rect,
    /// 已测量的气泡外框尺寸。
    pub size: Vec2,
    /// 气泡的首选放置位置。
    pub placement: Placement,
    /// 显示前的延迟时长（毫秒）。
    pub delay_ms: f32,
    /// [`Self::with_wrapped_text`] 与自动换行排版使用的最大气泡宽度。
    pub max_width: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 工具提示文本内容（同步用于无障碍语义标签与声明式排版）。
    pub label: String,
    /// 气泡与触发器之间的间距（逻辑像素）。
    pub gap: f32,
    /// 气泡与窗口边缘保持的最小安全内边距（逻辑像素）。
    pub viewport_margin: f32,
    /// 气泡内部四边非对称内边距（逻辑像素，默认 WinUI 3 `9, 6, 9, 8`）。
    pub padding: TooltipPadding,
}

impl TooltipDesc {
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::TopStart,
            delay_ms: TOOLTIP_DELAY_MS,
            max_width: TOOLTIP_MAX_WIDTH,
            radius: TOOLTIP_RADIUS,
            z: 940,
            label: String::new(),
            gap: POPUP_GAP,
            viewport_margin: 0.0,
            padding: TOOLTIP_PADDING,
        }
    }

    /// 创建带文本标签的工具提示描述符，并自动估算气泡尺寸与启用窗口边缘安全间距。
    #[must_use]
    pub fn labeled(anchor: Rect, viewport: Rect, label: impl Into<String>) -> Self {
        let label = label.into();
        let padding = TOOLTIP_PADDING;
        let max_content_w = (TOOLTIP_MAX_WIDTH - padding.horizontal()).max(1.0);
        let lines = wrap_text(&label, max_content_w, 7.2);
        let line_count = lines.len().max(1) as f32;
        let widest = lines
            .iter()
            .map(|l| l.chars().count() as f32 * 7.2)
            .fold(40.0_f32, f32::max);
        let size = Vec2::new(
            (widest + padding.horizontal()).min(TOOLTIP_MAX_WIDTH),
            line_count * TOOLTIP_LINE_HEIGHT + padding.vertical(),
        );
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::Top,
            delay_ms: TOOLTIP_DELAY_MS,
            max_width: TOOLTIP_MAX_WIDTH,
            radius: TOOLTIP_RADIUS,
            z: 940,
            label,
            gap: 6.0,
            viewport_margin: crate::overlay::TOOLTIP_VIEWPORT_MARGIN,
            padding,
        }
    }

    /// 按自定义富内容区域尺寸 `content_size` 创建工具提示描述符（ADR-0030）。
    ///
    /// 自动叠加 WinUI 3 非对称内边距（[`TOOLTIP_PADDING`]）并按 [`TOOLTIP_MAX_WIDTH`] 限制外框宽度，
    /// 同时启用默认安全边距与窗口内自动翻转。若需超过 `320px` 的自定义宽度，
    /// 可继续链式调用 [`Self::with_max_width`] 与 [`Self::with_content_size`]。
    #[must_use]
    pub fn rich(anchor: Rect, viewport: Rect, content_size: Vec2) -> Self {
        let mut desc = Self {
            anchor,
            viewport,
            size: Vec2::ZERO,
            placement: Placement::Top,
            delay_ms: TOOLTIP_DELAY_MS,
            max_width: TOOLTIP_MAX_WIDTH,
            radius: TOOLTIP_RADIUS,
            z: 940,
            label: String::new(),
            gap: 6.0,
            viewport_margin: crate::overlay::TOOLTIP_VIEWPORT_MARGIN,
            padding: TOOLTIP_PADDING,
        };
        desc.size = desc.outer_size_for_content(content_size);
        desc
    }

    /// 设置工具提示文本标签（纯文本场景下用于绘制，富内容场景下用于无障碍语义名称）。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置首选放置方向。
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// 设置悬停延迟时长（毫秒）。
    #[must_use]
    pub fn with_delay_ms(mut self, delay_ms: f32) -> Self {
        self.delay_ms = delay_ms.max(0.0);
        self
    }

    /// 设置最大换行与容器宽度。
    #[must_use]
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width.max(40.0);
        self
    }

    /// 设置气泡与触发器之间的间距。
    #[must_use]
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// 设置距离窗口视口四边的安全内边距。
    #[must_use]
    pub fn with_viewport_margin(mut self, margin: f32) -> Self {
        self.viewport_margin = margin.max(0.0);
        self
    }

    /// 设置气泡内边距（支持 [`TooltipPadding`]、`Vec2` 对称值或 `[left, top, right, bottom]`）。
    #[must_use]
    pub fn with_padding(mut self, padding: impl Into<TooltipPadding>) -> Self {
        self.padding = padding.into().sanitized();
        self
    }

    /// 显式设置左/上/右/下四边非对称内边距。
    #[must_use]
    pub fn with_asymmetric_padding(mut self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        self.padding = TooltipPadding::new(left, top, right, bottom).sanitized();
        self
    }

    /// 按给定的内部富内容尺寸 `content_size` 重新计算气泡外框尺寸。
    ///
    /// 宽度会在叠加水平内边距后受 [`Self::max_width`] 约束，高度等于内容高度叠加垂直内边距。
    #[must_use]
    pub fn with_content_size(mut self, content_size: Vec2) -> Self {
        self.size = self.outer_size_for_content(content_size);
        self
    }

    /// 在当前 `padding` 与 `max_width` 约束下，由内容区域尺寸推导气泡外框尺寸。
    #[must_use]
    pub fn outer_size_for_content(&self, content_size: Vec2) -> Vec2 {
        let min_outer_w = self.padding.horizontal();
        let max_outer_w = self.max_width.max(min_outer_w);
        Vec2::new(
            (content_size.x.max(0.0) + self.padding.horizontal()).min(max_outer_w),
            content_size.y.max(0.0) + self.padding.vertical(),
        )
    }

    /// 当前气泡扣除水平内边距后的最大内容宽度。
    #[must_use]
    pub fn max_content_width(&self) -> f32 {
        (self.max_width - self.padding.horizontal()).max(1.0)
    }

    /// 测量自动换行的多行工具提示。
    ///
    /// `char_width` 为调用方文本引擎报告的平均步进，`line_height` 为行框高度。
    /// 生成的气泡宽度绝不会超过 [`Self::max_width`]。
    #[must_use]
    pub fn with_wrapped_text(mut self, text: &str, char_width: f32, line_height: f32) -> Self {
        self.label = text.to_string();
        let lines = wrap_text(text, self.max_width, char_width);
        let widest = lines
            .iter()
            .map(|line| line.chars().count() as f32 * char_width)
            .fold(0.0, f32::max);
        self.size = Vec2::new(widest.min(self.max_width), lines.len() as f32 * line_height);
        self
    }

    /// 按该气泡最大宽度换行后的 `text` 文本行列表。
    #[must_use]
    pub fn wrapped_lines(&self, text: &str, char_width: f32) -> Vec<String> {
        wrap_text(text, self.max_width, char_width)
    }

    /// 计算在窗口内的完整定位结果（包含最终矩形、实际采用的放置方向与箭头尖端坐标）。
    #[must_use]
    pub fn placed(&self) -> PlacedPopup {
        place_tooltip_in_window(
            self.anchor,
            self.size,
            self.viewport,
            self.placement,
            self.gap,
            self.viewport_margin,
        )
    }

    /// 一次求解气泡外框、扣除内边距后的内容矩形、实际方向与箭头尖端（供富内容绘制复用）。
    #[must_use]
    pub fn layout(&self) -> TooltipLayout {
        let placed = self.placed();
        let r = placed.rect;
        let content_rect = Rect::new(
            r.origin.x + self.padding.left,
            r.origin.y + self.padding.top,
            (r.width() - self.padding.horizontal()).max(0.0),
            (r.height() - self.padding.vertical()).max(0.0),
        );
        TooltipLayout {
            outer_rect: r,
            content_rect,
            chosen_placement: placed.chosen_placement,
            arrow_tip: placed.arrow_tip,
        }
    }

    /// 已放置的气泡矩形（始终位于 `viewport` 内部）。
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.placed().rect
    }

    /// 实际选定的放置方向（发生防溢出翻转后可能与 `self.placement` 不同）。
    #[must_use]
    pub fn chosen_placement(&self) -> Placement {
        self.placed().chosen_placement
    }

    /// 气泡朝向锚点的箭头尖端坐标。
    #[must_use]
    pub fn arrow_tip(&self) -> Vec2 {
        self.placed().arrow_tip
    }

    /// 气泡内部扣除 `padding` 后的内容排版矩形。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        self.layout().content_rect
    }

    /// 发射包含已整形文本字形的工具提示气泡。
    #[must_use]
    pub fn render_with_glyphs(
        &self,
        node: u64,
        anchor_node: u64,
        state: ControlState,
        glyphs: &[GlyphQuad],
    ) -> (WidgetOutput, OverlayEntry) {
        let (mut output, entry) = emit_tooltip(node, anchor_node, self, state);
        crate::primitive::glyphs(
            glyphs,
            Token::TEXT_PRIMARY,
            self.z.saturating_add(1),
            &mut output.primitives,
            &mut output.payloads,
        );
        (output, entry)
    }

    /// 发射工具提示气泡外壳并合并调用方在 `content_rect` 内生成的纯展示富内容（ADR-0030）。
    ///
    /// 闭包接收扣除内边距后的 `content_rect` 与内容层起始 `z`（`self.z + 1`），
    /// 返回的子内容图元与载荷会自动拼接至气泡外壳之后并重定位载荷索引；
    /// 最终输出强制保持 `hit: None`、`sub_hits` 为空与 `focusable: false`，
    /// 严格守住 `OverlayKind::Tooltip` 的鼠标穿透、不抢焦点、纯展示契约。
    #[must_use]
    pub fn render_with_content(
        &self,
        node: u64,
        anchor_node: u64,
        state: ControlState,
        build_content: impl FnOnce(Rect, u16) -> WidgetOutput,
    ) -> (WidgetOutput, OverlayEntry) {
        let layout = self.layout();
        let (mut output, entry) = emit_tooltip(node, anchor_node, self, state);
        let content_z = self.z.saturating_add(1);
        let child = build_content(layout.content_rect, content_z);
        output.append_child_primitives(child);
        // 守住纯展示契约：即使子内容携带交互图元也绝不产出命中区或焦点能力。
        output.hit = None;
        output.sub_hits.clear();
        output.focusable = false;
        (output, entry)
    }
}

/// 单个工具提示的延迟状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TooltipState {
    hovered_ms: f32,
    visible: bool,
    hovered_anchor: Option<u64>,
    now_ms: f32,
    deadline: Option<TooltipDelay>,
}

impl TooltipState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 气泡当前是否处于显示状态。
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 指针已停留的时长（毫秒）。
    #[must_use]
    pub const fn hovered_ms(&self) -> f32 {
        self.hovered_ms
    }

    /// 若指针正停留在触发器上，返回已注册的截止时间。
    #[must_use]
    pub const fn delay(&self) -> Option<TooltipDelay> {
        self.deadline
    }

    /// 在调用方时钟下为 `hovered` 注册悬停延迟。
    ///
    /// 这是定时器队列风格的入口：仅当悬停节点变化时才创建新截止时间，
    /// 因此每帧调用也不会重置计时器。调用方随后使用帧时钟调用 [`Self::poll`]。
    pub fn schedule(&mut self, desc: &TooltipDesc, hovered: Option<u64>, now_ms: f32) {
        self.now_ms = now_ms;
        match hovered {
            Some(node) => {
                if self.hovered_anchor != Some(node) {
                    self.hovered_anchor = Some(node);
                    self.hovered_ms = 0.0;
                    self.visible = false;
                    self.deadline = Some(TooltipDelay::start(now_ms, desc.delay_ms));
                }
            }
            None => {
                self.hovered_anchor = None;
                self.hovered_ms = 0.0;
                self.deadline = None;
                self.visible = false;
            }
        }
    }

    /// 推进时钟并返回可见性是否发生变化。
    ///
    /// 这是 [`Self::schedule`] 的轮询半边：注册截止时间是 `TimerQueue` 风格操作，
    /// 而每帧读取一次时钟仅是一次廉价的数值比较。
    pub fn poll(&mut self, now_ms: f32) -> bool {
        self.now_ms = now_ms;
        let was = self.visible;
        if let Some(deadline) = self.deadline {
            self.hovered_ms = (now_ms - deadline.started_ms).max(0.0);
            self.visible = deadline.is_due(now_ms);
        } else {
            self.hovered_ms = 0.0;
            self.visible = false;
        }
        self.visible != was
    }

    /// 按帧增量推进延迟。
    ///
    /// 为仅有增量而非帧时钟的调用方保留；内部驱动与定时器队列风格 API 相同的截止时间。
    ///
    /// `hovered` 为指针下的节点（未悬停任何节点时为 `None`）。移动到不同节点会重启计时器，
    /// 从而防止扫过一排按钮时每个按钮都弹出气泡。返回可见性是否发生变化。
    pub fn update(&mut self, desc: &TooltipDesc, hovered: Option<u64>, dt_ms: f32) -> bool {
        let dt = dt_ms.max(0.0);
        let now = self.now_ms + dt;
        let was = self.visible;
        match hovered {
            Some(node) if self.hovered_anchor == Some(node) => {}
            Some(node) => {
                self.hovered_anchor = Some(node);
                self.hovered_ms = 0.0;
                // 指针在本次 tick 前已停留了 `dt`，因此截止时间从过去一个增量处开始。
                self.deadline = Some(TooltipDelay::start(now - dt, desc.delay_ms));
            }
            None => {
                self.hovered_anchor = None;
                self.hovered_ms = 0.0;
                self.deadline = None;
                self.visible = false;
                self.now_ms = now;
                return self.visible != was;
            }
        }
        self.poll(now)
    }

    /// 立即隐藏气泡（例如发生按下事件时）。
    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered_ms = 0.0;
        self.deadline = None;
    }
}

/// 绑定了节点标识的工具提示描述符。
#[derive(Clone, Debug)]
pub struct TooltipWidget {
    /// 气泡节点。
    pub node: u64,
    /// 触发器节点。
    pub anchor_node: u64,
    /// 描述符。
    pub desc: TooltipDesc,
}

impl WidgetDesc for TooltipWidget {
    fn rect(&self) -> Rect {
        self.desc.rect()
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        self.desc
            .render_with_glyphs(self.node, self.anchor_node, state, glyphs)
            .0
    }
}

/// 输出工具提示气泡外壳。输出结果永不包含命中区域且不可聚焦。
#[must_use]
pub fn emit_tooltip(
    node: u64,
    anchor_node: u64,
    desc: &TooltipDesc,
    state: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect();
    let radii = [desc.radius; 4];
    let primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            TOOLTIP_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::OVERLAY_BG, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::CARD_BORDER,
            TOOLTIP_BORDER_WIDTH,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];

    let a11y = unbound_a11y::A11yNode::new(Role::Tooltip, desc.label.clone(), rect)
        .disabled(state.disabled);

    (
        WidgetOutput {
            primitives,
            payloads: Vec::new(),
            // 鼠标穿透：工具提示绝不能窃取维持其打开状态的悬停事件。
            hit: None,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        OverlayEntry::tooltip(node, anchor_node, rect),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayKind;
    use unbound_input::HitRegion;

    fn desc() -> TooltipDesc {
        TooltipDesc::new(
            Rect::new(100.0, 100.0, 80.0, 32.0),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            Vec2::new(120.0, 28.0),
        )
    }

    #[test]
    fn the_bubble_waits_for_the_delay() {
        let desc = desc();
        let mut state = TooltipState::new();
        assert!(!state.visible());
        assert!(!state.update(&desc, Some(7), 399.0));
        assert!(!state.visible());
        assert!(state.update(&desc, Some(7), 2.0));
        assert!(state.visible());
    }

    #[test]
    fn moving_to_another_trigger_restarts_the_timer() {
        let desc = desc();
        let mut state = TooltipState::new();
        state.update(&desc, Some(7), 399.0);
        assert!(!state.update(&desc, Some(8), 16.0));
        assert!(!state.visible());
        assert_eq!(state.hovered_ms(), 16.0);
    }

    #[test]
    fn leaving_the_trigger_hides_it_immediately() {
        let desc = desc();
        let mut state = TooltipState::new();
        state.update(&desc, Some(7), 500.0);
        assert!(state.visible());
        assert!(state.update(&desc, None, 16.0));
        assert!(!state.visible());
        assert_eq!(state.hovered_ms(), 0.0);
    }

    #[test]
    fn a_tooltip_never_receives_pointer_events() {
        let desc = desc();
        let (output, entry) = emit_tooltip(3, 2, &desc, ControlState::default());
        assert!(output.hit.is_none());
        assert!(!output.focusable);
        assert_eq!(entry.kind, OverlayKind::Tooltip);
        assert!(desc.viewport.contains_rect(&entry.bounds));
        assert_eq!(output.primitives.len(), 3);
    }

    #[test]
    fn win11_fluent_default_tokens_radius_shadow_and_asymmetric_padding() {
        let d = desc();
        assert_eq!(d.radius, 4.0);
        assert_eq!(d.max_width, 320.0);
        assert_eq!(d.padding, TooltipPadding::new(9.0, 6.0, 9.0, 8.0));
        assert_eq!(d.padding.horizontal(), 18.0);
        assert_eq!(d.padding.vertical(), 14.0);

        let (output, entry) = emit_tooltip(10, 1, &d, ControlState::default());
        assert_eq!(entry.kind, OverlayKind::Tooltip);
        assert_eq!(output.primitives.len(), 3);

        let shadow = &output.primitives[0];
        let bg = &output.primitives[1];
        let border = &output.primitives[2];

        // 轻量阴影、OVERLAY_BG 填充与 1px CARD_BORDER 描边。
        assert_eq!(shadow.shadow_blur(), TOOLTIP_SHADOW);
        assert_eq!(bg.fill_token_or_uv, u32::from(Token::OVERLAY_BG.raw()));
        assert_eq!(
            border.border_token_or_tex,
            u32::from(Token::CARD_BORDER.raw())
        );
        assert_eq!(border.border_width(), TOOLTIP_BORDER_WIDTH);
    }

    #[test]
    fn rich_tooltip_layout_applies_asymmetric_padding_and_strips_child_hits() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let anchor = Rect::new(300.0, 200.0, 80.0, 32.0);
        let content_size = Vec2::new(180.0, 44.0);
        let d = TooltipDesc::rich(anchor, viewport, content_size).with_label("Shortcut hint");

        assert_eq!(d.size, Vec2::new(180.0 + 18.0, 44.0 + 14.0));
        let layout = d.layout();
        assert_eq!(
            layout.content_rect.origin.x,
            layout.outer_rect.origin.x + 9.0
        );
        assert_eq!(
            layout.content_rect.origin.y,
            layout.outer_rect.origin.y + 6.0
        );
        assert_eq!(layout.content_rect.width(), 180.0);
        assert_eq!(layout.content_rect.height(), 44.0);

        let (output, entry) =
            d.render_with_content(42, 7, ControlState::default(), |content_r, z| {
                let mut child = WidgetOutput::default();
                child
                    .primitives
                    .push(surface(content_r, 2.0, Token::SURFACE_RAISED, z));
                // 故意塞入命中区与可聚焦标记，验证纯展示契约会将其剥离。
                child.hit = Some(HitRegion::ui(99, u32::from(z), content_r));
                child
                    .sub_hits
                    .push(HitRegion::ui(100, u32::from(z), content_r));
                child.focusable = true;
                child
            });
        assert_eq!(entry.kind, OverlayKind::Tooltip);
        assert_eq!(
            output.primitives.len(),
            4,
            "3 shell primitives + 1 rich child primitive"
        );
        assert!(output.hit.is_none());
        assert!(output.sub_hits.is_empty());
        assert!(!output.focusable);
    }

    #[test]
    fn the_bubble_flips_above_a_trigger_near_the_top() {
        let mut desc = desc();
        desc.anchor = Rect::new(100.0, 0.0, 80.0, 30.0);
        desc.placement = Placement::TopStart;
        let rect = desc.rect();
        assert!(rect.origin.y >= desc.viewport.origin.y);
        assert!(desc.viewport.contains_rect(&rect));
        assert_eq!(desc.chosen_placement(), Placement::BottomStart);
    }

    #[test]
    fn labeled_tooltip_never_overflows_any_window_corner() {
        let viewport = Rect::new(0.0, 0.0, 640.0, 480.0);
        let corners = [
            Rect::new(2.0, 2.0, 72.0, 28.0),
            Rect::new(566.0, 2.0, 72.0, 28.0),
            Rect::new(2.0, 450.0, 72.0, 28.0),
            Rect::new(566.0, 450.0, 72.0, 28.0),
        ];
        for anchor in corners {
            for placement in [
                Placement::Top,
                Placement::Bottom,
                Placement::Left,
                Placement::Right,
            ] {
                let t = TooltipDesc::labeled(anchor, viewport, "Auto-adjusted in-window tooltip")
                    .with_placement(placement);
                let r = t.rect();
                assert!(
                    viewport.contains_rect(&r),
                    "tooltip {r:?} overflowed viewport {viewport:?} at anchor {anchor:?}"
                );
            }
        }
    }
}

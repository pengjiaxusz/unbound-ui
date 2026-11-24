//! 跨窗拖拽停靠区域预览叠加层与拖拽代理协议（ADR 0037）。
//!
//! 本模块提供纯几何与纯状态机层实现，彻底摒弃老式五向指南针悬浮按钮：
//! - 根据跨窗统一坐标系下的候选窗口与面板矩形，直接计算 `Edge` / `RootEdge` 半区、
//!   避开顶部标签栏的 `Center` 内容区、`Reorder` `2px` 标签插入指示线与 `Nest` 嵌套目标；
//! - `TearOut`（窗外自由空间）不在宿主内发射任何预览矩形，改由外部 [`DockDragProxy`] 表达；
//! - 内置整窗外壳（`Shell`）跨视口 `500ms / 12px` 防闪烁驻留防抖、多层级滚轮 `nudge_level`
//!   下钻切换、三态语义配色（[`DockOverlayTokens`]）、消费 [`MotionToken::OverlayPreview`] 的
//!   过渡动画以及自动贴边翻转并强钳制在叠加层边界内的光标提示胶囊（`HintPill`）。

use unbound_anim::{MotionPolicy, MotionToken, Tween};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::dock::DOCK_TAB_BAR;
use crate::primitive::{glyphs, instance, outlined, surface};
use crate::WidgetOutput;

/// 整窗外壳跨视口拖入目标窗口时的防闪烁驻留时长（500 ms）。
pub const SHELL_DWELL_SECS: f32 = 0.5;

/// 整窗驻留防抖允许的指针抖动半径（12 px）。
pub const SHELL_DWELL_TOLERANCE_PX: f32 = 12.0;

/// 标签栏重排插入指示线宽度（2 px）。
pub const REORDER_INDICATOR_WIDTH: f32 = 2.0;

/// 整窗外缘吸附带厚度（逻辑像素）。
pub const ROOT_EDGE_BAND_PX: f32 = 24.0;

/// 允许停靠语义下的预览填充不透明度（0.22）。
pub const ALLOWED_FILL_ALPHA: f32 = 0.22;

/// 禁止停靠语义下的预览填充不透明度（0.14，落在规范 `0.12..=0.16` 区间）。
pub const BLOCKED_FILL_ALPHA: f32 = 0.14;

/// 嵌套停靠语义下的预览填充不透明度（0.22）。
pub const NEST_FILL_ALPHA: f32 = 0.22;

/// 停靠预览边框厚度（1 px）。
pub const PREVIEW_BORDER_WIDTH: f32 = 1.0;

/// 预览矩形圆角半径（与停靠面板 `ControlCornerRadius = 4px` 保持一致）。
const OVERLAY_CORNER: f32 = 4.0;

/// 光标提示胶囊最大宽度（逻辑像素）。
pub const HINT_PILL_MAX_WIDTH: f32 = 220.0;

/// 光标提示胶囊高度（逻辑像素）。
pub const HINT_PILL_HEIGHT: f32 = 26.0;

/// 光标提示胶囊相对光标的默认偏移。
pub const HINT_PILL_OFFSET: Vec2 = Vec2::new(14.0, 18.0);

/// 光标提示胶囊内边距。
const HINT_PILL_PAD_X: f32 = 10.0;

/// 边缘吸附方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockEdge {
    /// 左侧半区。
    Left,
    /// 右侧半区。
    Right,
    /// 顶部半区。
    Top,
    /// 底部半区。
    Bottom,
}

impl DockEdge {
    /// 返回对应半区的预览矩形。
    #[must_use]
    pub fn half_rect(self, rect: Rect) -> Rect {
        match self {
            Self::Left => Rect::new(rect.origin.x, rect.origin.y, rect.size.x * 0.5, rect.size.y),
            Self::Right => Rect::new(
                rect.origin.x + rect.size.x * 0.5,
                rect.origin.y,
                rect.size.x * 0.5,
                rect.size.y,
            ),
            Self::Top => Rect::new(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y * 0.5),
            Self::Bottom => Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.y * 0.5,
                rect.size.x,
                rect.size.y * 0.5,
            ),
        }
    }

    /// 返回面向提示胶囊的英文方向短标签。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }
}

/// 拖拽发起源类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockDragSource {
    /// 普通单标签拖拽：无驻留延迟，即时响应。
    Tab {
        /// 源窗口标识。
        window_id: u64,
        /// 源面板标识。
        pane_id: u64,
        /// 被拖拽标签页标识。
        tab_id: u64,
    },
    /// 整窗外壳跨视口拖拽：跨入目标窗口时须在 12px 容差内驻留满 500ms 才武装预览。
    Shell {
        /// 被拖拽的源窗口标识。
        window_id: u64,
    },
}

impl DockDragSource {
    /// 源窗口标识。
    #[must_use]
    pub const fn window_id(self) -> u64 {
        match self {
            Self::Tab { window_id, .. } | Self::Shell { window_id } => window_id,
        }
    }

    /// 是否属于整窗外壳拖拽。
    #[must_use]
    pub const fn is_shell(self) -> bool {
        matches!(self, Self::Shell { .. })
    }
}

/// 候选窗口内的单个可停靠面板描述。
#[derive(Clone, Debug, PartialEq)]
pub struct DockCandidatePane {
    /// 面板标识。
    pub pane_id: u64,
    /// 面板在目标坐标系中的矩形。
    pub rect: Rect,
    /// 顶部标签栏高度（默认 [`DOCK_TAB_BAR`]）。
    pub tab_bar_height: f32,
    /// 标签栏内各标签页矩形（用于计算 `Reorder` 2px 插入指示线）。
    pub tab_rects: Vec<Rect>,
    /// 嵌套层级深度（数值越大越靠近最内层面板）。
    pub nesting_depth: u8,
    /// 是否允许中心合并。
    pub allow_center: bool,
    /// 是否允许嵌套停靠（输出 `Nest` / `NestArmed` 候选）。
    pub allow_nest: bool,
    /// 是否显式禁止放置（例如受限边界或只读面板）。
    pub blocked: bool,
}

impl DockCandidatePane {
    /// 使用标准标签栏高度构造可停靠面板候选。
    #[must_use]
    pub fn new(pane_id: u64, rect: Rect) -> Self {
        Self {
            pane_id,
            rect,
            tab_bar_height: DOCK_TAB_BAR,
            tab_rects: Vec::new(),
            nesting_depth: 0,
            allow_center: true,
            allow_nest: false,
            blocked: false,
        }
    }

    /// 设置嵌套深度。
    #[must_use]
    pub const fn with_depth(mut self, depth: u8) -> Self {
        self.nesting_depth = depth;
        self
    }

    /// 绑定标签栏各标签页矩形。
    #[must_use]
    pub fn with_tab_rects(mut self, tab_rects: Vec<Rect>) -> Self {
        self.tab_rects = tab_rects;
        self
    }

    /// 启用嵌套停靠槽位。
    #[must_use]
    pub const fn with_nest(mut self, allow_nest: bool) -> Self {
        self.allow_nest = allow_nest;
        self
    }

    /// 标记该面板为禁止停靠。
    #[must_use]
    pub const fn with_blocked(mut self, blocked: bool) -> Self {
        self.blocked = blocked;
        self
    }

    /// 面板顶部标签栏矩形。
    #[must_use]
    pub fn tab_bar_rect(&self) -> Rect {
        let h = self.tab_bar_height.max(0.0).min(self.rect.size.y.max(0.0));
        Rect::new(self.rect.origin.x, self.rect.origin.y, self.rect.size.x, h)
    }

    /// 避开顶部标签栏后的内容区矩形（`Center` 预览目标）。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        let bar = self.tab_bar_rect();
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + bar.size.y,
            self.rect.size.x,
            (self.rect.size.y - bar.size.y).max(0.0),
        )
    }
}

/// 跨窗停靠仲裁中的单个候选窗口描述。
#[derive(Clone, Debug, PartialEq)]
pub struct DockCandidateWindow {
    /// 窗口唯一标识。
    pub window_id: u64,
    /// 窗口在统一坐标系下的包围矩形。
    pub bounds: Rect,
    /// 窗口内可命中的面板列表（可包含多层嵌套面板）。
    pub panes: Vec<DockCandidatePane>,
    /// 是否允许整窗外缘吸附带（`RootEdge`）。
    pub allow_root_edge: bool,
    /// 是否整窗禁止停靠。
    pub blocked: bool,
}

impl DockCandidateWindow {
    /// 构造标准候选窗口。
    #[must_use]
    pub fn new(window_id: u64, bounds: Rect, panes: Vec<DockCandidatePane>) -> Self {
        Self {
            window_id,
            bounds,
            panes,
            allow_root_edge: true,
            blocked: false,
        }
    }
}

/// 停靠落点形态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DockDropKind {
    /// 面板边缘半区分割。
    Edge(DockEdge),
    /// 整窗外缘吸附带半区分割。
    RootEdge(DockEdge),
    /// 面板中心合并（避开顶部标签栏的内容区）。
    Center,
    /// 标签栏内重排（`2px` 竖向插入指示线）。
    Reorder {
        /// 目标插入索引。
        insert_index: usize,
        /// 插入指示线的 X 坐标。
        line_x: f32,
    },
    /// 嵌套停靠（可选次级强调色语义）。
    Nest,
    /// 窗外自由空间（撕出独立窗，不在宿主内绘制预览框）。
    TearOut,
}

/// 停靠落点的三态语义分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockDropSemantic {
    /// 允许停靠：主题强调色 `Token::PRIMARY`（0.22 alpha + 1px 边框）。
    Allowed,
    /// 禁止停靠：中性静音灰（0.12–0.16 alpha + 1px 灰边框）。
    Blocked,
    /// 嵌套停靠武装：可选次级强调色槽位（未配置时回退 `Token::PRIMARY`）。
    NestArmed,
}

/// 单个已仲裁的停靠落点候选。
#[derive(Clone, Debug, PartialEq)]
pub struct DockDropTarget {
    /// 命中的目标窗口（`TearOut` 时为 `None`）。
    pub window_id: Option<u64>,
    /// 命中的目标面板（`RootEdge` / `TearOut` 时为 `None`）。
    pub pane_id: Option<u64>,
    /// 落点形态。
    pub kind: DockDropKind,
    /// 三态语义配色分类。
    pub semantic: DockDropSemantic,
    /// 预览矩形（`TearOut` 时为空矩形）。
    pub preview_rect: Rect,
    /// 光标提示胶囊短文本（英文）。
    pub hint: String,
    /// 该候选在同位置多层级列表中的索引（`0` 为最内层）。
    pub level_index: usize,
    /// 同位置候选层级总数。
    pub level_count: usize,
}

impl DockDropTarget {
    /// 构造窗外自由空间 `TearOut` 目标（无宿主预览框）。
    #[must_use]
    pub fn tear_out() -> Self {
        Self {
            window_id: None,
            pane_id: None,
            kind: DockDropKind::TearOut,
            semantic: DockDropSemantic::Allowed,
            preview_rect: Rect::ZERO,
            hint: "Detach Window".to_owned(),
            level_index: 0,
            level_count: 1,
        }
    }
}

/// 三态语义配色与胶囊、拖拽代理令牌。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockOverlayTokens {
    /// 允许停靠填充色（默认 `Token::PRIMARY`）。
    pub allowed_fill: Token,
    /// 允许停靠边框色（默认 `Token::PRIMARY`）。
    pub allowed_border: Token,
    /// 允许停靠填充不透明度（默认 `0.22`）。
    pub allowed_alpha: f32,
    /// 禁止停靠填充色（默认中性静音灰 `Token::ON_SURFACE_MUTED`）。
    pub blocked_fill: Token,
    /// 禁止停靠边框色（默认中性灰 `Token::OUTLINE`）。
    pub blocked_border: Token,
    /// 禁止停靠填充不透明度（默认 `0.14`，位于 `0.12..=0.16`）。
    pub blocked_alpha: f32,
    /// 嵌套停靠可选次级强调色填充（`None` 时回退 `allowed_fill`）。
    pub nest_fill: Option<Token>,
    /// 嵌套停靠可选次级强调色边框（`None` 时回退 `allowed_border`）。
    pub nest_border: Option<Token>,
    /// 嵌套停靠填充不透明度（默认 `0.22`）。
    pub nest_alpha: f32,
    /// 标签插入指示线颜色（默认 `Token::PRIMARY`）。
    pub reorder_line: Token,
    /// 光标提示胶囊背景色。
    pub pill_bg: Token,
    /// 光标提示胶囊边框色。
    pub pill_border: Token,
    /// 光标提示胶囊文字色。
    pub pill_text: Token,
    /// 拖拽代理卡片背景色。
    pub proxy_bg: Token,
    /// 拖拽代理卡片边框色。
    pub proxy_border: Token,
    /// 拖拽代理卡片强调条颜色。
    pub proxy_accent: Token,
}

impl Default for DockOverlayTokens {
    fn default() -> Self {
        Self::standard()
    }
}

impl DockOverlayTokens {
    /// 标准三态语义配色方案（符合 ADR 0037）。
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            allowed_fill: Token::PRIMARY,
            allowed_border: Token::PRIMARY,
            allowed_alpha: ALLOWED_FILL_ALPHA,
            blocked_fill: Token::ON_SURFACE_MUTED,
            blocked_border: Token::OUTLINE,
            blocked_alpha: BLOCKED_FILL_ALPHA,
            nest_fill: None,
            nest_border: None,
            nest_alpha: NEST_FILL_ALPHA,
            reorder_line: Token::PRIMARY,
            pill_bg: Token::SURFACE_RAISED,
            pill_border: Token::OUTLINE,
            pill_text: Token::TEXT_PRIMARY,
            proxy_bg: Token::SURFACE_RAISED,
            proxy_border: Token::OUTLINE_STRONG,
            proxy_accent: Token::PRIMARY,
        }
    }

    /// 配置可选的嵌套次级强调色槽位。
    #[must_use]
    pub const fn with_nest_accent(mut self, fill: Token, border: Token) -> Self {
        self.nest_fill = Some(fill);
        self.nest_border = Some(border);
        self
    }

    /// 解析三态语义对应的 `(填充令牌, 边框令牌, 填充不透明度)`。
    ///
    /// 当 `DockDropSemantic::NestArmed` 未显式配置次级强调色时，自动回退至 `allowed_fill` / `allowed_border`（即 `Token::PRIMARY`）。
    #[must_use]
    pub fn resolve_semantic(&self, semantic: DockDropSemantic) -> (Token, Token, f32) {
        match semantic {
            DockDropSemantic::Allowed => (
                self.allowed_fill,
                self.allowed_border,
                self.allowed_alpha.clamp(0.0, 1.0),
            ),
            DockDropSemantic::Blocked => (
                self.blocked_fill,
                self.blocked_border,
                self.blocked_alpha.clamp(0.12, 0.16),
            ),
            DockDropSemantic::NestArmed => (
                self.nest_fill.unwrap_or(self.allowed_fill),
                self.nest_border.unwrap_or(self.allowed_border),
                self.nest_alpha.clamp(0.0, 1.0),
            ),
        }
    }
}

/// 外部幽灵窗 / 拖拽代理视觉替身描述。
#[derive(Clone, Debug, PartialEq)]
pub struct DockDragProxy {
    /// 被拖拽标签页或窗口的可见标题。
    pub title: String,
    /// 可选状态角标文本（如标签数量或分离状态）。
    pub badge: Option<String>,
    /// 当前指针位置（目标视口坐标系）。
    pub pointer: Vec2,
    /// 代理卡片期望尺寸。
    pub size: Vec2,
    /// 是否处于窗外自由空间（`TearOut`）。
    pub detached: bool,
    /// 当前目标语义状态。
    pub semantic: DockDropSemantic,
    /// 状态提示文案。
    pub hint: String,
}

impl DockDragProxy {
    /// 构造拖拽代理描述。
    #[must_use]
    pub fn new(title: impl Into<String>, pointer: Vec2) -> Self {
        Self {
            title: title.into(),
            badge: None,
            pointer,
            size: Vec2::new(196.0, 44.0),
            detached: false,
            semantic: DockDropSemantic::Allowed,
            hint: String::new(),
        }
    }
}

/// 跨窗停靠预览叠加层纯状态机。
///
/// 负责：
/// 1. 仲裁多窗口、多层嵌套面板、标签栏重排与整窗外缘候选；
/// 2. 管理整窗外壳（`Shell`）跨视口 `500ms / 12px` 防闪烁驻留防抖；
/// 3. 支持通过 [`Self::nudge_level`] 在同指针位置的嵌套候选层级间切换（默认最内层）；
/// 4. 通过 [`MotionToken::OverlayPreview`]（120 ms）与 [`MotionPolicy`] 驱动预览显隐进度。
#[derive(Clone, Debug, PartialEq)]
pub struct DockOverlayState {
    active: bool,
    source: Option<DockDragSource>,
    pointer: Vec2,
    candidates: Vec<DockDropTarget>,
    selected_level: usize,
    dwell_target_window: Option<u64>,
    dwell_anchor: Vec2,
    dwell_elapsed: f32,
    dwell_armed: bool,
    visibility: Tween,
}

impl Default for DockOverlayState {
    fn default() -> Self {
        Self::new()
    }
}

impl DockOverlayState {
    /// 创建空闲态停靠预览叠加层状态机。
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: false,
            source: None,
            pointer: Vec2::ZERO,
            candidates: Vec::new(),
            selected_level: 0,
            dwell_target_window: None,
            dwell_anchor: Vec2::ZERO,
            dwell_elapsed: 0.0,
            dwell_armed: false,
            visibility: Tween::settled(0.0, MotionToken::OverlayPreview),
        }
    }

    /// 开始一次拖拽会话。
    pub fn begin_drag(&mut self, source: DockDragSource, pointer: Vec2) {
        self.active = true;
        self.source = Some(source);
        self.pointer = pointer;
        self.candidates.clear();
        self.selected_level = 0;
        self.dwell_target_window = None;
        self.dwell_anchor = pointer;
        self.dwell_elapsed = 0.0;
        self.dwell_armed = !source.is_shell();
        self.visibility = Tween::settled(0.0, MotionToken::OverlayPreview);
    }

    /// 结束或取消拖拽会话，立即清空候选并归零显隐状态。
    pub fn end_drag(&mut self) {
        self.active = false;
        self.source = None;
        self.candidates.clear();
        self.selected_level = 0;
        self.dwell_target_window = None;
        self.dwell_elapsed = 0.0;
        self.dwell_armed = false;
        self.visibility.snap_to(0.0);
    }

    /// 当前是否处于活跃拖拽会话中。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// 当前拖拽源。
    #[must_use]
    pub const fn source(&self) -> Option<DockDragSource> {
        self.source
    }

    /// 当前指针位置。
    #[must_use]
    pub const fn pointer(&self) -> Vec2 {
        self.pointer
    }

    /// 整窗跨视口驻留防抖是否已武装（普通单标签始终为 `true`）。
    #[must_use]
    pub const fn is_dwell_armed(&self) -> bool {
        self.dwell_armed
    }

    /// 当前指针位置下的全部候选层级列表（索引 `0` 为最内层）。
    #[must_use]
    pub fn candidates(&self) -> &[DockDropTarget] {
        &self.candidates
    }

    /// 当前选中的候选层级索引。
    #[must_use]
    pub const fn selected_level(&self) -> usize {
        self.selected_level
    }

    /// 当前显隐过渡进度（`0.0..=1.0`，由 [`MotionToken::OverlayPreview`] 驱动）。
    #[must_use]
    pub fn preview_opacity(&self) -> f32 {
        self.visibility.value().clamp(0.0, 1.0)
    }

    /// 原始仲裁结果（不考虑整窗驻留防抖）。
    #[must_use]
    pub fn raw_target(&self) -> Option<&DockDropTarget> {
        if !self.active {
            return None;
        }
        self.candidates.get(self.selected_level)
    }

    /// 当前可显示的宿主停靠预览目标。
    ///
    /// - 未激活拖拽时返回 `None`；
    /// - 整窗跨视口拖入未满 500ms 驻留防抖时返回 `None`（静默隐藏零闪烁）；
    /// - 处于 `TearOut` 时返回 `None`（宿主内绝不绘制预览框）。
    #[must_use]
    pub fn visible_target(&self) -> Option<&DockDropTarget> {
        let target = self.raw_target()?;
        if target.kind == DockDropKind::TearOut {
            return None;
        }
        if self.requires_dwell_for(target.window_id) && !self.dwell_armed {
            return None;
        }
        Some(target)
    }

    /// 当前指针是否处于窗外自由空间（`TearOut`）。
    #[must_use]
    pub fn is_tear_out(&self) -> bool {
        self.raw_target()
            .is_some_and(|target| target.kind == DockDropKind::TearOut)
    }

    /// 通过滚轮增量在同位置的多层级候选间切换（`delta > 0` 向外层切换，`delta < 0` 向内层切换）。
    ///
    /// 返回更新后的层级索引。
    pub fn nudge_level(&mut self, delta: i32) -> usize {
        let count = self.candidates.len();
        if count <= 1 || delta == 0 {
            self.selected_level = 0;
            return self.selected_level;
        }
        let next = (self.selected_level as i64 + i64::from(delta)).clamp(0, (count - 1) as i64);
        self.selected_level = next as usize;
        self.selected_level
    }

    /// 推进单帧指针位置、整窗驻留防抖与 `MotionToken::OverlayPreview` 显隐动画。
    pub fn update(
        &mut self,
        pointer: Vec2,
        windows: &[DockCandidateWindow],
        dt: f32,
        policy: MotionPolicy,
    ) {
        if !self.active {
            self.visibility.set_target(0.0);
            self.visibility.step(dt, policy);
            return;
        }
        let Some(source) = self.source else {
            return;
        };
        self.pointer = pointer;

        let new_candidates = resolve_candidates(source, pointer, windows);
        let prev_level = self.selected_level;
        let target_window = new_candidates.first().and_then(|c| c.window_id);
        let same_window = target_window == self.dwell_target_window;
        self.candidates = new_candidates;
        // 同一窗口内若候选层数足够则保留用户滚轮选中的层级，否则回退到最内层（0）。
        self.selected_level = if same_window && prev_level < self.candidates.len() {
            prev_level
        } else {
            0
        };

        // 更新整窗外壳跨视口 500ms / 12px 驻留防抖状态机。
        if source.is_shell() {
            match target_window {
                Some(dst_win) if dst_win != source.window_id() => {
                    if self.dwell_target_window != Some(dst_win) {
                        self.dwell_target_window = Some(dst_win);
                        self.dwell_anchor = pointer;
                        self.dwell_elapsed = 0.0;
                        self.dwell_armed = false;
                    } else if !self.dwell_armed {
                        let dist = (pointer - self.dwell_anchor).length();
                        if dist > SHELL_DWELL_TOLERANCE_PX {
                            self.dwell_anchor = pointer;
                            self.dwell_elapsed = 0.0;
                        } else if dt.is_finite() && dt > 0.0 {
                            self.dwell_elapsed += dt;
                            if self.dwell_elapsed >= SHELL_DWELL_SECS {
                                self.dwell_armed = true;
                            }
                        }
                    }
                }
                _ => {
                    self.dwell_target_window = target_window;
                    self.dwell_anchor = pointer;
                    self.dwell_elapsed = 0.0;
                    self.dwell_armed = false;
                }
            }
        } else {
            self.dwell_target_window = target_window;
            self.dwell_armed = true;
        }

        // 驱动 `MotionToken::OverlayPreview`（120ms）显隐过渡；减弱动画时单步瞬切。
        let target_alpha = if self.visible_target().is_some() {
            1.0
        } else {
            0.0
        };
        self.visibility.set_target(target_alpha);
        self.visibility.step(dt, policy);
    }

    /// 基于当前叠加层状态生成拖拽代理描述（供外部幽灵窗或浮岛宿主消费）。
    #[must_use]
    pub fn drag_proxy(
        &self,
        title: impl Into<String>,
        badge: Option<String>,
    ) -> Option<DockDragProxy> {
        if !self.active {
            return None;
        }
        let target = self.raw_target()?;
        let detached = target.kind == DockDropKind::TearOut;
        Some(DockDragProxy {
            title: title.into(),
            badge,
            pointer: self.pointer,
            size: Vec2::new(196.0, 44.0),
            detached,
            semantic: target.semantic,
            hint: target.hint.clone(),
        })
    }

    fn requires_dwell_for(&self, target_window: Option<u64>) -> bool {
        match (self.source, target_window) {
            (Some(DockDragSource::Shell { window_id: src }), Some(dst)) => dst != src,
            _ => false,
        }
    }
}

/// 在给定候选窗口集合上仲裁同一指针位置的多层级停靠目标（索引 `0` 为最内层）。
#[must_use]
pub fn resolve_candidates(
    source: DockDragSource,
    pointer: Vec2,
    windows: &[DockCandidateWindow],
) -> Vec<DockDropTarget> {
    let Some(window) = windows.iter().find(|w| {
        w.bounds.size.x > 0.0 && w.bounds.size.y > 0.0 && w.bounds.contains_point(pointer)
    }) else {
        return vec![DockDropTarget::tear_out()];
    };

    // 整窗拖回自身窗口不做停靠预览（视为禁止）。
    let dragging_self_shell = matches!(
        source,
        DockDragSource::Shell { window_id } if window_id == window.window_id
    );

    let mut matched_panes: Vec<&DockCandidatePane> = window
        .panes
        .iter()
        .filter(|p| p.rect.size.x > 0.0 && p.rect.size.y > 0.0 && p.rect.contains_point(pointer))
        .collect();
    // 按嵌套深度降序、面积升序排序，保证最内层面板排在索引 0。
    matched_panes.sort_by(|a, b| {
        b.nesting_depth.cmp(&a.nesting_depth).then_with(|| {
            let area_a = a.rect.size.x * a.rect.size.y;
            let area_b = b.rect.size.x * b.rect.size.y;
            area_a
                .partial_cmp(&area_b)
                .unwrap_or(core::cmp::Ordering::Equal)
        })
    });

    let mut targets = Vec::new();
    for pane in matched_panes {
        let pane_target = resolve_pane_target(source, window, pane, pointer, dragging_self_shell);
        targets.push(pane_target);
    }

    // 整窗外缘吸附带作为最外层候选加入层级列表，允许滚轮下钻切换。
    if window.allow_root_edge {
        if let Some(edge) = detect_root_edge(window.bounds, pointer) {
            let blocked = window.blocked || dragging_self_shell;
            let semantic = if blocked {
                DockDropSemantic::Blocked
            } else {
                DockDropSemantic::Allowed
            };
            let hint = if blocked {
                "Cannot Dock Here".to_owned()
            } else {
                format!("Dock Window {}", edge.label())
            };
            targets.push(DockDropTarget {
                window_id: Some(window.window_id),
                pane_id: None,
                kind: DockDropKind::RootEdge(edge),
                semantic,
                preview_rect: edge.half_rect(window.bounds),
                hint,
                level_index: 0,
                level_count: 1,
            });
        }
    }

    if targets.is_empty() {
        let semantic = if window.blocked || dragging_self_shell {
            DockDropSemantic::Blocked
        } else {
            DockDropSemantic::Allowed
        };
        targets.push(DockDropTarget {
            window_id: Some(window.window_id),
            pane_id: None,
            kind: DockDropKind::Center,
            semantic,
            preview_rect: window.bounds,
            hint: if semantic == DockDropSemantic::Blocked {
                "Cannot Dock Here".to_owned()
            } else {
                "Dock Center".to_owned()
            },
            level_index: 0,
            level_count: 1,
        });
    }

    let count = targets.len();
    for (idx, item) in targets.iter_mut().enumerate() {
        item.level_index = idx;
        item.level_count = count;
    }
    targets
}

fn resolve_pane_target(
    source: DockDragSource,
    window: &DockCandidateWindow,
    pane: &DockCandidatePane,
    pointer: Vec2,
    dragging_self_shell: bool,
) -> DockDropTarget {
    let tab_bar = pane.tab_bar_rect();
    let is_self_pane = matches!(
        source,
        DockDragSource::Tab {
            window_id,
            pane_id,
            ..
        } if window_id == window.window_id && pane_id == pane.pane_id
    );

    // 1. 指针落在标签栏内且存在标签几何时，生成 `Reorder` 2px 插入指示线。
    if !source.is_shell()
        && tab_bar.size.y > 0.0
        && tab_bar.contains_point(pointer)
        && !pane.tab_rects.is_empty()
    {
        let (insert_index, line_x) = resolve_tab_reorder_line(&pane.tab_rects, tab_bar, pointer.x);
        let blocked = window.blocked || pane.blocked;
        let preview_rect = Rect::new(
            (line_x - REORDER_INDICATOR_WIDTH * 0.5).clamp(
                tab_bar.origin.x,
                (tab_bar.max().x - REORDER_INDICATOR_WIDTH).max(tab_bar.origin.x),
            ),
            tab_bar.origin.y + 2.0,
            REORDER_INDICATOR_WIDTH,
            (tab_bar.size.y - 4.0).max(2.0),
        );
        return DockDropTarget {
            window_id: Some(window.window_id),
            pane_id: Some(pane.pane_id),
            kind: DockDropKind::Reorder {
                insert_index,
                line_x,
            },
            semantic: if blocked {
                DockDropSemantic::Blocked
            } else {
                DockDropSemantic::Allowed
            },
            preview_rect,
            hint: if blocked {
                "Cannot Reorder Here".to_owned()
            } else {
                "Reorder Tab".to_owned()
            },
            level_index: 0,
            level_count: 1,
        };
    }

    // 2. 在面板内判定四边外侧 25% 区域或中心区。
    let nx = ((pointer.x - pane.rect.origin.x) / pane.rect.size.x).clamp(0.0, 1.0);
    let ny = ((pointer.y - pane.rect.origin.y) / pane.rect.size.y).clamp(0.0, 1.0);
    let mut best_edge: Option<DockEdge> = None;
    let mut best_dist = 0.25_f32;
    if nx < best_dist {
        best_dist = nx;
        best_edge = Some(DockEdge::Left);
    }
    if 1.0 - nx < best_dist {
        best_dist = 1.0 - nx;
        best_edge = Some(DockEdge::Right);
    }
    if ny < best_dist {
        best_dist = ny;
        best_edge = Some(DockEdge::Top);
    }
    if 1.0 - ny < best_dist {
        best_edge = Some(DockEdge::Bottom);
    }

    if let Some(edge) = best_edge {
        let blocked = window.blocked || pane.blocked || dragging_self_shell;
        return DockDropTarget {
            window_id: Some(window.window_id),
            pane_id: Some(pane.pane_id),
            kind: DockDropKind::Edge(edge),
            semantic: if blocked {
                DockDropSemantic::Blocked
            } else {
                DockDropSemantic::Allowed
            },
            preview_rect: edge.half_rect(pane.rect),
            hint: if blocked {
                "Cannot Dock Here".to_owned()
            } else {
                format!("Dock {}", edge.label())
            },
            level_index: 0,
            level_count: 1,
        };
    }

    // 3. 中心区域：避开顶部标签栏，仅高亮内容区；拖回自身原组中心视为 `Blocked`。
    let content = pane.content_rect();
    if pane.allow_nest && !window.blocked && !pane.blocked && !dragging_self_shell && !is_self_pane
    {
        let inset = 6.0_f32
            .min(content.size.x * 0.15)
            .min(content.size.y * 0.15);
        let nest_rect = Rect::new(
            content.origin.x + inset,
            content.origin.y + inset,
            (content.size.x - inset * 2.0).max(0.0),
            (content.size.y - inset * 2.0).max(0.0),
        );
        return DockDropTarget {
            window_id: Some(window.window_id),
            pane_id: Some(pane.pane_id),
            kind: DockDropKind::Nest,
            semantic: DockDropSemantic::NestArmed,
            preview_rect: nest_rect,
            hint: "Nest Pane".to_owned(),
            level_index: 0,
            level_count: 1,
        };
    }

    let blocked =
        window.blocked || pane.blocked || !pane.allow_center || is_self_pane || dragging_self_shell;
    DockDropTarget {
        window_id: Some(window.window_id),
        pane_id: Some(pane.pane_id),
        kind: DockDropKind::Center,
        semantic: if blocked {
            DockDropSemantic::Blocked
        } else {
            DockDropSemantic::Allowed
        },
        preview_rect: content,
        hint: if blocked {
            "Cannot Dock Here".to_owned()
        } else {
            "Dock Center".to_owned()
        },
        level_index: 0,
        level_count: 1,
    }
}

fn detect_root_edge(bounds: Rect, pointer: Vec2) -> Option<DockEdge> {
    if bounds.size.x <= 0.0 || bounds.size.y <= 0.0 || !bounds.contains_point(pointer) {
        return None;
    }
    let band_x = ROOT_EDGE_BAND_PX.min(bounds.size.x * 0.25);
    let band_y = ROOT_EDGE_BAND_PX.min(bounds.size.y * 0.25);
    let left = pointer.x - bounds.origin.x;
    let right = bounds.max().x - pointer.x;
    let top = pointer.y - bounds.origin.y;
    let bottom = bounds.max().y - pointer.y;

    let mut best: Option<(DockEdge, f32)> = None;
    if left <= band_x {
        best = Some((DockEdge::Left, left));
    }
    if right <= band_x && best.is_none_or(|(_, d)| right < d) {
        best = Some((DockEdge::Right, right));
    }
    if top <= band_y && best.is_none_or(|(_, d)| top < d) {
        best = Some((DockEdge::Top, top));
    }
    if bottom <= band_y && best.is_none_or(|(_, d)| bottom < d) {
        best = Some((DockEdge::Bottom, bottom));
    }
    best.map(|(edge, _)| edge)
}

fn resolve_tab_reorder_line(tab_rects: &[Rect], tab_bar: Rect, pointer_x: f32) -> (usize, f32) {
    for (idx, rect) in tab_rects.iter().enumerate() {
        let mid_x = rect.origin.x + rect.size.x * 0.5;
        if pointer_x < mid_x {
            return (idx, rect.origin.x);
        }
    }
    let last_end = tab_rects
        .last()
        .map_or(tab_bar.origin.x, |r| r.max().x)
        .min(tab_bar.max().x);
    (tab_rects.len(), last_end)
}

/// 将目标矩形强钳制裁剪到 `bounds` 范围内，保证绝不溢出叠加层视口。
#[must_use]
pub fn clamp_rect_to_bounds(rect: Rect, bounds: Rect) -> Option<Rect> {
    if rect.size.x <= 0.0 || rect.size.y <= 0.0 || bounds.size.x <= 0.0 || bounds.size.y <= 0.0 {
        return None;
    }
    let x0 = rect.origin.x.max(bounds.origin.x);
    let y0 = rect.origin.y.max(bounds.origin.y);
    let x1 = rect.max().x.min(bounds.max().x);
    let y1 = rect.max().y.min(bounds.max().y);
    let w = x1 - x0;
    let h = y1 - y0;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(Rect::new(x0, y0, w, h))
}

/// 计算紧贴光标的提示胶囊布局矩形：靠近右/下边界时自动向光标对侧翻转，并强钳制在 `overlay_bounds` 内。
#[must_use]
pub fn layout_hint_pill(pointer: Vec2, desired_width: f32, overlay_bounds: Rect) -> Option<Rect> {
    if overlay_bounds.size.x <= 0.0 || overlay_bounds.size.y <= 0.0 {
        return None;
    }
    let max_w = (overlay_bounds.size.x - 8.0).clamp(0.0, HINT_PILL_MAX_WIDTH);
    let pill_w = desired_width.max(48.0).min(max_w);
    let pill_h = HINT_PILL_HEIGHT.min(overlay_bounds.size.y);
    if pill_w <= 0.0 || pill_h <= 0.0 {
        return None;
    }

    // 默认在光标右下方；触碰右/下边缘时向对侧翻转避免裁切或遮挡。
    let mut x = pointer.x + HINT_PILL_OFFSET.x;
    if x + pill_w > overlay_bounds.max().x {
        x = pointer.x - HINT_PILL_OFFSET.x - pill_w;
    }
    let mut y = pointer.y + HINT_PILL_OFFSET.y;
    if y + pill_h > overlay_bounds.max().y {
        y = pointer.y - HINT_PILL_OFFSET.y - pill_h;
    }

    let max_x = (overlay_bounds.max().x - pill_w).max(overlay_bounds.origin.x);
    let max_y = (overlay_bounds.max().y - pill_h).max(overlay_bounds.origin.y);
    Some(Rect::new(
        x.clamp(overlay_bounds.origin.x, max_x),
        y.clamp(overlay_bounds.origin.y, max_y),
        pill_w,
        pill_h,
    ))
}

/// 对提示胶囊单行文案做窄宽安全省略截断（不换行）。
#[must_use]
pub fn elide_hint_label(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut out: String = text.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

/// 发射停靠预览叠加层图元：半透明高亮区域、`1px` 语义边框（或 `2px` 插入线）及光标提示胶囊。
///
/// 严格非交互穿透（`hit = None`、`focusable = false`、`a11y = None`），绝不发射五向指南针按钮；
/// 无活跃目标、整窗未满 500ms 驻留防抖、处于 `TearOut` 或视口/面板尺寸 `<= 0` 时安全返回空输出。
#[must_use]
pub fn emit_docking_overlay(
    state: &DockOverlayState,
    overlay_bounds: Rect,
    tokens: DockOverlayTokens,
    pill_glyphs: &[GlyphQuad],
    z: u16,
) -> WidgetOutput {
    if !state.is_active() || overlay_bounds.size.x <= 0.0 || overlay_bounds.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    let Some(target) = state.visible_target() else {
        return WidgetOutput::default();
    };
    let progress = state.preview_opacity();
    if progress <= 0.0 {
        return WidgetOutput::default();
    }
    let Some(preview_rect) = clamp_rect_to_bounds(target.preview_rect, overlay_bounds) else {
        return WidgetOutput::default();
    };

    let (fill_token, border_token, fill_alpha) = tokens.resolve_semantic(target.semantic);
    let mut primitives = Vec::with_capacity(5 + pill_glyphs.len());
    let mut payloads = Vec::with_capacity(pill_glyphs.len());

    match target.kind {
        DockDropKind::Reorder { .. } => {
            let line_color = if target.semantic == DockDropSemantic::Blocked {
                tokens.blocked_border
            } else {
                tokens.reorder_line
            };
            let mut line = instance(
                preview_rect,
                [1.0; 4],
                line_color,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Content,
                z,
            );
            line.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, progress);
            primitives.push(line);
        }
        DockDropKind::TearOut => {
            return WidgetOutput::default();
        }
        DockDropKind::Edge(_)
        | DockDropKind::RootEdge(_)
        | DockDropKind::Center
        | DockDropKind::Nest => {
            // 1. 半透明语义填充层（Allowed: 0.22 alpha / Blocked: 0.14 alpha / NestArmed: 0.22 alpha）。
            let mut fill = surface(preview_rect, OVERLAY_CORNER, fill_token, z);
            fill.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(z, (fill_alpha * progress).clamp(0.0, 1.0));
            primitives.push(fill);

            // 2. 1px 清晰边框层。
            let mut border = instance(
                preview_rect,
                [OVERLAY_CORNER; 4],
                Token::NONE,
                border_token,
                PREVIEW_BORDER_WIDTH,
                0.0,
                PrimType::Stroke,
                SubElement::Border,
                z,
            );
            border.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, progress);
            primitives.push(border);
        }
    }

    // 3. 紧贴光标的提示胶囊（贴边自动翻转并强钳制在叠加层边界内）。
    let desired_pill_w =
        (target.hint.chars().count() as f32 * 7.2 + HINT_PILL_PAD_X * 2.0).max(72.0);
    if let Some(pill_rect) = layout_hint_pill(state.pointer(), desired_pill_w, overlay_bounds) {
        let pill_z = z.saturating_add(1);
        let mut pill = outlined(
            pill_rect,
            pill_rect.size.y * 0.5,
            tokens.pill_bg,
            tokens.pill_border,
            PREVIEW_BORDER_WIDTH,
            6.0,
            pill_z,
        );
        pill.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(pill_z, progress);
        primitives.push(pill);

        if !pill_glyphs.is_empty() {
            glyphs(
                pill_glyphs,
                tokens.pill_text,
                pill_z,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 发射外部拖拽代理卡片图元（用于 `TearOut` 幽灵窗或跨窗跟随浮岛）。
///
/// 严格非交互穿透，当尺寸或视口 `<= 0` 时安全返回空输出。
#[must_use]
pub fn emit_drag_proxy(
    proxy: &DockDragProxy,
    viewport: Rect,
    tokens: DockOverlayTokens,
    label_glyphs: &[GlyphQuad],
    z: u16,
) -> WidgetOutput {
    if proxy.size.x <= 0.0
        || proxy.size.y <= 0.0
        || viewport.size.x <= 0.0
        || viewport.size.y <= 0.0
    {
        return WidgetOutput::default();
    }
    let w = proxy.size.x.min(viewport.size.x);
    let h = proxy.size.y.min(viewport.size.y);
    let max_x = (viewport.max().x - w).max(viewport.origin.x);
    let max_y = (viewport.max().y - h).max(viewport.origin.y);
    let rect = Rect::new(
        (proxy.pointer.x + 12.0).clamp(viewport.origin.x, max_x),
        (proxy.pointer.y + 12.0).clamp(viewport.origin.y, max_y),
        w,
        h,
    );

    let (_, semantic_border, _) = tokens.resolve_semantic(proxy.semantic);
    let border = if proxy.detached {
        tokens.proxy_accent
    } else {
        semantic_border
    };

    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(outlined(
        rect,
        OVERLAY_CORNER,
        tokens.proxy_bg,
        border,
        PREVIEW_BORDER_WIDTH,
        10.0,
        z,
    ));

    // 左侧 3px 状态强调条，区分允许/禁止/分离语义。
    let bar_rect = Rect::new(
        rect.origin.x + 2.0,
        rect.origin.y + 6.0,
        3.0,
        (rect.size.y - 12.0).max(2.0),
    );
    primitives.push(surface(bar_rect, 1.5, border, z));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            tokens.pill_text,
            z,
            &mut primitives,
            &mut payloads,
        );
    }

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window() -> DockCandidateWindow {
        let pane =
            DockCandidatePane::new(10, Rect::new(0.0, 0.0, 400.0, 300.0)).with_tab_rects(vec![
                Rect::new(0.0, 0.0, 100.0, DOCK_TAB_BAR),
                Rect::new(100.0, 0.0, 100.0, DOCK_TAB_BAR),
            ]);
        DockCandidateWindow::new(1, Rect::new(0.0, 0.0, 400.0, 300.0), vec![pane])
    }

    #[test]
    fn edge_root_edge_center_reorder_and_tear_out_geometry_match_spec() {
        let win = sample_window();
        let source = DockDragSource::Tab {
            window_id: 2,
            pane_id: 99,
            tab_id: 7,
        };

        // 1. Edge 半区预览（左侧外 25%，避开 24px RootEdge 带外仍选中左半区）
        let left_targets =
            resolve_candidates(source, Vec2::new(40.0, 150.0), core::slice::from_ref(&win));
        assert_eq!(left_targets[0].kind, DockDropKind::Edge(DockEdge::Left));
        assert_eq!(
            left_targets[0].preview_rect,
            Rect::new(0.0, 0.0, 200.0, 300.0)
        );

        // 2. RootEdge 整窗外缘吸附带作为外层候选存在
        let root_targets =
            resolve_candidates(source, Vec2::new(6.0, 150.0), core::slice::from_ref(&win));
        assert!(root_targets
            .iter()
            .any(|t| t.kind == DockDropKind::RootEdge(DockEdge::Left)
                && t.preview_rect == Rect::new(0.0, 0.0, 200.0, 300.0)));

        // 3. Center 避开顶部标签栏高度
        let center_targets =
            resolve_candidates(source, Vec2::new(200.0, 160.0), core::slice::from_ref(&win));
        assert_eq!(center_targets[0].kind, DockDropKind::Center);
        assert_eq!(center_targets[0].preview_rect.origin.y, DOCK_TAB_BAR);
        assert_eq!(center_targets[0].preview_rect.size.y, 300.0 - DOCK_TAB_BAR);

        // 4. Reorder 在标签栏生成 2px 插入指示线
        let reorder_targets =
            resolve_candidates(source, Vec2::new(90.0, 12.0), core::slice::from_ref(&win));
        assert!(matches!(
            reorder_targets[0].kind,
            DockDropKind::Reorder {
                insert_index: 1,
                ..
            }
        ));
        assert_eq!(
            reorder_targets[0].preview_rect.size.x,
            REORDER_INDICATOR_WIDTH
        );

        // 5. TearOut 不产生宿主预览框
        let mut state = DockOverlayState::new();
        state.begin_drag(source, Vec2::new(900.0, 900.0));
        state.update(
            Vec2::new(900.0, 900.0),
            &[win],
            0.2,
            MotionPolicy::reduced(),
        );
        assert!(state.is_tear_out());
        assert!(state.visible_target().is_none());
        let output = emit_docking_overlay(
            &state,
            Rect::new(0.0, 0.0, 400.0, 300.0),
            DockOverlayTokens::standard(),
            &[],
            50,
        );
        assert!(
            output.primitives.is_empty(),
            "TearOut must not emit host preview rect"
        );
    }

    #[test]
    fn three_state_semantic_tokens_map_allowed_blocked_and_nest_fallback() {
        let standard = DockOverlayTokens::standard();
        let (a_fill, a_border, a_alpha) = standard.resolve_semantic(DockDropSemantic::Allowed);
        assert_eq!(a_fill, Token::PRIMARY);
        assert_eq!(a_border, Token::PRIMARY);
        assert!((a_alpha - 0.22).abs() < 1e-6);

        let (b_fill, b_border, b_alpha) = standard.resolve_semantic(DockDropSemantic::Blocked);
        assert_eq!(b_fill, Token::ON_SURFACE_MUTED);
        assert_eq!(b_border, Token::OUTLINE);
        assert!((0.12..=0.16).contains(&b_alpha));

        // 未配置次级强调色时 NestArmed 回退 Token::PRIMARY
        let (n_fill, n_border, _) = standard.resolve_semantic(DockDropSemantic::NestArmed);
        assert_eq!(n_fill, Token::PRIMARY);
        assert_eq!(n_border, Token::PRIMARY);

        // 配置次级强调色后使用自定义槽位
        let custom = standard.with_nest_accent(Token::ACCENT, Token::ACCENT);
        let (cn_fill, cn_border, _) = custom.resolve_semantic(DockDropSemantic::NestArmed);
        assert_eq!(cn_fill, Token::ACCENT);
        assert_eq!(cn_border, Token::ACCENT);
    }

    #[test]
    fn dragging_tab_back_to_own_center_is_blocked() {
        let win = sample_window();
        let source = DockDragSource::Tab {
            window_id: 1,
            pane_id: 10,
            tab_id: 1,
        };
        let targets = resolve_candidates(source, Vec2::new(200.0, 160.0), &[win]);
        assert_eq!(targets[0].kind, DockDropKind::Center);
        assert_eq!(targets[0].semantic, DockDropSemantic::Blocked);
    }

    #[test]
    fn shell_cross_window_dwell_500ms_12px_debounce_and_tab_immediate() {
        let win = sample_window();
        let policy = MotionPolicy::reduced();

        // 1. 普通单标签拖拽：无延迟立即显示
        let mut tab_state = DockOverlayState::new();
        tab_state.begin_drag(
            DockDragSource::Tab {
                window_id: 2,
                pane_id: 20,
                tab_id: 5,
            },
            Vec2::new(200.0, 160.0),
        );
        tab_state.update(
            Vec2::new(200.0, 160.0),
            core::slice::from_ref(&win),
            0.016,
            policy,
        );
        assert!(tab_state.visible_target().is_some());
        assert_eq!(tab_state.preview_opacity(), 1.0);

        // 2. 整窗外壳跨窗拖入：未满 500ms 静默隐藏
        let mut shell_state = DockOverlayState::new();
        shell_state.begin_drag(
            DockDragSource::Shell { window_id: 2 },
            Vec2::new(200.0, 160.0),
        );
        shell_state.update(
            Vec2::new(200.0, 160.0),
            core::slice::from_ref(&win),
            0.20,
            policy,
        );
        assert!(!shell_state.is_dwell_armed());
        assert!(shell_state.visible_target().is_none());

        // 超过 12px 大幅移动会重置驻留计时器
        shell_state.update(
            Vec2::new(225.0, 160.0),
            core::slice::from_ref(&win),
            0.40,
            policy,
        );
        assert!(!shell_state.is_dwell_armed());
        assert!(shell_state.visible_target().is_none());

        // 在 12px 容差内停留累计满 500ms 后武装显示
        shell_state.update(
            Vec2::new(230.0, 162.0),
            core::slice::from_ref(&win),
            0.25,
            policy,
        );
        assert!(!shell_state.is_dwell_armed());
        shell_state.update(
            Vec2::new(232.0, 163.0),
            core::slice::from_ref(&win),
            0.26,
            policy,
        );
        assert!(shell_state.is_dwell_armed());
        assert!(shell_state.visible_target().is_some());

        // 已武装后在同一窗口内微调或移动不再重置
        shell_state.update(Vec2::new(350.0, 200.0), &[win], 0.016, policy);
        assert!(shell_state.is_dwell_armed());
        assert!(shell_state.visible_target().is_some());
    }

    #[test]
    fn nudge_level_switches_between_inner_outer_and_root_edge() {
        let outer = DockCandidatePane::new(1, Rect::new(0.0, 0.0, 400.0, 300.0)).with_depth(0);
        let inner = DockCandidatePane::new(2, Rect::new(0.0, 0.0, 200.0, 300.0)).with_depth(1);
        let win =
            DockCandidateWindow::new(1, Rect::new(0.0, 0.0, 400.0, 300.0), vec![outer, inner]);

        let mut state = DockOverlayState::new();
        state.begin_drag(
            DockDragSource::Tab {
                window_id: 9,
                pane_id: 99,
                tab_id: 3,
            },
            Vec2::new(8.0, 150.0),
        );
        state.update(Vec2::new(8.0, 150.0), &[win], 0.2, MotionPolicy::reduced());
        assert_eq!(state.candidates().len(), 3);
        // 默认优先最内层面板（pane_id = 2）
        assert_eq!(state.selected_level(), 0);
        assert_eq!(state.visible_target().unwrap().pane_id, Some(2));

        // 向上滚轮切换到外层面板（pane_id = 1）
        assert_eq!(state.nudge_level(1), 1);
        assert_eq!(state.visible_target().unwrap().pane_id, Some(1));

        // 再切换一层到整窗外缘（RootEdge）
        assert_eq!(state.nudge_level(1), 2);
        assert!(matches!(
            state.visible_target().unwrap().kind,
            DockDropKind::RootEdge(DockEdge::Left)
        ));

        // 向下滚轮回退
        assert_eq!(state.nudge_level(-2), 0);
        assert_eq!(state.visible_target().unwrap().pane_id, Some(2));
    }

    #[test]
    fn hint_pill_flips_and_clamps_within_overlay_bounds_and_skips_zero_size() {
        let bounds = Rect::new(0.0, 0.0, 300.0, 200.0);
        // 靠近右下角时必须向光标左上方翻转，并严格留在 bounds 内
        let near_br = layout_hint_pill(Vec2::new(295.0, 195.0), 140.0, bounds).expect("pill");
        assert!(near_br.origin.x >= bounds.origin.x);
        assert!(near_br.origin.y >= bounds.origin.y);
        assert!(near_br.max().x <= bounds.max().x);
        assert!(near_br.max().y <= bounds.max().y);
        assert!(near_br.origin.x < 295.0, "pill should flip left of cursor");
        assert!(near_br.origin.y < 195.0, "pill should flip above cursor");

        // 零尺寸安全跳过
        assert!(layout_hint_pill(Vec2::new(10.0, 10.0), 100.0, Rect::ZERO).is_none());
        assert_eq!(elide_hint_label("Dock Window Left", 10), "Dock Wi...");

        let win = sample_window();
        let mut state = DockOverlayState::new();
        state.begin_drag(
            DockDragSource::Tab {
                window_id: 2,
                pane_id: 1,
                tab_id: 1,
            },
            Vec2::new(200.0, 150.0),
        );
        state.update(
            Vec2::new(200.0, 150.0),
            &[win],
            0.2,
            MotionPolicy::reduced(),
        );
        let zero_out =
            emit_docking_overlay(&state, Rect::ZERO, DockOverlayTokens::standard(), &[], 10);
        assert!(zero_out.primitives.is_empty());

        let normal_out = emit_docking_overlay(
            &state,
            Rect::new(0.0, 0.0, 400.0, 300.0),
            DockOverlayTokens::standard(),
            &[],
            10,
        );
        assert!(!normal_out.primitives.is_empty());
        assert!(normal_out.hit.is_none());
        assert!(!normal_out.focusable);
        assert!(normal_out.a11y.is_none());
        // 校验填充 alpha 接近 Allowed 0.22
        assert!((normal_out.primitives[0].opacity() - ALLOWED_FILL_ALPHA).abs() < 0.01);
    }

    #[test]
    fn drag_proxy_emits_non_interactive_card_and_skips_zero_bounds() {
        let proxy = DockDragProxy::new("Inspector", Vec2::new(280.0, 190.0));
        let out = emit_drag_proxy(
            &proxy,
            Rect::new(0.0, 0.0, 300.0, 200.0),
            DockOverlayTokens::standard(),
            &[],
            20,
        );
        assert!(out.primitives.len() >= 2);
        assert!(out.hit.is_none());
        assert!(!out.focusable);
        // 代理卡片不得超出视口右/下边缘
        let card = &out.primitives[0];
        assert!(card.local_pos[0] + card.size[0] <= 300.0 + 1e-4);
        assert!(card.local_pos[1] + card.size[1] <= 200.0 + 1e-4);

        let empty = emit_drag_proxy(&proxy, Rect::ZERO, DockOverlayTokens::standard(), &[], 20);
        assert!(empty.primitives.is_empty());
    }
}

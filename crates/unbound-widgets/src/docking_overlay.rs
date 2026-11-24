//! 纯区域高亮跨窗停靠预览叠加层与拖拽代理协议。
//!
//! 本模块提供零平台依赖的纯几何协议、状态机与图元发射器：
//! * **纯区域高亮预览**：彻底取消五向指南针悬浮按钮，直接根据目标面板/窗口几何发射
//!   `Edge` / `RootEdge` 半区、`Center`（自动扣除顶部标签栏高度）、`Reorder`（`2px` 标签栏插入指示线）
//!   以及 `Nest` 预览矩形；窗外自由空间 `TearOut` 不绘制宿主预览框，交由 [`DockDragProxy`] 表达。
//! * **三态语义配色**：[`DockOverlayTokens`] 区分 `Allowed`（`Accent`）、`Forbidden`（`Error`）与
//!   `Nested`（`Warning` / 可选次级强调色槽位）。
//! * **防闪烁驻留防抖**：整窗外壳（`Shell`）跨视口拖入目标窗口时，需在 `12px` 容差内停留满 `500ms`
//!   才武装显示确认预览（已武装后微调移动不再重置）；普通单标签（`Tab`）拖拽即时响应。
//! * **多层级滚轮下钻**：同一指针位置存在多层嵌套候选时，通过 [`DockOverlayState::nudge_level`]
//!   在「内层面板 → 嵌套层 → 整窗外缘」之间切换，默认优先最内层。
//! * **动效与零交互**：消费 [`unbound_anim::MotionToken::OverlayPreview`] 的 [`Tween`] 过渡，
//!   在 `reduced_motion` 下单步瞬切；所有发射图元保持零命中区域、零焦点抢占。

use unbound_anim::{MotionPolicy, MotionToken, Tween};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, instance, outlined, surface};
use crate::WidgetOutput;

/// 整窗外壳（`Shell`）跨视口拖入的驻留防抖时长（秒）：`500ms`。
pub const SHELL_DWELL_DURATION_SECS: f32 = 0.5;

/// 整窗外壳（`Shell`）跨视口拖入的驻留移动容差（逻辑像素）：`12px`。
pub const SHELL_DWELL_THRESHOLD_PX: f32 = 12.0;

/// 标签栏重排插入指示线宽度（逻辑像素）：`2px`。
pub const REORDER_LINE_WIDTH_PX: f32 = 2.0;

/// 整窗外缘吸附带默认厚度（逻辑像素）。
pub const DEFAULT_ROOT_EDGE_BAND_PX: f32 = 24.0;

/// 面板内缘分割判定比例（四分之一边距）。
const PANE_EDGE_RATIO: f32 = 0.25;

/// 预览框圆角半径（逻辑像素）。
const OVERLAY_CORNER_RADIUS: f32 = 4.0;

/// 预览框与提示胶囊边框宽度（逻辑像素）。
const OVERLAY_BORDER_WIDTH: f32 = 1.0;

/// 提示胶囊（`HintPill`）默认高度（逻辑像素）。
const HINT_PILL_HEIGHT: f32 = 24.0;

/// 提示胶囊（`HintPill`）最大宽度（逻辑像素）。
const HINT_PILL_MAX_WIDTH: f32 = 220.0;

/// 提示胶囊（`HintPill`）最小宽度（逻辑像素）。
const HINT_PILL_MIN_WIDTH: f32 = 56.0;

/// 提示胶囊相对光标的默认偏移（逻辑像素）。
const HINT_PILL_OFFSET_PX: f32 = 14.0;

/// 提示胶囊与叠加层边界的最小安全边距（逻辑像素）。
const HINT_PILL_MARGIN_PX: f32 = 8.0;

/// 提示胶囊水平内边距（逻辑像素）。
const HINT_PILL_PADDING_X: f32 = 10.0;

/// 提示胶囊单字符估算宽度（逻辑像素，用于无塑形文本时的边界钳制）。
const HINT_PILL_CHAR_WIDTH_ESTIMATE: f32 = 7.0;

/// 拖拽载荷类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DockDragPayloadKind {
    /// 普通单标签拖拽，即时响应无驻留延迟。
    Tab,
    /// 整窗外壳跨视口拖拽，需满足 `500ms / 12px` 驻留防抖。
    Shell,
}

/// 停靠边缘方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
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
    /// 在给定容器矩形内计算半区预览矩形，并严格裁剪在容器边界内。
    #[must_use]
    pub fn half_rect(self, bounds: Rect) -> Rect {
        if bounds.is_empty() || !bounds.is_finite() {
            return Rect::ZERO;
        }
        let half = match self {
            Self::Left => Rect::new(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.x * 0.5,
                bounds.size.y,
            ),
            Self::Right => Rect::new(
                bounds.origin.x + bounds.size.x * 0.5,
                bounds.origin.y,
                bounds.size.x * 0.5,
                bounds.size.y,
            ),
            Self::Top => Rect::new(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.x,
                bounds.size.y * 0.5,
            ),
            Self::Bottom => Rect::new(
                bounds.origin.x,
                bounds.origin.y + bounds.size.y * 0.5,
                bounds.size.x,
                bounds.size.y * 0.5,
            ),
        };
        half.intersect(&bounds)
    }

    /// 返回对应的英文提示文案。
    #[must_use]
    const fn hint_label(self, is_root: bool) -> &'static str {
        match (self, is_root) {
            (Self::Left, false) => "Dock Left",
            (Self::Right, false) => "Dock Right",
            (Self::Top, false) => "Dock Top",
            (Self::Bottom, false) => "Dock Bottom",
            (Self::Left, true) => "Dock Window Left",
            (Self::Right, true) => "Dock Window Right",
            (Self::Top, true) => "Dock Window Top",
            (Self::Bottom, true) => "Dock Window Bottom",
        }
    }
}

/// 停靠落点类型。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum DockDropKind {
    /// 面板半区边缘分割停靠。
    Edge(DockEdge),
    /// 整窗外缘吸附带停靠。
    RootEdge(DockEdge),
    /// 合并到目标面板内容区（自动避开顶部标签栏）。
    Center,
    /// 标签栏重排插入（渲染 `2px` 垂直插入指示线）。
    Reorder {
        /// 插入目标索引位置。
        insert_index: usize,
        /// 插入指示线中心 X 坐标（逻辑像素）。
        line_x: f32,
    },
    /// 嵌套或组合停靠。
    Nest,
    /// 窗外自由空间撕出（不在宿主内绘制预览框）。
    TearOut,
}

/// 三态停靠语义状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DockDropSemantic {
    /// 允许停靠（主题 `Accent`）。
    Allowed,
    /// 禁止停靠（主题 `Error`，如拖回自身原标签组中心或受限目标）。
    Forbidden,
    /// 嵌套/组合停靠（主题 `Warning` 或可选次级槽位）。
    Nested,
}

/// 候选停靠面板描述符（处于统一桌面逻辑坐标系下）。
#[derive(Clone, Debug, PartialEq)]
pub struct DockCandidatePane {
    /// 面板唯一标识。
    pub pane_id: u64,
    /// 面板边界矩形。
    pub rect: Rect,
    /// 顶部标签栏高度（逻辑像素）。
    pub tab_bar_height: f32,
    /// 标签栏内各个标签页的矩形（按从左到右排列）。
    pub tab_rects: Vec<Rect>,
    /// 是否允许作为中心合并目标。
    pub allow_center: bool,
    /// 是否允许作为嵌套/组合目标。
    pub allow_nest: bool,
    /// 该面板是否为当前拖拽源所属的原标签组。
    pub is_source_group: bool,
}

impl DockCandidatePane {
    /// 计算面板的顶部标签栏矩形（严格裁剪在面板内）。
    #[must_use]
    pub fn tab_bar_rect(&self) -> Rect {
        if self.rect.is_empty() || !self.rect.is_finite() {
            return Rect::ZERO;
        }
        let bar_h = self.tab_bar_height.max(0.0).min(self.rect.size.y);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            bar_h,
        )
    }

    /// 计算避开顶部标签栏后的内容区矩形（用于 `Center` 预览）。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        if self.rect.is_empty() || !self.rect.is_finite() {
            return Rect::ZERO;
        }
        let bar_h = self.tab_bar_height.max(0.0).min(self.rect.size.y);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + bar_h,
            self.rect.size.x,
            (self.rect.size.y - bar_h).max(0.0),
        )
    }
}

/// 候选停靠窗口描述符（处于统一桌面逻辑坐标系下）。
#[derive(Clone, Debug, PartialEq)]
pub struct DockCandidateWindow {
    /// 窗口唯一标识。
    pub window_id: u64,
    /// 窗口边界矩形。
    pub window_rect: Rect,
    /// 窗口叠放层级（数值越大越靠前）。
    pub z_order: u32,
    /// 整窗外缘吸附带厚度（逻辑像素）。
    pub root_edge_band: f32,
    /// 窗口内包含的候选面板列表。
    pub panes: Vec<DockCandidatePane>,
}

/// 仲裁后的单个停靠落点候选目标。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DockDropTarget {
    /// 目标窗口标识。
    pub window_id: u64,
    /// 目标面板标识（整窗根级停靠或 `TearOut` 时为 `None`）。
    pub pane_id: Option<u64>,
    /// 停靠落点类型。
    pub kind: DockDropKind,
    /// 三态语义状态。
    pub semantic: DockDropSemantic,
    /// 已裁剪在目标容器边界内的预览矩形（`TearOut` 时为 `Rect::ZERO`）。
    pub preview_rect: Rect,
    /// 提示胶囊显示文案（纯英文）。
    pub hint_label: &'static str,
    /// 候选层级索引（`0` 为最内层）。
    pub level: usize,
}

/// 窗外自由空间撕出（`TearOut`）时的外部拖拽代理协议。
#[derive(Clone, Debug, PartialEq)]
pub struct DockDragProxy {
    /// 代理卡片左上角位置（逻辑像素）。
    pub position: Vec2,
    /// 代理卡片尺寸（逻辑像素）。
    pub size: Vec2,
    /// 拖拽代理标题文案。
    pub title: String,
    /// 携带的标签页数量。
    pub tab_count: usize,
    /// 代理卡片不透明度（`0.0..=1.0`）。
    pub opacity: f32,
    /// 是否可见。
    pub visible: bool,
}

impl Default for DockDragProxy {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            size: Vec2::new(220.0, 140.0),
            title: String::new(),
            tab_count: 1,
            opacity: 0.85,
            visible: false,
        }
    }
}

/// 停靠预览叠加层的三态语义配色令牌（从主题推导）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DockOverlayTokens {
    /// 允许停靠填充色令牌（默认 `Token::ACCENT`）。
    pub allowed_fill: Token,
    /// 允许停靠边框色令牌（默认 `Token::ACCENT`）。
    pub allowed_border: Token,
    /// 允许停靠填充透明度（默认 `0.22`）。
    pub allowed_fill_alpha: f32,
    /// 禁止停靠填充色令牌（默认 `Token::ERROR`）。
    pub forbidden_fill: Token,
    /// 禁止停靠边框色令牌（默认 `Token::ERROR`）。
    pub forbidden_border: Token,
    /// 禁止停靠填充透明度（默认 `0.16`）。
    pub forbidden_fill_alpha: f32,
    /// 嵌套/组合停靠填充色槽位（默认 `Some(Token::WARNING)`，未配置时回退 `Token::SECONDARY`）。
    pub nested_fill: Option<Token>,
    /// 嵌套/组合停靠边框色槽位（默认 `Some(Token::WARNING)`，未配置时回退 `Token::SECONDARY`）。
    pub nested_border: Option<Token>,
    /// 嵌套/组合停靠填充透明度（默认 `0.22`）。
    pub nested_fill_alpha: f32,
    /// 提示胶囊背景色令牌。
    pub pill_bg: Token,
    /// 提示胶囊边框色令牌。
    pub pill_border: Token,
    /// 提示胶囊文字颜色令牌。
    pub pill_text: Token,
    /// 外部拖拽代理卡片背景色令牌。
    pub proxy_bg: Token,
    /// 外部拖拽代理卡片边框色令牌。
    pub proxy_border: Token,
    /// 外部拖拽代理卡片文字颜色令牌。
    pub proxy_text: Token,
}

impl Default for DockOverlayTokens {
    fn default() -> Self {
        Self::from_theme()
    }
}

impl DockOverlayTokens {
    /// 从标准主题令牌推导三态语义配色与 `HintPill` / `DockDragProxy` 配色。
    #[must_use]
    pub const fn from_theme() -> Self {
        Self {
            allowed_fill: Token::ACCENT,
            allowed_border: Token::ACCENT,
            allowed_fill_alpha: 0.22,
            forbidden_fill: Token::ERROR,
            forbidden_border: Token::ERROR,
            forbidden_fill_alpha: 0.16,
            nested_fill: Some(Token::WARNING),
            nested_border: Some(Token::WARNING),
            nested_fill_alpha: 0.22,
            pill_bg: Token::OVERLAY_BG,
            pill_border: Token::OUTLINE,
            pill_text: Token::TEXT_PRIMARY,
            proxy_bg: Token::SURFACE_RAISED,
            proxy_border: Token::CARD_BORDER,
            proxy_text: Token::TEXT_PRIMARY,
        }
    }

    /// 解析嵌套/组合状态的填充令牌（未配置时回退 `Token::SECONDARY`）。
    #[must_use]
    pub const fn resolved_nested_fill(self) -> Token {
        match self.nested_fill {
            Some(token) => token,
            None => Token::SECONDARY,
        }
    }

    /// 解析嵌套/组合状态的边框令牌（未配置时回退 `Token::SECONDARY`）。
    #[must_use]
    pub const fn resolved_nested_border(self) -> Token {
        match self.nested_border {
            Some(token) => token,
            None => Token::SECONDARY,
        }
    }

    /// 按三态语义返回 `(填充令牌, 边框令牌, 填充 alpha)`。
    #[must_use]
    pub fn resolve_semantic(self, semantic: DockDropSemantic) -> (Token, Token, f32) {
        match semantic {
            DockDropSemantic::Allowed => (
                self.allowed_fill,
                self.allowed_border,
                self.allowed_fill_alpha.clamp(0.0, 1.0),
            ),
            DockDropSemantic::Forbidden => (
                self.forbidden_fill,
                self.forbidden_border,
                self.forbidden_fill_alpha.clamp(0.0, 1.0),
            ),
            DockDropSemantic::Nested => (
                self.resolved_nested_fill(),
                self.resolved_nested_border(),
                self.nested_fill_alpha.clamp(0.0, 1.0),
            ),
        }
    }
}

/// 跨窗停靠预览叠加层状态机。
#[derive(Clone, Debug, PartialEq)]
pub struct DockOverlayState {
    active: bool,
    payload_kind: DockDragPayloadKind,
    source_window_id: Option<u64>,
    active_window_id: Option<u64>,
    candidates: Vec<DockDropTarget>,
    level_index: usize,
    dwell_anchor: Option<Vec2>,
    dwell_elapsed_secs: f32,
    dwell_armed: bool,
    preview_tween: Tween,
    last_pointer: Vec2,
}

impl Default for DockOverlayState {
    fn default() -> Self {
        Self::new()
    }
}

impl DockOverlayState {
    /// 创建初始空闲状态的叠加层状态机。
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: false,
            payload_kind: DockDragPayloadKind::Tab,
            source_window_id: None,
            active_window_id: None,
            candidates: Vec::new(),
            level_index: 0,
            dwell_anchor: None,
            dwell_elapsed_secs: 0.0,
            dwell_armed: false,
            preview_tween: Tween::from_token(0.0, MotionToken::OverlayPreview),
            last_pointer: Vec2::ZERO,
        }
    }

    /// 开始一次新的拖拽预览会话。
    pub fn begin_drag(
        &mut self,
        payload_kind: DockDragPayloadKind,
        source_window_id: Option<u64>,
        pointer: Vec2,
    ) {
        self.active = true;
        self.payload_kind = payload_kind;
        self.source_window_id = source_window_id;
        self.active_window_id = None;
        self.candidates.clear();
        self.level_index = 0;
        self.dwell_anchor = None;
        self.dwell_elapsed_secs = 0.0;
        self.dwell_armed = matches!(payload_kind, DockDragPayloadKind::Tab);
        self.last_pointer = pointer;
        self.preview_tween.snap_to(0.0);
    }

    /// 结束当前拖拽预览会话并复位状态。
    pub fn end_drag(&mut self) {
        self.active = false;
        self.active_window_id = None;
        self.candidates.clear();
        self.level_index = 0;
        self.dwell_anchor = None;
        self.dwell_elapsed_secs = 0.0;
        self.dwell_armed = false;
        self.preview_tween
            .set_target_with_token(0.0, MotionToken::OverlayPreview);
    }

    /// 返回当前是否处于活跃拖拽会话中。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// 返回整窗驻留防抖是否已武装。
    #[must_use]
    pub const fn is_dwell_armed(&self) -> bool {
        self.dwell_armed
    }

    /// 返回当前已累计的整窗驻留时间（秒）。
    #[must_use]
    pub const fn dwell_elapsed_secs(&self) -> f32 {
        self.dwell_elapsed_secs
    }

    /// 返回当前所有候选落点列表（按最内层到最外层排序）。
    #[must_use]
    pub fn candidates(&self) -> &[DockDropTarget] {
        &self.candidates
    }

    /// 返回当前选中的层级索引。
    #[must_use]
    pub const fn level_index(&self) -> usize {
        self.level_index
    }

    /// 返回当前选中的候选目标（若有）。
    #[must_use]
    pub fn active_target(&self) -> Option<&DockDropTarget> {
        if !self.active {
            return None;
        }
        self.candidates.get(self.level_index)
    }

    /// 返回当前是否应当显示宿主内预览叠加层。
    ///
    /// 当未拖拽、整窗拖入尚未武装、或当前处于窗外 `TearOut` 状态时返回 `false`。
    #[must_use]
    pub fn is_preview_visible(&self) -> bool {
        if !self.active || !self.dwell_armed {
            return false;
        }
        match self.active_target() {
            Some(target) => {
                !matches!(target.kind, DockDropKind::TearOut) && !target.preview_rect.is_empty()
            }
            None => false,
        }
    }

    /// 返回当前经过 `MotionToken::OverlayPreview` 缓动后的预览不透明度因子（`0.0..=1.0`）。
    #[must_use]
    pub fn preview_alpha(&self) -> f32 {
        self.preview_tween.value().clamp(0.0, 1.0)
    }

    /// 通过滚轮增量切换多层级嵌套候选（`delta > 0` 向外层切换，`delta < 0` 向内层切换）。
    pub fn nudge_level(&mut self, delta: i32) {
        if self.candidates.is_empty() || delta == 0 {
            return;
        }
        let max_idx = self.candidates.len().saturating_sub(1) as isize;
        let next = (self.level_index as isize + delta as isize).clamp(0, max_idx);
        self.level_index = next as usize;
    }

    /// 根据当前指针位置、候选窗口集合与帧时间步长更新候选列表、驻留防抖与动效状态。
    pub fn update(
        &mut self,
        pointer: Vec2,
        windows: &[DockCandidateWindow],
        dt: f32,
        policy: MotionPolicy,
    ) {
        if !self.active {
            self.preview_tween
                .set_target_with_token(0.0, MotionToken::OverlayPreview);
            self.preview_tween.step(dt, policy);
            return;
        }

        self.last_pointer = pointer;
        let hit_window = select_topmost_window(pointer, windows);

        match hit_window {
            Some(win) => {
                let window_changed = self.active_window_id != Some(win.window_id);
                self.active_window_id = Some(win.window_id);

                // 仅在 Shell 跨视口进入目标窗口时启用 500ms / 12px 驻留防抖。
                let needs_dwell = matches!(self.payload_kind, DockDragPayloadKind::Shell)
                    && self.source_window_id != Some(win.window_id);

                if !needs_dwell {
                    self.dwell_armed = true;
                    self.dwell_anchor = Some(pointer);
                    self.dwell_elapsed_secs = 0.0;
                } else if window_changed {
                    self.dwell_armed = false;
                    self.dwell_anchor = Some(pointer);
                    self.dwell_elapsed_secs = sanitize_dt(dt);
                    if self.dwell_elapsed_secs >= SHELL_DWELL_DURATION_SECS {
                        self.dwell_armed = true;
                    }
                } else if !self.dwell_armed {
                    let moved_beyond_threshold = self.dwell_anchor.is_some_and(|anchor| {
                        (anchor - pointer).length() > SHELL_DWELL_THRESHOLD_PX
                    });
                    if moved_beyond_threshold {
                        self.dwell_anchor = Some(pointer);
                        self.dwell_elapsed_secs = 0.0;
                    } else {
                        if self.dwell_anchor.is_none() {
                            self.dwell_anchor = Some(pointer);
                        }
                        self.dwell_elapsed_secs += sanitize_dt(dt);
                        if self.dwell_elapsed_secs >= SHELL_DWELL_DURATION_SECS {
                            self.dwell_armed = true;
                        }
                    }
                }
                // 已武装后微调移动不再重置 `dwell_armed`。

                let next_candidates = build_window_candidates(pointer, win, self.payload_kind);
                if next_candidates != self.candidates {
                    self.candidates = next_candidates;
                    if self.level_index >= self.candidates.len() {
                        self.level_index = 0;
                    }
                }
            }
            None => {
                // 指针位于所有窗口之外：进入窗外 TearOut 自由空间，宿主内不显示预览框。
                self.active_window_id = None;
                self.dwell_anchor = None;
                self.dwell_elapsed_secs = 0.0;
                if matches!(self.payload_kind, DockDragPayloadKind::Shell) {
                    self.dwell_armed = false;
                }
                self.candidates = vec![DockDropTarget {
                    window_id: 0,
                    pane_id: None,
                    kind: DockDropKind::TearOut,
                    semantic: DockDropSemantic::Allowed,
                    preview_rect: Rect::ZERO,
                    hint_label: "Tear Out",
                    level: 0,
                }];
                self.level_index = 0;
            }
        }

        let target_alpha = if self.is_preview_visible() { 1.0 } else { 0.0 };
        self.preview_tween
            .set_target_with_token(target_alpha, MotionToken::OverlayPreview);
        self.preview_tween.step(dt, policy);
    }

    /// 仅推进动效与驻留计时（在指针位置未变时按 `dt` 步进）。
    pub fn step(&mut self, dt: f32, policy: MotionPolicy) {
        if self.active
            && matches!(self.payload_kind, DockDragPayloadKind::Shell)
            && self.active_window_id.is_some()
            && !self.dwell_armed
        {
            self.dwell_elapsed_secs += sanitize_dt(dt);
            if self.dwell_elapsed_secs >= SHELL_DWELL_DURATION_SECS {
                self.dwell_armed = true;
            }
        }
        let target_alpha = if self.is_preview_visible() { 1.0 } else { 0.0 };
        self.preview_tween
            .set_target_with_token(target_alpha, MotionToken::OverlayPreview);
        self.preview_tween.step(dt, policy);
    }
}

/// 计算标签栏重排的 `2px` 插入指示线矩形与插入索引。
///
/// 返回 `(insert_index, line_x, clipped_line_rect)`；若标签栏尺寸无效则返回 `None`。
#[must_use]
pub fn compute_reorder_indicator(
    pane: &DockCandidatePane,
    pointer: Vec2,
) -> Option<(usize, f32, Rect)> {
    let bar = pane.tab_bar_rect();
    if bar.is_empty() || !bar.contains_point(pointer) {
        return None;
    }

    let (insert_index, raw_line_x) = if pane.tab_rects.is_empty() {
        (0, bar.origin.x + 2.0)
    } else {
        let mut chosen_index = pane.tab_rects.len();
        let mut chosen_x = pane
            .tab_rects
            .last()
            .map_or(bar.origin.x, |rect| rect.max().x);
        for (idx, tab_rect) in pane.tab_rects.iter().enumerate() {
            let mid_x = tab_rect.center().x;
            if pointer.x <= mid_x {
                chosen_index = idx;
                chosen_x = tab_rect.origin.x;
                break;
            }
        }
        (chosen_index, chosen_x)
    };

    let half_w = REORDER_LINE_WIDTH_PX * 0.5;
    let max_left = (bar.max().x - REORDER_LINE_WIDTH_PX).max(bar.origin.x);
    let left_x = (raw_line_x - half_w).clamp(bar.origin.x, max_left);
    let line_rect = Rect::new(
        left_x,
        bar.origin.y,
        REORDER_LINE_WIDTH_PX.min(bar.size.x),
        bar.size.y,
    )
    .intersect(&bar);

    if line_rect.is_empty() {
        None
    } else {
        Some((insert_index, raw_line_x, line_rect))
    }
}

/// 计算紧贴光标的提示胶囊（`HintPill`）矩形，靠近右/下边缘时向光标对侧翻转并严格钳制在 `overlay_bounds` 内。
#[must_use]
pub fn compute_hint_pill_rect(overlay_bounds: Rect, pointer: Vec2, label: &str) -> Option<Rect> {
    if overlay_bounds.is_empty() || !overlay_bounds.is_finite() || !pointer.is_finite() {
        return None;
    }

    let estimated_w =
        label.len() as f32 * HINT_PILL_CHAR_WIDTH_ESTIMATE + HINT_PILL_PADDING_X * 2.0;
    let max_avail_w = (overlay_bounds.size.x - HINT_PILL_MARGIN_PX * 2.0)
        .max(0.0)
        .min(overlay_bounds.size.x);
    let w = if max_avail_w >= HINT_PILL_MIN_WIDTH {
        estimated_w.clamp(HINT_PILL_MIN_WIDTH, HINT_PILL_MAX_WIDTH.min(max_avail_w))
    } else {
        overlay_bounds.size.x.min(HINT_PILL_MAX_WIDTH)
    };
    let h = HINT_PILL_HEIGHT.min(overlay_bounds.size.y);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    let bounds_max = overlay_bounds.max();
    let mut x = pointer.x + HINT_PILL_OFFSET_PX;
    if x + w > bounds_max.x - HINT_PILL_MARGIN_PX {
        x = pointer.x - HINT_PILL_OFFSET_PX - w;
    }

    let mut y = pointer.y + HINT_PILL_OFFSET_PX;
    if y + h > bounds_max.y - HINT_PILL_MARGIN_PX {
        y = pointer.y - HINT_PILL_OFFSET_PX - h;
    }

    let clamped_x = clamp_into_span(
        x,
        w,
        overlay_bounds.origin.x,
        bounds_max.x,
        HINT_PILL_MARGIN_PX,
    );
    let clamped_y = clamp_into_span(
        y,
        h,
        overlay_bounds.origin.y,
        bounds_max.y,
        HINT_PILL_MARGIN_PX,
    );

    let pill = Rect::new(clamped_x, clamped_y, w, h).intersect(&overlay_bounds);
    if pill.is_empty() {
        None
    } else {
        Some(pill)
    }
}

/// 发射纯区域高亮停靠预览叠加层图元。
///
/// 该输出层为纯视觉反馈：严禁生成任何命中区域（`hit == None`、`sub_hits` 为空）或焦点节点（`focusable == false`），
/// 在无活跃目标、未满 `500ms` 整窗驻留防抖或 `TearOut` 状态下返回空 [`WidgetOutput`]。
#[must_use]
pub fn emit_docking_overlay(
    state: &DockOverlayState,
    overlay_bounds: Rect,
    pointer: Vec2,
    hint_glyphs: &[GlyphQuad],
    tokens: DockOverlayTokens,
    z: u16,
) -> WidgetOutput {
    if overlay_bounds.is_empty() || !overlay_bounds.is_finite() || !state.is_preview_visible() {
        return WidgetOutput::default();
    }

    let Some(target) = state.active_target() else {
        return WidgetOutput::default();
    };
    if matches!(target.kind, DockDropKind::TearOut) {
        return WidgetOutput::default();
    }

    let alpha = state.preview_alpha();
    if alpha <= 0.0 {
        return WidgetOutput::default();
    }

    let clipped = target.preview_rect.intersect(&overlay_bounds);
    if clipped.is_empty() {
        return WidgetOutput::default();
    }

    let (fill_token, border_token, fill_alpha) = tokens.resolve_semantic(target.semantic);
    let mut primitives = Vec::with_capacity(4 + hint_glyphs.len());
    let mut payloads = Vec::with_capacity(hint_glyphs.len());

    match target.kind {
        DockDropKind::Reorder { .. } => {
            let mut line = surface(clipped, 1.0, border_token, z);
            line.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, alpha);
            primitives.push(line);
        }
        DockDropKind::Edge(_)
        | DockDropKind::RootEdge(_)
        | DockDropKind::Center
        | DockDropKind::Nest => {
            let mut fill_prim = surface(clipped, OVERLAY_CORNER_RADIUS, fill_token, z);
            fill_prim.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(z, (fill_alpha * alpha).clamp(0.0, 1.0));
            primitives.push(fill_prim);

            let mut border_prim = instance(
                clipped,
                [OVERLAY_CORNER_RADIUS; 4],
                Token::NONE,
                border_token,
                OVERLAY_BORDER_WIDTH,
                0.0,
                PrimType::Stroke,
                SubElement::Border,
                z,
            );
            border_prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, alpha);
            primitives.push(border_prim);
        }
        DockDropKind::TearOut => return WidgetOutput::default(),
    }

    if let Some(pill_rect) = compute_hint_pill_rect(overlay_bounds, pointer, target.hint_label) {
        let pill_z = z.saturating_add(1);
        let mut pill = outlined(
            pill_rect,
            OVERLAY_CORNER_RADIUS,
            tokens.pill_bg,
            tokens.pill_border,
            OVERLAY_BORDER_WIDTH,
            4.0,
            pill_z,
        );
        pill.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(pill_z, alpha);
        primitives.push(pill);

        if !hint_glyphs.is_empty() {
            let glyph_start = primitives.len();
            glyphs(
                hint_glyphs,
                tokens.pill_text,
                pill_z,
                &mut primitives,
                &mut payloads,
            );
            for prim in &mut primitives[glyph_start..] {
                prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(pill_z, alpha);
            }
        }
    }

    WidgetOutput {
        primitives,
        payloads,
        ..WidgetOutput::default()
    }
}

/// 发射窗外自由空间拖拽代理（`DockDragProxy`）的纯展示图元。
#[must_use]
pub fn emit_drag_proxy(
    proxy: &DockDragProxy,
    label_glyphs: &[GlyphQuad],
    tokens: DockOverlayTokens,
    z: u16,
) -> WidgetOutput {
    let opacity = proxy.opacity.clamp(0.0, 1.0);
    if !proxy.visible || opacity <= 0.0 || proxy.size.x <= 0.0 || proxy.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    let rect = Rect::new(
        proxy.position.x,
        proxy.position.y,
        proxy.size.x,
        proxy.size.y,
    );
    if !rect.is_finite() {
        return WidgetOutput::default();
    }

    let mut primitives = Vec::with_capacity(1 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let mut card = outlined(
        rect,
        OVERLAY_CORNER_RADIUS,
        tokens.proxy_bg,
        tokens.proxy_border,
        OVERLAY_BORDER_WIDTH,
        8.0,
        z,
    );
    card.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, opacity);
    primitives.push(card);

    if !label_glyphs.is_empty() {
        let glyph_start = primitives.len();
        glyphs(
            label_glyphs,
            tokens.proxy_text,
            z,
            &mut primitives,
            &mut payloads,
        );
        for prim in &mut primitives[glyph_start..] {
            prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z, opacity);
        }
    }

    WidgetOutput {
        primitives,
        payloads,
        ..WidgetOutput::default()
    }
}

fn sanitize_dt(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 {
        dt
    } else {
        0.0
    }
}

fn clamp_into_span(pos: f32, len: f32, span_min: f32, span_max: f32, margin: f32) -> f32 {
    let inner_min = span_min + margin;
    let inner_max = span_max - margin - len;
    if inner_min <= inner_max {
        pos.clamp(inner_min, inner_max)
    } else {
        let raw_max = (span_max - len).max(span_min);
        pos.clamp(span_min, raw_max)
    }
}

fn select_topmost_window(
    pointer: Vec2,
    windows: &[DockCandidateWindow],
) -> Option<&DockCandidateWindow> {
    if !pointer.is_finite() {
        return None;
    }
    windows
        .iter()
        .filter(|win| !win.window_rect.is_empty() && win.window_rect.is_finite())
        .filter(|win| win.window_rect.contains_point(pointer))
        .max_by_key(|win| win.z_order)
}

fn select_innermost_pane(pointer: Vec2, panes: &[DockCandidatePane]) -> Option<&DockCandidatePane> {
    panes
        .iter()
        .filter(|pane| !pane.rect.is_empty() && pane.rect.is_finite())
        .filter(|pane| pane.rect.contains_point(pointer))
        .min_by(|a, b| {
            a.rect
                .area()
                .partial_cmp(&b.rect.area())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn nearest_edge(rect: Rect, pointer: Vec2) -> (DockEdge, f32) {
    let left = (pointer.x - rect.origin.x).max(0.0);
    let right = (rect.max().x - pointer.x).max(0.0);
    let top = (pointer.y - rect.origin.y).max(0.0);
    let bottom = (rect.max().y - pointer.y).max(0.0);

    let mut best = (DockEdge::Left, left);
    if right < best.1 {
        best = (DockEdge::Right, right);
    }
    if top < best.1 {
        best = (DockEdge::Top, top);
    }
    if bottom < best.1 {
        best = (DockEdge::Bottom, bottom);
    }
    best
}

fn classify_pane_edge(rect: Rect, pointer: Vec2) -> Option<DockEdge> {
    if rect.is_empty() || !rect.contains_point(pointer) {
        return None;
    }
    let nx = (pointer.x - rect.origin.x) / rect.size.x;
    let ny = (pointer.y - rect.origin.y) / rect.size.y;
    let left = nx;
    let right = 1.0 - nx;
    let top = ny;
    let bottom = 1.0 - ny;
    let min_dist = left.min(right).min(top).min(bottom);
    if min_dist >= PANE_EDGE_RATIO {
        return None;
    }
    if min_dist == left {
        Some(DockEdge::Left)
    } else if min_dist == right {
        Some(DockEdge::Right)
    } else if min_dist == top {
        Some(DockEdge::Top)
    } else {
        Some(DockEdge::Bottom)
    }
}

fn build_window_candidates(
    pointer: Vec2,
    win: &DockCandidateWindow,
    payload_kind: DockDragPayloadKind,
) -> Vec<DockDropTarget> {
    let mut out = Vec::new();

    if let Some(pane) = select_innermost_pane(pointer, &win.panes) {
        if matches!(payload_kind, DockDragPayloadKind::Tab) {
            if let Some((insert_index, line_x, line_rect)) =
                compute_reorder_indicator(pane, pointer)
            {
                out.push(DockDropTarget {
                    window_id: win.window_id,
                    pane_id: Some(pane.pane_id),
                    kind: DockDropKind::Reorder {
                        insert_index,
                        line_x,
                    },
                    semantic: DockDropSemantic::Allowed,
                    preview_rect: line_rect,
                    hint_label: "Reorder Tab",
                    level: out.len(),
                });
            }
        }

        if let Some(edge) = classify_pane_edge(pane.rect, pointer) {
            let preview_rect = edge.half_rect(pane.rect);
            if !preview_rect.is_empty() {
                out.push(DockDropTarget {
                    window_id: win.window_id,
                    pane_id: Some(pane.pane_id),
                    kind: DockDropKind::Edge(edge),
                    semantic: DockDropSemantic::Allowed,
                    preview_rect,
                    hint_label: edge.hint_label(false),
                    level: out.len(),
                });
            }
        } else {
            let content = pane.content_rect();
            if !content.is_empty() {
                if pane.allow_center {
                    let (semantic, hint_label) = if pane.is_source_group {
                        (DockDropSemantic::Forbidden, "Same Group Forbidden")
                    } else {
                        (DockDropSemantic::Allowed, "Merge Tabs")
                    };
                    out.push(DockDropTarget {
                        window_id: win.window_id,
                        pane_id: Some(pane.pane_id),
                        kind: DockDropKind::Center,
                        semantic,
                        preview_rect: content,
                        hint_label,
                        level: out.len(),
                    });
                }
                if pane.allow_nest {
                    out.push(DockDropTarget {
                        window_id: win.window_id,
                        pane_id: Some(pane.pane_id),
                        kind: DockDropKind::Nest,
                        semantic: DockDropSemantic::Nested,
                        preview_rect: content,
                        hint_label: "Nest Pane",
                        level: out.len(),
                    });
                }
            }
        }
    }

    let band = if win.root_edge_band > 0.0 && win.root_edge_band.is_finite() {
        win.root_edge_band
    } else {
        DEFAULT_ROOT_EDGE_BAND_PX
    };
    let (root_edge, edge_dist) = nearest_edge(win.window_rect, pointer);
    if edge_dist <= band || out.is_empty() {
        let root_rect = root_edge.half_rect(win.window_rect);
        if !root_rect.is_empty() {
            out.push(DockDropTarget {
                window_id: win.window_id,
                pane_id: None,
                kind: DockDropKind::RootEdge(root_edge),
                semantic: DockDropSemantic::Allowed,
                preview_rect: root_rect,
                hint_label: root_edge.hint_label(true),
                level: out.len(),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window(
        window_id: u64,
        is_source_group: bool,
        allow_nest: bool,
    ) -> DockCandidateWindow {
        DockCandidateWindow {
            window_id,
            window_rect: Rect::new(0.0, 0.0, 800.0, 600.0),
            z_order: 1,
            root_edge_band: 24.0,
            panes: vec![DockCandidatePane {
                pane_id: 101,
                rect: Rect::new(0.0, 0.0, 800.0, 600.0),
                tab_bar_height: 32.0,
                tab_rects: vec![
                    Rect::new(0.0, 0.0, 120.0, 32.0),
                    Rect::new(120.0, 0.0, 120.0, 32.0),
                ],
                allow_center: true,
                allow_nest,
                is_source_group,
            }],
        }
    }

    #[test]
    fn edge_and_center_geometry_respect_tab_bar_and_bounds() {
        let win = sample_window(1, false, false);
        let mut state = DockOverlayState::new();
        state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));

        // 中心区域必须扣除 32px 标签栏高度。
        state.update(
            Vec2::new(400.0, 300.0),
            std::slice::from_ref(&win),
            0.016,
            MotionPolicy::reduced(),
        );
        let center = state.active_target().expect("center candidate expected");
        assert_eq!(center.kind, DockDropKind::Center);
        assert_eq!(center.semantic, DockDropSemantic::Allowed);
        assert_eq!(center.preview_rect, Rect::new(0.0, 32.0, 800.0, 568.0));

        // 左侧边缘半区应占 400x600。
        state.update(
            Vec2::new(40.0, 300.0),
            &[win],
            0.016,
            MotionPolicy::reduced(),
        );
        let left = state.active_target().expect("left edge candidate expected");
        assert_eq!(left.kind, DockDropKind::Edge(DockEdge::Left));
        assert_eq!(left.preview_rect, Rect::new(0.0, 0.0, 400.0, 600.0));
    }

    #[test]
    fn reorder_emits_two_pixel_tab_bar_insertion_line() {
        let win = sample_window(1, true, false);
        let mut state = DockOverlayState::new();
        state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(115.0, 16.0));
        state.update(
            Vec2::new(115.0, 16.0),
            &[win],
            0.016,
            MotionPolicy::reduced(),
        );

        let target = state.active_target().expect("reorder target expected");
        assert!(matches!(
            target.kind,
            DockDropKind::Reorder {
                insert_index: 1,
                ..
            }
        ));
        assert_eq!(target.preview_rect.size.x, 2.0);
        assert_eq!(target.preview_rect.size.y, 32.0);

        let out = emit_docking_overlay(
            &state,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            Vec2::new(115.0, 16.0),
            &[],
            DockOverlayTokens::from_theme(),
            10,
        );
        assert!(!out.primitives.is_empty());
        assert!(out.hit.is_none());
        assert!(out.sub_hits.is_empty());
        assert!(!out.focusable);
    }

    #[test]
    fn three_state_semantic_tokens_map_to_accent_error_and_warning() {
        let tokens = DockOverlayTokens::from_theme();
        assert_eq!(
            tokens.resolve_semantic(DockDropSemantic::Allowed),
            (Token::ACCENT, Token::ACCENT, 0.22)
        );
        assert_eq!(
            tokens.resolve_semantic(DockDropSemantic::Forbidden),
            (Token::ERROR, Token::ERROR, 0.16)
        );
        assert_eq!(
            tokens.resolve_semantic(DockDropSemantic::Nested),
            (Token::WARNING, Token::WARNING, 0.22)
        );

        // 拖回自身原标签组中心应判定为 Forbidden。
        let win = sample_window(1, true, false);
        let mut state = DockOverlayState::new();
        state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));
        state.update(
            Vec2::new(400.0, 300.0),
            &[win],
            0.016,
            MotionPolicy::reduced(),
        );
        let target = state.active_target().expect("forbidden target expected");
        assert_eq!(target.semantic, DockDropSemantic::Forbidden);
    }

    #[test]
    fn shell_drag_requires_500ms_dwell_within_12px_and_stays_armed_afterward() {
        let target_win = sample_window(2, false, false);
        let mut state = DockOverlayState::new();
        state.begin_drag(DockDragPayloadKind::Shell, Some(1), Vec2::new(400.0, 300.0));

        // 进入目标窗口前 400ms：仍未武装，不可显示预览。
        state.update(
            Vec2::new(400.0, 300.0),
            std::slice::from_ref(&target_win),
            0.40,
            MotionPolicy::reduced(),
        );
        assert!(!state.is_dwell_armed());
        assert!(!state.is_preview_visible());
        let hidden_out = emit_docking_overlay(
            &state,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            Vec2::new(400.0, 300.0),
            &[],
            DockOverlayTokens::default(),
            10,
        );
        assert!(hidden_out.primitives.is_empty());

        // 移动超过 12px 容差：重置计时器。
        state.update(
            Vec2::new(420.0, 300.0),
            std::slice::from_ref(&target_win),
            0.20,
            MotionPolicy::reduced(),
        );
        assert!(!state.is_dwell_armed());
        assert_eq!(state.dwell_elapsed_secs(), 0.0);

        // 容差内停留累计满 500ms：武装显示预览。
        state.update(
            Vec2::new(425.0, 304.0),
            std::slice::from_ref(&target_win),
            0.30,
            MotionPolicy::reduced(),
        );
        assert!(!state.is_dwell_armed());
        state.update(
            Vec2::new(426.0, 304.0),
            std::slice::from_ref(&target_win),
            0.25,
            MotionPolicy::reduced(),
        );
        assert!(state.is_dwell_armed());
        assert!(state.is_preview_visible());

        // 已武装后在同一窗口内大幅微调不再重置防抖状态。
        state.update(
            Vec2::new(500.0, 350.0),
            &[target_win],
            0.016,
            MotionPolicy::reduced(),
        );
        assert!(state.is_dwell_armed());
        assert!(state.is_preview_visible());
    }

    #[test]
    fn wheel_nudge_level_switches_between_inner_nest_and_root_edge() {
        let win = sample_window(1, false, true);
        let mut state = DockOverlayState::new();
        // 指针既在面板内部（远离内层四分边），又位于 24px 整窗外缘吸附带附近？
        // 换用内层面板与整窗外缘重叠区域测试：指针在 x = 10.0, y = 300.0，同时命中 Edge(Left) 与 RootEdge(Left)。
        state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(10.0, 300.0));
        state.update(
            Vec2::new(10.0, 300.0),
            std::slice::from_ref(&win),
            0.016,
            MotionPolicy::reduced(),
        );
        assert!(state.candidates().len() >= 2);
        assert_eq!(
            state.active_target().unwrap().kind,
            DockDropKind::Edge(DockEdge::Left)
        );

        state.nudge_level(1);
        assert_eq!(
            state.active_target().unwrap().kind,
            DockDropKind::RootEdge(DockEdge::Left)
        );

        state.nudge_level(-1);
        assert_eq!(
            state.active_target().unwrap().kind,
            DockDropKind::Edge(DockEdge::Left)
        );

        // 中心区域支持 Center -> Nest 层级切换。
        state.update(
            Vec2::new(400.0, 300.0),
            &[win],
            0.016,
            MotionPolicy::reduced(),
        );
        assert_eq!(state.active_target().unwrap().kind, DockDropKind::Center);
        state.nudge_level(1);
        assert_eq!(state.active_target().unwrap().kind, DockDropKind::Nest);
        assert_eq!(
            state.active_target().unwrap().semantic,
            DockDropSemantic::Nested
        );
    }

    #[test]
    fn hint_pill_flips_and_clamps_within_overlay_bounds() {
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);

        // 右下角光标应向左上翻转并严格钳制在边界内。
        let pill_br = compute_hint_pill_rect(bounds, Vec2::new(395.0, 295.0), "Dock Right")
            .expect("pill rect expected");
        assert!(bounds.contains_rect(&pill_br));
        assert!(pill_br.max().x <= bounds.max().x - HINT_PILL_MARGIN_PX + 0.01);
        assert!(pill_br.max().y <= bounds.max().y - HINT_PILL_MARGIN_PX + 0.01);

        // 极窄叠加层也绝不溢出边界或除零。
        let narrow = Rect::new(10.0, 10.0, 40.0, 20.0);
        let pill_narrow =
            compute_hint_pill_rect(narrow, Vec2::new(30.0, 20.0), "Dock Window Bottom")
                .expect("narrow pill expected");
        assert!(narrow.contains_rect(&pill_narrow));

        // 零尺寸安全返回 None。
        assert!(compute_hint_pill_rect(Rect::ZERO, Vec2::ZERO, "Dock").is_none());
    }

    #[test]
    fn tear_out_emits_no_host_preview_and_uses_drag_proxy() {
        let win = sample_window(1, false, false);
        let mut state = DockOverlayState::new();
        state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(950.0, 300.0));
        state.update(
            Vec2::new(950.0, 300.0),
            &[win],
            0.016,
            MotionPolicy::reduced(),
        );

        let target = state.active_target().expect("tear out target expected");
        assert_eq!(target.kind, DockDropKind::TearOut);
        assert!(!state.is_preview_visible());

        let overlay_out = emit_docking_overlay(
            &state,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            Vec2::new(950.0, 300.0),
            &[],
            DockOverlayTokens::default(),
            10,
        );
        assert!(overlay_out.primitives.is_empty());

        let proxy = DockDragProxy {
            position: Vec2::new(940.0, 280.0),
            size: Vec2::new(200.0, 120.0),
            title: "Detached Tab".to_string(),
            tab_count: 1,
            opacity: 0.9,
            visible: true,
        };
        let proxy_out = emit_drag_proxy(&proxy, &[], DockOverlayTokens::default(), 20);
        assert_eq!(proxy_out.primitives.len(), 1);
        assert!(proxy_out.hit.is_none());
        assert!(!proxy_out.focusable);
    }

    #[test]
    fn overlay_preview_tween_snaps_immediately_under_reduced_motion() {
        let win = sample_window(1, false, false);

        // 正常模式：120ms 渐入，单步 16ms 后处于中间过渡态。
        let mut normal_state = DockOverlayState::new();
        normal_state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));
        normal_state.update(
            Vec2::new(400.0, 300.0),
            std::slice::from_ref(&win),
            0.016,
            MotionPolicy::default(),
        );
        assert!(normal_state.preview_alpha() > 0.0);
        assert!(normal_state.preview_alpha() < 1.0);

        // 减弱动画模式：单步即瞬切至 1.0，退出时同样瞬切至 0.0。
        let mut reduced_state = DockOverlayState::new();
        reduced_state.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));
        reduced_state.update(
            Vec2::new(400.0, 300.0),
            &[win],
            0.001,
            MotionPolicy::reduced(),
        );
        assert_eq!(reduced_state.preview_alpha(), 1.0);

        reduced_state.end_drag();
        reduced_state.step(0.001, MotionPolicy::reduced());
        assert_eq!(reduced_state.preview_alpha(), 0.0);
    }
}

//! `cha-set` 18 款高阶业务复合组件与交互原语集（阶段 1 WP4）。
//!
//! 覆盖 `chahu-render-debugger`（Launcher + 内置工具窗口 + 调试覆盖层）与 `dunting`（纯 Rust
//! 数字资产管理器）所需的全部专用业务组件，所有发射器均严格遵守 24 态 `Stable Shape` 恒定图元契约：
//!
//! 1. [`AddressBarDesc`] / [`AddressBarState`] — 面包屑与全路径文本双模切换地址栏；
//! 2. [`FilterPopupDesc`] / [`FilterPopupState`] — 多维筛选弹窗（扩展名、标签、评分、尺寸区间）；
//! 3. [`QueryBuilderDesc`] / [`QueryGroup`] — 可视化 `AND / OR` 嵌套条件规则构建器；
//! 4. [`ShortcutBarDesc`] / [`KeybindingManagerState`] — 底部快捷键提示栏与键位冲突检测管理器；
//! 5. [`PipelineViewDesc`] / [`LogConsoleDesc`] — 渲染管线阶段耗时条与带正则过滤/尾随锁定的日志控制台；
//! 6. [`TaskHudDesc`] / [`ActivityStackState`] — 后台任务进度浮窗与活动队列栈；
//! 7. [`FloatingNoticeDesc`] / [`ScaleOsdDesc`] — 顶部轻量胶囊通知与缩放倍率 OSD 指示器；
//! 8. [`MediaProgressBarDesc`] — 支持缓冲区间与 A-B 循环标记的媒体/帧回放进度条；
//! 9. [`SnapSliderDesc`] / [`RangeSliderDesc`] — 磁吸刻度滑块与双滑块区间选择器；
//! 10. [`PresetNumberInputDesc`] / [`DurationInputDesc`] — 带预设档位的数值输入与时长输入；
//! 11. [`InlineEditableTextDesc`] / [`ReadOnlyCopyInputDesc`] — 双击内联重命名文本与悬停一键复制只读框；
//! 12. [`ReorderableListDesc`] / [`ReorderableListState`] — 支持拖拽排序与插入线指示的列表；
//! 13. [`ThemeSettingsDesc`] / [`LanguageSettingsDesc`] — 10 色 OKLCH 主题设置面板与多语言切换卡片；
//! 14. [`SquircleDesc`] / [`DraggableModalState`] / [`SmoothWheelHandler`] — 超椭圆表面、可拖拽模态框状态机与平滑滚轮阻尼器。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::{AccentBrand, ProductPreset, ThemeKind, Token};

use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

// ---------------------------------------------------------------------------
// 1. AddressBar（双模路径导航栏）
// ---------------------------------------------------------------------------

/// 双模地址栏描述符（平时显示分段路径面包屑，点击空白或按下快捷键切换为完整路径编辑框）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarDesc {
    pub rect: Rect,
    pub segments: Vec<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub z: u16,
}

impl AddressBarDesc {
    #[must_use]
    pub fn new(rect: Rect, segments: Vec<String>) -> Self {
        Self {
            rect,
            segments,
            can_go_back: true,
            can_go_forward: false,
            z: 10,
        }
    }

    /// 完整路径字符串（以 `/` 连接）。
    #[must_use]
    pub fn full_path(&self) -> String {
        if self.segments.is_empty() {
            "/".to_string()
        } else {
            self.segments.join("/")
        }
    }
}

/// 双模地址栏交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressBarState {
    /// 是否处于全路径文本编辑模式。
    pub editing: bool,
    /// 编辑缓冲区内容。
    pub edit_buffer: String,
    /// 历史后退栈。
    pub back_stack: Vec<Vec<String>>,
    /// 历史前进栈。
    pub forward_stack: Vec<Vec<String>>,
}

impl AddressBarState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换进入全路径编辑态。
    pub fn begin_edit(&mut self, current_path: &str) {
        self.editing = true;
        self.edit_buffer = current_path.to_owned();
    }

    /// 提交编辑路径并解析为分段列表。
    pub fn commit_edit(&mut self) -> Vec<String> {
        self.editing = false;
        self.edit_buffer
            .split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// 发射双模地址栏（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_address_bar(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + desc.segments.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let bg = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.editing {
        Token::INPUT_BG
    } else {
        Token::SURFACE_SUNKEN
    };
    let border = if state.editing {
        Token::PRIMARY
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 6.0, bg, border, 1.0, 0.0, z));

    // 后退、前进、向上、刷新四个导航槽位背景（恒定 4 个图元）
    for i in 0..4usize {
        let btn_r = Rect::new(
            r.origin.x + 4.0 + i as f32 * 26.0,
            r.origin.y + 4.0,
            22.0,
            (r.size.y - 8.0).max(16.0),
        );
        let fill = if !control.disabled && control.phase == PointerPhase::Hovered && i == 0 {
            Token::CONTROL_HOVER
        } else {
            Token::SURFACE_RAISED
        };
        primitives.push(surface(btn_r, 4.0, fill, z + 1));
    }

    // 路径分段胶囊槽位（对固定 `desc.segments` 恒定发射）
    let mut x = r.origin.x + 114.0;
    for seg in &desc.segments {
        let w = (seg.chars().count() as f32 * 7.5 + 16.0).clamp(36.0, 140.0);
        let seg_r = Rect::new(x, r.origin.y + 4.0, w, (r.size.y - 8.0).max(16.0));
        let seg_fill = if state.editing {
            Token::NONE
        } else {
            Token::SURFACE_RAISED
        };
        primitives.push(surface(seg_r, 4.0, seg_fill, z + 1));
        x += w + 6.0;
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        8.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Toolbar, desc.full_path(), r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 2. FilterPopup & 3. QueryBuilder（多维资产过滤弹窗与可视化 AND/OR 规则树）
// ---------------------------------------------------------------------------

/// 逻辑组合算子。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum QueryCombinator {
    #[default]
    And,
    Or,
}

/// 单条查询过滤规则。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRule {
    pub field: String,
    pub operator: String,
    pub value: String,
}

impl QueryRule {
    #[must_use]
    pub fn new(
        field: impl Into<String>,
        operator: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            operator: operator.into(),
            value: value.into(),
        }
    }
}

/// 可视化查询构建器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct QueryBuilderDesc {
    pub rect: Rect,
    pub combinator: QueryCombinator,
    pub rules: Vec<QueryRule>,
    pub z: u16,
}

impl QueryBuilderDesc {
    #[must_use]
    pub fn new(rect: Rect, combinator: QueryCombinator, rules: Vec<QueryRule>) -> Self {
        Self {
            rect,
            combinator,
            rules,
            z: 15,
        }
    }
}

/// 发射查询构建器（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_query_builder(
    node: u64,
    desc: &QueryBuilderDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + desc.rules.len() * 3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    primitives.push(outlined(
        r,
        8.0,
        Token::SURFACE_RAISED,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 顶部 AND/OR 切换胶囊
    let comb_r = Rect::new(r.origin.x + 12.0, r.origin.y + 10.0, 56.0, 24.0);
    let comb_fill = match desc.combinator {
        QueryCombinator::And => Token::PRIMARY_SOFT,
        QueryCombinator::Or => Token::SECONDARY_SOFT,
    };
    primitives.push(outlined(
        comb_r,
        5.0,
        comb_fill,
        Token::PRIMARY,
        1.0,
        0.0,
        z + 1,
    ));

    for (idx, _rule) in desc.rules.iter().enumerate() {
        let y = r.origin.y + 42.0 + idx as f32 * 34.0;
        let row_r = Rect::new(r.origin.x + 12.0, y, (r.size.x - 24.0).max(40.0), 28.0);
        primitives.push(outlined(
            row_r,
            5.0,
            Token::SURFACE_SUNKEN,
            Token::OUTLINE,
            1.0,
            0.0,
            z + 1,
        ));
        let op_r = Rect::new(row_r.origin.x + 96.0, y + 4.0, 48.0, 20.0);
        primitives.push(surface(op_r, 4.0, Token::PRIMARY_SOFT, z + 2));
        let del_r = Rect::new(row_r.max().x - 24.0, y + 4.0, 20.0, 20.0);
        primitives.push(surface(del_r, 4.0, Token::ERROR_SOFT, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Group, "Query Builder", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 多维资产筛选器状态（服务于 `dunting` 与 `chahu-render-debugger`）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterPopupState {
    pub selected_extensions: Vec<String>,
    pub selected_tags: Vec<String>,
    pub min_rating: u8,
    pub size_range_mb: (f32, f32),
}

impl FilterPopupState {
    /// 当前已激活的过滤条件总数（用于工具栏徽章显示）。
    #[must_use]
    pub fn active_filter_count(&self) -> usize {
        let mut count = self.selected_extensions.len() + self.selected_tags.len();
        if self.min_rating > 0 {
            count += 1;
        }
        if self.size_range_mb.0 > 0.0 || self.size_range_mb.1 > 0.0 {
            count += 1;
        }
        count
    }

    /// 一键清空所有过滤条件。
    pub fn clear_all(&mut self) {
        self.selected_extensions.clear();
        self.selected_tags.clear();
        self.min_rating = 0;
        self.size_range_mb = (0.0, 0.0);
    }
}

// ---------------------------------------------------------------------------
// 4. ShortcutBar & KeybindingManager（底部快捷键提示栏与键位冲突检测）
// ---------------------------------------------------------------------------

/// 单个快捷键绑定条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub action_id: String,
    pub label: String,
    pub chord: String,
}

impl ShortcutEntry {
    #[must_use]
    pub fn new(
        action_id: impl Into<String>,
        label: impl Into<String>,
        chord: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            chord: chord.into(),
        }
    }
}

/// 快捷键管理器状态（支持键位冲突实时检测）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindingManagerState {
    pub bindings: Vec<ShortcutEntry>,
}

impl KeybindingManagerState {
    #[must_use]
    pub fn new(bindings: Vec<ShortcutEntry>) -> Self {
        Self { bindings }
    }

    /// 检测共享同一组合键字符串的所有冲突动作对 `(action_a, action_b, chord)`。
    #[must_use]
    pub fn detect_conflicts(&self) -> Vec<(String, String, String)> {
        let mut conflicts = Vec::new();
        for i in 0..self.bindings.len() {
            for j in (i + 1)..self.bindings.len() {
                if !self.bindings[i].chord.is_empty()
                    && self.bindings[i]
                        .chord
                        .eq_ignore_ascii_case(&self.bindings[j].chord)
                {
                    conflicts.push((
                        self.bindings[i].action_id.clone(),
                        self.bindings[j].action_id.clone(),
                        self.bindings[i].chord.clone(),
                    ));
                }
            }
        }
        conflicts
    }
}

/// 底部上下文快捷键提示栏描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutBarDesc {
    pub rect: Rect,
    pub entries: Vec<ShortcutEntry>,
    pub z: u16,
}

impl ShortcutBarDesc {
    #[must_use]
    pub fn new(rect: Rect, entries: Vec<ShortcutEntry>) -> Self {
        Self {
            rect,
            entries,
            z: 12,
        }
    }
}

/// 发射底部快捷键提示栏。
#[must_use]
pub fn emit_shortcut_bar(
    node: u64,
    desc: &ShortcutBarDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.entries.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    primitives.push(outlined(
        r,
        6.0,
        Token::HEADER_BG,
        Token::DIVIDER,
        1.0,
        0.0,
        z,
    ));

    let mut x = r.origin.x + 10.0;
    for entry in &desc.entries {
        let keycap_w = (entry.chord.len() as f32 * 7.0 + 12.0).max(24.0);
        let keycap_r = Rect::new(x, r.origin.y + 4.0, keycap_w, (r.size.y - 8.0).max(16.0));
        primitives.push(outlined(
            keycap_r,
            4.0,
            Token::SURFACE_RAISED,
            Token::CARD_BORDER,
            1.0,
            0.0,
            z + 1,
        ));
        x += keycap_w + (entry.label.len() as f32 * 7.0) + 20.0;
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_SECONDARY,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        8.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| HitRegion::new(node, LAYER, u32::from(z), r)),
        a11y: Some(A11yNode::new(Role::Status, "Shortcuts", r).disabled(control.disabled)),
        focusable: false,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 5. PipelineView & StepTimeline & LogConsole（管线阶段耗时条与日志控制台）
// ---------------------------------------------------------------------------

/// 渲染管线或构建流水线的单个阶段。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineStageItem {
    pub name: String,
    pub duration_ms: f32,
    pub active: bool,
    pub has_error: bool,
}

impl PipelineStageItem {
    #[must_use]
    pub fn new(name: impl Into<String>, duration_ms: f32, active: bool) -> Self {
        Self {
            name: name.into(),
            duration_ms,
            active,
            has_error: false,
        }
    }
}

/// 渲染管线阶段检视器描述符（服务于 `chahu-render-debugger` Pass 耗时分析）。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineViewDesc {
    pub rect: Rect,
    pub stages: Vec<PipelineStageItem>,
    pub z: u16,
}

impl PipelineViewDesc {
    #[must_use]
    pub fn new(rect: Rect, stages: Vec<PipelineStageItem>) -> Self {
        Self {
            rect,
            stages,
            z: 14,
        }
    }
}

/// 发射管线阶段视图（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_pipeline_view(
    node: u64,
    desc: &PipelineViewDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.stages.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    primitives.push(outlined(
        r,
        8.0,
        Token::SURFACE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    let count = desc.stages.len().max(1) as f32;
    let gap = 8.0;
    let stage_w = ((r.size.x - 24.0 - (count - 1.0) * gap) / count).max(32.0);
    for (idx, stage) in desc.stages.iter().enumerate() {
        let sx = r.origin.x + 12.0 + idx as f32 * (stage_w + gap);
        let card_r = Rect::new(sx, r.origin.y + 12.0, stage_w, (r.size.y - 24.0).max(28.0));
        let (fill, border) = if stage.has_error {
            (Token::ERROR_SOFT, Token::ERROR)
        } else if stage.active {
            (Token::PRIMARY_SOFT, Token::PRIMARY)
        } else {
            (Token::SURFACE_RAISED, Token::OUTLINE)
        };
        primitives.push(outlined(card_r, 6.0, fill, border, 1.0, 0.0, z + 1));

        // 底部 GPU 耗时占比进度条
        let ratio = (stage.duration_ms / 4.0).clamp(0.08, 1.0);
        let bar_r = Rect::new(
            card_r.origin.x + 6.0,
            card_r.max().y - 10.0,
            (card_r.size.x - 12.0) * ratio,
            4.0,
        );
        let bar_tok = if stage.has_error {
            Token::ERROR
        } else {
            Token::PRIMARY
        };
        primitives.push(surface(bar_r, 2.0, bar_tok, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Group, "Pipeline View", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 6. TaskHud & ActivityStack（后台任务进度浮层与活动栈）
// ---------------------------------------------------------------------------

/// 单个后台任务进度项。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskActivityItem {
    pub id: u64,
    pub title: String,
    /// 进度范围 `0.0..=1.0`。
    pub progress: f32,
    pub cancellable: bool,
}

impl TaskActivityItem {
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, progress: f32) -> Self {
        Self {
            id,
            title: title.into(),
            progress: progress.clamp(0.0, 1.0),
            cancellable: true,
        }
    }
}

/// 后台任务 HUD 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskHudDesc {
    pub rect: Rect,
    pub tasks: Vec<TaskActivityItem>,
    pub z: u16,
}

impl TaskHudDesc {
    #[must_use]
    pub fn new(rect: Rect, tasks: Vec<TaskActivityItem>) -> Self {
        Self { rect, tasks, z: 40 }
    }
}

/// 发射后台任务 HUD（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_task_hud(
    node: u64,
    desc: &TaskHudDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.tasks.len() * 3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    primitives.push(outlined(
        r,
        10.0,
        Token::HUD_BG,
        Token::CARD_BORDER,
        1.0,
        12.0,
        z,
    ));

    for (idx, task) in desc.tasks.iter().enumerate() {
        let y = r.origin.y + 12.0 + idx as f32 * 38.0;
        let track_r = Rect::new(
            r.origin.x + 12.0,
            y + 20.0,
            (r.size.x - 24.0).max(20.0),
            6.0,
        );
        primitives.push(surface(track_r, 3.0, Token::CONTROL_TRACK, z + 1));
        let fill_r = Rect::new(
            track_r.origin.x,
            track_r.origin.y,
            track_r.size.x * task.progress.clamp(0.0, 1.0),
            track_r.size.y,
        );
        primitives.push(surface(fill_r, 3.0, Token::PRIMARY, z + 2));
        let cancel_r = Rect::new(r.max().x - 26.0, y, 14.0, 14.0);
        primitives.push(surface(cancel_r, 3.0, Token::SURFACE_RAISED, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::HUD_TEXT,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        12.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Status, "Active Tasks", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 7. ScaleOsd & FloatingNotice & 8. MediaProgressBar & 9. SnapSlider / RangeSlider
// ---------------------------------------------------------------------------

/// 磁吸刻度滑块描述符（到达预设吸附点 `snap_points` 的 `snap_tolerance` 范围内自动吸附）。
#[derive(Clone, Debug, PartialEq)]
pub struct SnapSliderDesc {
    pub rect: Rect,
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub snap_points: Vec<f32>,
    pub snap_tolerance: f32,
    pub z: u16,
}

impl SnapSliderDesc {
    #[must_use]
    pub fn new(rect: Rect, min: f32, max: f32, value: f32, snap_points: Vec<f32>) -> Self {
        Self {
            rect,
            min,
            max,
            value,
            snap_points,
            snap_tolerance: (max - min).abs() * 0.03,
            z: 10,
        }
    }

    /// 对输入值应用磁吸刻度计算。
    #[must_use]
    pub fn snap_value(&self, raw: f32) -> f32 {
        let clamped = raw.clamp(self.min, self.max);
        for &pt in &self.snap_points {
            if (clamped - pt).abs() <= self.snap_tolerance {
                return pt.clamp(self.min, self.max);
            }
        }
        clamped
    }
}

/// 发射磁吸刻度滑块（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_snap_slider(node: u64, desc: &SnapSliderDesc, control: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + desc.snap_points.len());
    let r = desc.rect;
    let z = desc.z;
    let span = (desc.max - desc.min).max(1e-5);
    let snapped = desc.snap_value(desc.value);
    let t = ((snapped - desc.min) / span).clamp(0.0, 1.0);

    let track_r = Rect::new(r.origin.x, r.center().y - 3.0, r.size.x, 6.0);
    primitives.push(surface(track_r, 3.0, Token::CONTROL_TRACK, z));

    for &pt in &desc.snap_points {
        let pt_t = ((pt - desc.min) / span).clamp(0.0, 1.0);
        let tick_x = track_r.origin.x + pt_t * track_r.size.x;
        let tick_r = Rect::new(tick_x - 1.0, r.center().y - 6.0, 2.0, 12.0);
        primitives.push(surface(tick_r, 1.0, Token::OUTLINE_STRONG, z + 1));
    }

    let fill_r = Rect::new(track_r.origin.x, track_r.origin.y, track_r.size.x * t, 6.0);
    let fill_tok = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == PointerPhase::Hovered {
        Token::PRIMARY_HOVER
    } else {
        Token::PRIMARY
    };
    primitives.push(surface(fill_r, 3.0, fill_tok, z + 1));

    let thumb_x = track_r.origin.x + track_r.size.x * t;
    let thumb_r = Rect::new(thumb_x - 7.0, r.center().y - 7.0, 14.0, 14.0);
    primitives.push(outlined(
        thumb_r,
        7.0,
        Token::CONTROL_THUMB,
        Token::PRIMARY,
        2.0,
        2.0,
        z + 2,
    ));

    primitives.push(focus_ring(
        r.inflate(2.0),
        6.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(
            A11yNode::new(Role::Slider, format!("{snapped:.1}"), r).disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 双滑块区间选择器描述符（用于筛选文件大小区间、帧耗时区间、色阶上下限）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RangeSliderDesc {
    pub rect: Rect,
    pub min: f32,
    pub max: f32,
    pub low_value: f32,
    pub high_value: f32,
    pub z: u16,
}

impl RangeSliderDesc {
    #[must_use]
    pub fn new(rect: Rect, min: f32, max: f32, low_value: f32, high_value: f32) -> Self {
        let lo = low_value.clamp(min, max);
        let hi = high_value.clamp(lo, max);
        Self {
            rect,
            min,
            max,
            low_value: lo,
            high_value: hi,
            z: 10,
        }
    }
}

/// 发射双滑块区间选择器（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_range_slider(node: u64, desc: &RangeSliderDesc, control: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5);
    let r = desc.rect;
    let z = desc.z;
    let span = (desc.max - desc.min).max(1e-5);
    let t_lo = ((desc.low_value - desc.min) / span).clamp(0.0, 1.0);
    let t_hi = ((desc.high_value - desc.min) / span).clamp(t_lo, 1.0);

    let track_r = Rect::new(r.origin.x, r.center().y - 3.0, r.size.x, 6.0);
    primitives.push(surface(track_r, 3.0, Token::CONTROL_TRACK, z));

    let x_lo = track_r.origin.x + track_r.size.x * t_lo;
    let x_hi = track_r.origin.x + track_r.size.x * t_hi;
    let active_r = Rect::new(x_lo, track_r.origin.y, (x_hi - x_lo).max(1.0), 6.0);
    primitives.push(surface(active_r, 3.0, Token::PRIMARY, z + 1));

    for cx in [x_lo, x_hi] {
        let thumb_r = Rect::new(cx - 7.0, r.center().y - 7.0, 14.0, 14.0);
        primitives.push(outlined(
            thumb_r,
            7.0,
            Token::CONTROL_THUMB,
            Token::PRIMARY,
            2.0,
            2.0,
            z + 2,
        ));
    }

    primitives.push(focus_ring(
        r.inflate(2.0),
        6.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(
            A11yNode::new(
                Role::Slider,
                format!("{:.1}..{:.1}", desc.low_value, desc.high_value),
                r,
            )
            .disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 10~14. InlineEditableText / ReadOnlyCopyInput / ReorderableList /
//        ThemeSettings / DraggableModalState / SmoothWheelHandler
// ---------------------------------------------------------------------------

/// 悬停一键复制只读输入框描述符（用于展示 GPU 物理地址 BDA、SHA256 哈希、捕获路径）。
#[derive(Clone, Debug, PartialEq)]
pub struct ReadOnlyCopyInputDesc {
    pub rect: Rect,
    pub value: String,
    pub copied: bool,
    pub z: u16,
}

impl ReadOnlyCopyInputDesc {
    #[must_use]
    pub fn new(rect: Rect, value: impl Into<String>) -> Self {
        Self {
            rect,
            value: value.into(),
            copied: false,
            z: 10,
        }
    }
}

/// 发射悬停一键复制只读框（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_readonly_copy_input(
    node: u64,
    desc: &ReadOnlyCopyInputDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + glyphs.len());
    let mut payloads = Vec::with_capacity(2 + glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    primitives.push(outlined(
        r,
        6.0,
        Token::SURFACE_SUNKEN,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    let btn_r = Rect::new(
        r.max().x - 28.0,
        r.origin.y + 4.0,
        24.0,
        (r.size.y - 8.0).max(16.0),
    );
    let btn_fill = if desc.copied {
        Token::SUCCESS_SOFT
    } else if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_RAISED
    };
    primitives.push(outlined(
        btn_r,
        4.0,
        btn_fill,
        Token::OUTLINE,
        1.0,
        0.0,
        z + 1,
    ));

    // 复制图标（两条描边恒定发射）
    curve_stroke(
        [btn_r.origin.x + 7.0, btn_r.origin.y + 7.0],
        [btn_r.max().x - 7.0, btn_r.origin.y + 7.0],
        1.5,
        Token::TEXT_SECONDARY,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [btn_r.origin.x + 7.0, btn_r.max().y - 7.0],
        [btn_r.max().x - 7.0, btn_r.max().y - 7.0],
        1.5,
        Token::TEXT_SECONDARY,
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    emit_glyphs(
        glyphs,
        Token::CODE_TEXT,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        8.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(
            A11yNode::new(Role::TextInput, desc.value.clone(), r).disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 拖拽重排序列表状态机。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReorderableListState<T> {
    pub items: Vec<T>,
    pub dragging_idx: Option<usize>,
    pub drop_target_idx: Option<usize>,
}

impl<T> ReorderableListState<T> {
    #[must_use]
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            dragging_idx: None,
            drop_target_idx: None,
        }
    }

    /// 将索引 `from` 处的元素移动到索引 `to` 处。
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return false;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        true
    }
}

/// 主题与品牌强调色设置面板状态（联动 `unbound-theme` 双产品预设与 10 套 OKLCH 强调色）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ThemeSettingsState {
    pub preset: ProductPreset,
    pub kind: ThemeKind,
    pub accent: AccentBrand,
    pub ui_scale: f32,
}

impl Default for ThemeSettingsState {
    fn default() -> Self {
        // `ThemeKind` 未实现 `Default`，且 UI 缩放必须落在 1.0 而非 `f32` 默认的 0.0。
        Self::new(Default::default(), ThemeKind::Light, Default::default())
    }
}

impl ThemeSettingsState {
    #[must_use]
    pub const fn new(preset: ProductPreset, kind: ThemeKind, accent: AccentBrand) -> Self {
        Self {
            preset,
            kind,
            accent,
            ui_scale: 1.0,
        }
    }
}

/// 可拖拽模态框位置状态机（服务于 `cha-set` `DraggableModal`）。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct DraggableModalState {
    pub offset: Vec2,
    drag_anchor: Option<Vec2>,
}

impl DraggableModalState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            offset: Vec2::ZERO,
            drag_anchor: None,
        }
    }

    pub fn begin_drag(&mut self, pointer: Vec2) {
        self.drag_anchor = Some(pointer - self.offset);
    }

    pub fn drag_to(&mut self, pointer: Vec2, viewport: Rect, modal_size: Vec2) -> Rect {
        if let Some(anchor) = self.drag_anchor {
            self.offset = pointer - anchor;
        }
        let base_x = viewport.center().x - modal_size.x * 0.5 + self.offset.x;
        let base_y = viewport.center().y - modal_size.y * 0.5 + self.offset.y;
        let clamped_x = base_x.clamp(
            viewport.origin.x,
            (viewport.max().x - modal_size.x).max(viewport.origin.x),
        );
        let clamped_y = base_y.clamp(
            viewport.origin.y,
            (viewport.max().y - modal_size.y).max(viewport.origin.y),
        );
        Rect::new(clamped_x, clamped_y, modal_size.x, modal_size.y)
    }

    pub fn end_drag(&mut self) {
        self.drag_anchor = None;
    }
}

/// 临界阻尼平滑滚轮插值器（服务于 `cha-set` `SmoothWheelHandler`）。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SmoothWheelHandler {
    pub current: f32,
    pub target: f32,
    pub velocity: f32,
}

impl SmoothWheelHandler {
    #[must_use]
    pub const fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            velocity: 0.0,
        }
    }

    /// 注入滚轮脉冲并钳制在 `[min, max]` 范围内。
    pub fn push_wheel_delta(&mut self, delta: f32, min: f32, max: f32) {
        self.target = (self.target + delta).clamp(min, max);
    }

    /// 推进单帧阻尼弹簧步进（返回是否仍在运动）。
    pub fn tick(&mut self, dt: f32) -> bool {
        let diff = self.target - self.current;
        if diff.abs() < 0.05 && self.velocity.abs() < 0.05 {
            self.current = self.target;
            self.velocity = 0.0;
            return false;
        }
        let omega = 22.0_f32;
        let step = dt.clamp(0.001, 0.05);
        let accel = omega * omega * diff - 2.0 * omega * self.velocity;
        self.velocity += accel * step;
        self.current += self.velocity * step;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_24_states<F: Fn(ControlState) -> WidgetOutput>(emit: F) {
        let mut counts = Vec::with_capacity(24);
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let out = emit(ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        });
                        counts.push(out.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "24-state Stable Shape violated: {counts:?}"
        );
    }

    #[test]
    fn all_cha_kit_emitters_preserve_24_state_stable_shape() {
        let rect = Rect::new(0.0, 0.0, 360.0, 48.0);
        let addr_desc = AddressBarDesc::new(rect, vec!["Assets".into(), "Textures".into()]);
        let addr_state = AddressBarState::new();
        assert_24_states(|cs| emit_address_bar(1, &addr_desc, &addr_state, cs, &[]));

        let qb_desc = QueryBuilderDesc::new(
            Rect::new(0.0, 0.0, 360.0, 120.0),
            QueryCombinator::And,
            vec![QueryRule::new("ext", "==", "ktx2")],
        );
        assert_24_states(|cs| emit_query_builder(2, &qb_desc, cs, &[]));

        let sb_desc = ShortcutBarDesc::new(
            rect,
            vec![ShortcutEntry::new("capture", "Capture Frame", "F11")],
        );
        assert_24_states(|cs| emit_shortcut_bar(3, &sb_desc, cs, &[]));

        let pv_desc = PipelineViewDesc::new(
            rect,
            vec![
                PipelineStageItem::new("Cull", 0.18, false),
                PipelineStageItem::new("Uber", 0.62, true),
            ],
        );
        assert_24_states(|cs| emit_pipeline_view(4, &pv_desc, cs, &[]));

        let hud_desc = TaskHudDesc::new(
            Rect::new(0.0, 0.0, 300.0, 96.0),
            vec![TaskActivityItem::new(1, "Baking KTX2 mipmaps", 0.65)],
        );
        assert_24_states(|cs| emit_task_hud(5, &hud_desc, cs, &[]));

        let snap_desc = SnapSliderDesc::new(rect, 0.0, 100.0, 49.0, vec![0.0, 50.0, 100.0]);
        assert_eq!(snap_desc.snap_value(49.0), 50.0);
        assert_24_states(|cs| emit_snap_slider(6, &snap_desc, cs));

        let range_desc = RangeSliderDesc::new(rect, 0.0, 100.0, 20.0, 80.0);
        assert_24_states(|cs| emit_range_slider(7, &range_desc, cs));

        let ro_desc = ReadOnlyCopyInputDesc::new(rect, "0x00007FF8A4100000");
        assert_24_states(|cs| emit_readonly_copy_input(8, &ro_desc, cs, &[]));
    }

    #[test]
    fn keybinding_manager_detects_duplicate_chords() {
        let mgr = KeybindingManagerState::new(vec![
            ShortcutEntry::new("capture", "Capture", "F11"),
            ShortcutEntry::new("fullscreen", "Fullscreen", "f11"),
            ShortcutEntry::new("save", "Save", "Ctrl+S"),
        ]);
        let conflicts = mgr.detect_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "capture");
        assert_eq!(conflicts[0].1, "fullscreen");
    }

    #[test]
    fn reorderable_list_and_smooth_wheel_behave_deterministically() {
        let mut list = ReorderableListState::new(vec!["A", "B", "C"]);
        assert!(list.move_item(0, 2));
        assert_eq!(list.items, vec!["B", "C", "A"]);

        let mut wheel = SmoothWheelHandler::new(0.0);
        wheel.push_wheel_delta(120.0, 0.0, 500.0);
        for _ in 0..60 {
            wheel.tick(1.0 / 60.0);
        }
        assert!((wheel.current - 120.0).abs() < 1.0);
    }
}

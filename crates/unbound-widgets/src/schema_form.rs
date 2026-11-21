//! 声明式 Schema 驱动表单与属性检查器（`SchemaForm`）。

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::elided_text::{elide_text_with_measure, ElideMode};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 单个字段输入框的高度。
///
/// 对齐 WinUI `TextControlThemeMinHeight = 32`（与已校准的 TextBox/Button 一致）。
const FIELD_HEIGHT: f32 = 32.0;
/// 输入框圆角半径。
///
/// 对齐 WinUI `ControlCornerRadius = 4`。
const FIELD_RADIUS: f32 = 4.0;
/// 字段之间的垂直间距。
///
/// Fluent 2 表单节奏取 16（与 `form::FORM_FIELD_GAP` 对齐）。
const FIELD_GAP: f32 = 16.0;
/// 左侧状态指示点及留白占用的水平缩进（`2px` 左边距 + `6px` 圆点 + `4px` 间距）。
const LABEL_LEFT_INSET: f32 = 12.0;
/// 标签列右端预留的微小安全余量，防止字形右沿贴边。
const LABEL_RIGHT_PADDING: f32 = 4.0;
/// 输入控件最小可用宽度。
const MIN_CONTROL_WIDTH: f32 = 32.0;

/// 标签列与输入控件之间的最小水平间距（逻辑像素，确保零碰撞重叠）。
pub const LABEL_CONTROL_GAP: f32 = 12.0;
/// 输入控件与行尾动作按钮之间的水平间距。
pub const TRAILING_ACTION_GAP: f32 = 8.0;
/// 行尾动作按钮的默认宽度。
pub const DEFAULT_TRAILING_ACTION_WIDTH: f32 = 76.0;
/// 标签列默认最小宽度。
pub const DEFAULT_LABEL_MIN_WIDTH: f32 = 160.0;
/// 标签列默认最大宽度。
pub const DEFAULT_LABEL_MAX_WIDTH: f32 = 360.0;
/// 标签列默认占表单总宽度的最大比例。
pub const DEFAULT_LABEL_MAX_RATIO: f32 = 0.45;
/// 标签默认字号（逻辑像素，用于自适应宽度估算与省略测量）。
pub const LABEL_FONT_SIZE: f32 = 12.0;

/// 计算指定行尾动作按钮的子命中节点 ID。
#[must_use]
pub const fn schema_form_trailing_action_node(node: u64, row_index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x5F00 + row_index as u64)
}

/// 判断字符是否属于东亚全角/双宽字形区间（用于离线宽度估算）。
#[inline]
const fn is_wide_cjk_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2FA1F
    )
}

/// 估算单行标签文本在给定字号下的逻辑像素宽度（支持 CJK 双宽估算）。
#[must_use]
pub fn estimate_label_text_width(text: &str, font_size: f32) -> f32 {
    if text.is_empty() || !font_size.is_finite() || font_size <= 0.0 {
        return 0.0;
    }
    let mut width = 0.0_f32;
    for ch in text.chars() {
        let factor = if is_wide_cjk_char(ch) {
            1.0
        } else if ch.is_ascii_whitespace() {
            0.35
        } else if ch.is_ascii_uppercase() {
            0.65
        } else if ch.is_ascii() {
            0.56
        } else {
            0.75
        };
        width += font_size * factor;
    }
    width
}

/// 字段值的继承与覆写来源状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FieldInheritanceState {
    /// 使用 Schema 默认值。
    #[default]
    DefaultValue,
    /// 继承自父级预设（携带父预设名称）。
    InheritedFrom(String),
    /// 当前预设显式覆写。
    Overridden,
}

/// `SchemaForm` 行级交互事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaFormEvent {
    /// 点击了指定索引字段的左侧重置/状态指示点。
    ResetDot(usize),
    /// 点击了指定索引字段的输入控件区域。
    ControlClicked(usize),
    /// 点击了指定索引字段的行尾动作按钮。
    TrailingAction(usize),
}

/// Schema 字段值类型。
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaValue {
    Bool(bool),
    Number(f64),
    Text(String),
    EnumIndex(usize),
    ColorRgba([u8; 4]),
    Vec3([f32; 3]),
}

impl SchemaValue {
    /// 将当前 Schema 字段值格式化为适合表单控件槽位显示的文本。
    #[must_use]
    pub fn display_text(&self, kind: &SchemaFieldKind) -> String {
        match (self, kind) {
            (Self::Bool(on), _) => {
                if *on {
                    "● Enabled (true)".to_owned()
                } else {
                    "○ Disabled (false)".to_owned()
                }
            }
            (Self::Number(v), SchemaFieldKind::Number { unit, .. }) => {
                if unit.is_empty() {
                    format!("{v}")
                } else {
                    format!("{v} {unit}")
                }
            }
            (Self::Number(v), _) => format!("{v}"),
            (Self::Text(s), SchemaFieldKind::Text { placeholder, .. }) => {
                if s.is_empty() {
                    (*placeholder).to_owned()
                } else {
                    s.clone()
                }
            }
            (Self::Text(s), _) => s.clone(),
            (Self::EnumIndex(idx), SchemaFieldKind::Select { options }) => options
                .get(*idx)
                .cloned()
                .unwrap_or_else(|| format!("#{idx}")),
            (Self::EnumIndex(idx), _) => format!("#{idx}"),
            (Self::ColorRgba([r, g, b, a]), _) => {
                format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
            }
            (Self::Vec3([x, y, z]), _) => format!("[{x:.2}, {y:.2}, {z:.2}]"),
        }
    }
}

/// Schema 字段控件类型声明。
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaFieldKind {
    Switch,
    Number {
        min: f64,
        max: f64,
        step: f64,
        unit: &'static str,
    },
    Text {
        placeholder: &'static str,
        required: bool,
    },
    FilePath {
        extensions: &'static [&'static str],
    },
    Select {
        options: Vec<String>,
    },
    Color,
    Vec3,
    ReadOnly,
}

/// 单个 Schema 字段定义（支持跨字段联动显隐/启用、继承来源标注与行尾动作槽）。
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaFieldSpec {
    pub key: String,
    pub label: String,
    pub section: String,
    pub kind: SchemaFieldKind,
    pub default_value: SchemaValue,
    /// 当指定的布尔字段为 `true` 时才激活本字段。
    pub enabled_when_field: Option<String>,
    /// 可选的继承来源状态标注。
    pub inheritance: FieldInheritanceState,
    /// 可选的行尾动作按钮文案（例如 `"重置"` / `"还原"`）；为 `Some` 时在同行右侧预留动作槽。
    pub trailing_action_label: Option<String>,
}

impl SchemaFieldSpec {
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        section: impl Into<String>,
        kind: SchemaFieldKind,
        default_value: SchemaValue,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            section: section.into(),
            kind,
            default_value,
            enabled_when_field: None,
            inheritance: FieldInheritanceState::DefaultValue,
            trailing_action_label: None,
        }
    }

    #[must_use]
    pub fn enabled_when(mut self, controlling_bool_key: impl Into<String>) -> Self {
        self.enabled_when_field = Some(controlling_bool_key.into());
        self
    }

    #[must_use]
    pub fn with_inheritance(mut self, inheritance: FieldInheritanceState) -> Self {
        self.inheritance = inheritance;
        self
    }

    /// 为该字段行配置可选的行尾动作按钮文案（同行右侧渲染，输入控件自动收缩避让）。
    #[must_use]
    pub fn with_trailing_action(mut self, label: impl Into<String>) -> Self {
        self.trailing_action_label = Some(label.into());
        self
    }

    /// 返回包含继承状态后缀的完整展示标签文本。
    #[must_use]
    pub fn full_label_text(&self) -> String {
        match &self.inheritance {
            FieldInheritanceState::DefaultValue => self.label.clone(),
            FieldInheritanceState::InheritedFrom(parent) => {
                format!("{} [inherited:{}]", self.label, parent)
            }
            FieldInheritanceState::Overridden => {
                format!("{} [override]", self.label)
            }
        }
    }
}

/// 标签列自适应宽度配置。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SchemaLabelColumnConfig {
    /// 是否启用基于字段最大显示宽度（含 CJK 双宽与继承后缀）的自适应计算（默认 `true`）。
    pub adaptive: bool,
    /// 标签列最小宽度（逻辑像素，默认 `160.0`）。
    pub min_width: f32,
    /// 标签列最大宽度（逻辑像素，默认 `360.0`）。
    pub max_width: f32,
    /// 标签列占表单总宽度的最大比例（默认 `0.45`）。
    pub max_ratio: f32,
    /// 标签列与输入控件之间的水平间距（逻辑像素，默认 `12.0`，运行时强制不低于 `12.0`）。
    pub gap: f32,
}

impl Default for SchemaLabelColumnConfig {
    fn default() -> Self {
        Self {
            adaptive: true,
            min_width: DEFAULT_LABEL_MIN_WIDTH,
            max_width: DEFAULT_LABEL_MAX_WIDTH,
            max_ratio: DEFAULT_LABEL_MAX_RATIO,
            gap: LABEL_CONTROL_GAP,
        }
    }
}

/// 单行表单的精确几何分区（渲染、文本省略与点击命中共享同一口径）。
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaFormRowLayout {
    /// 字段索引。
    pub index: usize,
    /// 整行内容边界矩形。
    pub row_rect: Rect,
    /// 左侧修改/继承状态指示点矩形。
    pub dot_rect: Rect,
    /// 标签文本可用矩形（右边界严格小于 `control_rect.origin.x` 且间隔 `>= 12px`）。
    pub label_rect: Rect,
    /// 按 `elided_text` 规则截断省略后的标签展示文本。
    pub display_label: String,
    /// 标签文本是否发生了实际省略。
    pub label_elided: bool,
    /// 右侧输入控件矩形（存在行尾动作时自动收缩，与动作按钮零重叠）。
    pub control_rect: Rect,
    /// 可选的行尾动作按钮矩形（仅当字段配置了 `trailing_action_label` 时为 `Some`）。
    pub trailing_action_rect: Option<Rect>,
}

/// Schema 驱动表单状态机（支持脏字段追踪、单项/全量默认值重置、联动校验与行级事件记录）。
#[derive(Clone, Debug, Default)]
pub struct SchemaFormState {
    values: FxHashMap<String, SchemaValue>,
    errors: FxHashMap<String, String>,
    last_event: Option<SchemaFormEvent>,
}

impl SchemaFormState {
    /// 根据字段定义列表初始化默认值。
    #[must_use]
    pub fn from_schema(fields: &[SchemaFieldSpec]) -> Self {
        let mut values = FxHashMap::default();
        for f in fields {
            values.insert(f.key.clone(), f.default_value.clone());
        }
        Self {
            values,
            errors: FxHashMap::default(),
            last_event: None,
        }
    }

    /// 读取指定字段的当前值。
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&SchemaValue> {
        self.values.get(key)
    }

    /// 更新指定字段的值。
    pub fn set(&mut self, key: impl Into<String>, value: SchemaValue) {
        self.values.insert(key.into(), value);
    }

    /// 获取全部字段值映射表。
    #[must_use]
    pub const fn values(&self) -> &FxHashMap<String, SchemaValue> {
        &self.values
    }

    /// 判断某字段是否已偏离其 Schema 默认值。
    #[must_use]
    pub fn is_modified(&self, spec: &SchemaFieldSpec) -> bool {
        self.values
            .get(&spec.key)
            .is_some_and(|v| *v != spec.default_value)
    }

    /// 将单个字段重置为默认值。
    pub fn reset_field(&mut self, spec: &SchemaFieldSpec) {
        self.values
            .insert(spec.key.clone(), spec.default_value.clone());
        self.errors.remove(&spec.key);
    }

    /// 将整张表单重置为默认值。
    pub fn reset_all(&mut self, fields: &[SchemaFieldSpec]) {
        for f in fields {
            self.reset_field(f);
        }
    }

    /// 判断字段在当前联动条件下是否可用。
    #[must_use]
    pub fn is_field_enabled(&self, spec: &SchemaFieldSpec) -> bool {
        let Some(ctrl_key) = &spec.enabled_when_field else {
            return true;
        };
        matches!(self.values.get(ctrl_key), Some(SchemaValue::Bool(true)))
    }

    /// 校验全部字段，返回是否全部通过。
    pub fn validate(&mut self, fields: &[SchemaFieldSpec]) -> bool {
        self.errors.clear();
        for f in fields {
            if !self.is_field_enabled(f) {
                continue;
            }
            if let (SchemaFieldKind::Text { required: true, .. }, Some(SchemaValue::Text(s))) =
                (&f.kind, self.values.get(&f.key))
            {
                if s.trim().is_empty() {
                    self.errors
                        .insert(f.key.clone(), format!("{} is required", f.label));
                }
            }
        }
        self.errors.is_empty()
    }

    /// 当前校验错误表。
    #[must_use]
    pub const fn errors(&self) -> &FxHashMap<String, String> {
        &self.errors
    }

    /// 记录最近一次触发的行级交互事件（例如行尾动作按钮点击）。
    pub fn record_event(&mut self, event: SchemaFormEvent) {
        self.last_event = Some(event);
    }

    /// 查看最近一次触发的行级交互事件。
    #[must_use]
    pub const fn last_event(&self) -> Option<&SchemaFormEvent> {
        self.last_event.as_ref()
    }

    /// 消费并返回最近一次触发的行级交互事件。
    pub fn take_last_event(&mut self) -> Option<SchemaFormEvent> {
        self.last_event.take()
    }
}

/// 声明式 `SchemaForm` 渲染描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaFormDesc {
    pub rect: Rect,
    pub fields: Vec<SchemaFieldSpec>,
    pub row_height: f32,
    pub z: u16,
    /// 标签列自适应宽度配置（默认开启，范围 `[160.0, min(rect.w * 0.45, 360.0)]`）。
    pub label_column: SchemaLabelColumnConfig,
    /// 行尾动作按钮预留宽度（逻辑像素，默认 `76.0`）。
    pub trailing_action_width: f32,
}

impl SchemaFormDesc {
    #[must_use]
    pub fn new(rect: Rect, fields: Vec<SchemaFieldSpec>) -> Self {
        Self {
            rect,
            fields,
            // 行距 = 输入框高度 32 + 字段间距 16，保证相邻输入框之间恰好留出 Fluent 2 的 16px。
            row_height: FIELD_HEIGHT + FIELD_GAP,
            z: 10,
            label_column: SchemaLabelColumnConfig::default(),
            trailing_action_width: DEFAULT_TRAILING_ACTION_WIDTH,
        }
    }

    /// 创建无 Rect 的声明式 Schema 表单描述符。
    #[must_use]
    pub fn form(fields: Vec<SchemaFieldSpec>) -> Self {
        Self::new(Rect::ZERO, fields)
    }

    /// 显式覆写完整的标签列自适应配置。
    #[must_use]
    pub const fn with_label_column(mut self, config: SchemaLabelColumnConfig) -> Self {
        self.label_column = config;
        self
    }

    /// 显式覆写标签列的最小与最大宽度（逻辑像素）。
    #[must_use]
    pub const fn with_label_width_bounds(mut self, min_width: f32, max_width: f32) -> Self {
        self.label_column.min_width = min_width;
        self.label_column.max_width = max_width;
        self
    }

    /// 显式覆写标签列占表单宽度的最大比例。
    #[must_use]
    pub const fn with_label_max_ratio(mut self, max_ratio: f32) -> Self {
        self.label_column.max_ratio = max_ratio;
        self
    }

    /// 显式开启或关闭标签列自适应宽度计算。
    #[must_use]
    pub const fn with_adaptive_label_column(mut self, adaptive: bool) -> Self {
        self.label_column.adaptive = adaptive;
        self
    }

    /// 显式覆写行尾动作按钮的预留宽度（逻辑像素）。
    #[must_use]
    pub const fn with_trailing_action_width(mut self, width: f32) -> Self {
        self.trailing_action_width = width;
        self
    }

    /// 根据表单总宽度与当前字段标签集合计算最终标签列宽。
    ///
    /// 默认在 `[min_width, min(form_width * max_ratio, max_width)]` 区间内按字段最大显示宽度
    /// （含 CJK 双宽估算与继承状态后缀）自适应取值；当表单本身极窄导致上限低于下限时，自动钳制避免越界或 panic。
    #[must_use]
    pub fn resolve_label_column_width(&self, form_width: f32) -> f32 {
        let safe_w = if form_width.is_finite() {
            form_width.max(0.0)
        } else {
            0.0
        };
        let min_w = self.label_column.min_width.max(48.0);
        let max_w = self.label_column.max_width.max(min_w);
        let ratio = self.label_column.max_ratio.clamp(0.15, 0.70);
        let upper = (safe_w * ratio).min(max_w).max(36.0);
        let effective_min = min_w.min(upper);

        if !self.label_column.adaptive {
            return (safe_w * 0.36).clamp(effective_min, upper);
        }

        let max_text_w = self.fields.iter().fold(0.0_f32, |acc, field| {
            let full = field.full_label_text();
            acc.max(estimate_label_text_width(&full, LABEL_FONT_SIZE))
        });
        let desired = LABEL_LEFT_INSET + max_text_w + LABEL_RIGHT_PADDING;
        desired.clamp(effective_min, upper)
    }

    /// 按指定外框矩形计算每一行的标签、输入控件与行尾动作槽几何布局。
    #[must_use]
    pub fn layout_rows_in(&self, rect: Rect) -> Vec<SchemaFormRowLayout> {
        let label_col_w = self.resolve_label_column_width(rect.size.x);
        let gap = self.label_column.gap.max(LABEL_CONTROL_GAP);
        let mut rows = Vec::with_capacity(self.fields.len());

        for (idx, field) in self.fields.iter().enumerate() {
            let y = rect.origin.y + 10.0 + idx as f32 * self.row_height;
            // 字段行高固定为输入框高度，剩余行距全部让给字段间距。
            let row_rect = Rect::new(
                rect.origin.x + 8.0,
                y,
                (rect.size.x - 16.0).max(40.0),
                FIELD_HEIGHT,
            );
            let dot_rect = Rect::new(row_rect.origin.x + 2.0, row_rect.center().y - 3.0, 6.0, 6.0);

            let label_x = row_rect.origin.x + LABEL_LEFT_INSET;
            let label_w = (label_col_w - LABEL_LEFT_INSET).max(24.0);
            let label_rect = Rect::new(label_x, row_rect.origin.y, label_w, FIELD_HEIGHT);

            let full_label = field.full_label_text();
            let elided = elide_text_with_measure(
                &full_label,
                label_rect.size.x,
                ElideMode::End,
                |sample| estimate_label_text_width(sample, LABEL_FONT_SIZE),
            );

            // 输入控件左边界严格从 `label_rect.max().x + gap` 起算，保证至少 12px 间距与零重叠。
            let ctrl_x = label_rect.max().x + gap;
            let available = (row_rect.max().x - ctrl_x).max(0.0);
            let (ctrl_w, trailing_action_rect) = if field.trailing_action_label.is_some() {
                let max_action_w = (available - MIN_CONTROL_WIDTH - TRAILING_ACTION_GAP).max(0.0);
                let action_w = self.trailing_action_width.max(32.0).min(max_action_w);
                if action_w > 0.0 {
                    let action_x = row_rect.max().x - action_w;
                    let cw = (action_x - TRAILING_ACTION_GAP - ctrl_x).max(MIN_CONTROL_WIDTH);
                    (
                        cw,
                        Some(Rect::new(action_x, row_rect.origin.y, action_w, FIELD_HEIGHT)),
                    )
                } else {
                    (available.max(MIN_CONTROL_WIDTH), None)
                }
            } else {
                (available.max(MIN_CONTROL_WIDTH), None)
            };

            let control_rect = Rect::new(ctrl_x, row_rect.origin.y, ctrl_w, FIELD_HEIGHT);

            rows.push(SchemaFormRowLayout {
                index: idx,
                row_rect,
                dot_rect,
                label_rect,
                display_label: elided.display,
                label_elided: elided.was_elided,
                control_rect,
                trailing_action_rect,
            });
        }

        rows
    }

    /// 使用当前 `self.rect` 计算每一行的布局几何。
    #[must_use]
    pub fn layout_rows(&self) -> Vec<SchemaFormRowLayout> {
        self.layout_rows_in(self.rect)
    }

    /// 在指定外框矩形内执行行级命中测试，返回命中的事件（行尾动作槽、左侧重置点或输入控件）。
    #[must_use]
    pub fn hit_test_in(&self, rect: Rect, pos: Vec2) -> Option<SchemaFormEvent> {
        for row in self.layout_rows_in(rect) {
            if !row.row_rect.contains_point(pos) {
                continue;
            }
            if let Some(action_r) = row.trailing_action_rect {
                if action_r.contains_point(pos) {
                    return Some(SchemaFormEvent::TrailingAction(row.index));
                }
            }
            if pos.x <= row.dot_rect.max().x + 12.0 {
                return Some(SchemaFormEvent::ResetDot(row.index));
            }
            if row.control_rect.contains_point(pos) {
                return Some(SchemaFormEvent::ControlClicked(row.index));
            }
            return None;
        }
        None
    }

    /// 使用当前 `self.rect` 执行行级命中测试。
    #[must_use]
    pub fn hit_test(&self, pos: Vec2) -> Option<SchemaFormEvent> {
        self.hit_test_in(self.rect, pos)
    }
}

/// 发射 `SchemaForm`（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_schema_form(
    node: u64,
    desc: &SchemaFormDesc,
    state: &SchemaFormState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rows = desc.layout_rows();
    let trailing_count = rows
        .iter()
        .filter(|row| row.trailing_action_rect.is_some())
        .count();
    let mut primitives = Vec::with_capacity(2 + rows.len() * 3 + trailing_count + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 8.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    let mut pending_sub_controls: Vec<(u64, HitRegion, A11yNode, bool)> =
        Vec::with_capacity(trailing_count);

    for (row, field) in rows.iter().zip(desc.fields.iter()) {
        let enabled = !control.disabled && state.is_field_enabled(field);
        let modified = state.is_modified(field)
            || matches!(field.inheritance, FieldInheritanceState::Overridden);

        // 左侧已修改/继承高亮指示点（恒定发射，未修改时根据继承状态着色）
        let dot_tok = if modified {
            Token::PRIMARY
        } else if matches!(field.inheritance, FieldInheritanceState::InheritedFrom(_)) {
            Token::ACCENT
        } else {
            Token::NONE
        };
        primitives.push(surface(row.dot_rect, 3.0, dot_tok, z + 1));

        // 右侧控件槽位表面：与标签列至少保持 `LABEL_CONTROL_GAP`（>= 12px）。
        let (fill, border) = if !enabled {
            (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
        } else if state.errors.contains_key(&field.key) {
            (Token::INPUT_BG, Token::INPUT_BORDER_ERROR)
        } else {
            // WinUI `TextControlBorderBrush` = `ControlStrokeColorDefault`。
            (Token::INPUT_BG, Token::CONTROL_STROKE)
        };
        primitives.push(outlined(
            row.control_rect,
            FIELD_RADIUS,
            fill,
            border,
            1.0,
            0.0,
            z + 1,
        ));

        // 可选行尾动作按钮（仅取决于描述符定义，24 态下图元数严格恒定）。
        if let (Some(action_r), Some(action_label)) =
            (row.trailing_action_rect, field.trailing_action_label.as_ref())
        {
            let (action_fill, action_border) = if !enabled {
                (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
            } else {
                (Token::SURFACE_RAISED, Token::CONTROL_STROKE)
            };
            primitives.push(outlined(
                action_r,
                FIELD_RADIUS,
                action_fill,
                action_border,
                1.0,
                0.0,
                z + 1,
            ));

            if !control.disabled {
                let sub_id = schema_form_trailing_action_node(node, row.index);
                let hit = HitRegion::new(sub_id, LAYER, u32::from(z + 2), action_r)
                    .with_cursor(CursorKind::Pointer);
                let a11y = A11yNode::new(Role::Button, action_label.clone(), action_r)
                    .disabled(!enabled);
                pending_sub_controls.push((sub_id, hit, a11y, enabled));
            }
        }

        // 行底部分隔细线：落在字段间距的下沿，避免贴住输入框。
        let div_r = Rect::new(
            row.row_rect.origin.x,
            row.row_rect.origin.y + desc.row_height - 1.0,
            row.row_rect.size.x,
            1.0,
        );
        primitives.push(surface(div_r, 0.0, Token::DIVIDER, z + 1));
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
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Form, "Schema Form", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    };

    for (sub_id, hit, a11y, focusable) in pending_sub_controls {
        out.with_sub_control(sub_id, hit, a11y, focusable);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_form_tracks_modifications_dependencies_and_validation() {
        let fields = vec![
            SchemaFieldSpec::new(
                "exe_path",
                "Executable",
                "Target",
                SchemaFieldKind::Text {
                    placeholder: "Select .exe",
                    required: true,
                },
                SchemaValue::Text(String::new()),
            ),
            SchemaFieldSpec::new(
                "auto_capture",
                "Auto Capture",
                "Capture",
                SchemaFieldKind::Switch,
                SchemaValue::Bool(false),
            ),
            SchemaFieldSpec::new(
                "capture_frame",
                "Frame Index",
                "Capture",
                SchemaFieldKind::Number {
                    min: 1.0,
                    max: 10000.0,
                    step: 1.0,
                    unit: "frame",
                },
                SchemaValue::Number(100.0),
            )
            .enabled_when("auto_capture"),
        ];

        let mut state = SchemaFormState::from_schema(&fields);
        assert!(!state.is_field_enabled(&fields[2]));
        assert!(!state.validate(&fields));
        assert!(state.errors().contains_key("exe_path"));

        state.set("exe_path", SchemaValue::Text("D:/game/demo.exe".into()));
        state.set("auto_capture", SchemaValue::Bool(true));
        assert!(state.is_field_enabled(&fields[2]));
        assert!(state.is_modified(&fields[0]));
        assert!(state.validate(&fields));

        state.reset_field(&fields[0]);
        assert!(!state.is_modified(&fields[0]));
    }
}

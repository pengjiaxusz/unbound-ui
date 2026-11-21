//! 声明式 Schema 驱动表单与属性检查器（`SchemaForm`）。

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

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
/// 标签列与输入框之间的间距。
///
/// Fluent 2 表单布局的标签间距取 4（与 `form::FORM_LABEL_GAP` 对齐）。
const LABEL_GAP: f32 = 4.0;
/// 字段之间的垂直间距。
///
/// Fluent 2 表单节奏取 16（与 `form::FORM_FIELD_GAP` 对齐）。
const FIELD_GAP: f32 = 16.0;

/// 字段值的继承与覆写来源状态（供下游渲染工具启动预设配置继承链展示徽章）。
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

/// 单个 Schema 字段定义（支持跨字段联动显隐/启用与继承来源标注）。
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
}

/// Schema 驱动表单状态机（支持脏字段追踪、单项/全量默认值重置、联动校验）。
#[derive(Clone, Debug, Default)]
pub struct SchemaFormState {
    values: FxHashMap<String, SchemaValue>,
    errors: FxHashMap<String, String>,
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
}

/// 声明式 `SchemaForm` 渲染描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaFormDesc {
    pub rect: Rect,
    pub fields: Vec<SchemaFieldSpec>,
    pub row_height: f32,
    pub z: u16,
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
        }
    }

    /// 创建无 Rect 的声明式 Schema 表单描述符。
    #[must_use]
    pub fn form(fields: Vec<SchemaFieldSpec>) -> Self {
        Self::new(Rect::ZERO, fields)
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
    let mut primitives = Vec::with_capacity(2 + desc.fields.len() * 3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 8.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    let label_col_w = (r.size.x * 0.36).clamp(90.0, 220.0);
    for (idx, field) in desc.fields.iter().enumerate() {
        let y = r.origin.y + 10.0 + idx as f32 * desc.row_height;
        // 字段行高固定为输入框高度，剩余行距全部让给字段间距。
        let row_r = Rect::new(
            r.origin.x + 8.0,
            y,
            (r.size.x - 16.0).max(40.0),
            FIELD_HEIGHT,
        );
        let enabled = !control.disabled && state.is_field_enabled(field);
        let modified = state.is_modified(field)
            || matches!(field.inheritance, FieldInheritanceState::Overridden);

        // 左侧已修改/继承高亮指示点（恒定发射，未修改时根据继承状态着色）
        let dot_r = Rect::new(row_r.origin.x + 2.0, row_r.center().y - 3.0, 6.0, 6.0);
        let dot_tok = if modified {
            Token::PRIMARY
        } else if matches!(field.inheritance, FieldInheritanceState::InheritedFrom(_)) {
            Token::ACCENT
        } else {
            Token::NONE
        };
        primitives.push(surface(dot_r, 3.0, dot_tok, z + 1));

        // 右侧控件槽位表面：标签列与输入框之间留出 `LABEL_GAP`。
        let ctrl_x = row_r.origin.x + label_col_w + LABEL_GAP;
        let ctrl_r = Rect::new(
            ctrl_x,
            row_r.origin.y,
            (row_r.max().x - ctrl_x).max(32.0),
            FIELD_HEIGHT,
        );
        let (fill, border) = if !enabled {
            (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
        } else if state.errors.contains_key(&field.key) {
            (Token::INPUT_BG, Token::INPUT_BORDER_ERROR)
        } else {
            // WinUI `TextControlBorderBrush` = `ControlStrokeColorDefault`。
            (Token::INPUT_BG, Token::CONTROL_STROKE)
        };
        primitives.push(outlined(
            ctrl_r,
            FIELD_RADIUS,
            fill,
            border,
            1.0,
            0.0,
            z + 1,
        ));

        // 行底部分隔细线：落在字段间距的下沿，避免贴住输入框。
        let div_r = Rect::new(row_r.origin.x, y + desc.row_height - 1.0, row_r.size.x, 1.0);
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

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Form, "Schema Form", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
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

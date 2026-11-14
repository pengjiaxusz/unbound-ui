//! 声明式 `SchemaForm` 属性检查器与 `GenericDataTable` 虚拟化泛型数据表格（阶段 1 WP5）。
//!
//! 面向两大宿主的核心业务引擎：
//! * **`chahu-render-debugger`**：需要通过 `SchemaForm` 自动渲染启动配置（可执行文件路径、工作目录、
//!   命令行参数、注入 DLL 开关、API 捕获帧数、环境变量），并通过 `GenericDataTable` 承载进程列表、
//!   DrawCall / 事件列表与历史捕获记录（支持多列排序、列宽拖拽、键盘上下行导航、拼音/子串过滤与汇总行）；
//! * **`dunting`**：需要通过 `SchemaForm` 渲染选中资产的材质/贴图/模型属性面板（含条件显隐与默认值重置），
//!   并通过 `GenericDataTable` 承载资产清单视图。

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::{GlyphQuad, PinyinMatcher};
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

const LAYER: u8 = 1;

// ---------------------------------------------------------------------------
// 1. SchemaForm（声明式 Schema 驱动表单与属性检查器）
// ---------------------------------------------------------------------------

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

/// 单个 Schema 字段定义（支持联动显隐 `visible_when_bool_field`）。
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaFieldSpec {
    pub key: String,
    pub label: String,
    pub section: String,
    pub kind: SchemaFieldKind,
    pub default_value: SchemaValue,
    /// 当指定的布尔字段为 `true` 时才激活本字段（用于联动表单，如开启 `"自动帧捕获"` 后才激活 `"触发快捷键"`）。
    pub enabled_when_field: Option<String>,
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
        }
    }

    #[must_use]
    pub fn enabled_when(mut self, controlling_bool_key: impl Into<String>) -> Self {
        self.enabled_when_field = Some(controlling_bool_key.into());
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
            row_height: 34.0,
            z: 10,
        }
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

    primitives.push(outlined(
        r,
        8.0,
        Token::SURFACE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    let label_col_w = (r.size.x * 0.36).clamp(90.0, 180.0);
    for (idx, field) in desc.fields.iter().enumerate() {
        let y = r.origin.y + 10.0 + idx as f32 * desc.row_height;
        let row_r = Rect::new(
            r.origin.x + 8.0,
            y,
            (r.size.x - 16.0).max(40.0),
            desc.row_height - 4.0,
        );
        let enabled = !control.disabled && state.is_field_enabled(field);
        let modified = state.is_modified(field);

        // 左侧已修改高亮指示点（恒定发射，未修改时透明）
        let dot_r = Rect::new(row_r.origin.x + 2.0, row_r.center().y - 3.0, 6.0, 6.0);
        let dot_tok = if modified {
            Token::PRIMARY
        } else {
            Token::NONE
        };
        primitives.push(surface(dot_r, 3.0, dot_tok, z + 1));

        // 右侧控件槽位表面
        let ctrl_r = Rect::new(
            row_r.origin.x + label_col_w,
            row_r.origin.y + 2.0,
            (row_r.size.x - label_col_w - 4.0).max(32.0),
            (row_r.size.y - 4.0).max(18.0),
        );
        let (fill, border) = if !enabled {
            (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
        } else if state.errors.contains_key(&field.key) {
            (Token::INPUT_BG, Token::INPUT_BORDER_ERROR)
        } else {
            (Token::INPUT_BG, Token::OUTLINE)
        };
        primitives.push(outlined(ctrl_r, 5.0, fill, border, 1.0, 0.0, z + 1));

        // 行底部分隔细线
        let div_r = Rect::new(row_r.origin.x, row_r.max().y + 1.0, row_r.size.x, 1.0);
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

// ---------------------------------------------------------------------------
// 2. GenericDataTable（支持排序、列宽拖拽、中英文拼音过滤与汇总行的泛型数据表）
// ---------------------------------------------------------------------------

/// 列排序方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// 泛型数据表列定义。
#[derive(Clone, Debug, PartialEq)]
pub struct DataColumnSpec {
    pub id: String,
    pub title: String,
    pub width: f32,
    pub min_width: f32,
    pub sortable: bool,
    pub resizable: bool,
}

impl DataColumnSpec {
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, width: f32) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            width: width.max(40.0),
            min_width: 48.0,
            sortable: true,
            resizable: true,
        }
    }
}

/// 泛型数据表交互状态（管理排序列、列宽调整、选中行集合、虚拟滚动偏移与拼音搜索过滤）。
#[derive(Clone, Debug, Default)]
pub struct GenericDataTableState {
    pub sort_column: Option<(usize, SortDirection)>,
    pub selected_rows: Vec<usize>,
    pub focused_row: Option<usize>,
    pub scroll_offset_y: f32,
    pub filter_query: String,
}

impl GenericDataTableState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换指定列的排序状态（升序 -> 降序 -> 取消排序）。
    pub fn cycle_sort(&mut self, col_idx: usize) {
        self.sort_column = match self.sort_column {
            Some((idx, SortDirection::Ascending)) if idx == col_idx => {
                Some((col_idx, SortDirection::Descending))
            }
            Some((idx, SortDirection::Descending)) if idx == col_idx => None,
            _ => Some((col_idx, SortDirection::Ascending)),
        };
    }

    /// 拖拽调整指定列的宽度。
    pub fn resize_column(columns: &mut [DataColumnSpec], col_idx: usize, delta_x: f32) {
        if let Some(col) = columns.get_mut(col_idx) {
            if col.resizable {
                col.width = (col.width + delta_x).max(col.min_width);
            }
        }
    }

    /// 对给定行数据执行拼音/子串过滤与当前列排序，返回可见的原始行索引序列。
    #[must_use]
    pub fn resolve_visible_indices(&self, rows: &[Vec<String>]) -> Vec<usize> {
        let matcher = PinyinMatcher::new();
        let q = self.filter_query.trim();
        let mut indices: Vec<usize> = (0..rows.len())
            .filter(|&row_idx| {
                if q.is_empty() {
                    return true;
                }
                rows[row_idx]
                    .iter()
                    .any(|cell| matcher.match_text(cell, q).is_some())
            })
            .collect();

        if let Some((col_idx, dir)) = self.sort_column {
            indices.sort_by(|&a, &b| {
                let va = rows[a].get(col_idx).map_or("", String::as_str);
                let vb = rows[b].get(col_idx).map_or("", String::as_str);
                let ord = va.cmp(vb);
                match dir {
                    SortDirection::Ascending => ord,
                    SortDirection::Descending => ord.reverse(),
                }
            });
        }
        indices
    }

    /// 键盘上下行导航。
    pub fn move_focus(&mut self, visible_len: usize, delta: isize) {
        if visible_len == 0 {
            self.focused_row = None;
            return;
        }
        let cur = self.focused_row.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, (visible_len - 1) as isize) as usize;
        self.focused_row = Some(next);
        self.selected_rows = vec![next];
    }
}

/// 泛型数据表渲染描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct GenericDataTableDesc {
    pub rect: Rect,
    pub columns: Vec<DataColumnSpec>,
    pub visible_row_slots: usize,
    pub row_height: f32,
    pub show_summary_footer: bool,
    pub z: u16,
}

impl GenericDataTableDesc {
    #[must_use]
    pub fn new(rect: Rect, columns: Vec<DataColumnSpec>, visible_row_slots: usize) -> Self {
        Self {
            rect,
            columns,
            visible_row_slots: visible_row_slots.max(1),
            row_height: 30.0,
            show_summary_footer: true,
            z: 10,
        }
    }
}

/// 发射泛型数据表（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_generic_data_table(
    node: u64,
    desc: &GenericDataTableDesc,
    state: &GenericDataTableState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives =
        Vec::with_capacity(4 + desc.columns.len() + desc.visible_row_slots + glyphs.len());
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

    // 表头栏
    let header_r = Rect::new(
        r.origin.x + 1.0,
        r.origin.y + 1.0,
        (r.size.x - 2.0).max(1.0),
        32.0,
    );
    primitives.push(surface(header_r, 7.0, Token::HEADER_BG, z + 1));

    // 列分隔把手线
    let mut col_x = r.origin.x;
    for col in &desc.columns {
        col_x += col.width;
        let sep_r = Rect::new(col_x.min(r.max().x - 2.0), r.origin.y + 6.0, 1.0, 20.0);
        primitives.push(surface(sep_r, 0.0, Token::DIVIDER, z + 2));
    }

    // 固定数量的虚拟化可视行槽位（保证 24 态恒定图元数）
    for slot_idx in 0..desc.visible_row_slots {
        let ry = header_r.max().y + slot_idx as f32 * desc.row_height;
        let row_r = Rect::new(
            r.origin.x + 2.0,
            ry,
            (r.size.x - 4.0).max(1.0),
            desc.row_height,
        );
        let is_selected = state.selected_rows.contains(&slot_idx);
        let fill = if is_selected {
            Token::SELECTION
        } else if slot_idx % 2 == 1 {
            Token::SURFACE_SUNKEN
        } else {
            Token::SURFACE
        };
        primitives.push(surface(row_r, 4.0, fill, z + 1));
    }

    // 底部汇总统计栏（Summary Footer）
    if desc.show_summary_footer {
        let footer_r = Rect::new(
            r.origin.x + 1.0,
            r.max().y - 28.0,
            (r.size.x - 2.0).max(1.0),
            27.0,
        );
        primitives.push(outlined(
            footer_r,
            6.0,
            Token::SURFACE_RAISED,
            Token::DIVIDER,
            1.0,
            0.0,
            z + 2,
        ));
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
        a11y: Some(A11yNode::new(Role::Table, "Data Table", r).disabled(control.disabled)),
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

    #[test]
    fn generic_data_table_sorts_resizes_and_filters_with_pinyin() {
        let mut cols = vec![
            DataColumnSpec::new("name", "Name", 160.0),
            DataColumnSpec::new("type", "Type", 100.0),
        ];
        let rows = vec![
            vec!["着色器_Uber.spv".into(), "SPV".into()],
            vec!["材质球_PBR.mat".into(), "MAT".into()],
            vec!["角色网格.glb".into(), "GLTF".into()],
        ];

        let mut state = GenericDataTableState::new();
        state.filter_query = "zsq".into(); // 拼音首字母匹配 "着色器"
        let visible = state.resolve_visible_indices(&rows);
        assert_eq!(visible, vec![0]);

        state.filter_query.clear();
        state.cycle_sort(1); // 按 Type 升序：GLTF (2), MAT (1), SPV (0)
        assert_eq!(state.resolve_visible_indices(&rows), vec![2, 1, 0]);

        GenericDataTableState::resize_column(&mut cols, 0, -500.0);
        assert_eq!(cols[0].width, cols[0].min_width);
    }
}

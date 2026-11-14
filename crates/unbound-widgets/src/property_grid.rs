//! 检查器风格的属性网格：分组、修改标记与重置。
//!
//! 该网格是检查器的只读/轻交互半边，同样不持有数据模型：
//! [`PropertyDesc`] 同时携带当前值*与*默认值，因此“属性是否已修改”与“重置”均为纯比较操作。
//! 重置请求通过 [`PropertyGridState::take_reset`] 上报，由应用程序将默认值写回自身模型。
//!
//!
//!
//! 分组可独立折叠。折叠后的行会被布局直接跳过，而非仅仅绘制为透明，
//! 因此包含 300 个属性的文档也只会输出用户当前可见的行。
//!

use accesskit::Role;
use rustc_hash::FxHashSet;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 属性的编辑方式。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PropertyKind {
    /// 自由格式字符串。
    #[default]
    Text,
    /// 带可选单位的数字。
    Number {
        /// 显示在数值后的单位。
        unit: String,
    },
    /// 布尔开关。
    Bool,
    /// 颜色值（以十六进制字符串显示）。
    Color,
    /// 固定字符串集合中的枚举项。
    Select(Vec<String>),
}

impl PropertyKind {
    /// 值单元格对外暴露的无障碍角色。
    #[must_use]
    pub fn role(&self) -> Role {
        match self {
            Self::Text => Role::TextInput,
            Self::Number { .. } => Role::SpinButton,
            Self::Bool => Role::CheckBox,
            Self::Color => Role::ColorWell,
            Self::Select(_) => Role::ComboBox,
        }
    }
}

/// 单个属性的值。
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// 字符串。
    Text(String),
    /// 数值（按调用方精度渲染）。
    Number(f64),
    /// 布尔值。
    Bool(bool),
    /// `#rrggbb` 形式的颜色。
    Color(String),
}

impl PropertyValue {
    /// 值列中绘制的文本。
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Text(text) | Self::Color(text) => text.clone(),
            Self::Number(number) => {
                let text = format!("{number:.2}");
                let trimmed = text.trim_end_matches('0').trim_end_matches('.');
                trimmed.to_owned()
            }
            Self::Bool(value) => {
                if *value {
                    "开".to_owned()
                } else {
                    "关".to_owned()
                }
            }
        }
    }
}

/// 单个属性行。
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDesc {
    /// 前导列中的标签。
    pub label: String,
    /// 当前值。
    pub value: PropertyValue,
    /// 重置后恢复的默认值。
    pub default: PropertyValue,
    /// 编辑器类型。
    pub kind: PropertyKind,
    /// 可选的工具提示 / 无障碍描述。
    pub description: String,
}

impl PropertyDesc {
    /// 创建以 `value` 自身作为默认值的属性。
    #[must_use]
    pub fn new(label: impl Into<String>, value: PropertyValue) -> Self {
        Self {
            label: label.into(),
            default: value.clone(),
            value,
            kind: PropertyKind::default(),
            description: String::new(),
        }
    }

    /// 指定编辑器类型。
    #[must_use]
    pub fn with_kind(mut self, kind: PropertyKind) -> Self {
        self.kind = kind;
        self
    }

    /// 指定显式默认值。
    #[must_use]
    pub fn with_default(mut self, default: PropertyValue) -> Self {
        self.default = default;
        self
    }

    /// 指定描述文本。
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// 当前值是否与默认值不同。
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.value != self.default
    }

    /// 格式化后的值文本（数字包含单位）。
    #[must_use]
    pub fn value_text(&self) -> String {
        let mut text = self.value.display();
        if let PropertyKind::Number { unit } = &self.kind {
            if !unit.is_empty() {
                text.push(' ');
                text.push_str(unit);
            }
        }
        text
    }
}

/// 单个可折叠的属性分组。
#[derive(Clone, Debug, PartialEq)]
pub struct PropertySection {
    /// 分组标题。
    pub title: String,
    /// 按顺序排列的属性行。
    pub rows: Vec<PropertyDesc>,
}

impl PropertySection {
    /// 创建空分组。
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    /// 向分组追加一行属性。
    #[must_use]
    pub fn with_row(mut self, property: PropertyDesc) -> Self {
        self.rows.push(property);
        self
    }
}

/// 属性网格的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyGridDesc {
    /// 整个网格矩形。
    pub rect: Rect,
    /// 无障碍名称。
    pub label: String,
    /// 前导标签列宽度。
    pub label_width: f32,
    /// 统一行高。
    pub row_height: f32,
    /// 分组标题行高度。
    pub header_height: f32,
    /// 分组列表。
    pub sections: Vec<PropertySection>,
    /// 绘制顺序键。
    pub z: u16,
}

impl PropertyGridDesc {
    /// 创建带 140 px 标签列与 26 px 行高的属性网格。
    #[must_use]
    pub fn new(rect: Rect, sections: Vec<PropertySection>) -> Self {
        Self {
            rect,
            label: String::new(),
            label_width: 140.0,
            row_height: 26.0,
            header_height: 28.0,
            sections,
            z: 620,
        }
    }

    /// 设置无障碍名称。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 总行数（包含已折叠分组）。
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.sections.iter().map(|section| section.rows.len()).sum()
    }

    /// 值单元格内的重置按钮矩形。
    #[must_use]
    pub fn reset_rect(&self, row: Rect) -> Rect {
        let size = (self.row_height * 0.5).min(16.0);
        Rect::new(
            row.max().x - size - 6.0,
            row.origin.y + (row.size.y - size) * 0.5,
            size,
            size,
        )
    }

    /// 指定行的值单元格矩形。
    #[must_use]
    pub fn value_rect(&self, row: Rect) -> Rect {
        Rect::new(
            row.origin.x + self.label_width,
            row.origin.y,
            (row.size.x - self.label_width).max(0.0),
            row.size.y,
        )
    }
}

/// 单个可见行的布局矩形信息。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyRowLayout {
    /// 分组索引。
    pub section: usize,
    /// 分组内的行索引。
    pub row: usize,
    /// 整行矩形。
    pub rect: Rect,
    /// 标签单元格矩形。
    pub label_rect: Rect,
    /// 值单元格矩形。
    pub value_rect: Rect,
    /// 重置按钮触发区矩形。
    pub reset_rect: Rect,
    /// 若该行为分组首行，则包含分组标题矩形。
    pub header: Option<Rect>,
}

/// 属性网格的可变状态。
#[derive(Clone, Debug, Default)]
pub struct PropertyGridState {
    collapsed: FxHashSet<usize>,
    selected: Option<(usize, usize)>,
    reset: Option<(usize, usize)>,
    hovered: Option<(usize, usize)>,
}

impl PropertyGridState {
    /// 创建全部展开的初始状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换分组折叠状态并返回新状态。
    pub fn toggle_section(&mut self, section: usize) -> bool {
        if !self.collapsed.remove(&section) {
            self.collapsed.insert(section);
            true
        } else {
            false
        }
    }

    /// 查询分组是否处于折叠状态。
    #[must_use]
    pub fn is_collapsed(&self, section: usize) -> bool {
        self.collapsed.contains(&section)
    }

    /// 选中单元格。
    pub fn select(&mut self, section: usize, row: usize) {
        self.selected = Some((section, row));
    }

    /// 当前选中的单元格。
    #[must_use]
    pub fn selected(&self) -> Option<(usize, usize)> {
        self.selected
    }

    /// 标记悬停行。
    pub fn hover(&mut self, cell: Option<(usize, usize)>) {
        self.hovered = cell;
    }

    /// 当前悬停的单元格。
    #[must_use]
    pub fn hovered(&self) -> Option<(usize, usize)> {
        self.hovered
    }

    /// 请求重置；调用方通过 [`Self::take_reset`] 消费该请求。
    pub fn reset_row(&mut self, section: usize, row: usize) {
        self.reset = Some((section, row));
    }

    /// 取出待处理的重置请求。
    pub fn take_reset(&mut self) -> Option<(usize, usize)> {
        self.reset.take()
    }

    /// 在 `desc` 内布局所有可见行。
    #[must_use]
    pub fn layout(&self, desc: &PropertyGridDesc) -> Vec<PropertyRowLayout> {
        let mut out = Vec::new();
        let mut y = desc.rect.origin.y;
        for (section_index, section) in desc.sections.iter().enumerate() {
            if y >= desc.rect.max().y {
                break;
            }
            let header = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.header_height);
            y += desc.header_height;
            if self.is_collapsed(section_index) {
                out.push(PropertyRowLayout {
                    section: section_index,
                    row: usize::MAX,
                    rect: header,
                    label_rect: header,
                    value_rect: header,
                    reset_rect: header,
                    header: Some(header),
                });
                continue;
            }
            for (row_index, _) in section.rows.iter().enumerate() {
                if y >= desc.rect.max().y {
                    break;
                }
                let rect = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.row_height);
                out.push(PropertyRowLayout {
                    section: section_index,
                    row: row_index,
                    rect,
                    label_rect: Rect::new(
                        rect.origin.x,
                        rect.origin.y,
                        desc.label_width,
                        rect.size.y,
                    ),
                    value_rect: desc.value_rect(rect),
                    reset_rect: desc.reset_rect(rect),
                    header: (row_index == 0).then_some(header),
                });
                y += desc.row_height;
            }
        }
        out
    }
}

/// 属性网格绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyGridTokens {
    /// 网格表面背景。
    pub background: Token,
    /// 分组标题。
    pub header: Token,
    /// 标签文本。
    pub label: Token,
    /// 值文本。
    pub value: Token,
    /// 已修改标记。
    pub modified: Token,
    /// 行分隔线。
    pub rule: Token,
    /// 选中单元格。
    pub selected: Token,
}

impl PropertyGridTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            background: Token::SURFACE,
            header: Token::HEADER_BG,
            label: Token::TEXT_SECONDARY,
            value: Token::TEXT_PRIMARY,
            modified: Token::PRIMARY,
            rule: Token::DIVIDER,
            selected: Token::SELECTION,
        }
    }
}

/// 输出所有可见行及其分组标题。
#[must_use]
pub fn emit_property_grid(
    node: u64,
    desc: &PropertyGridDesc,
    state: &PropertyGridState,
    row_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let tokens = PropertyGridTokens::standard();
    let layout = state.layout(desc);
    let mut primitives = Vec::with_capacity(4 + layout.len() * 3 + row_glyphs.len());
    let mut payloads = Vec::with_capacity(row_glyphs.len());

    primitives.push(surface(desc.rect, 0.0, tokens.background, desc.z));
    for row in &layout {
        if let Some(header) = row.header {
            primitives.push(surface(header, 0.0, tokens.header, desc.z + 1));
        }
        if row.row == usize::MAX {
            primitives.push(line(
                Rect::new(
                    row.rect.origin.x,
                    row.rect.max().y - 1.0,
                    row.rect.size.x,
                    1.0,
                ),
                tokens.rule,
                desc.z + 2,
            ));
            continue;
        }
        if state.selected() == Some((row.section, row.row)) {
            primitives.push(surface(row.rect, 0.0, tokens.selected, desc.z + 1));
        }
        primitives.push(line(
            Rect::new(
                row.rect.origin.x,
                row.rect.max().y - 1.0,
                row.rect.size.x,
                1.0,
            ),
            tokens.rule,
            desc.z + 2,
        ));
        if let Some(property) = desc
            .sections
            .get(row.section)
            .and_then(|section| section.rows.get(row.row))
        {
            if property.is_modified() {
                let dot = Rect::new(
                    row.label_rect.origin.x + 6.0,
                    row.label_rect.origin.y + (row.label_rect.size.y - 5.0) * 0.5,
                    5.0,
                    5.0,
                );
                primitives.push(surface(dot, 2.5, tokens.modified, desc.z + 3));
            }
            // 重置按钮底色保持固定槽位：悬停仅改变其不透明度，不改变图元数量。
            //
            let mut reset = surface(row.reset_rect.inflate(3.0), 4.0, tokens.header, desc.z + 3);
            reset.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
                reset.depth_u16(),
                if state.hovered() == Some((row.section, row.row)) {
                    1.0
                } else {
                    0.0
                },
            );
            primitives.push(reset);
        }
    }

    glyphs(
        row_glyphs,
        tokens.value,
        desc.z + 4,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        desc.rect,
        0.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 5,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).clickable();
    a11y.description = format!("{} 个属性", desc.row_count());
    a11y.disabled = control.disabled;

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Default),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    };

    if !control.disabled {
        for row in &layout {
            if row.row == usize::MAX {
                let sec_key = property_section_node(node, row.section);
                out.with_sub_control(
                    sec_key,
                    HitRegion::new(sec_key, CONTROL_LAYER, u32::from(desc.z) + 1, row.rect)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(
                        Role::Button,
                        format!("Section {}", row.section + 1),
                        row.rect,
                    )
                    .clickable(),
                    false,
                );
            } else {
                let row_key = property_row_node(node, row.section, row.row);
                out.with_sub_control(
                    row_key,
                    HitRegion::new(row_key, CONTROL_LAYER, u32::from(desc.z) + 1, row.rect)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(
                        Role::ListItem,
                        format!("Property {}:{}", row.section, row.row),
                        row.rect,
                    )
                    .clickable(),
                    false,
                );
                let reset_key = property_reset_node(node, row.section, row.row);
                out.with_sub_control(
                    reset_key,
                    HitRegion::new(
                        reset_key,
                        CONTROL_LAYER,
                        u32::from(desc.z) + 3,
                        row.reset_rect.inflate(3.0),
                    )
                    .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Reset", row.reset_rect.inflate(3.0)).clickable(),
                    false,
                );
            }
        }
    }

    out
}

/// 属性网格分组标题的确定性子节点键。
#[must_use]
pub const fn property_section_node(base: u64, section: usize) -> u64 {
    base ^ (0x5047_5300_u64 | (section as u64 + 1))
}

/// 属性网格行的确定性子节点键。
#[must_use]
pub const fn property_row_node(base: u64, section: usize, row: usize) -> u64 {
    base ^ (((section as u64 + 1) << 20) | (row as u64 + 1))
}

/// 属性行恢复默认按钮的确定性子节点键。
#[must_use]
pub const fn property_reset_node(base: u64, section: usize, row: usize) -> u64 {
    base ^ 0x5047_5200_0000_u64 ^ (((section as u64 + 1) << 20) | (row as u64 + 1))
}

/// 实心线段图元。
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PropertyGridDesc {
        PropertyGridDesc::new(
            Rect::new(0.0, 0.0, 300.0, 200.0),
            vec![
                PropertySection::new("Transform")
                    .with_row(PropertyDesc::new("X", PropertyValue::Number(10.0)))
                    .with_row(
                        PropertyDesc::new("Y", PropertyValue::Number(20.0))
                            .with_default(PropertyValue::Number(0.0)),
                    ),
                PropertySection::new("Appearance")
                    .with_row(PropertyDesc::new("Name", PropertyValue::Text("Box".into()))),
            ],
        )
        .with_label("Inspector")
    }

    #[test]
    fn modification_is_a_comparison_with_the_default() {
        let desc = desc();
        assert!(!desc.sections[0].rows[0].is_modified());
        assert!(desc.sections[0].rows[1].is_modified());
    }

    #[test]
    fn numbers_include_the_unit() {
        let property = PropertyDesc::new("Width", PropertyValue::Number(12.5)).with_kind(
            PropertyKind::Number {
                unit: "px".to_owned(),
            },
        );
        assert_eq!(property.value_text(), "12.5 px");
    }

    #[test]
    fn collapsing_a_section_skips_its_rows() {
        let desc = desc();
        let mut state = PropertyGridState::new();
        let expanded = state.layout(&desc);
        assert_eq!(expanded.len(), 3, "three rows");
        state.toggle_section(0);
        let collapsed = state.layout(&desc);
        assert_eq!(
            collapsed.len(),
            2,
            "collapsed header plus the other section"
        );
        assert_eq!(collapsed[0].row, usize::MAX);
    }

    #[test]
    fn rows_are_stacked_under_their_header() {
        let desc = desc();
        let state = PropertyGridState::new();
        let layout = state.layout(&desc);
        let first = layout[0];
        assert_eq!(first.rect.origin.y, desc.header_height);
        assert_eq!(
            layout[1].rect.origin.y,
            desc.header_height + desc.row_height
        );
        assert_eq!(
            layout[2].header.map(|header| header.origin.y),
            Some(desc.header_height + desc.row_height * 2.0)
        );
    }

    #[test]
    fn reset_requests_are_drained_once() {
        let mut state = PropertyGridState::new();
        state.reset_row(0, 1);
        assert_eq!(state.take_reset(), Some((0, 1)));
        assert_eq!(state.take_reset(), None);
    }

    #[test]
    fn the_value_cell_starts_after_the_label_column() {
        let desc = desc();
        let state = PropertyGridState::new();
        let row = state.layout(&desc)[1];
        assert_eq!(row.label_rect.size.x, 140.0);
        assert_eq!(row.value_rect.origin.x, 140.0);
        assert!(row.reset_rect.max().x <= desc.rect.max().x);
    }

    #[test]
    fn the_emitter_reports_a_group_with_the_row_count() {
        let desc = desc();
        let state = PropertyGridState::new();
        let output = emit_property_grid(9, &desc, &state, &[], ControlState::default());
        let a11y = output.a11y.expect("group");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.description, "3 个属性");
        assert!(output.hit.is_some());
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let mut counts = Vec::new();
        for hover in [None, Some((0, 1))] {
            let mut state = PropertyGridState::new();
            state.hover(hover);
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_property_grid(9, &desc, &state, &[], control);
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

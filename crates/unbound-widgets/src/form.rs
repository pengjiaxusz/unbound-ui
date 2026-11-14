//! 表单布局、校验聚合与“跳转至首个错误”。
//!
//! 表单不是单个控件，而是标签、输入框与提示消息的*组合*。本模块负责三项真正共享的逻辑：
//!
//! * 每个标签、输入框与消息的放置位置（[`layout_form`]），
//! * 哪个字段校验失败以及错误消息是什么（[`FormState`]），
//! * 将首个失败字段滚动到视野内并聚焦。
//!
//! 校验在结构上天然防循环依赖：reducer 接收的是校验结果而非控件本身，
//! 因此依赖其他字段值的交叉校验只需编写一个应用级闭包即可。

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{instance, surface};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::WidgetOutput;

/// 标准输入行高度。
pub const FORM_INPUT_HEIGHT: f32 = 36.0;
/// 标签或消息内单行文本的高度。
pub const FORM_LINE: f32 = 18.0;
/// 标签与输入框之间的间距。
pub const FORM_LABEL_GAP: f32 = 6.0;
/// 字段之间的间距。
pub const FORM_FIELD_GAP: f32 = 18.0;
/// 输入框与消息之间的间距。
pub const FORM_MESSAGE_GAP: f32 = 4.0;

/// 标签相对于输入框的位置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormLayout {
    /// 标签位于输入框上方。
    #[default]
    Vertical,
    /// 标签位于左侧固定列中。
    Horizontal,
}

/// 单个字段的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct FormFieldDesc {
    /// 字段标签。
    pub label: String,
    /// 字段是否必填。
    pub required: bool,
    /// 无错误时显示的简短帮助文本。
    pub help: String,
    /// 该字段使用的布局方式。
    pub layout: FormLayout,
    /// 输入控件高度。
    pub input_height: f32,
}

impl FormFieldDesc {
    /// 创建标准高度的垂直布局字段。
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            required: false,
            help: String::new(),
            layout: FormLayout::Vertical,
            input_height: FORM_INPUT_HEIGHT,
        }
    }

    /// 将字段标记为必填。
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// 附加帮助文本。
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }
}

/// 字段及其稳定标识。
#[derive(Clone, Debug, PartialEq)]
pub struct FormField {
    /// 应用程序用于关联校验结果的标识。
    pub key: u64,
    /// 字段展示描述符。
    pub desc: FormFieldDesc,
}

impl FormField {
    /// 创建带标识的字段。
    #[must_use]
    pub fn new(key: u64, desc: FormFieldDesc) -> Self {
        Self { key, desc }
    }
}

/// 表单的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct FormDesc {
    /// 表单内容矩形。
    pub rect: Rect,
    /// 水平布局时的标签列宽度。
    pub label_width: f32,
    /// 字段间距。
    pub spacing: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl FormDesc {
    /// 使用默认度量的表单。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label_width: 110.0,
            spacing: FORM_FIELD_GAP,
            z: 720,
        }
    }
}

/// 单个字段测量后的几何布局。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormFieldLayout {
    /// 字段标识。
    pub key: u64,
    /// 标签矩形。
    pub label: Rect,
    /// 预留的输入框矩形。
    pub input: Rect,
    /// 消息矩形（帮助文本或错误提示）。
    pub message: Rect,
}

/// 从上到下布局所有字段。
///
/// 只要字段带有帮助文本就会预留消息行；出错时错误提示直接替换帮助行，
/// 而不会把表单其余部分向下推移，从而防止校验通过/失败时布局发生跳动。
#[must_use]
pub fn layout_form(desc: &FormDesc, fields: &[FormField]) -> Vec<FormFieldLayout> {
    let mut y = desc.rect.origin.y;
    let mut layouts = Vec::with_capacity(fields.len());
    for field in fields {
        let message_height = if field.desc.help.is_empty() {
            0.0
        } else {
            FORM_LINE + FORM_MESSAGE_GAP
        };
        let (label, input, message) = match field.desc.layout {
            FormLayout::Vertical => {
                let label = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, FORM_LINE);
                let input = Rect::new(
                    desc.rect.origin.x,
                    label.max().y + FORM_LABEL_GAP,
                    desc.rect.size.x,
                    field.desc.input_height,
                );
                let message = Rect::new(
                    desc.rect.origin.x,
                    input.max().y + FORM_MESSAGE_GAP,
                    desc.rect.size.x,
                    FORM_LINE,
                );
                (label, input, message)
            }
            FormLayout::Horizontal => {
                let label = Rect::new(
                    desc.rect.origin.x,
                    y + (field.desc.input_height - FORM_LINE) * 0.5,
                    desc.label_width,
                    FORM_LINE,
                );
                let input = Rect::new(
                    desc.rect.origin.x + desc.label_width,
                    y,
                    (desc.rect.size.x - desc.label_width).max(0.0),
                    field.desc.input_height,
                );
                let message = Rect::new(
                    input.origin.x,
                    input.max().y + FORM_MESSAGE_GAP,
                    input.size.x,
                    FORM_LINE,
                );
                (label, input, message)
            }
        };
        y = input.max().y + message_height + desc.spacing;
        layouts.push(FormFieldLayout {
            key: field.key,
            label,
            input,
            message,
        });
    }
    layouts
}

/// 表单的实时校验状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormState {
    errors: FxHashMap<u64, String>,
    focused: Option<u64>,
}

impl FormState {
    /// 创建无错误的表单状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取字段的错误消息（若校验失败）。
    #[must_use]
    pub fn error(&self, key: u64) -> Option<&str> {
        self.errors.get(&key).map(String::as_str)
    }

    /// 应当持有焦点的字段。
    #[must_use]
    pub fn focused(&self) -> Option<u64> {
        self.focused
    }

    /// 所有字段是否全部通过校验。
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// 记录或清除单个字段的错误。
    pub fn set_error(&mut self, key: u64, message: Option<String>) -> bool {
        match message.filter(|message| !message.is_empty()) {
            Some(message) => {
                let previous = self.errors.insert(key, message.clone());
                previous.as_deref() != Some(message.as_str())
            }
            None => self.errors.remove(&key).is_some(),
        }
    }

    /// 用一次校验的结果替换所有错误。
    ///
    /// 按表单顺序返回首个失败字段并使其获得焦点。表单顺序即调用方传入顺序，
    /// 也是视觉顺序，因此“首个错误”指“最上方的错误”，而非“恰好先失败的字段”。
    pub fn validate<I>(&mut self, results: I) -> Option<u64>
    where
        I: IntoIterator<Item = (u64, Option<String>)>,
    {
        self.errors.clear();
        let mut first = None;
        for (key, message) in results {
            if let Some(message) = message.filter(|message| !message.is_empty()) {
                self.errors.insert(key, message);
                first.get_or_insert(key);
            }
        }
        self.focused = first;
        first
    }

    /// 聚焦指定字段。
    pub fn focus(&mut self, key: u64) {
        self.focused = Some(key);
    }

    /// 在 `viewport` 中显露 `field` 所需的垂直滚动偏移量。
    #[must_use]
    pub fn reveal_offset(viewport: Rect, field: Rect) -> f32 {
        if field.origin.y < viewport.origin.y {
            field.origin.y - viewport.origin.y
        } else if field.max().y > viewport.max().y {
            field.max().y - viewport.max().y
        } else {
            0.0
        }
    }
}

/// 将 `field` 滚动到视野内，返回偏移量是否发生变化。
pub fn reveal_field(
    scroll: &mut ScrollState,
    viewport: Rect,
    field: Rect,
    content_height: f32,
) -> bool {
    let delta = FormState::reveal_offset(viewport, field);
    if delta.abs() <= f32::EPSILON {
        return false;
    }
    let desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
    let before = scroll.target().y;
    scroll.scroll_by(&desc, Vec2::new(0.0, delta));
    (scroll.target().y - before).abs() > f32::EPSILON
}

/// 线程安全自定义字段校验器闭包的类型别名。
pub type CustomValidatorFn = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// [`FormValidator`] 的声明式校验规则。
#[derive(Clone)]
pub enum ValidationRule {
    /// 值（去除首尾空白后）不能为空。
    Required { message: String },
    /// 字符长度上下界。
    Length {
        min: Option<usize>,
        max: Option<usize>,
        message: String,
    },
    /// `f64` 数值上下界。
    Range {
        min: Option<f64>,
        max: Option<f64>,
        message: String,
    },
    /// 必须包含的子串/模式匹配。
    Pattern { needle: String, message: String },
    /// 自定义校验闭包，失败时返回 `Some(error)`。
    Custom(CustomValidatorFn),
}

impl std::fmt::Debug for ValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Required { message } => f
                .debug_struct("Required")
                .field("message", message)
                .finish(),
            Self::Length { min, max, message } => f
                .debug_struct("Length")
                .field("min", min)
                .field("max", max)
                .field("message", message)
                .finish(),
            Self::Range { min, max, message } => f
                .debug_struct("Range")
                .field("min", min)
                .field("max", max)
                .field("message", message)
                .finish(),
            Self::Pattern { needle, message } => f
                .debug_struct("Pattern")
                .field("needle", needle)
                .field("message", message)
                .finish(),
            Self::Custom(_) => f.debug_tuple("Custom").field(&"<fn>").finish(),
        }
    }
}

/// 表单字段的可组合校验器链。
#[derive(Clone, Debug, Default)]
pub struct FormValidator {
    rules: Vec<ValidationRule>,
}

impl FormValidator {
    /// 创建空校验器（放行所有输入）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 要求去除空白后输入非空。
    #[must_use]
    pub fn required(mut self, message: impl Into<String>) -> Self {
        self.rules.push(ValidationRule::Required {
            message: message.into(),
        });
        self
    }

    /// 要求最小字符长度。
    #[must_use]
    pub fn min_len(mut self, min: usize, message: impl Into<String>) -> Self {
        self.rules.push(ValidationRule::Length {
            min: Some(min),
            max: None,
            message: message.into(),
        });
        self
    }

    /// 要求最大字符长度。
    #[must_use]
    pub fn max_len(mut self, max: usize, message: impl Into<String>) -> Self {
        self.rules.push(ValidationRule::Length {
            min: None,
            max: Some(max),
            message: message.into(),
        });
        self
    }

    /// 要求数值位于 `[min, max]` 区间内。
    #[must_use]
    pub fn range(mut self, min: Option<f64>, max: Option<f64>, message: impl Into<String>) -> Self {
        self.rules.push(ValidationRule::Range {
            min,
            max,
            message: message.into(),
        });
        self
    }

    /// 要求输入包含 `needle`。
    #[must_use]
    pub fn pattern(mut self, needle: impl Into<String>, message: impl Into<String>) -> Self {
        self.rules.push(ValidationRule::Pattern {
            needle: needle.into(),
            message: message.into(),
        });
        self
    }

    /// 添加自定义校验函数。
    #[must_use]
    pub fn custom(mut self, f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        self.rules
            .push(ValidationRule::Custom(std::sync::Arc::new(f)));
        self
    }

    /// 按顺序评估所有规则，返回第一条失败消息。
    #[must_use]
    pub fn validate_value(&self, value: &str) -> Option<String> {
        for rule in &self.rules {
            match rule {
                ValidationRule::Required { message } => {
                    if value.trim().is_empty() {
                        return Some(message.clone());
                    }
                }
                ValidationRule::Length { min, max, message } => {
                    let len = value.chars().count();
                    if min.is_some_and(|m| len < m) || max.is_some_and(|m| len > m) {
                        return Some(message.clone());
                    }
                }
                ValidationRule::Range { min, max, message } => {
                    let Ok(parsed) = value.trim().parse::<f64>() else {
                        return Some(message.clone());
                    };
                    if min.is_some_and(|m| parsed < m) || max.is_some_and(|m| parsed > m) {
                        return Some(message.clone());
                    }
                }
                ValidationRule::Pattern { needle, message } => {
                    if !value.contains(needle.as_str()) {
                        return Some(message.clone());
                    }
                }
                ValidationRule::Custom(f) => {
                    if let Some(err) = f(value) {
                        return Some(err);
                    }
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
struct ManagedField {
    key: u64,
    initial: String,
    value: String,
    touched: bool,
    dirty: bool,
    validator: FormValidator,
}

/// 有状态表单控制器，跟踪字段值、`touched`/`dirty` 标记、校验以及提交时的首个错误聚焦与滚动显露。
#[derive(Clone, Debug, Default)]
pub struct FormController {
    fields: Vec<ManagedField>,
    state: FormState,
    submitted: bool,
}

impl FormController {
    /// 创建新的空 [`FormController`]。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册（或更新）字段及其初始值与校验器。
    pub fn register_field(
        &mut self,
        key: u64,
        initial: impl Into<String>,
        validator: FormValidator,
    ) {
        let initial = initial.into();
        if let Some(existing) = self.fields.iter_mut().find(|f| f.key == key) {
            existing.validator = validator;
            return;
        }
        self.fields.push(ManagedField {
            key,
            initial: initial.clone(),
            value: initial,
            touched: false,
            dirty: false,
            validator,
        });
    }

    /// 更新字段值，更新 `dirty` 标记，并在已触碰或已提交时重新校验。
    pub fn set_value(&mut self, key: u64, value: impl Into<String>) {
        let value = value.into();
        if let Some(field) = self.fields.iter_mut().find(|f| f.key == key) {
            field.dirty = value != field.initial;
            field.value = value;
            if field.touched || self.submitted {
                let err = field.validator.validate_value(&field.value);
                self.state.set_error(key, err);
            }
        }
    }

    /// 将字段标记为已触碰（例如失焦时）并运行其校验器。
    pub fn mark_touched(&mut self, key: u64) {
        if let Some(field) = self.fields.iter_mut().find(|f| f.key == key) {
            field.touched = true;
            let err = field.validator.validate_value(&field.value);
            self.state.set_error(key, err);
        }
    }

    /// 获取 `key` 的当前值。
    #[must_use]
    pub fn value(&self, key: u64) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
    }

    /// 判断 `key` 是否已被触碰。
    #[must_use]
    pub fn is_touched(&self, key: u64) -> bool {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .is_some_and(|f| f.touched)
    }

    /// 判断 `key` 是否与初始值不同。
    #[must_use]
    pub fn is_dirty(&self, key: u64) -> bool {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .is_some_and(|f| f.dirty)
    }

    /// 判断表单中是否存在任何已修改字段。
    #[must_use]
    pub fn any_dirty(&self) -> bool {
        self.fields.iter().any(|f| f.dirty)
    }

    /// 将所有字段重置为初始值，清除错误与 `dirty`/`touched` 标记。
    pub fn reset(&mut self) {
        self.submitted = false;
        for field in &mut self.fields {
            field.value = field.initial.clone();
            field.touched = false;
            field.dirty = false;
        }
        let empty: [(u64, Option<String>); 0] = [];
        self.state.validate(empty);
    }

    /// 借用底层 [`FormState`] 用于渲染与错误查询。
    #[must_use]
    pub fn state(&self) -> &FormState {
        &self.state
    }

    /// 可变借用底层 [`FormState`]。
    pub fn state_mut(&mut self) -> &mut FormState {
        &mut self.state
    }

    /// 按顺序校验所有字段，聚焦首个错误并可选将其滚动到视野内。
    pub fn submit(
        &mut self,
        layouts: &[FormFieldLayout],
        scroll: Option<(&mut ScrollState, Rect, f32)>,
    ) -> Result<(), u64> {
        self.submitted = true;
        let mut results = Vec::with_capacity(self.fields.len());
        for field in &mut self.fields {
            field.touched = true;
            results.push((field.key, field.validator.validate_value(&field.value)));
        }
        if let Some(first_err) = self.state.validate(results) {
            if let Some((scroll_state, viewport, content_height)) = scroll {
                if let Some(layout) = layouts.iter().find(|l| l.key == first_err) {
                    reveal_field(scroll_state, viewport, layout.input, content_height);
                }
            }
            Err(first_err)
        } else {
            Ok(())
        }
    }
}

/// 以双图元圆点绘制的必填标记：无需字形。
#[must_use]
pub fn required_marker(rect: Rect, z: u16) -> Vec<GpuPrimitiveInstance> {
    vec![
        instance(
            Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.y * 0.5 - 2.0,
                4.0,
                4.0,
            ),
            [2.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Content,
            z,
        ),
        instance(
            Rect::new(
                rect.origin.x + 2.0,
                rect.origin.y + rect.size.y - 1.0,
                2.0,
                1.0,
            ),
            [0.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            z,
        ),
    ]
}

/// 表单的语义根节点。
///
/// 屏幕阅读器会先播报一次表单分组再逐个播报带标签的子控件，
/// 因此表单自身是无命中区域、无焦点槽位的 `Form` 节点。
#[must_use]
pub fn emit_form_root(rect: Rect, label: impl Into<String>) -> WidgetOutput {
    WidgetOutput {
        a11y: Some(A11yNode::new(Role::Form, label.into(), rect)),
        ..WidgetOutput::default()
    }
}

/// 发射字段标签及其必填标记。
#[must_use]
pub fn emit_form_label(
    field: &FormField,
    layout: &FormFieldLayout,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());
    if field.desc.required {
        primitives.extend(required_marker(layout.label, 0));
    }
    if !glyphs.is_empty() {
        crate::primitive::glyphs(
            glyphs,
            Token::TEXT_SECONDARY,
            0,
            &mut primitives,
            &mut payloads,
        );
    }
    let mut a11y = A11yNode::new(Role::Label, field.desc.label.clone(), layout.label);
    a11y.description = field.desc.help.clone();
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

/// 发射单个字段的消息行。
///
/// 错误消息带有填充表面与错误色下划线；帮助文本保持扁平。调用方的字形会据此着色。
#[must_use]
pub fn emit_form_message(
    field: &FormField,
    layout: &FormFieldLayout,
    error: Option<&str>,
    glyphs: &mut [GlyphQuad],
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());
    let has_message = error.is_some() || !field.desc.help.is_empty();
    if has_message {
        primitives.push(surface(
            layout.message,
            0.0,
            if error.is_some() {
                Token::ERROR_SOFT
            } else {
                Token::NONE
            },
            z,
        ));
    }
    if error.is_some() {
        primitives.push(instance(
            Rect::new(
                layout.message.origin.x,
                layout.message.origin.y + 1.0,
                2.0,
                layout.message.size.y - 2.0,
            ),
            [1.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Content,
            z,
        ));
    }
    crate::primitive::glyphs(
        glyphs,
        if error.is_some() {
            Token::ERROR
        } else {
            Token::TEXT_SECONDARY
        },
        z,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(
        Role::Label,
        error.unwrap_or(&field.desc.help).to_owned(),
        layout.message,
    );
    a11y.description = field.desc.label.clone();
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<FormField> {
        vec![
            FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            ),
            FormField::new(2, FormFieldDesc::new("Email").required(true)),
        ]
    }

    #[test]
    fn vertical_fields_stack_labels_inputs_and_messages() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let layouts = layout_form(&desc, &fields());
        assert_eq!(layouts.len(), 2);
        assert!(layouts[0].label.max().y <= layouts[0].input.origin.y);
        assert!(layouts[0].message.origin.y > layouts[0].input.max().y);
        assert!(layouts[1].label.origin.y > layouts[0].message.origin.y);
    }

    #[test]
    fn horizontal_fields_put_the_label_in_a_column() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let mut field = FormFieldDesc::new("Weight");
        field.layout = FormLayout::Horizontal;
        let layouts = layout_form(&desc, &[FormField::new(1, field)]);
        assert_eq!(layouts[0].label.size.x, desc.label_width);
        assert_eq!(
            layouts[0].input.origin.x,
            desc.rect.origin.x + desc.label_width
        );
        assert!((layouts[0].input.size.x + desc.label_width - desc.rect.size.x).abs() < 1e-5);
    }

    #[test]
    fn validation_reports_and_focuses_the_first_failure() {
        let mut state = FormState::new();
        let first = state.validate([
            (1, None),
            (2, Some("required".into())),
            (3, Some("too long".into())),
        ]);
        assert_eq!(first, Some(2));
        assert_eq!(state.focused(), Some(2));
        assert_eq!(state.error(2), Some("required"));
        assert!(!state.is_valid());
        assert_eq!(state.error(1), None);
    }

    #[test]
    fn a_clean_pass_clears_previous_errors() {
        let mut state = FormState::new();
        state.validate([(1, Some("bad".into()))]);
        assert!(!state.is_valid());
        assert_eq!(state.validate([(1, None)]), None);
        assert!(state.is_valid());
        assert_eq!(state.focused(), None);
    }

    #[test]
    fn an_empty_message_counts_as_valid() {
        let mut state = FormState::new();
        assert_eq!(state.validate([(1, Some(String::new()))]), None);
        assert!(state.is_valid());
    }

    #[test]
    fn reveal_offsets_only_when_the_field_is_outside() {
        let viewport = Rect::new(0.0, 100.0, 300.0, 200.0);
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 120.0, 300.0, 40.0)),
            0.0
        );
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 40.0, 300.0, 40.0)),
            -60.0
        );
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 320.0, 300.0, 40.0)),
            60.0
        );
    }

    #[test]
    fn revealing_scrolls_a_spring_toward_the_field() {
        let viewport = Rect::new(0.0, 100.0, 300.0, 200.0);
        let mut scroll = ScrollState::new();
        assert!(reveal_field(
            &mut scroll,
            viewport,
            Rect::new(0.0, 400.0, 300.0, 40.0),
            1000.0
        ));
        assert!(scroll.target().y > 0.0);
        assert!(!reveal_field(
            &mut scroll,
            viewport,
            Rect::new(0.0, 150.0, 300.0, 40.0),
            1000.0
        ));
    }

    #[test]
    fn the_required_marker_has_two_primitives() {
        assert_eq!(
            required_marker(Rect::new(0.0, 0.0, 100.0, 18.0), 5).len(),
            2
        );
    }

    #[test]
    fn the_form_root_is_a_form_node() {
        let root = emit_form_root(Rect::new(0.0, 0.0, 320.0, 400.0), "Account");
        let a11y = root.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Form);
        assert_eq!(a11y.label, "Account");
        assert!(root.primitives.is_empty());
    }

    #[test]
    fn labels_and_messages_are_reachable() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let fields = fields();
        let layouts = layout_form(&desc, &fields);
        let label = emit_form_label(&fields[0], &layouts[0], &mut []);
        assert_eq!(label.a11y.expect("reachable").role, Role::Label);
        assert_eq!(label.primitives.len(), 2);

        let message = emit_form_message(&fields[0], &layouts[0], Some("Required"), &mut [], 0);
        let a11y = message.a11y.expect("reachable");
        assert_eq!(a11y.label, "Required");
        assert_eq!(message.primitives.len(), 2, "error surface and bar");
    }
}

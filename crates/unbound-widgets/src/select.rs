//! 下拉选择控件，由气泡框与菜单图元组合而成。
//!
//! 选择控件本身不持有菜单：[`SelectDesc::menu`] 在输入框下方构建一个
//! 锚定 [`MenuDesc`]，调用方再将生成的条目推入 [`crate::OverlayStack`]。
//! 这使系统内所有弹窗的定位与关闭逻辑统一在一处管理。
//!
//!
//! 键盘交互符合常规预期：`Up`/`Down` 移动高亮，`Enter` 确认选择，`Escape` 取消并关闭。
//!

use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::menu::{MenuItem, MenuState};
use crate::overlay::place_anchored_popup;
use crate::primitive::{focus_ring, instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 选择框的命中测试层。
pub const SELECT_LAYER: u8 = 1;
/// 单个选项行的高度。
pub const SELECT_ROW_HEIGHT: f32 = 30.0;
const CHEVRON_HALF_WIDTH: f32 = 4.0;
/// 展开/收起箭头的半高。
const CHEVRON_HALF_HEIGHT: f32 = 3.0;
/// 展开/收起箭头的描边宽度。
const CHEVRON_STROKE: f32 = 1.5;

/// 下拉选择框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SelectDesc {
    /// 收起状态下的输入框矩形。
    pub rect: Rect,
    /// 选项标签列表。
    pub options: Vec<String>,
    /// 无障碍标签。
    pub label: String,
    /// 未选中任何项时显示的占位文本。
    pub placeholder: String,
    /// 禁止选中的选项索引列表。
    pub disabled_options: Vec<usize>,
    /// 供声明式构建器使用的可选初始选中索引。
    pub initial_selection: Option<usize>,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl SelectDesc {
    #[must_use]
    pub fn new(rect: Rect, options: Vec<String>) -> Self {
        Self {
            rect,
            options,
            label: String::new(),
            placeholder: String::new(),
            disabled_options: Vec::new(),
            initial_selection: None,
            radius: 8.0,
            z: 730,
        }
    }

    /// 设置未选中时显示的占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 禁用指定选项。
    #[must_use]
    pub fn with_disabled_option(mut self, index: usize) -> Self {
        if !self.disabled_options.contains(&index) {
            self.disabled_options.push(index);
        }
        self
    }

    /// 查询某个选项是否被禁用。
    #[must_use]
    pub fn option_disabled(&self, index: usize) -> bool {
        self.disabled_options.contains(&index)
    }

    /// 收起状态下显示的文本。
    #[must_use]
    pub fn display_text<'a>(&'a self, state: &SelectState) -> &'a str {
        self.options
            .get(state.selected.unwrap_or(usize::MAX))
            .map_or(self.placeholder.as_str(), String::as_str)
    }

    /// 显示文本的颜色槽位。
    ///
    /// 占位文本使用次要文本色，真实选中项使用主要文本色。
    /// `state.selected` 是唯一依据而非判断标签是否为空，
    /// 从而避免标签本身为 `""` 的选项被误当作占位文本。
    #[must_use]
    pub fn text_token(&self, state: &SelectState, control: ControlState) -> Token {
        if control.disabled {
            Token::TEXT_DISABLED
        } else if state.selected.is_some() {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        }
    }

    /// 打开下拉列表时对应的菜单描述符。
    #[must_use]
    pub fn menu(&self, viewport: Rect) -> crate::menu::MenuDesc {
        let size = Vec2::new(
            self.rect.width(),
            self.options.len() as f32 * SELECT_ROW_HEIGHT,
        );
        let placed = place_anchored_popup(
            self.rect,
            size,
            viewport,
            crate::overlay::Placement::BottomStart,
        );
        let mut menu = crate::menu::MenuDesc::new(
            placed,
            self.options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    let item = MenuItem::action(option.clone());
                    if self.option_disabled(index) {
                        item.disabled()
                    } else {
                        item
                    }
                })
                .collect(),
        );
        menu.z = self.z + 1;
        menu.viewport = Some(viewport);
        menu.clamp_height();
        menu
    }
}

/// 选择框的开关与选中状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectState {
    /// 下拉列表是否处于打开状态。
    pub open: bool,
    /// 当前选中的选项索引。
    pub selected: Option<usize>,
    /// 打开状态下当前高亮的选项索引。
    pub highlighted: Option<usize>,
}

impl SelectState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 打开下拉列表，并高亮当前选中项。
    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = self.selected;
    }

    /// 不做选择直接关闭。
    pub fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
    }

    /// 切换下拉列表开关状态。
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// 移动高亮索引，到达两端时循环并跳过禁用选项。
    ///
    /// 当所有选项均被禁用时，高亮保持原位。
    pub fn step(&mut self, desc: &SelectDesc, forward: bool) {
        let count = desc.options.len();
        if count == 0 {
            return;
        }
        let start = self
            .highlighted
            .unwrap_or(if forward { count - 1 } else { 0 });
        for offset in 1..=count {
            let index = if forward {
                (start + offset) % count
            } else {
                (start + count - offset) % count
            };
            if !desc.option_disabled(index) {
                self.highlighted = Some(index);
                return;
            }
        }
    }

    /// 提交当前高亮项，返回选中的索引。
    pub fn commit(&mut self) -> Option<usize> {
        let index = self.highlighted?;
        self.selected = Some(index);
        self.close();
        Some(index)
    }

    /// 获取当前选中项的标签。
    #[must_use]
    pub fn label<'a>(&self, desc: &'a SelectDesc) -> Option<&'a str> {
        desc.options
            .get(self.selected?)
            .map(std::string::String::as_str)
    }

    /// 打开下拉列表时的菜单高亮状态。
    #[must_use]
    pub fn menu_state(&self) -> MenuState {
        MenuState {
            highlighted: self.highlighted,
            ..MenuState::default()
        }
    }
}

/// 绑定了节点标识的选择框描述符。
#[derive(Clone, Debug)]
pub struct SelectWidget {
    /// 控件标识。
    pub node: u64,
    /// 描述符。
    pub desc: SelectDesc,
    /// 状态。
    pub state: SelectState,
}

impl WidgetDesc for SelectWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_select_with_glyphs(self.node, &self.desc, &self.state, state, glyphs)
    }
}

/// 输出收起状态的输入框：填充、边框、展开箭头、调用方字形与焦点环。
#[must_use]
pub fn emit_select(
    node: u64,
    desc: &SelectDesc,
    state: &SelectState,
    control: ControlState,
) -> WidgetOutput {
    emit_select_with_glyphs(node, desc, state, control, &[])
}

/// 输出带有已塑形标签字形的收起选择框。
///
/// 字形使用 [`SelectDesc::text_token`] 着色，因此空选择框会像文本输入框一样
/// 使用 [`Token::TEXT_SECONDARY`] 绘制占位文本。
#[must_use]
pub fn emit_select_with_glyphs(
    node: u64,
    desc: &SelectDesc,
    state: &SelectState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let fill = if control.disabled {
        Token::DISABLED
    } else if state.open || control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_RAISED
    };
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(rect, desc.radius, fill, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads: Vec<GpuPayload> = Vec::new();
    // 展开箭头：在顶点交汇的两条线段，收起时朝下，打开时朝上。
    //
    let apex_y = if state.open {
        -CHEVRON_HALF_HEIGHT
    } else {
        CHEVRON_HALF_HEIGHT
    };
    let center = Vec2::new(rect.max().x - 16.0, rect.origin.y + rect.height() * 0.5);
    let left = Vec2::new(center.x - CHEVRON_HALF_WIDTH, center.y - apex_y);
    let apex = Vec2::new(center.x, center.y + apex_y);
    let right = Vec2::new(center.x + CHEVRON_HALF_WIDTH, center.y - apex_y);
    for (a, b) in [(left, apex), (apex, right)] {
        let (primitive, payload) = chevron_segment(a, b, Token::TEXT_SECONDARY, desc.z);
        primitives.push(primitive);
        payloads.push(payload);
    }

    let ink = desc.text_token(state, control);
    for quad in glyphs {
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
        let mut glyph = instance(
            quad.rect,
            [0.0; 4],
            ink,
            Token::NONE,
            0.0,
            0.0,
            if quad.color {
                PrimType::GlyphColor
            } else {
                PrimType::Glyph
            },
            SubElement::Content,
            desc.z,
        );
        glyph.border_token_or_tex = quad.layer;
        glyph.extra_payload_idx = payload_idx;
        if quad.msdf {
            glyph.prim_type_and_flags |= u32::from(unbound_gpu::data::PrimFlags::MSDF.bits()) << 16;
        }
        primitives.push(glyph);
    }

    primitives.push(focus_ring(
        rect.inflate(3.0),
        desc.radius + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, SELECT_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });
    let display = desc.display_text(state);
    let mut a11y = unbound_a11y::A11yNode::new(
        accesskit::Role::ComboBox,
        if desc.label.is_empty() {
            display.to_owned()
        } else {
            desc.label.clone()
        },
        rect,
    )
    .clickable()
    .disabled(control.disabled);
    a11y.value = display.to_owned();
    a11y.actions.insert(unbound_a11y::A11yAction::Expand);
    a11y.actions.insert(unbound_a11y::A11yAction::Collapse);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 展开箭头的单条二次曲线描边。
fn chevron_segment(a: Vec2, b: Vec2, color: Token, z: u16) -> (GpuPrimitiveInstance, GpuPayload) {
    let origin = Vec2::new(a.x.min(b.x), a.y.min(b.y));
    let size = Vec2::new((a.x - b.x).abs().max(1.0), (a.y - b.y).abs().max(1.0));
    let primitive = instance(
        Rect::new(origin.x, origin.y, size.x, size.y),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    let payload = GpuPayload::curve(
        [a.x - origin.x, a.y - origin.y],
        [(a.x + b.x) * 0.5 - origin.x, (a.y + b.y) * 0.5 - origin.y],
        [b.x - origin.x, b.y - origin.y],
        CHEVRON_STROKE,
    );
    (primitive, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SelectDesc {
        SelectDesc::new(
            Rect::new(20.0, 20.0, 200.0, 36.0),
            vec!["Apple".into(), "Banana".into(), "Cherry".into()],
        )
    }

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 600.0)
    }

    #[test]
    fn keyboard_navigation_wraps_and_commits() {
        let desc = desc();
        let mut state = SelectState::new();
        state.open();
        state.step(&desc, true);
        assert_eq!(state.highlighted, Some(0));
        state.step(&desc, true);
        assert_eq!(state.highlighted, Some(1));
        assert_eq!(state.commit(), Some(1));
        assert_eq!(state.selected, Some(1));
        assert!(!state.open);
        assert_eq!(state.label(&desc), Some("Banana"));
    }

    #[test]
    fn escape_closes_without_changing_the_selection() {
        let desc = desc();
        let mut state = SelectState::new();
        state.selected = Some(2);
        state.open();
        assert_eq!(state.highlighted, Some(2));
        state.close();
        assert_eq!(state.selected, Some(2));
        assert_eq!(state.label(&desc), Some("Cherry"));
    }

    #[test]
    fn the_dropdown_is_placed_under_the_field_and_inside_the_viewport() {
        let desc = desc();
        let menu = desc.menu(viewport());
        assert_eq!(menu.rect.origin.x, 20.0);
        assert_eq!(menu.rect.origin.y, 60.0);
        assert_eq!(menu.rect.height(), 90.0);
        assert!(viewport().contains_rect(&menu.rect));
    }

    #[test]
    fn a_dropdown_near_the_bottom_flips_up() {
        let mut desc = desc();
        desc.rect = Rect::new(20.0, 540.0, 200.0, 36.0);
        let menu = desc.menu(viewport());
        assert!(menu.rect.max().y <= desc.rect.origin.y);
        assert!(viewport().contains_rect(&menu.rect));
    }

    #[test]
    fn the_closed_field_is_pickable_and_exposes_combobox_semantics() {
        let desc = desc();
        let mut state = SelectState::new();
        state.selected = Some(1);
        let output = emit_select(4, &desc, &state, ControlState::default());
        assert_eq!(output.primitives.len(), 5);
        assert!(output.hit.is_some());
        let a11y = output.a11y.as_ref().unwrap();
        assert_eq!(a11y.role, accesskit::Role::ComboBox);
        assert_eq!(a11y.label, "Banana");
        assert_eq!(a11y.value, "Banana");
        assert!(output.focusable);
    }

    #[test]
    fn a_disabled_select_is_not_pickable() {
        let desc = desc();
        let state = SelectState::new();
        let disabled = ControlState {
            disabled: true,
            ..ControlState::default()
        };
        let output = emit_select(4, &desc, &state, disabled);
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

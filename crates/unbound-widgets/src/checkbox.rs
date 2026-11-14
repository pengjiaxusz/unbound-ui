//! 复选框（Checkbox）：带有勾选标记的方框控件。

use accesskit::{Role, Toggled};
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认方框尺寸。
pub const CHECKBOX_BOX: f32 = 18.0;

/// 方框与标签之间的间距。
pub const CHECKBOX_GAP: f32 = 28.0;

/// 复选框的三种状态。
///
/// `Indeterminate` 表示“部分子节点已选中”：三态列表使用该状态，绝不能与“已选中”混淆。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    /// 方框为空（未选中）。
    #[default]
    Unchecked,
    /// 方框内带有勾选标记（已选中）。
    Checked,
    /// 方框内带有短横线（半选/不确定）。
    Indeterminate,
}

impl CheckState {
    /// 判断复选框是否处于完全选中状态。
    #[must_use]
    pub const fn is_checked(self) -> bool {
        matches!(self, Self::Checked)
    }

    /// 判断复选框是否处于混合（半选）状态。
    #[must_use]
    pub const fn is_indeterminate(self) -> bool {
        matches!(self, Self::Indeterminate)
    }

    /// 该状态映射到的 AccessKit `Toggled` 值。
    ///
    /// `Indeterminate` 必须作为 `Mixed` 传递给辅助技术，而非第二个“关闭”：
    /// 屏幕阅读器需要播报三态值。
    #[must_use]
    pub const fn toggled(self) -> Toggled {
        match self {
            Self::Unchecked => Toggled::False,
            Self::Checked => Toggled::True,
            Self::Indeterminate => Toggled::Mixed,
        }
    }
}

/// 复选框绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckboxTokens {
    /// 未选中时的方框填充色。
    pub box_empty: Token,
    /// 悬停时的方框填充色。
    pub box_hover: Token,
    /// 选中时的方框填充色。
    pub box_checked: Token,
    /// 禁用时的方框填充色。
    pub box_disabled: Token,
    /// 未选中方框的边框色。
    pub border: Token,
    /// 禁用且未选中方框的边框色。
    pub border_disabled: Token,
    /// 勾选标记颜色。
    pub mark: Token,
    /// 禁用状态下的勾选标记颜色。
    pub mark_disabled: Token,
    /// 标签文字颜色。
    pub label: Token,
    /// 禁用状态下的标签文字颜色。
    pub label_disabled: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl CheckboxTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            box_empty: Token::CARD_BG,
            box_hover: Token::CONTROL_HOVER,
            box_checked: Token::PRIMARY,
            box_disabled: Token::CONTROL_DISABLED_BG,
            border: Token::OUTLINE,
            border_disabled: Token::CONTROL_DISABLED_BORDER,
            mark: Token::ON_PRIMARY,
            mark_disabled: Token::CONTROL_DISABLED_THUMB,
            label: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 复选框的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CheckboxDesc {
    /// 复选框所占的行矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 方框状态。
    pub state: CheckState,
    /// 方框尺寸。
    pub box_size: f32,
    /// 方框与标签之间的距离。
    pub gap: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl CheckboxDesc {
    /// 使用默认度量的复选框。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            state: CheckState::Unchecked,
            box_size: CHECKBOX_BOX,
            gap: CHECKBOX_GAP,
            z: 700,
        }
    }

    /// 设置二态选中状态。
    ///
    /// 保留为布尔快捷方法以兼容既有调用方；三态调用方请使用 [`Self::with_state`]。
    #[must_use]
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.state = if checked {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        };
        self
    }

    /// 显式设置三态位置。
    #[must_use]
    pub fn with_state(mut self, state: CheckState) -> Self {
        self.state = state;
        self
    }

    /// 方框矩形：在行内左对齐且垂直居中。
    #[must_use]
    pub fn box_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.y - self.box_size) * 0.5,
            self.box_size,
            self.box_size,
        )
    }

    /// 标签矩形。
    #[must_use]
    pub fn label_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gap,
            self.rect.origin.y,
            (self.rect.size.x - self.gap).max(0.0),
            self.rect.size.y,
        )
    }
}

/// 发射复选框：包含方框、勾选标记、标签与焦点环。
///
/// 保留 `mark` 参数以维持源码兼容性；标记现由 [`CheckboxDesc::state`]
/// 直接绘制为矢量图元，因此会忽略该字形串。
#[must_use]
pub fn emit_checkbox(
    node: u64,
    desc: &CheckboxDesc,
    state: ControlState,
    _mark: &[GlyphQuad],
    label: &[GlyphQuad],
    tokens: CheckboxTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let box_rect = desc.box_rect();
    let disabled = state.disabled;

    let fill = if disabled {
        tokens.box_disabled
    } else {
        match desc.state {
            CheckState::Checked => tokens.box_checked,
            CheckState::Indeterminate => Token::INDETERMINATE_FILL,
            CheckState::Unchecked => {
                if state.phase == PointerPhase::Hovered {
                    tokens.box_hover
                } else {
                    tokens.box_empty
                }
            }
        }
    };
    let border = if desc.state == CheckState::Unchecked {
        if disabled {
            tokens.border_disabled
        } else {
            tokens.border
        }
    } else {
        Token::NONE
    };
    primitives.push(outlined(box_rect, 4.0, fill, border, 1.0, 0.0, z));

    // 标记使用几何图元而非字形：即使字体不含 `✓` 也能正确绘制复选框，
    // 且描边粗细会随方框尺寸等比缩放。
    let mark = if disabled {
        tokens.mark_disabled
    } else {
        tokens.mark
    };
    match desc.state {
        CheckState::Checked => check_mark(box_rect, mark, z, &mut primitives, &mut payloads),
        CheckState::Indeterminate => indeterminate_dash(box_rect, mark, z, &mut primitives),
        CheckState::Unchecked => {}
    }

    let label_color = if disabled {
        tokens.label_disabled
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // 焦点环始终发射：聚焦仅改变其不透明度而非池结构，
    // 因此聚焦仅是绘制属性变化，不会触发重新上传。
    primitives.push(focus_ring(
        box_rect.inflate(3.0),
        7.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = desc.state.is_checked();
    if desc.state.is_indeterminate() {
        // `A11yNode` 通过 `value` 携带开关值；具体数值映射由 [`CheckState::toggled`] 提供。
        a11y.value = "mixed".to_owned();
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 构成勾选标记的两段二次曲线。
fn check_mark(
    box_rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let x = box_rect.origin.x;
    let y = box_rect.origin.y;
    let w = box_rect.size.x;
    let h = box_rect.size.y;
    let width = (w * 0.16).max(1.5);
    let start = [x + w * 0.20, y + h * 0.52];
    let elbow = [x + w * 0.42, y + h * 0.72];
    let tip = [x + w * 0.79, y + h * 0.28];
    curve_stroke(
        [start, [x + w * 0.27, y + h * 0.63], elbow],
        width,
        color,
        z,
        primitives,
        payloads,
    );
    curve_stroke(
        [elbow, [x + w * 0.60, y + h * 0.78], tip],
        width,
        color,
        z,
        primitives,
        payloads,
    );
}

/// 单段二次曲线描边，打包进紧凑的局部包围盒。
fn curve_stroke(
    points: [[f32; 2]; 3],
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let min_x = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let min_y = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let local = points.map(|point| [point[0] - min_x, point[1] - min_y]);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(local[0], local[1], local[2], width));
    let mut primitive = instance(
        Rect::new(
            min_x,
            min_y,
            (max_x - min_x).max(0.5),
            (max_y - min_y).max(0.5),
        ),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

/// 圆角短横线，用于表示半选（混合）状态。
fn indeterminate_dash(
    box_rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let w = box_rect.size.x;
    let h = box_rect.size.y;
    let dash = Rect::new(
        box_rect.origin.x + w * 0.22,
        box_rect.origin.y + h * 0.5 - h * 0.07,
        w * 0.56,
        (h * 0.14).max(2.0),
    );
    primitives.push(outlined(
        dash,
        dash.size.y * 0.5,
        color,
        Token::NONE,
        0.0,
        0.0,
        z,
    ));
}

impl CheckboxDesc {
    /// 混合状态的无障碍语义值。
    #[must_use]
    pub const fn toggled(&self) -> Toggled {
        self.state.toggled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CheckboxDesc {
        CheckboxDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Enable telemetry")
    }

    #[test]
    fn the_box_is_left_aligned_and_centred() {
        let desc = desc();
        let rect = desc.box_rect();
        assert_eq!(rect.origin.x, 0.0);
        assert_eq!(rect.origin.y, 7.0);
        assert_eq!(rect.size.x, 18.0);
    }

    #[test]
    fn an_unchecked_box_keeps_its_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(1, &desc(), ControlState::default(), &[], &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_empty.raw() as u32
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border.raw() as u32
        );
    }

    #[test]
    fn a_checked_box_fills_and_drops_the_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_checked.raw() as u32
        );
        assert_eq!(output.primitives[0].border_token_or_tex, 0);
    }

    #[test]
    fn a_hovered_box_changes_its_fill() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_hover.raw() as u32
        );
    }

    #[test]
    fn the_semantic_node_reports_the_checked_state() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a checkbox is always reachable");
        assert_eq!(a11y.role, Role::CheckBox);
        assert!(a11y.selected);
    }

    #[test]
    fn a_disabled_checkbox_leaves_the_hit_set() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

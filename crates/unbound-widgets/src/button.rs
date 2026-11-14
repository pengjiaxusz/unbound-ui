//! 扁平按钮（Button）：首个标准控件。
//!
//! 按钮仅由三部分组成：
//!
//! * 纯数据描述符 [`ButtonDesc`]，
//! * 派生自指针路由器的视觉状态（[`ControlState`]），
//! * 纯发射器 [`emit_button`]：`(descriptor, state) -> 图元 + 命中区域 + 语义节点`。
//!
//! 交互瞬态（悬停 / 按下）绝不落入 `bool` 字段：每帧从路由器派生，
//! 因此“按下、拖出、释放”不会导致两个状态相互冲突。
//!
//! 图元通过 [`SubElement`] 分层
//! （`Shadow -> Background -> Content -> Border -> FocusRing`），而非手动调整
//! `z` 偏移，因此由 64 位排序键决定绘制顺序。

use accesskit::Role;
use slotmap::Key as _;
use unbound_a11y::A11yNode;
use unbound_abi::{NodeFlags, RawNodeDesc};
use unbound_gpu::data::{
    BlendMode, GpuPayload, GpuPrimitiveInstance, PrimFlags, PrimType, SubElement, CLIP_NONE,
};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{f16, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::{NodeId, UiWorld};

use crate::image::IconDesc;
use crate::primitive::focus_ring;
use crate::{ControlState, PointerPhase, WidgetDesc, WidgetOutput};

/// 按钮标签字体大小。字形预热必须使用相同字号，否则数字会被光栅化两次。
pub const BUTTON_FONT_SIZE: f32 = 14.0;

/// 按钮圆角半径。
pub const BUTTON_RADIUS: f32 = 10.0;

/// 标签与前置/后置图标之间的间距。
pub const BUTTON_CONTENT_GAP: f32 = 8.0;

/// 加载指示器直径。
pub const BUTTON_LOADING_SIZE: f32 = 12.0;

/// 按钮高度（逻辑像素）。
///
/// 尺寸属于元数据：已完成布局的调用方保留自身矩形，但工具栏可请求标准高度而非硬编码数值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// 紧凑操作按钮，高 28 px。
    Sm,
    /// 标准操作按钮，高 36 px。
    #[default]
    Md,
    /// 醒目操作按钮，高 44 px。
    Lg,
}

impl ButtonSize {
    /// 该尺寸对应的控件高度。
    #[must_use]
    pub const fn height(self) -> f32 {
        match self {
            Self::Sm => 28.0,
            Self::Md => 36.0,
            Self::Lg => 44.0,
        }
    }
}

/// 按钮命中区域所在的 UI 层级（`0` 画布、`1` UI、`2..` 浮层）。
pub const BUTTON_LAYER: u8 = 1;

/// 解析后阴影的模糊半径。`PrimType::Shadow` 仅绘制阴影，不绘制填充。
const BUTTON_SHADOW: f32 = 16.0;

/// 焦点环向按钮外侧延伸的距离。
const FOCUS_RING_GAP: f32 = 3.0;

/// 按钮的视觉角色。
///
/// 变体决定按钮使用*哪些*颜色槽位；主题决定这些槽位的外观。因此新增变体无需改动任何图元。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// 主行动号召：实心高对比度表面。
    #[default]
    Primary,
    /// 辅助操作：描边表面。
    Secondary,
    /// 低强调操作：无填充、无边框。
    Subtle,
    /// 紧凑的纯图标操作。
    Icon,
    /// 绘制在主视觉渐变背景上的按钮。
    OnHero,
}

impl ButtonVariant {
    /// 此变体绘制使用的颜色槽位。
    ///
    /// 每种状态都有独立槽位，因此悬停按钮不会重写实例数据——仅改变槽位解析出的颜色。
    #[must_use]
    pub fn tokens(self) -> ButtonTokens {
        match self {
            ButtonVariant::Primary => ButtonTokens {
                fill: Token::PRIMARY,
                fill_hover: Token::PRIMARY_HOVER,
                fill_pressed: Token::PRIMARY_PRESSED,
                fill_disabled: Token::DISABLED,
                border: Token::PRIMARY_PRESSED,
                label: Token::ON_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Secondary => ButtonTokens {
                fill: Token::SURFACE_RAISED,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::DISABLED,
                border: Token::OUTLINE,
                label: Token::ON_SURFACE,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Subtle => ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::ON_SURFACE_MUTED,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Icon => ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::ON_SURFACE,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::OnHero => ButtonTokens {
                fill: Token::HERO_SCRIM,
                fill_hover: Token::ON_HERO,
                fill_pressed: Token::HERO_START,
                fill_disabled: Token::HERO_SCRIM,
                border: Token::ON_HERO,
                label: Token::ON_HERO,
                label_disabled: Token::ON_HERO,
                focus_ring: Token::ON_HERO,
            },
        }
    }
}

/// 按钮绘制使用的颜色槽位。
///
/// 发射器只认槽位，主题决定颜色。每种状态在扁平颜色表中占据独立槽位，
/// 因此切换主题与悬停状态均无需重新上传任何图元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonTokens {
    /// 静息状态填充色。
    pub fill: Token,
    /// 悬停状态填充色。
    pub fill_hover: Token,
    /// 按下状态填充色。
    pub fill_pressed: Token,
    /// 禁用状态填充色。
    pub fill_disabled: Token,
    /// 轮廓线颜色。
    pub border: Token,
    /// 标签文字颜色。
    pub label: Token,
    /// 禁用状态下的标签文字颜色。
    pub label_disabled: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

/// 按钮的纯数据描述符。
///
/// 无任何可变方法，不携带闭包或类型擦除：这是 `RawNodeDesc` 收敛的目标形态，
/// 第二个控件只需编写同类结构体即可。
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonDesc {
    /// 按钮矩形区域。
    pub rect: Rect,
    /// 按钮标签文本。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 标签字号。
    pub font_size: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 视觉角色。
    pub variant: ButtonVariant,
    /// 标准尺寸（28 / 36 / 44 px）。
    pub size: ButtonSize,
    /// 绘制在标签旁的图标。
    pub icon: Option<IconDesc>,
    /// 将图标绘制在标签之后而非之前。
    pub icon_trailing: bool,
    /// 显示加载指示器并禁用交互。
    pub loading: bool,
    /// 解析投影模糊半径。`0.0` 表示不绘制阴影。
    pub shadow: f32,
    /// 按墨水边界居中标签。
    ///
    /// 若调用方已自行定位字形串（例如已在按钮矩形内完成排版），可清除此标记，
    /// 使发射器按测量位置原样绘制，而不再重新居中。
    pub centre_label: bool,
    /// 焦点环向按钮外侧延伸的距离。
    pub focus_ring_gap: f32,
}

impl ButtonDesc {
    /// 使用默认度量的主按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: BUTTON_RADIUS,
            font_size: BUTTON_FONT_SIZE,
            z: 703,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            icon: None,
            icon_trailing: false,
            loading: false,
            shadow: BUTTON_SHADOW,
            centre_label: true,
            focus_ring_gap: FOCUS_RING_GAP,
        }
    }

    /// 修改标签的同款按钮：点击计数器只需重建描述符，无需重建标识。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 切换视觉角色的同款按钮。
    #[must_use]
    pub fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// 切换标准尺寸的同款按钮。
    #[must_use]
    pub fn with_size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// 在标签旁附加图标的同款按钮。
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 将图标置于标签之后而非之前。
    #[must_use]
    pub fn with_icon_trailing(mut self, trailing: bool) -> Self {
        self.icon_trailing = trailing;
        self
    }

    /// 处于加载状态的同款按钮：不可点击，左侧显示指示器。
    #[must_use]
    pub fn with_loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// 修改阴影模糊半径的同款按钮（`0.0` 表示无阴影）。
    #[must_use]
    pub fn with_shadow(mut self, shadow: f32) -> Self {
        self.shadow = shadow;
        self
    }

    /// 按调用方布局位置绘制标签，不重新居中。
    #[must_use]
    pub fn with_centre_label(mut self, centre: bool) -> Self {
        self.centre_label = centre;
        self
    }

    /// 修改焦点环外扩间距的同款按钮。
    #[must_use]
    pub fn with_focus_ring_gap(mut self, gap: f32) -> Self {
        self.focus_ring_gap = gap;
        self
    }

    /// 此按钮绘制使用的颜色槽位。
    #[must_use]
    pub fn tokens(&self) -> ButtonTokens {
        self.variant.tokens()
    }

    /// 指示器矩形：左对齐且垂直居中。
    #[must_use]
    pub fn loading_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + BUTTON_CONTENT_GAP,
            self.rect.origin.y + (self.rect.size.y - BUTTON_LOADING_SIZE) * 0.5,
            BUTTON_LOADING_SIZE,
            BUTTON_LOADING_SIZE,
        )
    }
}

/// 将键盘激活折叠进视觉状态。
///
/// 指针路由器不会接收 Space 或 Enter。用这两个键激活聚焦按钮时，
/// 其绘制效果必须与指针按下完全一致，因此调用方在发射按钮前先经由此处路由按键；
/// 发射器只需响应 [`PointerPhase::Pressed`] 即可。
#[must_use]
pub fn keyboard_activation(state: ControlState, space_or_enter_down: bool) -> ControlState {
    if space_or_enter_down && state.focused && !state.disabled {
        ControlState {
            phase: PointerPhase::Pressed,
            ..state
        }
    } else {
        state
    }
}

/// 绑定了身份标识的按钮。
///
/// 描述符是纯数据，不含标识；测试工具与场景收集器操作的是此包装类型。
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonWidget {
    /// 控件标识。
    pub node: u64,
    /// 按钮描述符。
    pub desc: ButtonDesc,
}

impl WidgetDesc for ButtonWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput {
        emit_button(self.node, &self.desc, state, glyphs)
    }
}

/// 在 [`UiWorld`] 中注册按钮标识并返回其 [`NodeId`]。
///
/// 此处仅注册标识与布局意图：第一步直接按描述符确定按钮尺寸，完整布局引擎留待后续。
pub fn insert_button(world: &mut UiWorld, rect: Rect) -> NodeId {
    let desc = RawNodeDesc::container()
        .size(rect.width(), rect.height())
        .radius(BUTTON_RADIUS)
        .flags(NodeFlags::INTERACTIVE | NodeFlags::FOCUSABLE);
    let root = world.root();
    world.insert(root, desc, core::panic::Location::caller())
}

/// [`NodeId`] 与输入层、语义层共享的 `u64` 标识之间的桥梁。
#[must_use]
pub fn node_u64(node: NodeId) -> u64 {
    node.data().as_ffi()
}

/// 纯发射器：相同输入始终产出相同的图元、命中区域与语义节点。
///
/// 传入的标签为未排版字形（原点靠近左上角）；发射器将其在按钮矩形内居中，
/// 因此标签宽度变化不会破坏按钮自身布局，调用方仍能获得最终场景坐标。
#[must_use]
pub fn emit_button(
    node: u64,
    desc: &ButtonDesc,
    state: ControlState,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    emit_button_with_tokens(node, desc, state, desc.tokens(), label)
}

/// 使用显式颜色槽位集合的纯发射器。
#[must_use]
pub fn emit_button_with_tokens(
    node: u64,
    desc: &ButtonDesc,
    state: ControlState,
    tokens: ButtonTokens,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let z = desc.z;
    let mut primitives = Vec::with_capacity(8 + label.len());
    let mut payloads = Vec::with_capacity(label.len() + 1);

    // 加载中的按钮不是交互目标，也不显示焦点环：旋转指示器表示“请等待”，此时不应再接受第二次激活。
    let blocked = state.disabled || desc.loading;

    // 1. 阴影：专用 Shadow 图元，填充与边框均透明，因此仅绘制解析阴影。
    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        Token::NONE,
        0.0,
        desc.shadow,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));

    // 2. 背景：状态颜色来自颜色表槽位。
    primitives.push(instance(
        rect,
        radii,
        state_fill(state, tokens),
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z,
    ));

    // 3. 图标、加载指示器与标签共享同一居中内容行。指示器始终前置，
    //    因此加载状态切换时按钮不会产生水平跳动。
    let ink = ink_bounds(label);
    let leading_icon = desc.icon.is_some() && !desc.icon_trailing;
    let mut content_width = ink.map_or(0.0, |bounds| bounds.width());
    if desc.loading {
        content_width += BUTTON_LOADING_SIZE + BUTTON_CONTENT_GAP;
    } else if leading_icon {
        content_width += desc.font_size + BUTTON_CONTENT_GAP;
    }
    if desc.icon.is_some() && desc.icon_trailing {
        content_width += desc.font_size + BUTTON_CONTENT_GAP;
    }
    let mut cursor = rect.center().x - content_width * 0.5;

    if desc.loading {
        let indicator = desc.loading_rect();
        let color = if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        };
        primitives.push(instance(
            indicator,
            [BUTTON_LOADING_SIZE * 0.5; 4],
            Token::NONE,
            color,
            2.0,
            0.0,
            PrimType::Stroke,
            SubElement::Content,
            z,
        ));
        cursor += BUTTON_LOADING_SIZE + BUTTON_CONTENT_GAP;
    } else if leading_icon {
        if let Some(icon) = &desc.icon {
            icon_at(
                icon,
                cursor,
                rect.center().y,
                desc.font_size,
                z,
                &mut primitives,
                &mut payloads,
            );
            cursor += desc.font_size + BUTTON_CONTENT_GAP;
        }
    }

    // 标签：与按钮同 `z`，稍后通过 `SubElement::Content` 绘制。
    if let Some(ink) = ink {
        if desc.centre_label {
            let offset = Vec2::new(
                cursor - ink.min().x,
                rect.center().y - (ink.min().y + ink.height() * 0.5),
            );
            for quad in label.iter_mut() {
                quad.rect.origin.x += offset.x;
                quad.rect.origin.y += offset.y;
            }
        }
        let color = if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        };
        for quad in label {
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
                color,
                Token::NONE,
                0.0,
                0.0,
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                SubElement::Content,
                z,
            );
            // 图集层通过边框槽位传递，背景不使用该槽位。
            glyph.border_token_or_tex = quad.layer;
            glyph.extra_payload_idx = payload_idx;
            if quad.msdf {
                glyph.prim_type_and_flags |= u32::from(PrimFlags::MSDF.bits()) << 16;
            }
            primitives.push(glyph);
        }
        cursor += ink.width();
    }

    if desc.icon.is_some() && desc.icon_trailing {
        if let Some(icon) = &desc.icon {
            icon_at(
                icon,
                cursor + BUTTON_CONTENT_GAP,
                rect.center().y,
                desc.font_size,
                z,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    // 4. 边框：空心圆角矩形，属于独立子元素。
    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        tokens.border,
        1.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        z,
    ));

    // 5. 焦点环：外扩描边，位于最顶层。它始终被发射——聚焦仅改变其不透明度——
    //    因此聚焦按钮只需重绘一个图元，而不会打乱图元池。
    primitives.push(focus_ring(
        rect.inflate(desc.focus_ring_gap),
        desc.radius + desc.focus_ring_gap,
        tokens.focus_ring,
        2.0,
        z,
        state.focused && !desc.loading,
    ));

    let hit = (!blocked).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(blocked);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !blocked,
        ..Default::default()
    }
}

/// 发射单个图标图元，锚定于 `left` 并垂直居中。
#[allow(clippy::too_many_arguments)]
fn icon_at(
    icon: &IconDesc,
    left: f32,
    center_y: f32,
    side: f32,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let icon_rect = Rect::new(left, center_y - side * 0.5, side, side);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload {
        p0: [0.0, 0.0],
        p1: [1.0, 1.0],
        p2: [0.0, 0.0],
        line_width: 0.0,
        extra: 0,
        flags: 0,
        _reserved: [0; 3],
    });
    let mut primitive = instance(
        icon_rect,
        [0.0; 4],
        icon.tint,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Image,
        SubElement::Content,
        z,
    );
    primitive.border_token_or_tex = icon.texture_layer;
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

/// 状态绘制使用的填充槽位。
fn state_fill(state: ControlState, tokens: ButtonTokens) -> Token {
    if state.disabled {
        return tokens.fill_disabled;
    }
    match state.phase {
        PointerPhase::Idle => tokens.fill,
        PointerPhase::Hovered => tokens.fill_hover,
        PointerPhase::Pressed => tokens.fill_pressed,
    }
}

/// 字形串的墨水边界框，用于居中计算及无头文本测试。
#[must_use]
pub fn ink_bounds(glyphs: &[GlyphQuad]) -> Option<Rect> {
    let mut bounds = glyphs.first()?.rect;
    for glyph in &glyphs[1..] {
        bounds = bounds.union(&glyph.rect);
    }
    Some(bounds)
}

#[allow(clippy::too_many_arguments)]
fn instance(
    rect: Rect,
    radii: [f32; 4],
    fill: Token,
    border: Token,
    border_width: f32,
    shadow: f32,
    kind: PrimType,
    sub: SubElement,
    z: u16,
) -> GpuPrimitiveInstance {
    GpuPrimitiveInstance {
        chunk_id: [0, 0],
        local_pos: [rect.origin.x, rect.origin.y],
        size: [rect.size.x, rect.size.y],
        corner_radii: f16::pack_radii(radii),
        border_width: f16::f32_to_f16(border_width),
        shadow_blur: f16::f32_to_f16(shadow),
        transform_idx: 0,
        clip_idx: CLIP_NONE,
        fill_token_or_uv: u32::from(fill.raw()),
        border_token_or_tex: u32::from(border.raw()),
        prim_type_and_flags: GpuPrimitiveInstance::pack_type(kind, BlendMode::Normal, sub as u16),
        z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
        extra_payload_idx: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WidgetHarness;

    /// 位于视口中央的按钮描述符。
    fn desc() -> ButtonDesc {
        ButtonDesc::new(Rect::new(100.0, 100.0, 160.0, 44.0), "Press me")
    }

    /// 背景图元的填充令牌。
    fn fill_token(output: &WidgetOutput) -> u16 {
        output
            .primitives
            .iter()
            .find(|primitive| primitive.prim_type() == PrimType::RoundedRect)
            .map(|primitive| primitive.fill_token_or_uv as u16)
            .expect("the button always paints a background")
    }

    #[test]
    fn a_button_paints_the_slot_of_its_state() {
        let tokens = ButtonVariant::Primary.tokens();
        let mut glyphs: Vec<GlyphQuad> = Vec::new();

        let idle = emit_button(1, &desc(), ControlState::default(), &mut glyphs);
        assert_eq!(fill_token(&idle), tokens.fill.raw());

        let hovered = emit_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert_eq!(fill_token(&hovered), tokens.fill_hover.raw());

        let pressed = emit_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Pressed,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert_eq!(fill_token(&pressed), tokens.fill_pressed.raw());
    }

    #[test]
    fn a_disabled_button_leaves_the_hit_set() {
        let mut glyphs: Vec<GlyphQuad> = Vec::new();
        let output = emit_button(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert!(output.hit.is_none(), "不可交互的控件必须从命中集合里消失");
        assert!(output.a11y.is_some());
        assert!(!output.focusable);
    }

    #[test]
    fn every_variant_paints_a_distinguishable_combination() {
        let variants = [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Subtle,
            ButtonVariant::Icon,
            ButtonVariant::OnHero,
        ];
        let mut seen: Vec<ButtonTokens> = Vec::new();
        for variant in variants {
            let tokens = variant.tokens();
            assert!(
                !seen.contains(&tokens),
                "{:?} 与另一个变体的配色完全相同，视觉上无法区分",
                variant
            );
            seen.push(tokens);
        }
    }

    #[test]
    fn a_state_never_paints_the_resting_fill() {
        for variant in [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Subtle,
            ButtonVariant::Icon,
            ButtonVariant::OnHero,
        ] {
            let tokens = variant.tokens();
            assert_ne!(
                tokens.fill_hover, tokens.fill,
                "{:?} 的悬停色必须与静止色不同，否则悬停没有任何反馈",
                variant
            );
            assert_ne!(
                tokens.fill_pressed, tokens.fill_hover,
                "{:?} 的按下色必须与悬停色不同",
                variant
            );
        }
    }

    #[test]
    fn the_harness_drives_a_button_through_the_router() {
        let widget = ButtonWidget {
            node: 42,
            desc: desc(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        assert_eq!(harness.state().phase, PointerPhase::Idle);

        harness.hover();
        assert_eq!(harness.state().phase, PointerPhase::Hovered);
        assert_eq!(
            fill_token(&harness.output),
            ButtonVariant::Primary.tokens().fill_hover.raw()
        );

        harness.press();
        assert_eq!(harness.state().phase, PointerPhase::Pressed);

        // 指针已被捕获，因此即使移出较远仍保持按下状态。
        harness.hover_at(Vec2::new(0.0, 0.0));
        assert_eq!(harness.state().phase, PointerPhase::Pressed);

        harness.release();
        assert_eq!(harness.state().phase, PointerPhase::Idle);
    }

    #[test]
    fn a_focused_button_emits_an_outer_stroke() {
        let widget = ButtonWidget {
            node: 7,
            desc: desc(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        harness.focus();
        assert!(harness.state().focused);
        assert!(harness
            .output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == PrimType::Stroke));
    }
}

//! 第一个可交互控件：扁平按钮。
//!
//! 这里刻意**不**引入 "widget 框架"：一个按钮就是
//!
//! * 一个纯数据描述符 [`ButtonDesc`]，
//! * 一个由指针路由器实时派生的视觉状态 [`ButtonVisualState`]，
//! * 一个纯函数发射器 [`emit_button`]：`(描述符, 状态) -> 图元 + 热区 + 语义节点`。
//!
//! 交互瞬态（hover / pressed）不落任何 `bool` 字段，统一从
//! [`Router`] 的捕获状态派生；因此"按下后拖出边界再松开"这种经典场景不会
//! 出现两套状态彼此失步。组件身份来自 `unbound-world` 的 [`NodeId`]，后续
//! 收敛到 `RawNodeDesc` / 跨 ABI 时不需要换身份模型。
//!
//! 图元分层严格使用 [`SubElement`]（`Shadow → Background → Content → Border
//! → FocusRing`）而不是手工偏移 `z`，让 64 位排序键决定绘制顺序。

use accesskit::Role;
use slotmap::Key as _;
use unbound_a11y::A11yNode;
use unbound_abi::{NodeFlags, RawNodeDesc};
use unbound_gpu::data::{
    BlendMode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement, CLIP_NONE,
};
use unbound_input::{CursorKind, HitRegion, Router, Shape};
use unbound_math::{f16, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_world::{NodeId, UiWorld};

/// 按钮标签字号。字形预热必须使用同一个字号，否则数字会被光栅化两次。
pub const BUTTON_FONT_SIZE: f32 = 14.0;

/// 按钮圆角。
pub const BUTTON_RADIUS: f32 = 10.0;

/// 按钮热区所在的 UI 层（`docs` §6.1：`0` 画布、`1` UI、`2..` Portal）。
pub const BUTTON_LAYER: u8 = 1;

/// 解析阴影的模糊半径；`PrimType::Shadow` 只画阴影，不画填充。
const BUTTON_SHADOW: f32 = 16.0;

/// 焦点环向外扩张的距离。
const FOCUS_RING_GAP: f32 = 3.0;

/// 指针与按钮的关系，**不含**焦点：焦点与指针是正交维度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerPhase {
    #[default]
    Idle,
    Hovered,
    Pressed,
}

/// 一帧的按钮视觉状态。
///
/// `focused` 与 `phase` 正交：通过 Tab 聚焦的按钮在鼠标悬停时必须同时呈现
/// hover 背景与外部焦点环，所以它们不能塞进同一个互斥枚举。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonVisualState {
    pub phase: PointerPhase,
    pub focused: bool,
    pub disabled: bool,
}

impl ButtonVisualState {
    /// 从指针路由器与焦点槽位派生。
    ///
    /// 按下后指针被 [`Router`] 捕获，`pressed()` 会一直指向按钮，因此拖出
    /// 边界仍显示 pressed，与原生按钮一致；松开或被取消后自动回落。
    #[must_use]
    pub fn resolve(node: NodeId, router: &Router, focused: Option<NodeId>, disabled: bool) -> Self {
        let id = node_u64(node);
        let phase = if disabled {
            PointerPhase::Idle
        } else if router.pointer().pressed() == Some(id) {
            PointerPhase::Pressed
        } else if router.hover() == Some(id) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        Self {
            phase,
            focused: focused == Some(node),
            disabled,
        }
    }

    fn fill(self, tokens: ButtonTokens) -> u16 {
        if self.disabled {
            return tokens.fill_disabled;
        }
        match self.phase {
            PointerPhase::Idle => tokens.fill,
            PointerPhase::Hovered => tokens.fill_hover,
            PointerPhase::Pressed => tokens.fill_pressed,
        }
    }
}

/// 按钮用到的颜色 token。
///
/// 与 [`crate::gallery::Palette`] 解耦：发射器只认 token，颜色表由展台提供。
/// 所有状态色都占用独立的 4 KiB 颜色表槽位，因此切换主题时按钮图元一个
/// 字节都不用重传。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonTokens {
    pub fill: u16,
    pub fill_hover: u16,
    pub fill_pressed: u16,
    pub fill_disabled: u16,
    pub border: u16,
    pub label: u16,
    pub label_disabled: u16,
    pub focus_ring: u16,
}

/// 按钮的纯数据描述符。
///
/// 没有任何方法会改变它，也没有堆上的闭包或类型擦除：这正是 `RawNodeDesc`
/// 收敛后的形态，第二个控件只需要再写一个同样的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonDesc {
    pub rect: Rect,
    pub label: String,
    pub radius: f32,
    pub font_size: f32,
    pub z: u16,
}

impl ButtonDesc {
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: BUTTON_RADIUS,
            font_size: BUTTON_FONT_SIZE,
            z: 703,
        }
    }

    /// 同一按钮换文案：点击计数变化时只重建描述符，不重建身份。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

/// 一次发射的全部产物。
pub struct ButtonArtifacts {
    /// 阴影、背景、文字、边框、焦点环。
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// 字形 UV，索引与 `primitives[].extra_payload_idx` 对应。
    pub payloads: Vec<GpuPayload>,
    /// 圆角热区（命中测试只认它，不认图元矩形）。
    pub hit: HitRegion,
    /// `Role::Button` 语义节点。
    pub a11y: A11yNode,
}

/// 把按钮身份登记进 `UiWorld`，返回稳定的 [`NodeId`]。
///
/// 这里只登记身份与布局意图，不参与布局求解：第一步的尺寸由描述符直接
/// 给定，整页 Taffy 化留到第二步。
pub fn insert_button(world: &mut UiWorld, rect: Rect) -> NodeId {
    let desc = RawNodeDesc::container()
        .size(rect.width(), rect.height())
        .radius(BUTTON_RADIUS)
        .flags(NodeFlags::INTERACTIVE | NodeFlags::FOCUSABLE);
    let root = world.root();
    world.insert(root, desc, core::panic::Location::caller())
}

/// `NodeId` 与输入/语义层共享的 `u64` 身份之间的桥。
#[must_use]
pub fn node_u64(node: NodeId) -> u64 {
    node.data().as_ffi()
}

/// 纯函数发射器：同样的输入永远得到同样的图元、热区与语义节点。
///
/// `label` 是未定位的字形（左上角在原点附近），发射器会把它在按钮矩形里
/// 居中；这样文本换行/数字变宽都不会破坏按钮自身的布局。
#[must_use]
pub fn emit_button(
    node: u64,
    desc: &ButtonDesc,
    state: ButtonVisualState,
    tokens: ButtonTokens,
    label: &[GlyphQuad],
) -> ButtonArtifacts {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let z = desc.z;
    let mut primitives = Vec::with_capacity(5 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    // 1. 阴影：独立的 Shadow 图元，只画解析阴影（erf），不画填充。
    primitives.push(instance(
        rect,
        radii,
        0,
        0,
        0.0,
        BUTTON_SHADOW,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));

    // 2. 背景：状态色从颜色表槽位里取。
    primitives.push(instance(
        rect,
        radii,
        state.fill(tokens),
        0,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z,
    ));

    // 3. 文字：与按钮共享同一个 z，靠 SubElement::Content 排在后画。
    if let Some(ink) = ink_bounds(label) {
        let offset = Vec2::new(
            rect.center().x - (ink.min().x + ink.width() * 0.5),
            rect.center().y - (ink.min().y + ink.height() * 0.5),
        );
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
                Rect::new(
                    quad.rect.origin.x + offset.x,
                    quad.rect.origin.y + offset.y,
                    quad.rect.width(),
                    quad.rect.height(),
                ),
                [0.0; 4],
                color,
                0,
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
            // 图集层号走 border 槽位，与背景的 border token 不冲突。
            glyph.border_token_or_tex = quad.layer;
            glyph.extra_payload_idx = payload_idx;
            primitives.push(glyph);
        }
    }

    // 4. 边框：空心圆角矩形，单独一个 SubElement。
    primitives.push(instance(
        rect,
        radii,
        0,
        tokens.border,
        1.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        z,
    ));

    // 5. 焦点环：外扩的 Stroke，最上层。
    if state.focused {
        primitives.push(instance(
            rect.inflate(FOCUS_RING_GAP),
            [desc.radius + FOCUS_RING_GAP; 4],
            0,
            tokens.focus_ring,
            2.0,
            0.0,
            PrimType::Stroke,
            SubElement::FocusRing,
            z,
        ));
    }

    let hit = HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
        .with_shape(Shape::Rounded { radii })
        .with_cursor(CursorKind::Pointer);
    let a11y = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(state.disabled);

    ButtonArtifacts {
        primitives,
        payloads,
        hit,
        a11y,
    }
}

/// 字形集合的墨迹包围盒，用于居中与无头文字体检。
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
    fill: u16,
    border: u16,
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
        fill_token_or_uv: u32::from(fill),
        border_token_or_tex: u32::from(border),
        prim_type_and_flags: GpuPrimitiveInstance::pack_type(kind, BlendMode::Normal, sub as u16),
        z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
        extra_payload_idx: 0,
    }
}

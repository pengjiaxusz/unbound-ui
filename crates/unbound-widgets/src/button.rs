//! The flat button: the first standard control.
//!
//! A button is three pieces and nothing more:
//!
//! * a pure data descriptor [`ButtonDesc`],
//! * a visual state derived from the pointer router ([`ControlState`]),
//! * a pure emitter [`emit_button`]: `(descriptor, state) -> primitives + hit
//!   region + semantic node`.
//!
//! The interaction transient (hover / pressed) never lands in a `bool`
//! field: it is derived from the router on every frame, so "press, drag out,
//! release" cannot leave two states disagreeing with each other.
//!
//! The primitives are layered with [`SubElement`]
//! (`Shadow -> Background -> Content -> Border -> FocusRing`) rather than
//! with hand-tuned `z` offsets, so the 64-bit sort key decides the draw
//! order.

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

/// The button label font size. The glyph warm-up must use the same size,
/// otherwise the digits are rasterised twice.
pub const BUTTON_FONT_SIZE: f32 = 14.0;

/// The button corner radius.
pub const BUTTON_RADIUS: f32 = 10.0;

/// The gap between the label and a leading / trailing icon.
pub const BUTTON_CONTENT_GAP: f32 = 8.0;

/// The diameter of the loading indicator.
pub const BUTTON_LOADING_SIZE: f32 = 12.0;

/// The height of a button, in logical pixels.
///
/// The size is metadata: a caller that has already solved the layout keeps
/// its rectangle, but a toolbar can ask for the standard height instead of
/// hard-coding a number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// A compact action, 28 px tall.
    Sm,
    /// The standard action, 36 px tall.
    #[default]
    Md,
    /// A prominent action, 44 px tall.
    Lg,
}

impl ButtonSize {
    /// The control height for this size.
    #[must_use]
    pub const fn height(self) -> f32 {
        match self {
            Self::Sm => 28.0,
            Self::Md => 36.0,
            Self::Lg => 44.0,
        }
    }
}

/// The UI layer a button's hit region lives in (`0` canvas, `1` UI, `2..`
/// portals).
pub const BUTTON_LAYER: u8 = 1;

/// The blur radius of the resolved shadow. `PrimType::Shadow` only paints the
/// shadow, never the fill.
const BUTTON_SHADOW: f32 = 16.0;

/// How far the focus ring grows outside the button.
const FOCUS_RING_GAP: f32 = 3.0;

/// The visual role of a button.
///
/// The variant decides *which* colour slots a button paints with; the theme
/// decides what those slots look like. Adding a variant therefore never
/// touches a single primitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// The call to action: a filled, high-contrast surface.
    #[default]
    Primary,
    /// A supporting action: an outlined surface.
    Secondary,
    /// A low-emphasis action: no fill, no border.
    Subtle,
    /// A compact, glyph-only action.
    Icon,
    /// A button painted on top of a hero gradient.
    OnHero,
}

impl ButtonVariant {
    /// The colour slots this variant paints with.
    ///
    /// Every state has its own slot, so hovering a button never rewrites
    /// instance data — it only changes which colour the slot resolves to.
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

/// The colour slots a button paints with.
///
/// The emitter only knows slots; the theme decides the colours. Every state
/// occupies its own slot of the flat colour table, so a theme switch — and a
/// hover — re-uploads no primitive at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonTokens {
    /// The resting fill.
    pub fill: Token,
    /// The fill while hovered.
    pub fill_hover: Token,
    /// The fill while pressed.
    pub fill_pressed: Token,
    /// The fill while disabled.
    pub fill_disabled: Token,
    /// The outline.
    pub border: Token,
    /// The label colour.
    pub label: Token,
    /// The label colour while disabled.
    pub label_disabled: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

/// The pure data descriptor of a button.
///
/// No method mutates it, and it carries no closure and no type erasure: this
/// is the shape a `RawNodeDesc` converges to, and the second control only has
/// to write the same kind of struct.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonDesc {
    /// The button rectangle.
    pub rect: Rect,
    /// The label.
    pub label: String,
    /// The corner radius.
    pub radius: f32,
    /// The label font size.
    pub font_size: f32,
    /// The draw order key.
    pub z: u16,
    /// The visual role.
    pub variant: ButtonVariant,
    /// The standard size (28 / 36 / 44 px).
    pub size: ButtonSize,
    /// An icon painted next to the label.
    pub icon: Option<IconDesc>,
    /// Paint the icon after the label instead of before it.
    pub icon_trailing: bool,
    /// Show the loading indicator and refuse interaction.
    pub loading: bool,
    /// The analytic drop-shadow blur. `0.0` paints no shadow at all.
    pub shadow: f32,
    /// Centre the label on its ink bounds.
    ///
    /// A caller that already positioned the glyph run (a page that shaped it
    /// inside the button rectangle) clears this, so the emitter paints the
    /// run exactly where it was measured instead of re-centring it.
    pub centre_label: bool,
    /// How far the focus ring grows outside the button.
    pub focus_ring_gap: f32,
}

impl ButtonDesc {
    /// A primary button with the default metrics.
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

    /// The same button with another label: a click counter only rebuilds the
    /// descriptor, never the identity.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The same button in another visual role.
    #[must_use]
    pub fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The same button at another standard size.
    #[must_use]
    pub fn with_size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// The same button with an icon next to its label.
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Paint the icon after the label instead of before it.
    #[must_use]
    pub fn with_icon_trailing(mut self, trailing: bool) -> Self {
        self.icon_trailing = trailing;
        self
    }

    /// The same button in its loading state: no clicks, an indicator on the
    /// left.
    #[must_use]
    pub fn with_loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// The same button with another shadow blur (`0.0` paints none).
    #[must_use]
    pub fn with_shadow(mut self, shadow: f32) -> Self {
        self.shadow = shadow;
        self
    }

    /// Paint the label where the caller laid it out instead of re-centring it.
    #[must_use]
    pub fn with_centre_label(mut self, centre: bool) -> Self {
        self.centre_label = centre;
        self
    }

    /// The same button with another focus-ring gap.
    #[must_use]
    pub fn with_focus_ring_gap(mut self, gap: f32) -> Self {
        self.focus_ring_gap = gap;
        self
    }

    /// The colour slots this button paints with.
    #[must_use]
    pub fn tokens(&self) -> ButtonTokens {
        self.variant.tokens()
    }

    /// The indicator rectangle: left-aligned and vertically centred.
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

/// Fold a keyboard activation into the visual state.
///
/// The pointer router never sees Space or Enter. A focused button activated
/// with either key must paint exactly like a pointer press, so the caller
/// routes the key through here before it emits the button; the emitter then
/// only has to honour [`PointerPhase::Pressed`].
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

/// A button bound to an identity.
///
/// The descriptor is pure data and therefore identity-free; this wrapper is
/// what the harness and the scene collector work with.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonWidget {
    /// The widget identity.
    pub node: u64,
    /// The descriptor.
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

/// Register a button identity in the [`UiWorld`] and return its [`NodeId`].
///
/// Only the identity and the layout intent are registered here: the first
/// step sizes the button from the descriptor directly, the full layout
/// engine comes later.
pub fn insert_button(world: &mut UiWorld, rect: Rect) -> NodeId {
    let desc = RawNodeDesc::container()
        .size(rect.width(), rect.height())
        .radius(BUTTON_RADIUS)
        .flags(NodeFlags::INTERACTIVE | NodeFlags::FOCUSABLE);
    let root = world.root();
    world.insert(root, desc, core::panic::Location::caller())
}

/// The bridge between a [`NodeId`] and the `u64` identity the input and
/// semantic layers share.
#[must_use]
pub fn node_u64(node: NodeId) -> u64 {
    node.data().as_ffi()
}

/// The pure emitter: the same input always yields the same primitives, hit
/// region and semantic node.
///
/// The label arrives as unshaped glyphs (their origin is near the top left);
/// the emitter centres them inside the button rectangle, so a label that
/// changes width never breaks the button's own layout and the caller still
/// gets final scene coordinates.
#[must_use]
pub fn emit_button(
    node: u64,
    desc: &ButtonDesc,
    state: ControlState,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    emit_button_with_tokens(node, desc, state, desc.tokens(), label)
}

/// The pure emitter with an explicit set of colour slots.
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

    // A loading button is not a target and never shows the focus ring: the
    // spinner says "wait", so accepting a second activation would be wrong.
    let blocked = state.disabled || desc.loading;

    // 1. The shadow: a dedicated Shadow primitive, fill and border are
    //    transparent, so only the resolved shadow is painted.
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

    // 2. The background: the state colour comes from a colour table slot.
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

    // 3. The icon, the loading indicator and the label share one centred
    //    content row. The indicator always leads, so a loading button never
    //    jumps horizontally when the spinner appears.
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

    // The label: same `z` as the button, drawn later through
    // `SubElement::Content`.
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
            // The atlas layer travels in the border slot, which the
            // background does not use.
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

    // 4. The border: a hollow rounded rectangle, its own sub element.
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

    // 5. The focus ring: an outward stroke, on top of everything. It is
    //    always emitted — focus only changes its opacity — so that focusing
    //    a button repaints one primitive instead of reshuffling the pool.
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
    }
}

/// Emit one icon primitive, anchored at `left` and vertically centred.
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

/// The fill slot a state paints with.
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

/// The ink bounding box of a glyph run, used for centring and for the
/// headless text checks.
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

    /// A button descriptor in the middle of the viewport.
    fn desc() -> ButtonDesc {
        ButtonDesc::new(Rect::new(100.0, 100.0, 160.0, 44.0), "Press me")
    }

    /// The fill token of the background primitive.
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

        // The pointer is captured, so it stays pressed even far away.
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

//! Multi-level hierarchical Cascader selector.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_LEVELS: usize = 3;

/// Hierarchical option node for a [`CascaderDesc`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CascaderOption {
    /// Option value/id.
    pub value: String,
    /// Display label.
    pub label: String,
    /// Nested children.
    pub children: Vec<CascaderOption>,
}

impl CascaderOption {
    /// Create a leaf option.
    #[must_use]
    pub fn leaf(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            children: Vec::new(),
        }
    }

    /// Create a branch option with children.
    #[must_use]
    pub fn branch(
        value: impl Into<String>,
        label: impl Into<String>,
        children: Vec<CascaderOption>,
    ) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            children,
        }
    }
}

/// Interactive state for [`CascaderDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CascaderState {
    /// Selected value path across hierarchy levels.
    pub path: Vec<String>,
    /// Whether the multi-column popup is open.
    pub open: bool,
}

/// Descriptor for a [`Cascader`].
#[derive(Clone, Debug, PartialEq)]
pub struct CascaderDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Root options.
    pub options: Vec<CascaderOption>,
    /// Selected path.
    pub path: Vec<String>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl CascaderDesc {
    /// Create a cascader descriptor.
    #[must_use]
    pub fn new(rect: Rect, options: Vec<CascaderOption>, path: Vec<String>) -> Self {
        Self {
            rect,
            options,
            path,
            label: "Cascader".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape cascader (1 trigger + 3 level pills + 1 chevron + 1 focus ring = 6 primitives).
#[must_use]
pub fn emit_cascader(
    node: u64,
    desc: &CascaderDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE
        },
        if state.disabled {
            Token::DISABLED
        } else if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pill_w = ((desc.rect.width() - 36.0) / MAX_LEVELS as f32).max(16.0);
    let pill_h = (desc.rect.height() - 10.0).max(12.0);
    for level in 0..MAX_LEVELS {
        let r = Rect::new(
            desc.rect.origin.x + 6.0 + level as f32 * (pill_w + 4.0),
            desc.rect.origin.y + 5.0,
            pill_w,
            pill_h,
        );
        let active = level < desc.path.len();
        let mut inst = surface(
            r,
            4.0,
            if active {
                Token::PRIMARY_SOFT
            } else {
                Token::SURFACE_SUNKEN
            },
            desc.z + 1,
        );
        if !active && level > 0 {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);
    }

    let chev_r = Rect::new(
        desc.rect.max().x - 16.0,
        desc.rect.origin.y + (desc.rect.height() - 8.0) * 0.5,
        8.0,
        8.0,
    );
    primitives.push(surface(chev_r, 2.0, Token::TEXT_SECONDARY, desc.z + 2));

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y =
        A11yNode::new(Role::ComboBox, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

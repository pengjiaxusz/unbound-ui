//! Images and icons: object-fit maths and the textured primitive.
//!
//! Fitting is pure geometry and is unit-tested on its own. The emitter then
//! produces one `PrimType::Image` primitive whose texture layer comes from the
//! asset atlas and whose UV rectangle comes from [`fit_image`]. Keeping the UV
//! maths out of the renderer means `Cover` crops the *sampled* region instead
//! of stretching the image, which is the behaviour users expect.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, PrimType, SubElement};
use unbound_input::{HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::instance;
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of an image that accepts clicks.
pub const IMAGE_LAYER: u8 = 1;

/// How an image fills its box.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ObjectFit {
    /// Stretch to the box (may distort).
    #[default]
    Fill,
    /// Scale down until the whole image fits, letterboxing the rest.
    Contain,
    /// Scale up until the box is covered, cropping the overflow.
    Cover,
    /// Draw at natural size, centred, never upscaled.
    None,
}

/// A placed image: where to draw and which part of the texture to sample.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImageFit {
    /// The destination rectangle.
    pub rect: Rect,
    /// The sampled UV rectangle, normalised to `0..1`.
    pub uv: Rect,
}

/// Compute the destination rectangle and UV crop for an object fit.
#[must_use]
pub fn fit_image(box_rect: Rect, natural: Vec2, fit: ObjectFit) -> ImageFit {
    let full_uv = Rect::new(0.0, 0.0, 1.0, 1.0);
    if natural.x <= 0.0 || natural.y <= 0.0 || box_rect.is_empty() {
        return ImageFit {
            rect: box_rect,
            uv: full_uv,
        };
    }
    match fit {
        ObjectFit::Fill => ImageFit {
            rect: box_rect,
            uv: full_uv,
        },
        ObjectFit::Contain => {
            let scale = (box_rect.width() / natural.x).min(box_rect.height() / natural.y);
            let size = natural * scale;
            ImageFit {
                rect: Rect::new(
                    box_rect.origin.x + (box_rect.width() - size.x) * 0.5,
                    box_rect.origin.y + (box_rect.height() - size.y) * 0.5,
                    size.x,
                    size.y,
                ),
                uv: full_uv,
            }
        }
        ObjectFit::Cover => {
            let scale = (box_rect.width() / natural.x).max(box_rect.height() / natural.y);
            let visible = Vec2::new(box_rect.width() / scale, box_rect.height() / scale);
            let origin = Vec2::new(
                (natural.x - visible.x) * 0.5 / natural.x,
                (natural.y - visible.y) * 0.5 / natural.y,
            );
            ImageFit {
                rect: box_rect,
                uv: Rect::new(
                    origin.x,
                    origin.y,
                    visible.x / natural.x,
                    visible.y / natural.y,
                ),
            }
        }
        ObjectFit::None => {
            let scale = if natural.x > box_rect.width() || natural.y > box_rect.height() {
                (box_rect.width() / natural.x).min(box_rect.height() / natural.y)
            } else {
                1.0
            };
            let size = natural * scale;
            ImageFit {
                rect: Rect::new(
                    box_rect.origin.x + (box_rect.width() - size.x) * 0.5,
                    box_rect.origin.y + (box_rect.height() - size.y) * 0.5,
                    size.x,
                    size.y,
                ),
                uv: full_uv,
            }
        }
    }
}

/// An image descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDesc {
    /// The layout box.
    pub rect: Rect,
    /// The texture's natural size, in pixels.
    pub natural_size: Vec2,
    /// The texture layer in the atlas.
    pub texture_layer: u32,
    /// The object fit.
    pub fit: ObjectFit,
    /// The corner radius.
    pub radius: f32,
    /// The tint; [`Token::NONE`] leaves the texture untouched.
    pub tint: Token,
    /// The accessible description.
    pub alt: String,
    /// The draw order key.
    pub z: u16,
    /// Whether the image accepts clicks.
    pub interactive: bool,
}

impl ImageDesc {
    #[must_use]
    pub fn new(rect: Rect, natural_size: Vec2, texture_layer: u32) -> Self {
        Self {
            rect,
            natural_size,
            texture_layer,
            fit: ObjectFit::Cover,
            radius: 0.0,
            tint: Token::NONE,
            alt: String::new(),
            z: 690,
            interactive: false,
        }
    }

    /// The same image with an object fit.
    #[must_use]
    pub fn with_fit(mut self, fit: ObjectFit) -> Self {
        self.fit = fit;
        self
    }

    /// The same image with an accessible description.
    #[must_use]
    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = alt.into();
        self
    }

    /// The same image with rounded corners.
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

/// An icon: a square image painted with a theme tint.
#[derive(Clone, Debug, PartialEq)]
pub struct IconDesc {
    /// The square box.
    pub rect: Rect,
    /// The texture layer.
    pub texture_layer: u32,
    /// The tint colour.
    pub tint: Token,
    /// The accessible label.
    pub label: String,
    /// The draw order key.
    pub z: u16,
}

impl IconDesc {
    #[must_use]
    pub fn new(rect: Rect, texture_layer: u32, tint: Token) -> Self {
        Self {
            rect,
            texture_layer,
            tint,
            label: String::new(),
            z: 691,
        }
    }
}

/// An image bound to an identity.
#[derive(Clone, Debug)]
pub struct ImageWidget {
    /// The widget identity.
    pub node: u64,
    /// The descriptor.
    pub desc: ImageDesc,
}

impl WidgetDesc for ImageWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_image(self.node, &self.desc, state)
    }
}

/// Emit one textured primitive.
#[must_use]
pub fn emit_image(node: u64, desc: &ImageDesc, state: ControlState) -> WidgetOutput {
    let fit = fit_image(desc.rect, desc.natural_size, desc.fit);
    let mut primitive = instance(
        fit.rect,
        [desc.radius; 4],
        desc.tint,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Image,
        SubElement::Content,
        desc.z,
    );
    primitive.border_token_or_tex = desc.texture_layer;
    let payload = GpuPayload {
        p0: [fit.uv.origin.x, fit.uv.origin.y],
        p1: [fit.uv.max().x, fit.uv.max().y],
        p2: [0.0, 0.0],
        line_width: 0.0,
        extra: 0,
        flags: 0,
        _reserved: [0; 3],
    };

    let hit = (desc.interactive && !state.disabled).then(|| {
        HitRegion::new(node, IMAGE_LAYER, u32::from(desc.z), desc.rect).with_shape(Shape::Rounded {
            radii: [desc.radius; 4],
        })
    });
    let mut a11y = A11yNode::new(
        Role::Image,
        if desc.alt.is_empty() {
            String::new()
        } else {
            desc.alt.clone()
        },
        desc.rect,
    )
    .disabled(state.disabled);
    if desc.interactive {
        a11y = a11y.clickable();
    }

    WidgetOutput {
        primitives: vec![primitive],
        payloads: vec![payload],
        hit,
        a11y: Some(a11y),
        focusable: false,
    }
}

/// Emit an icon as a tinted image.
#[must_use]
pub fn emit_icon(node: u64, desc: &IconDesc, state: ControlState) -> WidgetOutput {
    let image = ImageDesc {
        rect: desc.rect,
        natural_size: desc.rect.size,
        texture_layer: desc.texture_layer,
        fit: ObjectFit::Contain,
        radius: 0.0,
        tint: desc.tint,
        alt: desc.label.clone(),
        z: desc.z,
        interactive: false,
    };
    emit_image(node, &image, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_rect() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn contain_letterboxes_and_keeps_the_aspect_ratio() {
        let fit = fit_image(box_rect(), Vec2::new(100.0, 100.0), ObjectFit::Contain);
        assert_eq!(fit.rect, Rect::new(50.0, 0.0, 100.0, 100.0));
        assert_eq!(fit.uv, Rect::new(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn cover_crops_the_texture_instead_of_stretching_it() {
        let fit = fit_image(box_rect(), Vec2::new(100.0, 100.0), ObjectFit::Cover);
        assert_eq!(fit.rect, box_rect());
        assert_eq!(fit.uv.width(), 1.0);
        assert!((fit.uv.height() - 0.5).abs() < 1e-6);
        assert!((fit.uv.origin.y - 0.25).abs() < 1e-6);
    }

    #[test]
    fn fill_uses_the_whole_texture() {
        let fit = fit_image(box_rect(), Vec2::new(64.0, 64.0), ObjectFit::Fill);
        assert_eq!(fit.rect, box_rect());
        assert_eq!(fit.uv, Rect::new(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn natural_size_is_never_upscaled() {
        let fit = fit_image(box_rect(), Vec2::new(40.0, 20.0), ObjectFit::None);
        assert_eq!(fit.rect.size, Vec2::new(40.0, 20.0));
        assert_eq!(fit.rect.origin, Vec2::new(80.0, 40.0));
    }

    #[test]
    fn an_empty_or_zero_sized_image_degrades_to_the_box() {
        let fit = fit_image(box_rect(), Vec2::ZERO, ObjectFit::Cover);
        assert_eq!(fit.rect, box_rect());
        assert_eq!(fit.uv, Rect::new(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn the_emitter_writes_the_uv_payload_and_the_texture_layer() {
        let desc = ImageDesc::new(box_rect(), Vec2::new(100.0, 100.0), 7)
            .with_fit(ObjectFit::Cover)
            .with_alt("portrait");
        let output = emit_image(3, &desc, ControlState::default());
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(output.primitives[0].border_token_or_tex, 7);
        assert_eq!(output.payloads[0].p0, [0.0, 0.25]);
        assert_eq!(output.payloads[0].p1, [1.0, 0.75]);
        assert_eq!(
            output.a11y.as_ref().map(|node| node.role),
            Some(Role::Image)
        );
        assert_eq!(output.a11y.as_ref().unwrap().label, "portrait");
        assert!(output.hit.is_none(), "a decorative image is not pickable");
    }

    #[test]
    fn an_interactive_image_is_pickable_unless_disabled() {
        let mut desc = ImageDesc::new(box_rect(), Vec2::new(100.0, 100.0), 1);
        desc.interactive = true;
        let output = emit_image(3, &desc, ControlState::default());
        assert!(output.hit.is_some());
        let disabled = ControlState {
            disabled: true,
            ..ControlState::default()
        };
        assert!(emit_image(3, &desc, disabled).hit.is_none());
    }
}

//! Images and icons: object-fit maths and the textured primitive.
//!
//! Fitting is pure geometry and is unit-tested on its own. The emitter then
//! produces one `PrimType::Image` primitive whose texture layer comes from the
//! asset atlas and whose UV rectangle comes from [`fit_image`]. Keeping the UV
//! maths out of the renderer means `Cover` crops the *sampled* region instead
//! of stretching the image, which is the behaviour users expect.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_asset::AssetId;
use unbound_gpu::data::{GpuPayload, PrimType, SubElement};
use unbound_input::{HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::instance;
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of an image that accepts clicks.
pub const IMAGE_LAYER: u8 = 1;

/// One nine-slice span: the start offset and the length along an axis.
type Span = (f32, f32);

/// Where an image's texels come from.
///
/// The atlas variant is the immediate path: the caller already knows the
/// bindless layer and the texture's natural size. The asset variant defers to
/// an [`ImageAssetResolver`], which is how an asynchronously decoded image
/// becomes a textured quad once its upload lands.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ImageSource {
    /// A texture already resident in a bindless atlas layer.
    AtlasLayer {
        /// The array layer.
        layer: u32,
        /// The texture's natural size, in pixels.
        natural_size: Vec2,
    },
    /// An asset that may still be decoding.
    Asset(AssetId),
}

/// A resolved image: the layer, the UV rectangle and the natural size.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedImage {
    /// The bindless atlas layer.
    pub layer: u32,
    /// The region of the layer the image occupies, normalised to `0..1`.
    pub uv: Rect,
    /// The image's natural size, in pixels.
    pub size: Vec2,
}

/// The seam between the widget layer and the asset hub.
///
/// The widgets crate must not depend on the application runtime, so it asks
/// this trait instead: `None` means "not ready yet" and paints the placeholder.
pub trait ImageAssetResolver {
    /// Look up an asset that has finished decoding.
    fn resolve(&self, id: AssetId) -> Option<ResolvedImage>;
}

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
    /// Where the texels come from. Defaults to the atlas layer above.
    pub source: ImageSource,
    /// The object fit.
    pub fit: ObjectFit,
    /// The per-corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radius: [f32; 4],
    /// The border width; `0.0` paints no border.
    pub border_width: f32,
    /// The border colour slot.
    pub border_color: Token,
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
            source: ImageSource::AtlasLayer {
                layer: texture_layer,
                natural_size,
            },
            fit: ObjectFit::Cover,
            radius: [0.0; 4],
            border_width: 0.0,
            border_color: Token::NONE,
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

    /// The same image with a uniform corner radius.
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = [radius; 4];
        self
    }

    /// The same image with per-corner radii.
    #[must_use]
    pub fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radius = radii;
        self
    }

    /// The same image with a border.
    #[must_use]
    pub fn with_border(mut self, width: f32, color: Token) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    /// The same image sourced from an asynchronous asset.
    #[must_use]
    pub fn with_asset(mut self, id: AssetId) -> Self {
        self.source = ImageSource::Asset(id);
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
    emit_image_inner(node, desc, state, None)
}

/// Emit an image whose texels may still be decoding.
///
/// An asset that the resolver does not know yet (or one that failed) becomes a
/// sunken rounded placeholder of the same box, so the layout never jumps when
/// the texture arrives — only the paint changes.
#[must_use]
pub fn emit_image_with_assets(
    node: u64,
    desc: &ImageDesc,
    state: ControlState,
    resolver: &dyn ImageAssetResolver,
) -> WidgetOutput {
    emit_image_inner(node, desc, state, Some(resolver))
}

fn emit_image_inner(
    node: u64,
    desc: &ImageDesc,
    state: ControlState,
    resolver: Option<&dyn ImageAssetResolver>,
) -> WidgetOutput {
    let resolved = match desc.source {
        ImageSource::AtlasLayer {
            layer,
            natural_size,
        } => Some(ResolvedImage {
            layer,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            size: natural_size,
        }),
        ImageSource::Asset(id) => resolver.and_then(|resolver| resolver.resolve(id)),
    };

    let mut primitives = Vec::with_capacity(2);
    let mut payloads = Vec::with_capacity(1);
    let bounds = match resolved {
        Some(image) => {
            let fit = fit_image(desc.rect, image.size, desc.fit);
            // Crop the atlas sub-rectangle, not the whole layer: the image is
            // one shelf of a shared page.
            let uv = Rect::new(
                image.uv.origin.x + fit.uv.origin.x * image.uv.width(),
                image.uv.origin.y + fit.uv.origin.y * image.uv.height(),
                fit.uv.width() * image.uv.width(),
                fit.uv.height() * image.uv.height(),
            );
            let mut primitive = instance(
                fit.rect,
                desc.radius,
                desc.tint,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Image,
                SubElement::Content,
                desc.z,
            );
            primitive.border_token_or_tex = image.layer;
            primitives.push(primitive);
            payloads.push(GpuPayload {
                p0: [uv.origin.x, uv.origin.y],
                p1: [uv.max().x, uv.max().y],
                p2: [0.0, 0.0],
                line_width: 0.0,
                extra: 0,
                flags: 0,
                _reserved: [0; 3],
            });
            fit.rect
        }
        None => {
            // Loading or failed: a sunken surface keeps the box and the radii.
            primitives.push(instance(
                desc.rect,
                desc.radius,
                Token::SURFACE_SUNKEN,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Background,
                desc.z,
            ));
            payloads.push(GpuPayload {
                p0: [0.0; 2],
                p1: [0.0; 2],
                p2: [0.0, 0.0],
                line_width: 0.0,
                extra: 0,
                flags: 0,
                _reserved: [0; 3],
            });
            desc.rect
        }
    };

    // A textured primitive spends its border slot on the texture layer, so a
    // border is a separate stroke drawn on top with the image's radii.
    if desc.border_width > 0.0 && desc.border_color != Token::NONE {
        primitives.push(instance(
            bounds,
            desc.radius,
            Token::NONE,
            desc.border_color,
            desc.border_width,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ));
    }

    let hit = (desc.interactive && !state.disabled).then(|| {
        HitRegion::new(node, IMAGE_LAYER, u32::from(desc.z), desc.rect)
            .with_shape(Shape::Rounded { radii: desc.radius })
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
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: false,
    }
}

/// A nine-patch image: a texture stretched with fixed edge insets.
#[derive(Clone, Debug, PartialEq)]
pub struct NinePatchDesc {
    /// The destination box.
    pub rect: Rect,
    /// The texture's natural size, in pixels.
    pub natural_size: Vec2,
    /// The texture layer in the atlas.
    pub texture_layer: u32,
    /// The insets in texture pixels: left, top, right, bottom.
    pub patch: [f32; 4],
    /// The tint; [`Token::NONE`] leaves the texture untouched.
    pub tint: Token,
    /// The accessible description.
    pub alt: String,
    /// The draw order key.
    pub z: u16,
}

impl NinePatchDesc {
    #[must_use]
    pub fn new(rect: Rect, natural_size: Vec2, texture_layer: u32, patch: [f32; 4]) -> Self {
        Self {
            rect,
            natural_size,
            texture_layer,
            patch,
            tint: Token::NONE,
            alt: String::new(),
            z: 690,
        }
    }

    /// The nine destination cells, row-major from the top-left.
    #[must_use]
    pub fn cells(&self) -> [Rect; 9] {
        let (columns, rows) = self.spans();
        let mut cells = [Rect::ZERO; 9];
        for row in 0..3 {
            for column in 0..3 {
                cells[row * 3 + column] = Rect::new(
                    columns[column].0,
                    rows[row].0,
                    columns[column].1,
                    rows[row].1,
                );
            }
        }
        cells
    }

    /// The nine UV cells, row-major from the top-left.
    #[must_use]
    pub fn uvs(&self) -> [Rect; 9] {
        let nw = self.natural_size.x.max(1.0);
        let nh = self.natural_size.y.max(1.0);
        let left = self.patch[0].clamp(0.0, nw);
        let top = self.patch[1].clamp(0.0, nh);
        let right = self.patch[2].clamp(0.0, nw - left);
        let bottom = self.patch[3].clamp(0.0, nh - top);
        let columns = [
            (0.0, left / nw),
            (left / nw, (1.0 - (left + right) / nw).max(0.0)),
            (1.0 - right / nw, right / nw),
        ];
        let rows = [
            (0.0, top / nh),
            (top / nh, (1.0 - (top + bottom) / nh).max(0.0)),
            (1.0 - bottom / nh, bottom / nh),
        ];
        let mut cells = [Rect::ZERO; 9];
        for row in 0..3 {
            for column in 0..3 {
                cells[row * 3 + column] = Rect::new(
                    columns[column].0,
                    rows[row].0,
                    columns[column].1,
                    rows[row].1,
                );
            }
        }
        cells
    }

    /// `(position, length)` for the three columns and three rows.
    fn spans(&self) -> ([Span; 3], [Span; 3]) {
        let width = self.rect.width();
        let height = self.rect.height();
        let left = self.patch[0].clamp(0.0, width);
        let top = self.patch[1].clamp(0.0, height);
        let right = self.patch[2].clamp(0.0, width - left);
        let bottom = self.patch[3].clamp(0.0, height - top);
        let middle = (width - left - right).max(0.0);
        let middle_h = (height - top - bottom).max(0.0);
        let x0 = self.rect.origin.x;
        let y0 = self.rect.origin.y;
        (
            [(x0, left), (x0 + left, middle), (x0 + left + middle, right)],
            [
                (y0, top),
                (y0 + top, middle_h),
                (y0 + top + middle_h, bottom),
            ],
        )
    }
}

/// Emit nine UV-split textured primitives for a nine-patch image.
#[must_use]
pub fn emit_nine_patch(_node: u64, desc: &NinePatchDesc, state: ControlState) -> WidgetOutput {
    let cells = desc.cells();
    let uvs = desc.uvs();
    let mut primitives = Vec::with_capacity(9);
    let mut payloads = Vec::with_capacity(9);
    for (cell, uv) in cells.iter().zip(uvs.iter()) {
        let mut primitive = instance(
            *cell,
            [0.0; 4],
            desc.tint,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Image,
            SubElement::Content,
            desc.z,
        );
        primitive.border_token_or_tex = desc.texture_layer;
        primitive.extra_payload_idx = payloads.len() as u32;
        primitives.push(primitive);
        payloads.push(GpuPayload {
            p0: [uv.origin.x, uv.origin.y],
            p1: [uv.max().x, uv.max().y],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
    }

    let a11y = A11yNode::new(Role::Image, desc.alt.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
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
        source: ImageSource::AtlasLayer {
            layer: desc.texture_layer,
            natural_size: desc.rect.size,
        },
        fit: ObjectFit::Contain,
        radius: [0.0; 4],
        border_width: 0.0,
        border_color: Token::NONE,
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

    struct NoImages;

    impl ImageAssetResolver for NoImages {
        fn resolve(&self, _id: AssetId) -> Option<ResolvedImage> {
            None
        }
    }

    struct OneImage(ResolvedImage);

    impl ImageAssetResolver for OneImage {
        fn resolve(&self, _id: AssetId) -> Option<ResolvedImage> {
            Some(self.0)
        }
    }

    #[test]
    fn an_unresolved_asset_paints_a_sunken_placeholder_of_the_same_box() {
        let desc = ImageDesc::new(box_rect(), Vec2::new(64.0, 64.0), 0)
            .with_radius(12.0)
            .with_asset(AssetId(4));
        let output = emit_image_with_assets(3, &desc, ControlState::default(), &NoImages);
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(output.primitives[0].prim_type(), PrimType::RoundedRect);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            u32::from(Token::SURFACE_SUNKEN.raw())
        );
        assert_eq!(output.primitives[0].size, [200.0, 100.0]);
        assert_eq!(output.payloads.len(), 1);
    }

    #[test]
    fn a_ready_asset_becomes_a_textured_quad_with_the_atlas_sub_rect() {
        let desc = ImageDesc::new(box_rect(), Vec2::new(64.0, 64.0), 0).with_asset(AssetId(9));
        let resolved = OneImage(ResolvedImage {
            layer: 5,
            uv: Rect::new(0.25, 0.5, 0.5, 0.25),
            size: Vec2::new(64.0, 32.0),
        });
        let output = emit_image_with_assets(3, &desc, ControlState::default(), &resolved);
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(output.primitives[0].prim_type(), PrimType::Image);
        assert_eq!(output.primitives[0].border_token_or_tex, 5);
        // `Fill` samples the whole image, so the payload is the atlas rect.
        assert_eq!(output.payloads[0].p0, [0.25, 0.5]);
        assert_eq!(output.payloads[0].p1, [0.75, 0.75]);
    }
}

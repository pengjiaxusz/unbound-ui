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
        ..Default::default()
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
        ..Default::default()
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

/// Built-in 24×24 SVG-path vector icon catalog (66 standard icons, S5.6).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconId {
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Plus,
    Minus,
    Close,
    Check,
    Search,
    Settings,
    Home,
    Menu,
    MoreHorizontal,
    MoreVertical,
    Edit,
    Trash,
    Copy,
    Paste,
    Cut,
    Save,
    Undo,
    Redo,
    Refresh,
    Download,
    Upload,
    Share,
    Link,
    ExternalLink,
    Folder,
    FolderOpen,
    File,
    FileCode,
    FileText,
    Image,
    Video,
    Audio,
    Lock,
    Unlock,
    Eye,
    EyeOff,
    Bell,
    BellOff,
    Calendar,
    Clock,
    User,
    Users,
    Star,
    StarFilled,
    Heart,
    Bookmark,
    Tag,
    Filter,
    SortAsc,
    SortDesc,
    Play,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    Info,
    Warning,
    Error,
}

impl IconId {
    /// All 66 built-in vector icons.
    pub const ALL: &'static [Self] = &[
        Self::ChevronUp,
        Self::ChevronDown,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ArrowUp,
        Self::ArrowDown,
        Self::ArrowLeft,
        Self::ArrowRight,
        Self::Plus,
        Self::Minus,
        Self::Close,
        Self::Check,
        Self::Search,
        Self::Settings,
        Self::Home,
        Self::Menu,
        Self::MoreHorizontal,
        Self::MoreVertical,
        Self::Edit,
        Self::Trash,
        Self::Copy,
        Self::Paste,
        Self::Cut,
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::Refresh,
        Self::Download,
        Self::Upload,
        Self::Share,
        Self::Link,
        Self::ExternalLink,
        Self::Folder,
        Self::FolderOpen,
        Self::File,
        Self::FileCode,
        Self::FileText,
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Lock,
        Self::Unlock,
        Self::Eye,
        Self::EyeOff,
        Self::Bell,
        Self::BellOff,
        Self::Calendar,
        Self::Clock,
        Self::User,
        Self::Users,
        Self::Star,
        Self::StarFilled,
        Self::Heart,
        Self::Bookmark,
        Self::Tag,
        Self::Filter,
        Self::SortAsc,
        Self::SortDesc,
        Self::Play,
        Self::Pause,
        Self::Stop,
        Self::SkipBack,
        Self::SkipForward,
        Self::Info,
        Self::Warning,
        Self::Error,
    ];

    /// Standard 24×24 SVG path `d` string for this icon.
    #[must_use]
    pub const fn svg_d(self) -> &'static str {
        match self {
            Self::ChevronUp => "M 6 15 L 12 9 L 18 15",
            Self::ChevronDown => "M 6 9 L 12 15 L 18 9",
            Self::ChevronLeft => "M 15 6 L 9 12 L 15 18",
            Self::ChevronRight => "M 9 6 L 15 12 L 9 18",
            Self::ArrowUp => "M 12 19 L 12 5 M 5 12 L 12 5 L 19 12",
            Self::ArrowDown => "M 12 5 L 12 19 M 5 12 L 12 19 L 19 12",
            Self::ArrowLeft => "M 19 12 L 5 12 M 12 5 L 5 12 L 12 19",
            Self::ArrowRight => "M 5 12 L 19 12 M 12 5 L 19 12 L 12 19",
            Self::Plus => "M 12 5 L 12 19 M 5 12 L 19 12",
            Self::Minus => "M 5 12 L 19 12",
            Self::Close => "M 6 6 L 18 18 M 18 6 L 6 18",
            Self::Check => "M 5 12 L 10 17 L 19 7",
            Self::Search => "M 11 4 A 7 7 0 1 0 11 18 A 7 7 0 1 0 11 4 M 20 20 L 16 16",
            Self::Settings => "M 12 9 A 3 3 0 1 0 12 15 A 3 3 0 1 0 12 9 M 12 2 L 12 5 M 12 19 L 12 22 M 2 12 L 5 12 M 19 12 L 22 12",
            Self::Home => "M 3 10 L 12 3 L 21 10 L 21 20 L 15 20 L 15 14 L 9 14 L 9 20 L 3 20 Z",
            Self::Menu => "M 4 6 L 20 6 M 4 12 L 20 12 M 4 18 L 20 18",
            Self::MoreHorizontal => "M 5 12 A 1 1 0 1 0 7 12 M 11 12 A 1 1 0 1 0 13 12 M 17 12 A 1 1 0 1 0 19 12",
            Self::MoreVertical => "M 12 5 A 1 1 0 1 0 12 7 M 12 11 A 1 1 0 1 0 12 13 M 12 17 A 1 1 0 1 0 12 19",
            Self::Edit => "M 16 4 L 20 8 L 8 20 L 4 20 L 4 16 Z",
            Self::Trash => "M 4 7 L 20 7 M 9 7 L 9 4 L 15 4 L 15 7 M 6 7 L 7 20 L 17 20 L 18 7",
            Self::Copy => "M 9 9 L 20 9 L 20 20 L 9 20 Z M 5 15 L 4 15 L 4 4 L 15 4 L 15 5",
            Self::Paste => "M 8 4 L 16 4 L 16 7 L 8 7 Z M 6 6 L 4 6 L 4 21 L 20 21 L 20 6 L 18 6",
            Self::Cut => "M 6 6 A 3 3 0 1 0 6 12 A 3 3 0 1 0 6 6 M 6 14 A 3 3 0 1 0 6 20 A 3 3 0 1 0 6 14 M 8 9 L 20 18 M 8 15 L 20 6",
            Self::Save => "M 4 4 L 17 4 L 20 7 L 20 20 L 4 20 Z M 8 4 L 8 10 L 15 10 L 15 4",
            Self::Undo => "M 9 14 L 4 9 L 9 4 M 4 9 L 15 9 C 19 9 20 12 20 15 C 20 18 18 20 14 20",
            Self::Redo => "M 15 14 L 20 9 L 15 4 M 20 9 L 9 9 C 5 9 4 12 4 15 C 4 18 6 20 10 20",
            Self::Refresh => "M 20 11 A 8 8 0 1 0 18 17 M 20 4 L 20 11 L 13 11",
            Self::Download => "M 12 4 L 12 15 M 7 10 L 12 15 L 17 10 M 4 19 L 20 19",
            Self::Upload => "M 12 15 L 12 4 M 7 9 L 12 4 L 17 9 M 4 19 L 20 19",
            Self::Share => "M 18 5 A 3 3 0 1 0 18 11 M 6 12 A 3 3 0 1 0 6 18 M 18 13 A 3 3 0 1 0 18 19 M 8 13 L 16 17 M 16 7 L 8 11",
            Self::Link => "M 10 14 L 14 10 M 7 17 L 5 15 A 4 4 0 0 1 11 9 L 12 10 M 17 7 L 19 9 A 4 4 0 0 1 13 15 L 12 14",
            Self::ExternalLink => "M 15 4 L 20 4 L 20 9 M 11 13 L 20 4 M 18 13 L 18 20 L 4 20 L 4 6 L 11 6",
            Self::Folder => "M 3 6 L 10 6 L 12 8 L 21 8 L 21 19 L 3 19 Z",
            Self::FolderOpen => "M 3 6 L 10 6 L 12 8 L 19 8 L 19 11 M 3 19 L 5 11 L 22 11 L 20 19 Z",
            Self::File => "M 6 3 L 14 3 L 19 8 L 19 21 L 6 21 Z M 14 3 L 14 8 L 19 8",
            Self::FileCode => "M 6 3 L 14 3 L 19 8 L 19 21 L 6 21 Z M 10 13 L 8 15 L 10 17 M 14 13 L 16 15 L 14 17",
            Self::FileText => "M 6 3 L 14 3 L 19 8 L 19 21 L 6 21 Z M 9 12 L 16 12 M 9 16 L 16 16",
            Self::Image => "M 4 4 L 20 4 L 20 20 L 4 20 Z M 4 16 L 9 11 L 14 16 L 17 13 L 20 16",
            Self::Video => "M 3 6 L 15 6 L 15 18 L 3 18 Z M 15 10 L 21 7 L 21 17 L 15 14 Z",
            Self::Audio => "M 9 18 L 9 6 L 19 4 L 19 16 M 6 18 A 3 3 0 1 0 9 18 M 16 16 A 3 3 0 1 0 19 16",
            Self::Lock => "M 5 11 L 19 11 L 19 21 L 5 21 Z M 8 11 L 8 7 A 4 4 0 0 1 16 7 L 16 11",
            Self::Unlock => "M 5 11 L 19 11 L 19 21 L 5 21 Z M 8 11 L 8 7 A 4 4 0 0 1 15 5",
            Self::Eye => "M 2 12 C 5 6 19 6 22 12 C 19 18 5 18 2 12 Z M 12 9 A 3 3 0 1 0 12 15 A 3 3 0 1 0 12 9",
            Self::EyeOff => "M 3 3 L 21 21 M 10 6 C 14 6 19 8 22 12 C 20 15 17 17 14 17 M 5 9 C 3 10 2 12 2 12 C 5 17 9 18 12 18",
            Self::Bell => "M 6 16 L 18 16 L 17 10 A 5 5 0 0 0 7 10 Z M 10 19 A 2 2 0 0 0 14 19",
            Self::BellOff => "M 3 3 L 21 21 M 6 16 L 18 16 M 10 19 A 2 2 0 0 0 14 19",
            Self::Calendar => "M 4 6 L 20 6 L 20 20 L 4 20 Z M 8 3 L 8 6 M 16 3 L 16 6 M 4 10 L 20 10",
            Self::Clock => "M 12 3 A 9 9 0 1 0 12 21 A 9 9 0 1 0 12 3 M 12 7 L 12 12 L 16 14",
            Self::User => "M 12 4 A 4 4 0 1 0 12 12 A 4 4 0 1 0 12 4 M 5 20 C 5 16 19 16 19 20",
            Self::Users => "M 9 4 A 4 4 0 1 0 9 12 A 4 4 0 1 0 9 4 M 3 20 C 3 16 15 16 15 20 M 16 6 A 3 3 0 0 1 16 12 M 21 20 C 21 17 18 16 16 16",
            Self::Star => "M 12 3 L 15 9 L 21 10 L 16 14 L 18 21 L 12 17 L 6 21 L 8 14 L 3 10 L 9 9 Z",
            Self::StarFilled => "M 12 3 L 15 9 L 21 10 L 16 14 L 18 21 L 12 17 L 6 21 L 8 14 L 3 10 L 9 9 Z",
            Self::Heart => "M 12 20 L 4 12 C 2 9 4 4 8 4 C 10 4 12 6 12 7 C 12 6 14 4 16 4 C 20 4 22 9 20 12 Z",
            Self::Bookmark => "M 6 4 L 18 4 L 18 21 L 12 16 L 6 21 Z",
            Self::Tag => "M 4 4 L 11 4 L 20 13 L 13 20 L 4 11 Z M 8 8 A 1 1 0 1 0 9 8",
            Self::Filter => "M 3 5 L 21 5 L 14 13 L 14 19 L 10 21 L 10 13 Z",
            Self::SortAsc => "M 7 18 L 7 6 M 4 9 L 7 6 L 10 9 M 13 8 L 20 8 M 13 13 L 18 13 M 13 18 L 16 18",
            Self::SortDesc => "M 7 6 L 7 18 M 4 15 L 7 18 L 10 15 M 13 8 L 20 8 M 13 13 L 18 13 M 13 18 L 16 18",
            Self::Play => "M 6 4 L 19 12 L 6 20 Z",
            Self::Pause => "M 6 4 L 10 4 L 10 20 L 6 20 Z M 14 4 L 18 4 L 18 20 L 14 20 Z",
            Self::Stop => "M 5 5 L 19 5 L 19 19 L 5 19 Z",
            Self::SkipBack => "M 6 5 L 6 19 M 18 5 L 9 12 L 18 19 Z",
            Self::SkipForward => "M 18 5 L 18 19 M 6 5 L 15 12 L 6 19 Z",
            Self::Info => "M 12 3 A 9 9 0 1 0 12 21 A 9 9 0 1 0 12 3 M 12 11 L 12 16 M 12 8 L 12 9",
            Self::Warning => "M 12 3 L 22 20 L 2 20 Z M 12 9 L 12 14 M 12 17 L 12 18",
            Self::Error => "M 12 3 A 9 9 0 1 0 12 21 A 9 9 0 1 0 12 3 M 9 9 L 15 15 M 15 9 L 9 15",
        }
    }
}

/// Built-in vector [`IconPack`] resolving [`IconId`] into cached [`unbound_vector::QuadSegment`]s.
#[derive(Debug, Default)]
pub struct IconPack {
    cache: unbound_vector::SvgPathCache,
}

impl IconPack {
    /// Create a new icon pack cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve an [`IconId`] into quadratic Bézier segments scaled to `target_rect`.
    pub fn resolve_segments(
        &mut self,
        icon: IconId,
        target_origin: Vec2,
        target_size: Vec2,
    ) -> Vec<unbound_vector::QuadSegment> {
        self.cache
            .get_or_parse(icon.svg_d(), 0.25)
            .map(|p| p.scaled_into(target_origin, target_size, 24.0))
            .unwrap_or_default()
    }
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

    #[test]
    fn icon_pack_has_at_least_64_valid_svg_icons() {
        assert!(IconId::ALL.len() >= 64);
        let mut pack = IconPack::new();
        for &icon in IconId::ALL {
            let segs = pack.resolve_segments(icon, Vec2::ZERO, Vec2::new(24.0, 24.0));
            assert!(!segs.is_empty(), "icon {icon:?} produced no segments");
        }
    }
}

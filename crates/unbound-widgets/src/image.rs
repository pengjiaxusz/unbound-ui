//! 图像与图标：对象填充（object-fit）数学与纹理图元。
//!
//! 填充拟合为纯几何计算，并已单独进行单元测试。发射器随后产出一个 `PrimType::Image` 图元，
//! 其纹理层来自资源图集，UV 矩形来自 [`fit_image`]。将 UV 计算移出渲染器意味着
//! `Cover` 会裁剪*采样*区域而非拉伸图像，这正是用户预期的行为。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_asset::AssetId;
use unbound_gpu::data::{GpuPayload, PrimType, SubElement};
use unbound_input::{HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::instance;
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 可点击图像的命中测试层级。
pub const IMAGE_LAYER: u8 = 1;

/// 单个九宫格区段：沿某轴的起始偏移与长度。
type Span = (f32, f32);

/// 图像纹素的来源。
///
/// 图集变体是即时路径：调用方已知无绑定层与纹理自然尺寸。
/// 资源变体则委托给 [`ImageAssetResolver`]，异步解码的图像在上传完成后即成为纹理四边形。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ImageSource {
    /// 已驻留在无绑定图集层中的纹理。
    AtlasLayer {
        /// 纹理数组层索引。
        layer: u32,
        /// 纹理自然尺寸（像素）。
        natural_size: Vec2,
    },
    /// 可能仍在解码中的异步资源。
    Asset(AssetId),
}

/// 已解析的图像：图层、UV 矩形与自然尺寸。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedImage {
    /// 无绑定图集层索引。
    pub layer: u32,
    /// 图像在图层中所占区域，归一化到 `0..1`。
    pub uv: Rect,
    /// 图像自然尺寸（像素）。
    pub size: Vec2,
}

/// 控件层与资源中心之间的接缝。
///
/// widgets crate 不得依赖应用运行时，因此通过此 trait 查询：`None` 表示“尚未就绪”并绘制占位符。
pub trait ImageAssetResolver {
    /// 查找已完成解码的资源。
    fn resolve(&self, id: AssetId) -> Option<ResolvedImage>;
}

/// 图像填充其包围盒的方式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ObjectFit {
    /// 拉伸填满包围盒（可能产生形变）。
    #[default]
    Fill,
    /// 等比缩放直至整张图像完全容纳，留白部分黑边填充。
    Contain,
    /// 等比放大直至完全覆盖包围盒，裁剪溢出部分。
    Cover,
    /// 按自然尺寸居中绘制，绝不放大。
    None,
}

/// 已放置的图像：绘制目标区域与纹理采样区域。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImageFit {
    /// 目标矩形。
    pub rect: Rect,
    /// 采样的 UV 矩形，归一化到 `0..1`。
    pub uv: Rect,
}

/// 计算指定对象填充模式的目标矩形与 UV 裁剪区域。
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

/// 图像描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDesc {
    /// 布局包围盒。
    pub rect: Rect,
    /// 纹理自然尺寸（像素）。
    pub natural_size: Vec2,
    /// 图集中的纹理层索引。
    pub texture_layer: u32,
    /// 纹素来源，默认为上方的图集层。
    pub source: ImageSource,
    /// 对象填充模式。
    pub fit: ObjectFit,
    /// 四角独立圆角半径：左上、右上、右下、左下。
    pub radius: [f32; 4],
    /// 边框宽度；`0.0` 表示不绘制边框。
    pub border_width: f32,
    /// 边框颜色槽位。
    pub border_color: Token,
    /// 着色色调；[`Token::NONE`] 表示保持纹理原色。
    pub tint: Token,
    /// 无障碍描述。
    pub alt: String,
    /// 绘制顺序键。
    pub z: u16,
    /// 图像是否接受点击。
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

    /// 设置对象填充模式。
    #[must_use]
    pub fn with_fit(mut self, fit: ObjectFit) -> Self {
        self.fit = fit;
        self
    }

    /// 设置无障碍描述。
    #[must_use]
    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = alt.into();
        self
    }

    /// 设置统一圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = [radius; 4];
        self
    }

    /// 设置四角独立圆角半径。
    #[must_use]
    pub fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radius = radii;
        self
    }

    /// 设置边框。
    #[must_use]
    pub fn with_border(mut self, width: f32, color: Token) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    /// 设置异步资源作为图像来源。
    #[must_use]
    pub fn with_asset(mut self, id: AssetId) -> Self {
        self.source = ImageSource::Asset(id);
        self
    }
}

/// 图标：带主题着色的正方形图像。
#[derive(Clone, Debug, PartialEq)]
pub struct IconDesc {
    /// 正方形包围盒。
    pub rect: Rect,
    /// 纹理层索引。
    pub texture_layer: u32,
    /// 着色颜色。
    pub tint: Token,
    /// 无障碍标签。
    pub label: String,
    /// 绘制顺序键。
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

/// 绑定了身份标识的图像。
#[derive(Clone, Debug)]
pub struct ImageWidget {
    /// 控件标识。
    pub node: u64,
    /// 图像描述符。
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

/// 发射单个纹理图元。
#[must_use]
pub fn emit_image(node: u64, desc: &ImageDesc, state: ControlState) -> WidgetOutput {
    emit_image_inner(node, desc, state, None)
}

/// 发射纹素可能仍在解码中的图像。
///
/// 解析器尚未就绪（或加载失败）的资源会显示为同尺寸的内凹圆角占位符，
/// 因此纹理到达时布局绝不会跳动——仅绘制内容发生变化。
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
            // 裁剪图集子矩形而非整层：该图像只是共享图集页中的一个槽位。
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
            // 加载中或失败：内凹表面保留原有包围盒与圆角。
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

    // 纹理图元将边框槽位用于传递纹理层索引，因此边框作为带相同圆角的独立描边绘制在上方。
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

/// 九宫格图像：按固定边缘内边距拉伸的纹理。
#[derive(Clone, Debug, PartialEq)]
pub struct NinePatchDesc {
    /// 目标包围盒。
    pub rect: Rect,
    /// 纹理自然尺寸（像素）。
    pub natural_size: Vec2,
    /// 图集中的纹理层索引。
    pub texture_layer: u32,
    /// 纹理像素中的内边距：左、上、右、下。
    pub patch: [f32; 4],
    /// 着色色调；[`Token::NONE`] 表示保持纹理原色。
    pub tint: Token,
    /// 无障碍描述。
    pub alt: String,
    /// 绘制顺序键。
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

    /// 九个目标单元格，从左上角起按行主序排列。
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

    /// 九个 UV 单元格，从左上角起按行主序排列。
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

    /// 三列与三行的 `(position, length)` 区间。
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

/// 为九宫格图像发射九个按 UV 切分的纹理图元。
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

/// 将图标发射为着色图像。
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

/// 内置 24×24 SVG 路径矢量图标目录（66 个标准图标，S5.6）。
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
    /// 全部 66 个内置矢量图标。
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

    /// 该图标的标准 24×24 SVG 路径 `d` 字符串。
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

/// 内置矢量 [`IconPack`]，将 [`IconId`] 解析为缓存的 [`unbound_vector::QuadSegment`]。
#[derive(Debug, Default)]
pub struct IconPack {
    cache: unbound_vector::SvgPathCache,
}

impl IconPack {
    /// 创建新的图标包缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 将 [`IconId`] 解析为缩放到 `target_rect` 的二次贝塞尔线段。
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
        // `Fill` 采样整张图像，因此载荷即为图集矩形。
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

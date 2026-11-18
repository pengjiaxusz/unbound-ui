//! Mica 纹理生成：壁纸 cover 取样 → 降采样 → 磨砂 → 主题色调。

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};

use crate::selection::MaterialKind;
use crate::wallpaper::load_wallpaper;

/// 生成宽度上限：磨砂后降到 640 宽无感知，显著降低模糊、编码与上传成本
/// （1920×1080 → 640×360）。
pub const DEFAULT_MAX_WIDTH: u32 = 640;

/// 磨砂 sigma，单位是**屏幕物理像素**。
///
/// Mica 是「轻柔化 + 淡化」，不是 Acrylic 的重磨砂：保留壁纸构图，只做低频
/// 柔化。强度锚定屏幕物理像素后，1080p 屏与 4K 屏上的磨砂半径一样宽，
/// 壁纸分辨率、屏幕分辨率与显示缩放都不再改变观感。
///
/// 标定值 16.0：以屏幕物理像素计，16 像素的半径已能把壁纸的硬边与内部笔触
/// 摊成渐变（8 像素时轮廓仍清晰可辨），而继续加大只会把壁纸压成一片单色
/// ——那是 Acrylic 的做法，Mica 要保留构图与明暗层次。
pub const DEFAULT_SIGMA: f32 = 16.0;

/// 一次生成请求。
#[derive(Clone, Copy, Debug)]
pub struct MicaRequest {
    /// 主屏物理宽。
    pub screen_width: u32,
    /// 主屏物理高。
    pub screen_height: u32,
    /// 主屏 DPI 缩放；缩放变化会改变逻辑构图，必须使纹理失效。
    pub dpi_scale: f32,
    /// 深色主题。
    pub dark: bool,
    /// 纹理最大宽度（0 视为 [`DEFAULT_MAX_WIDTH`]）。
    pub max_width: u32,
    /// 磨砂 sigma，单位是屏幕物理像素（0 表示不模糊）。
    pub sigma: f32,
    /// 壁纸源指纹；壁纸切换后旧纹理必须失效。
    pub wallpaper: Option<crate::wallpaper::WallpaperStamp>,
    /// 实际生效的材质种类；它决定色调配方，切换材质必须重建纹理。
    pub material: MaterialKind,
}

impl MicaRequest {
    /// 按主屏尺寸与主题创建请求，其余参数取默认值。
    #[must_use]
    pub fn new(screen_width: u32, screen_height: u32, dark: bool) -> Self {
        Self {
            screen_width: screen_width.max(1),
            screen_height: screen_height.max(1),
            dpi_scale: 1.0,
            dark,
            max_width: DEFAULT_MAX_WIDTH,
            sigma: DEFAULT_SIGMA,
            wallpaper: None,
            material: MaterialKind::Mica,
        }
    }

    /// 从后景计划给出的纹理规格与壁纸指纹构造请求。
    #[must_use]
    pub fn from_texture(
        texture: &crate::plan::WallpaperTexture,
        wallpaper: Option<crate::wallpaper::WallpaperStamp>,
    ) -> Self {
        Self {
            screen_width: texture.screen_width.max(1),
            screen_height: texture.screen_height.max(1),
            dpi_scale: if texture.dpi_scale.is_finite() && texture.dpi_scale > 0.0 {
                texture.dpi_scale
            } else {
                1.0
            },
            dark: texture.dark,
            max_width: texture.effective_max_width(),
            sigma: texture.effective_sigma(),
            wallpaper,
            material: texture.material,
        }
    }

    /// 两个请求是否产出视觉等价的结果。
    ///
    /// 纹理覆盖整块主屏，因此窗口位置与窗口尺寸都不参与比较：移动窗口只更新
    /// 取样 UV，不重新生成纹理。只有主屏尺寸、DPI、主题、材质、纹理参数或
    /// 壁纸源变化才使旧纹理失效。
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        if self.material != other.material {
            return false;
        }
        if self.dark != other.dark || self.max_width.max(1) != other.max_width.max(1) {
            return false;
        }
        if (self.sigma.max(0.0) - other.sigma.max(0.0)).abs() > f32::EPSILON {
            return false;
        }
        if self.screen_width.max(1) != other.screen_width.max(1)
            || self.screen_height.max(1) != other.screen_height.max(1)
        {
            return false;
        }
        let dpi = |value: f32| {
            if value.is_finite() && value > 0.0 {
                value
            } else {
                1.0
            }
        };
        if (dpi(self.dpi_scale) - dpi(other.dpi_scale)).abs() > 1.0e-4 {
            return false;
        }
        self.wallpaper == other.wallpaper
    }
}

/// 生成结果：PNG 字节 + 实际纹理尺寸。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicaImage {
    /// 纹理像素宽。
    pub width: u32,
    /// 纹理像素高。
    pub height: u32,
    /// 编码后的 PNG。
    pub png: Vec<u8>,
}

/// 按请求生成一张覆盖整块主屏的 Mica 纹理。
///
/// 纹理属于“壁纸空间”，窗口之后只从中采样自己的那块子矩形；因此窗口位置与
/// 尺寸不进入请求，移动窗口不会触发重新生成。
///
/// 返回 `None` 表示拿不到静态壁纸或主屏尺寸（调用方回退纯色底）。
/// 该函数会读盘并做 CPU 密集运算，只应在工作线程调用。
#[must_use]
pub fn generate(request: &MicaRequest) -> Option<MicaImage> {
    let screen = (request.screen_width.max(1), request.screen_height.max(1));
    let (wallpaper, ..) = load_wallpaper().ok()?;
    let rendered = render_texture(
        &wallpaper,
        screen,
        (0, 0, screen.0, screen.1),
        request.max_width.max(1),
        request.sigma.max(0.0),
    )?;
    encode(tint(
        &rendered.blurred,
        &ToneRecipe::for_material(request.material, request.dark),
    ))
}

/// 把像素图编码为 PNG。
fn encode(image: ImageBuffer<Rgba<u8>, Vec<u8>>) -> Option<MicaImage> {
    let (width, height) = image.dimensions();
    let mut buffer = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut buffer, image::ImageFormat::Png)
        .ok()?;
    Some(MicaImage {
        width,
        height,
        png: buffer.into_inner(),
    })
}

/// 渲染结果：磨砂后的纹理、降采样参考纹理与纹理几何。
pub(crate) struct RenderedTexture {
    /// 磨砂后的纹理像素（未叠加主题色调）。
    pub blurred: ImageBuffer<Rgba<u8>, Vec<u8>>,
    /// 降采样后未磨砂的纹理像素，用作色偏参考。
    pub reference: ImageBuffer<Rgba<u8>, Vec<u8>>,
    /// 纹理几何。
    pub geometry: TextureGeometry,
}

/// 纹理几何：cover 裁剪、降采样尺寸与屏幕跨度。
///
/// 这些数字把「壁纸像素」「纹理像素」「屏幕物理像素」三种尺度接在一起，
/// 磨砂强度只有经过它们换算，才能锚定在屏幕上。
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextureGeometry {
    /// cover 放大倍数：一个壁纸像素对应多少屏幕物理像素。
    pub scale: f64,
    /// 裁剪区域尺寸（壁纸像素）。
    pub crop_size: (u32, u32),
    /// 降采样后的纹理尺寸（纹理像素）。
    pub target_size: (u32, u32),
}

impl TextureGeometry {
    /// 裁剪区域在屏幕物理像素中的跨度。
    pub fn screen_span(&self) -> (f64, f64) {
        (
            f64::from(self.crop_size.0) * self.scale,
            f64::from(self.crop_size.1) * self.scale,
        )
    }

    /// 一个纹理像素对应多少屏幕物理像素。
    pub fn screen_per_texture(&self) -> (f64, f64) {
        let (span_x, span_y) = self.screen_span();
        (
            span_x / f64::from(self.target_size.0),
            span_y / f64::from(self.target_size.1),
        )
    }

    /// 屏幕物理像素的 sigma 换算成纹理像素的 sigma。
    ///
    /// 磨砂半径必须锚定屏幕物理像素：同一个 sigma 在 1080p 屏上宽 16 像素，
    /// 在 4K 屏上也必须宽 16 像素，否则壁纸或屏幕分辨率一变观感就跟着漂。
    /// 换算按「一个纹理像素等于多少屏幕像素」的比例反推。
    pub fn texture_sigma(&self, sigma_screen: f32) -> f32 {
        let (per_x, per_y) = self.screen_per_texture();
        let per_texture = 0.5 * (per_x + per_y);
        if per_texture <= 0.0 {
            return 0.0;
        }
        sigma_screen / per_texture as f32
    }
}

/// 按真实配方渲染一块纹理：cover 裁剪 → 降采样 → 屏幕锚定磨砂。
///
/// 生成路径与纹理统计接缝共用这一份配方，统计量因此反映真实产物，而不是
/// 另算一套近似。
///
/// 降采样先于模糊：Triangle 低通 + 小图模糊与原分辨率模糊视觉一致，成本低
/// 一个量级；模糊半径按屏幕锚定换算，缩小不会削弱磨砂。
pub(crate) fn render_texture(
    wallpaper: &DynamicImage,
    screen: (u32, u32),
    viewport: (i32, i32, u32, u32),
    max_width: u32,
    sigma: f32,
) -> Option<RenderedTexture> {
    let crop = cover_crop(
        wallpaper.dimensions(),
        (f64::from(screen.0), f64::from(screen.1)),
        viewport,
    )?;
    let crop_image = wallpaper.crop_imm(
        crop.rect.0,
        crop.rect.1,
        crop.rect.2 - crop.rect.0,
        crop.rect.3 - crop.rect.1,
    );
    let (crop_width, crop_height) = crop_image.dimensions();

    let ratio = (f64::from(max_width) / f64::from(crop_width)).min(1.0);
    let target_width = ((f64::from(crop_width) * ratio).round() as u32).max(1);
    let target_height = ((f64::from(crop_height) * ratio).round() as u32).max(1);
    let small = image::imageops::resize(
        &crop_image,
        target_width,
        target_height,
        FilterType::Triangle,
    );

    let geometry = TextureGeometry {
        scale: crop.scale,
        crop_size: (crop_width, crop_height),
        target_size: (target_width, target_height),
    };
    let sigma = sigma.max(0.0);
    let (blurred, reference) = if sigma <= f32::EPSILON {
        (small.clone(), small)
    } else {
        (blur(&small, geometry.texture_sigma(sigma)), small)
    };
    Some(RenderedTexture {
        blurred,
        reference,
        geometry,
    })
}

/// cover 裁剪结果：壁纸像素中的可见矩形与放大倍数。
#[derive(Clone, Copy, Debug)]
struct CoverCrop {
    /// 可见矩形 `(left, top, right, bottom)`，壁纸像素。
    rect: (u32, u32, u32, u32),
    /// cover 放大倍数：一个壁纸像素对应多少屏幕像素。
    scale: f64,
}

/// 计算视口矩形在壁纸像素坐标中的可见区域与 cover 放大倍数。
///
/// cover 规则：壁纸等比放大到覆盖主屏，居中裁掉溢出部分。放大倍数同时给出
/// 「壁纸像素 → 屏幕像素」的比例，调用方据此把锚定屏幕的磨砂强度换算到
/// 纹理像素。
fn cover_crop(
    wallpaper: (u32, u32),
    screen: (f64, f64),
    viewport: (i32, i32, u32, u32),
) -> Option<CoverCrop> {
    if wallpaper.0 == 0 || wallpaper.1 == 0 || screen.0 <= 0.0 || screen.1 <= 0.0 {
        return None;
    }
    let scale = (screen.0 / f64::from(wallpaper.0)).max(screen.1 / f64::from(wallpaper.1));
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let offset_x = (screen.0 - f64::from(wallpaper.0) * scale) / 2.0;
    let offset_y = (screen.1 - f64::from(wallpaper.1) * scale) / 2.0;

    let (x, y, width, height) = viewport;
    let clamp_width = i64::from(wallpaper.0);
    let clamp_height = i64::from(wallpaper.1);
    let left = (((f64::from(x) - offset_x) / scale).floor() as i64).clamp(0, clamp_width) as u32;
    let top = (((f64::from(y) - offset_y) / scale).floor() as i64).clamp(0, clamp_height) as u32;
    let right = (((f64::from(x) + f64::from(width) - offset_x) / scale).ceil() as i64)
        .clamp(0, clamp_width) as u32;
    let bottom = (((f64::from(y) + f64::from(height) - offset_y) / scale).ceil() as i64)
        .clamp(0, clamp_height) as u32;

    (right > left && bottom > top).then_some(CoverCrop {
        rect: (left, top, right, bottom),
        scale,
    })
}

/// 自绘材质的色调配方：一层明度压缩，外加一层可选的向更深基准的压缩。
///
/// 配方把「磨砂之后怎么上色」的全部自由度收在一处：明度基准、明度区间、
/// 明度保留系数、色偏保留量与限幅，以及第二层压缩的深度。磨砂强度与锚定
/// 方式不在这里——它们属于纹理几何，对所有材质完全一致。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ToneRecipe {
    /// 明度压缩的基准色（WinUI `SolidBackgroundFillColorBase`）。
    base: f32,
    /// 明度区间下沿。
    minimum: f32,
    /// 明度区间上沿。
    maximum: f32,
    /// 明度向基准色压缩的保留系数，等价于 `LuminosityOpacity`。
    luminance_keep: f32,
    /// 色偏稀释系数。
    chroma_keep: f32,
    /// 色偏限幅。
    chroma_limit: f32,
    /// 第二层压缩的基准色，比第一层更深。
    depth_base: f32,
    /// 第二层压缩的保留系数；`1.0` 表示不追加这一层。
    depth_keep: f32,
}

impl ToneRecipe {
    /// 按材质种类与主题取色调配方。
    ///
    /// 只有云母变体偏离标准云母配方：它在同一套磨砂锚点上再叠一层更深的
    /// 明度压缩，呈现更低、更窄的明度区间。其余材质（无、跟随系统、亚克力、
    /// 沉浸式）在自绘路径上不会产出纹理，一律按标准云母处理。
    pub(crate) const fn for_material(material: MaterialKind, dark: bool) -> Self {
        if matches!(material, MaterialKind::MicaAlt) {
            // 云母变体：明度保留系数降到 0.10，区间更靠下、更窄，再向更深的
            // 基准压缩一次，得到标准云母看不到的深层色调。
            if dark {
                Self {
                    base: 32.0,
                    minimum: 22.0,
                    maximum: 40.0,
                    luminance_keep: 0.10,
                    chroma_keep: 0.16,
                    chroma_limit: 5.0,
                    depth_base: 24.0,
                    depth_keep: 0.75,
                }
            } else {
                Self {
                    base: 231.0,
                    minimum: 206.0,
                    maximum: 238.0,
                    luminance_keep: 0.10,
                    chroma_keep: 0.16,
                    chroma_limit: 5.0,
                    depth_base: 216.0,
                    depth_keep: 0.70,
                }
            }
        } else if dark {
            // 标准云母：WinUI `SolidBackgroundFillColorBase` #202020，
            // 明度区间窄到 28..48，再亮的壁纸也不能把窗口烧出高光。
            Self {
                base: 32.0,
                minimum: 28.0,
                maximum: 48.0,
                luminance_keep: 0.18,
                chroma_keep: 0.16,
                chroma_limit: 5.0,
                depth_base: 48.0,
                depth_keep: 1.0,
            }
        } else {
            // 标准云母亮色：基准 #F3F3F3，明度区间 228..255。
            Self {
                base: 243.0,
                minimum: 228.0,
                maximum: 255.0,
                luminance_keep: 0.18,
                chroma_keep: 0.16,
                chroma_limit: 5.0,
                depth_base: 255.0,
                depth_keep: 1.0,
            }
        }
    }
}

/// 按配方给磨砂纹理上色：明度向基准色压缩，色偏稀释后叠加回去。
///
/// 只做「按比例向基准色混合」会把彩色壁纸整片染成暖棕，正是 Win11 不会
/// 出现的效果。这里分别处理明度与色度：
///
/// - 明度以基准色为轴压缩，保留系数由配方给出，保证任何壁纸上的文字对比度；
/// - 色偏按配方比例稀释并限幅，保留壁纸的低频色调，但不铺开彩雾；
/// - 配方需要更深层次时再向第二层基准压缩一次。
pub(crate) fn tint(
    image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    recipe: &ToneRecipe,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let (r, g, b) = (
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        );
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let compressed = (recipe.base + (luminance - recipe.base) * recipe.luminance_keep)
            .clamp(recipe.minimum, recipe.maximum);
        // 第二层向更深的基准压缩：标准云母的保留系数为 1，这一步是恒等变换。
        let depth = recipe.depth_base + (compressed - recipe.depth_base) * recipe.depth_keep;
        let mix = |channel: f32| {
            let bias = ((channel - luminance) * recipe.chroma_keep)
                .clamp(-recipe.chroma_limit, recipe.chroma_limit);
            (depth + bias).clamp(0.0, 255.0).round() as u8
        };
        Rgba([mix(r), mix(g), mix(b), 255])
    })
}

/// 磨砂模糊：三次 box blur 近似高斯，O(n) 前缀和滑动窗口。
///
/// `imageops::blur` 对每个像素重新求邻域和（O(n×半径)），sigma=20 时 debug
/// 构建约 1.27s；前缀和实现每像素 O(1)，同样参数约 30ms，视觉一致。
fn blur(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, sigma: f32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    // 三次 box blur 的等效方差 ≈ r(r+1)，反解 r ≈ sigma·√(3/4)+0.5。
    let radius = ((sigma * 0.866) as usize + 1).max(1);
    let mut result = image.clone();
    for _ in 0..3 {
        result = box_blur_pass(&result, radius);
    }
    result
}

/// 单次 box blur（水平 + 垂直各一遍），边界 clamp 复制，前缀和实现 O(1) 窗口均值。
fn box_blur_pass(
    image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    radius: usize,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let (width, height) = (width as usize, height as usize);
    let radius = radius.max(1);
    let mut result = image.clone();

    // 水平 pass
    {
        let mut prefix = vec![[0i64; 4]; width + 1];
        for y in 0..height {
            prefix[0] = [0; 4];
            for x in 0..width {
                let pixel = image.get_pixel(x as u32, y as u32);
                for channel in 0..4 {
                    prefix[x + 1][channel] = prefix[x][channel] + pixel[channel] as i64;
                }
            }
            for x in 0..width {
                let left = x.saturating_sub(radius);
                let right = (x + radius).min(width - 1);
                let count = (right - left + 1) as i64;
                let pixel = result.get_pixel_mut(x as u32, y as u32);
                for channel in 0..4 {
                    pixel[channel] =
                        ((prefix[right + 1][channel] - prefix[left][channel]) / count) as u8;
                }
            }
        }
    }
    // 垂直 pass
    {
        let mut prefix = vec![[0i64; 4]; height + 1];
        for x in 0..width {
            prefix[0] = [0; 4];
            for y in 0..height {
                let pixel = result.get_pixel(x as u32, y as u32);
                for channel in 0..4 {
                    prefix[y + 1][channel] = prefix[y][channel] + pixel[channel] as i64;
                }
            }
            for y in 0..height {
                let top = y.saturating_sub(radius);
                let bottom = (y + radius).min(height - 1);
                let count = (bottom - top + 1) as i64;
                let pixel = result.get_pixel_mut(x as u32, y as u32);
                for channel in 0..4 {
                    pixel[channel] =
                        ((prefix[bottom + 1][channel] - prefix[top][channel]) / count) as u8;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_centered_viewport_maps_to_the_whole_wallpaper() {
        // 3840×2160 wallpaper on 1920×1080: exact cover, no offsets.
        let crop = cover_crop((3840, 2160), (1920.0, 1080.0), (0, 0, 1920, 1080)).unwrap();
        assert_eq!(crop.rect, (0, 0, 3840, 2160));
        assert_eq!(crop.scale, 0.5);
    }

    #[test]
    fn a_taller_screen_crops_the_wallpaper_sides() {
        // 3840×2160 on 1920×1200: scale 1200/2160, cover width ≈ 2133,
        // so the visible region starts at x ≈ 192 and ends at x ≈ 3648.
        let crop = cover_crop((3840, 2160), (1920.0, 1200.0), (0, 0, 1920, 1200)).unwrap();
        assert_eq!((crop.rect.0, crop.rect.1), (192, 0));
        assert_eq!((crop.rect.2, crop.rect.3), (3648, 2160));
    }

    #[test]
    fn an_offscreen_viewport_is_rejected_instead_of_panicking() {
        assert!(cover_crop((100, 100), (100.0, 100.0), (-50, -50, 20, 20)).is_none());
    }

    #[test]
    fn a_degenerate_wallpaper_is_rejected() {
        assert!(cover_crop((0, 0), (1920.0, 1080.0), (0, 0, 10, 10)).is_none());
        assert!(cover_crop((100, 100), (0.0, 0.0), (0, 0, 10, 10)).is_none());
    }

    /// 按材质与主题给一张图上色。
    fn tint_with(
        image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
        material: MaterialKind,
        dark: bool,
    ) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
        tint(image, &ToneRecipe::for_material(material, dark))
    }

    /// 一张纹理的平均明度。
    fn mean_luminance(image: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> f64 {
        let sum: f64 = image
            .pixels()
            .map(|pixel| {
                0.2126 * f64::from(pixel[0])
                    + 0.7152 * f64::from(pixel[1])
                    + 0.0722 * f64::from(pixel[2])
            })
            .sum();
        sum / f64::from(image.width() * image.height())
    }

    #[test]
    fn dark_mica_compresses_luminance_into_the_win11_band() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        let tinted = tint_with(&image, MaterialKind::Mica, true);
        assert_eq!(tinted.get_pixel(0, 0)[0], 28);

        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        let tinted = tint_with(&image, MaterialKind::Mica, true);
        assert_eq!(tinted.get_pixel(0, 0)[0], 48);
    }

    #[test]
    fn light_mica_compresses_luminance_into_the_win11_band() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        let tinted = tint_with(&image, MaterialKind::Mica, false);
        assert_eq!(tinted.get_pixel(0, 0)[0], 228);

        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        let tinted = tint_with(&image, MaterialKind::Mica, false);
        assert_eq!(tinted.get_pixel(0, 0)[0], 245);
    }

    #[test]
    fn tinting_keeps_a_trace_of_the_wallpaper_hue() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
        let pixel = tint_with(&image, MaterialKind::Mica, true)
            .get_pixel(0, 0)
            .0;
        assert!(pixel[0] > pixel[1], "red bias must survive tinting");
        assert!(pixel[0] - pixel[1] <= 10, "bias must stay subtle");
    }

    #[test]
    fn the_mica_variant_lays_a_deeper_tone_over_standard_mica() {
        // 同一张中灰壁纸：变体必须比标准云母更暗，且明度区间更窄。
        let image = ImageBuffer::from_pixel(2, 2, Rgba([128, 128, 128, 255]));
        let standard = tint_with(&image, MaterialKind::Mica, true);
        let variant = tint_with(&image, MaterialKind::MicaAlt, true);
        let standard_mean = mean_luminance(&standard);
        let variant_mean = mean_luminance(&variant);
        assert!(
            variant_mean + 6.0 < standard_mean,
            "the mica variant must be clearly darker: {variant_mean} vs {standard_mean}"
        );
        assert!(
            variant.get_pixel(0, 0)[0] < standard.get_pixel(0, 0)[0],
            "every channel must sit lower in the variant"
        );
    }

    #[test]
    fn the_mica_variant_stays_darker_than_standard_mica_in_light_theme() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([128, 128, 128, 255]));
        let standard = tint_with(&image, MaterialKind::Mica, false);
        let variant = tint_with(&image, MaterialKind::MicaAlt, false);
        assert!(
            mean_luminance(&variant) + 6.0 < mean_luminance(&standard),
            "the light variant must be clearly darker than the light standard mica"
        );
    }

    #[test]
    fn the_two_materials_share_one_frosting_anchor() {
        use crate::test_support::wallpaper_at;

        let screen = (1920, 1080);
        let rendered = render_texture(
            &wallpaper_at(1920, 1080),
            screen,
            (0, 0, screen.0, screen.1),
            DEFAULT_MAX_WIDTH,
            DEFAULT_SIGMA,
        )
        .expect("the wallpaper must render");

        // 磨砂发生在色调之前：两档材质拿到的纹理像素是同一份输入，
        // 纹理空间的 sigma 也只有一个来源——差异只在配方，不在磨砂锚点。
        let sigma = rendered.geometry.texture_sigma(DEFAULT_SIGMA);
        assert!(
            sigma > 0.0,
            "the calibrated blur must stay anchored on the screen"
        );
        // 重复渲染必须逐位相同：锚点换算不依赖任何材质状态。
        let repeated = render_texture(
            &wallpaper_at(1920, 1080),
            screen,
            (0, 0, screen.0, screen.1),
            DEFAULT_MAX_WIDTH,
            DEFAULT_SIGMA,
        )
        .expect("the wallpaper must render");
        assert_eq!(rendered.geometry.target_size, repeated.geometry.target_size);
        assert_eq!(
            repeated.geometry.texture_sigma(DEFAULT_SIGMA),
            sigma,
            "the frosting anchor must be identical for every recipe"
        );
        assert_eq!(
            rendered.blurred, repeated.blurred,
            "the pre-tint texture must be the shared input of both recipes"
        );

        let dark_standard = tint(
            &rendered.blurred,
            &ToneRecipe::for_material(MaterialKind::Mica, true),
        );
        let dark_variant = tint(
            &rendered.blurred,
            &ToneRecipe::for_material(MaterialKind::MicaAlt, true),
        );
        assert_ne!(
            dark_standard, dark_variant,
            "the two materials must produce visibly different tints"
        );
        assert!(
            mean_luminance(&dark_variant) + 4.0 < mean_luminance(&dark_standard),
            "the variant must keep its deeper tone on the shared anchor"
        );
    }

    #[test]
    fn a_uniform_image_stays_uniform_after_blurring() {
        let image = ImageBuffer::from_pixel(8, 8, Rgba([12, 34, 56, 255]));
        let blurred = blur(&image, 4.0);
        for pixel in blurred.pixels() {
            assert_eq!(pixel.0, [12, 34, 56, 255]);
        }
    }

    #[test]
    fn the_same_picture_blurs_the_same_at_any_wallpaper_resolution() {
        use crate::test_support::{portrait_wallpaper, wallpaper_at};

        let screen = (1920, 1080);
        let viewport = (0, 0, screen.0, screen.1);
        let baseline = render_texture(
            &wallpaper_at(1920, 1080),
            screen,
            viewport,
            DEFAULT_MAX_WIDTH,
            DEFAULT_SIGMA,
        )
        .expect("the wallpaper must render");

        for wallpaper in [wallpaper_at(3840, 2160), portrait_wallpaper(1280)] {
            let rendered = render_texture(
                &wallpaper,
                screen,
                viewport,
                DEFAULT_MAX_WIDTH,
                DEFAULT_SIGMA,
            )
            .expect("the wallpaper must render");
            assert_eq!(rendered.geometry.target_size, baseline.geometry.target_size);
            let difference = mean_absolute_difference(&baseline.blurred, &rendered.blurred);
            assert!(
                difference < 3.0,
                "the backdrop drifted by {difference} levels at {:?}",
                wallpaper.dimensions()
            );
        }
    }

    /// 两张纹理的平均绝对差（0..255）。
    fn mean_absolute_difference(
        left: &ImageBuffer<Rgba<u8>, Vec<u8>>,
        right: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    ) -> f64 {
        let mut sum = 0.0;
        let mut count = 0.0;
        for (left_pixel, right_pixel) in left.pixels().zip(right.pixels()) {
            for channel in 0..3 {
                sum += f64::from(left_pixel[channel].abs_diff(right_pixel[channel]));
            }
            count += 3.0;
        }
        sum / count
    }

    #[test]
    fn only_screen_theme_or_wallpaper_changes_invalidate_the_texture() {
        use crate::wallpaper::WallpaperStamp;

        let mut base = MicaRequest::new(2560, 1440, true);
        base.dpi_scale = 1.5;
        base.wallpaper = Some(WallpaperStamp {
            path_hash: 7,
            modified_secs: 100,
            modified_nanos: 200,
            len: 300,
        });

        // 窗口位置与尺寸不在请求里：同一屏幕参数必须视为可复用。
        assert!(base.matches(&base));
        assert!(base.matches(&MicaRequest {
            dpi_scale: 1.50001,
            ..base
        }));

        let mut resized = base;
        resized.screen_width = 1920;
        assert!(!base.matches(&resized));

        let mut scaled = base;
        scaled.dpi_scale = 1.25;
        assert!(!base.matches(&scaled));

        let mut light = base;
        light.dark = false;
        assert!(!base.matches(&light));

        // 材质决定色调配方：换变体必须使旧纹理失效。
        assert_eq!(base.material, MaterialKind::Mica);
        let mut variant = base;
        variant.material = MaterialKind::MicaAlt;
        assert!(
            !base.matches(&variant),
            "a material switch must invalidate the request"
        );
        assert!(
            !variant.matches(&base),
            "the material comparison must be symmetric"
        );

        let mut repainted = base;
        repainted.wallpaper = Some(WallpaperStamp {
            path_hash: 7,
            modified_secs: 101,
            modified_nanos: 200,
            len: 300,
        });
        assert!(!base.matches(&repainted));
    }
}

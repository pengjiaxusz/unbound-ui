//! Mica 纹理生成：壁纸 cover 取样 → 降采样 → 磨砂 → 主题色调。

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};

use crate::wallpaper::{load_wallpaper, primary_screen_logical_size};

/// 生成宽度上限：磨砂后降到 640 宽无感知，显著降低模糊、编码与上传成本
/// （1920×1080 → 640×360）。
pub const DEFAULT_MAX_WIDTH: u32 = 640;

/// 磨砂 sigma。Mica 是「轻柔化 + 淡化」，不是 Acrylic 的重磨砂：
/// 保留壁纸构图，只做低频柔化。
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
    /// 磨砂 sigma（0 表示不模糊）。
    pub sigma: f32,
    /// 壁纸源指纹；壁纸切换后旧纹理必须失效。
    pub wallpaper: Option<crate::wallpaper::WallpaperStamp>,
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
        }
    }

    /// 两个请求是否产出视觉等价的结果。
    ///
    /// 纹理覆盖整块主屏，因此窗口位置与窗口尺寸都不参与比较：移动窗口只更新
    /// 取样 UV，不重新生成纹理。只有主屏尺寸、DPI、主题、纹理参数或壁纸源
    /// 变化才使旧纹理失效。
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
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
    let blurred = render_blurred_rect_for_screen(
        screen,
        (0, 0, screen.0, screen.1),
        request.max_width,
        request.sigma,
    )?;
    encode(tint(&blurred, request.dark))
}

/// 生成指定屏幕矩形（逻辑坐标）的磨砂壁纸纹理 PNG，**不叠加主题色调**。
///
/// WebView 时代由前端 CSS 叠加色调，因此保留这一层；原生窗口应使用
/// [`generate`]，它会把 WinUI 的 `TintColor` / `LuminosityOpacity` 烘进纹理。
#[must_use]
pub fn generate_blurred_rect(
    viewport: (i32, i32, u32, u32),
    max_width: u32,
    sigma: f32,
) -> Option<MicaImage> {
    encode(render_blurred_rect(viewport, max_width, sigma)?)
}

fn render_blurred_rect(
    viewport: (i32, i32, u32, u32),
    max_width: u32,
    sigma: f32,
) -> Option<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let screen = primary_screen_logical_size()?;
    render_blurred_rect_for_screen(screen, viewport, max_width, sigma)
}

fn render_blurred_rect_for_screen(
    screen: (u32, u32),
    viewport: (i32, i32, u32, u32),
    max_width: u32,
    sigma: f32,
) -> Option<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let (wallpaper, ..) = load_wallpaper().ok()?;
    blurred_viewport(
        &wallpaper,
        screen,
        viewport,
        max_width.max(1),
        sigma.max(0.0),
    )
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

/// 把「壁纸 cover 到主屏」结果中视口矩形对应的区域裁剪出来，
/// 降采样后磨砂模糊。
///
/// 降采样先于模糊：Triangle 低通 + 小图模糊（等效 sigma 按比例缩放）与
/// 原分辨率模糊视觉一致，成本低一个量级。
fn blurred_viewport(
    wallpaper: &DynamicImage,
    screen: (u32, u32),
    viewport: (i32, i32, u32, u32),
    max_width: u32,
    sigma: f32,
) -> Option<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let (left, top, right, bottom) = cover_crop(
        wallpaper.dimensions(),
        (f64::from(screen.0), f64::from(screen.1)),
        viewport,
    )?;
    let crop = wallpaper.crop_imm(left, top, right - left, bottom - top);
    let (crop_width, crop_height) = crop.dimensions();

    let ratio = (f64::from(max_width) / f64::from(crop_width)).min(1.0);
    let target_width = ((f64::from(crop_width) * ratio).round() as u32).max(1);
    let target_height = ((f64::from(crop_height) * ratio).round() as u32).max(1);
    let small = image::imageops::resize(&crop, target_width, target_height, FilterType::Triangle);

    if sigma <= f32::EPSILON {
        Some(small)
    } else {
        Some(blur(&small, sigma * ratio as f32))
    }
}

/// 计算视口矩形在壁纸像素坐标中的可见区域 `(left, top, right, bottom)`。
///
/// cover 规则：壁纸等比放大到覆盖主屏，居中裁掉溢出部分。
fn cover_crop(
    wallpaper: (u32, u32),
    screen: (f64, f64),
    viewport: (i32, i32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
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

    (right > left && bottom > top).then_some((left, top, right, bottom))
}

/// 按 WinUI 的三层色调合成 Mica：明度向基准色压缩，色偏稀释后叠加回去。
///
/// 只做「按比例向基准色混合」会把彩色壁纸整片染成暖棕，正是 Win11 不会
/// 出现的效果。这里分别处理明度与色度：
///
/// - 明度以基准色为轴压缩到 18%，等价于 `LuminosityOpacity`，
///   保证任何壁纸上的文字对比度；
/// - 色偏按 16% 稀释并限制在 ±5，保留壁纸的低频色调，但不铺开彩雾。
fn tint(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, dark: bool) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    // WinUI `SolidBackgroundFillColorBase`: dark #202020, light #F3F3F3.
    let base = if dark { 32.0 } else { 243.0 };
    // 云母的明度区间本来就很窄：再亮的壁纸也不能把窗口烧出高光，
    // 再暗的区域也不能塌成纯黑。
    let (minimum, maximum) = if dark { (28.0, 48.0) } else { (228.0, 255.0) };
    const LUMINANCE_KEEP: f32 = 0.18;
    const CHROMA_KEEP: f32 = 0.16;
    const CHROMA_LIMIT: f32 = 5.0;

    ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let (r, g, b) = (
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        );
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let compressed = (base + (luminance - base) * LUMINANCE_KEEP).clamp(minimum, maximum);
        let mix = |channel: f32| {
            let bias = ((channel - luminance) * CHROMA_KEEP).clamp(-CHROMA_LIMIT, CHROMA_LIMIT);
            (compressed + bias).clamp(0.0, 255.0).round() as u8
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
        assert_eq!(crop, (0, 0, 3840, 2160));
    }

    #[test]
    fn a_taller_screen_crops_the_wallpaper_sides() {
        // 3840×2160 on 1920×1200: scale 1200/2160, cover width ≈ 2133,
        // so the visible region starts at x ≈ 192 and ends at x ≈ 3648.
        let crop = cover_crop((3840, 2160), (1920.0, 1200.0), (0, 0, 1920, 1200)).unwrap();
        assert_eq!((crop.0, crop.1), (192, 0));
        assert_eq!((crop.2, crop.3), (3648, 2160));
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

    #[test]
    fn dark_mica_compresses_luminance_into_the_win11_band() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        let tinted = tint(&image, true);
        assert_eq!(tinted.get_pixel(0, 0)[0], 28);

        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        let tinted = tint(&image, true);
        assert_eq!(tinted.get_pixel(0, 0)[0], 48);
    }

    #[test]
    fn light_mica_compresses_luminance_into_the_win11_band() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        let tinted = tint(&image, false);
        assert_eq!(tinted.get_pixel(0, 0)[0], 228);

        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        let tinted = tint(&image, false);
        assert_eq!(tinted.get_pixel(0, 0)[0], 245);
    }

    #[test]
    fn tinting_keeps_a_trace_of_the_wallpaper_hue() {
        let image = ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
        let pixel = tint(&image, true).get_pixel(0, 0).0;
        assert!(pixel[0] > pixel[1], "red bias must survive tinting");
        assert!(pixel[0] - pixel[1] <= 10, "bias must stay subtle");
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

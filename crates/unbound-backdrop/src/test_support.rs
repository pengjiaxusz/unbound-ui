//! 单元测试用的合成壁纸：确定性画面，不依赖真实桌面壁纸。
//!
//! 画面由低频底色与若干软斑组成：软斑的半径覆盖「刚好被磨砂吃掉」到「仍然
//! 看得见的大尺度轮廓」两档，因此磨砂强度一变，统计量立刻会动。
//!
//! 纵向以 [`VERTICAL_PERIOD`] 行为周期且首尾无缝，横版与竖版壁纸因此覆盖的是
//! 同一套图案，壁纸分辨率或宽高比变化时统计量才可比。

use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, Rgba};

/// 基准画面尺寸。
pub const CANONICAL_SIZE: (u32, u32) = (1920, 1080);

/// 基准画面的纵向周期（行）：整幅画面含三个周期，可无缝上下拼接。
pub const VERTICAL_PERIOD: u32 = 360;

/// 软斑：圆心 X（像素）、圆心 Y（周期内像素）、半径（像素）、颜色权重。
const BLOBS: [(f64, f64, f64, [f64; 3]); 7] = [
    (320.0, 110.0, 105.0, [0.42, 0.12, 0.04]),
    (1060.0, 250.0, 78.0, [0.06, 0.28, 0.52]),
    (820.0, 60.0, 125.0, [0.10, 0.46, 0.16]),
    (1580.0, 190.0, 58.0, [0.48, 0.38, 0.06]),
    (170.0, 290.0, 67.0, [0.34, 0.12, 0.46]),
    (640.0, 200.0, 34.0, [0.30, 0.20, 0.10]),
    (1300.0, 320.0, 26.0, [0.10, 0.24, 0.30]),
];

/// 生成基准画面。
pub fn synthetic_picture() -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = CANONICAL_SIZE;
    ImageBuffer::from_fn(width, height, |x, y| pattern(f64::from(x), f64::from(y)))
}

/// 把基准画面缩放到指定分辨率，得到同一张画面的另一份壁纸。
pub fn wallpaper_at(width: u32, height: u32) -> DynamicImage {
    let picture = synthetic_picture();
    if (width, height) == CANONICAL_SIZE {
        return DynamicImage::ImageRgba8(picture);
    }
    DynamicImage::ImageRgba8(image::imageops::resize(
        &picture,
        width,
        height,
        FilterType::Triangle,
    ))
}

/// 生成竖版壁纸：可见带里放基准画面，其余部分是无缝重复的画面段。
///
/// 16:9 屏 cover 竖版壁纸时，可见区域正好是壁纸中间的一条 16:9 带；把画面沿
/// 纵向无缝重复，可以让这条可见带与横版壁纸覆盖同一套图案，于是宽高比变化
/// 不会把统计量带偏。
///
/// 画布高度取宽度的 21/16：可见带起点落在图案周期的整数倍上，竖版壁纸看到
/// 的画面与横版壁纸逐周期对齐。
pub fn portrait_wallpaper(width: u32) -> DynamicImage {
    // 16:9 屏在竖版壁纸上的可见带高度，以及让可见带起点对齐周期的画布高度。
    let band_height = (width * 9 / 16).max(1);
    let height = width * 21 / 16;
    let band = image::imageops::resize(
        &synthetic_picture(),
        width,
        band_height,
        FilterType::Triangle,
    );
    let mut canvas = ImageBuffer::new(width, height);
    for y in 0..height {
        let source_y = y % band_height;
        for x in 0..width {
            canvas.put_pixel(x, y, *band.get_pixel(x, source_y));
        }
    }
    DynamicImage::ImageRgba8(canvas)
}

/// 生成一张带硬边的壁纸：左半深色、右半浅色，中间是一条锐利的分界。
///
/// 这条分界代表壁纸里最刺眼的大尺度轮廓；磨砂之后它必须摊成一道缓坡，
/// 因此它用来守住磨砂强度的下限。
pub fn hard_edge_wallpaper(width: u32, height: u32) -> DynamicImage {
    let mut image = ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let level = if x < width / 2 { 20 } else { 235 };
            image.put_pixel(x, y, Rgba([level, level, level, 255]));
        }
    }
    DynamicImage::ImageRgba8(image)
}

/// 画面图案：低频底色叠加若干软斑，纵向偏移按周期折返。
fn pattern(x: f64, y: f64) -> Rgba<u8> {
    let period = f64::from(VERTICAL_PERIOD);
    let base = 0.40
        + 0.14 * (std::f64::consts::TAU * x / f64::from(CANONICAL_SIZE.0)).cos()
        + 0.08 * (std::f64::consts::TAU * y / period).sin();
    let mut channels = [base, base * 0.94 + 0.02, base * 1.06];
    for (center_x, center_y, radius, tint) in BLOBS {
        let delta_x = x - center_x;
        let delta_y = wrap(y - center_y, period);
        let distance = (delta_x * delta_x + delta_y * delta_y).sqrt() / radius;
        let weight = (-distance * distance).exp();
        for (channel, offset) in channels.iter_mut().zip(tint) {
            *channel += offset * weight;
        }
    }
    Rgba([
        to_u8(channels[0]),
        to_u8(channels[1]),
        to_u8(channels[2]),
        255,
    ])
}

/// 把偏移折回 `[-period/2, period/2)`，保证纵向无缝。
fn wrap(value: f64, period: f64) -> f64 {
    let wrapped = value.rem_euclid(period);
    if wrapped > period / 2.0 {
        wrapped - period
    } else {
        wrapped
    }
}

/// 通道值钳到 0..255。
fn to_u8(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

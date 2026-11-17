//! 展厅与一致性校验共用的 PNG 读写。
//!
//! 渲染器回读的像素是 **BGRA**（交换链格式），而 PNG 容器使用 **RGBA**；
//! 写出时统一在此交换通道，读取时忽略 alpha 差异只保留直通 RGBA，
//! 使差分器、热力图与人工查看共享同一份解码结果。

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// 解码后的 RGBA8 图像。
pub struct Image {
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
    /// 行优先 RGBA 字节，长度 = `width * height * 4`。
    pub rgba: Vec<u8>,
}

impl Image {
    /// 读取某个像素的 RGB（忽略 alpha）。
    #[must_use]
    pub fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let index = ((y * self.width + x) * 4) as usize;
        [self.rgba[index], self.rgba[index + 1], self.rgba[index + 2]]
    }
}

/// 把渲染器回读的 BGRA 像素写成 PNG，必要时创建父目录。
pub fn write_bgra_png(path: &str, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let file = File::create(path).map_err(|error| error.to_string())?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    writer
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())
}

/// 把已是 RGBA 的像素写成 PNG（热力图等 CPU 生成图）。
pub fn write_rgba_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let file = File::create(path).map_err(|error| error.to_string())?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(rgba)
        .map_err(|error| error.to_string())
}

/// 解码任意 PNG 为 RGBA8。
pub fn read_rgba_png(path: &Path) -> Result<Image, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("{}: unknown output size", path.display()))?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let (width, height) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        other => {
            return Err(format!(
                "{}: unsupported color type {other:?}",
                path.display()
            ))
        }
    };
    Ok(Image {
        width,
        height,
        rgba,
    })
}

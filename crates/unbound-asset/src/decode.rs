//! 后台解码器。
//!
//! 解码是资产加载中最耗时的部分：一张 4K PNG 需要数毫秒，KTX2 转码耗时更长。
//! 这些操作绝不能在绘制帧的线程上执行。
//!
//!
//! [`Decoder`] trait 是解耦边界：管线本身不了解 PNG 等格式细节，
//! 只负责将字节流转换为 [`DecodedAsset`]。

use crate::transfer::{AssetError, AssetKind, DecodedAsset};

/// 将原始字节解码为资产载荷。
pub trait Decoder: Send + Sync {
    /// 解码单个载荷。
    ///
    /// # Errors
    /// 当载荷格式损坏时返回可读的错误描述。
    fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError>;
}

/// 原样保留载荷的解码器。
///
/// 用于由应用自行解释的资产，或作为未知类型的回退解码器。
///
#[derive(Copy, Clone, Debug, Default)]
pub struct RawDecoder;

impl Decoder for RawDecoder {
    fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError> {
        Ok(DecodedAsset {
            kind,
            bytes: bytes.to_vec(),
            width: 0,
            height: 0,
        })
    }
}

/// PNG 图像解码器。
///
/// 输出始终为 RGBA8，这正是无绑定图集（bindless atlas）所期望的格式。
///
#[derive(Copy, Clone, Debug, Default)]
pub struct PngDecoder;

impl Decoder for PngDecoder {
    fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError> {
        let decoder = png::Decoder::new(bytes);
        let mut reader = decoder
            .read_info()
            .map_err(|error| AssetError::Decode(error.to_string()))?;
        let mut buffer = vec![0u8; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|error| AssetError::Decode(error.to_string()))?;
        let width = info.width;
        let height = info.height;
        // 将所有输入归一化为 RGBA8：图集使用统一格式，
        // 在此处完成转换可避免着色器中出现按格式分支。
        let rgba = match info.color_type {
            png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
            png::ColorType::Rgb => {
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for pixel in buffer[..info.buffer_size()].chunks_exact(3) {
                    out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
                out
            }
            png::ColorType::Grayscale => {
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for pixel in buffer[..info.buffer_size()].chunks_exact(1) {
                    out.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]);
                }
                out
            }
            png::ColorType::GrayscaleAlpha => {
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for pixel in buffer[..info.buffer_size()].chunks_exact(2) {
                    out.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
                }
                out
            }
            png::ColorType::Indexed => {
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for index in buffer[..info.buffer_size()].iter() {
                    out.extend_from_slice(&[*index, *index, *index, 255]);
                }
                out
            }
        };
        Ok(DecodedAsset {
            kind: if kind == AssetKind::Raw {
                AssetKind::Image
            } else {
                kind
            },
            bytes: rgba,
            width,
            height,
        })
    }
}

/// 按资产类型分发的组合解码器。
pub struct KindDecoder {
    png: PngDecoder,
    raw: RawDecoder,
}

impl Default for KindDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl KindDecoder {
    /// 构建解码分发器。
    #[must_use]
    pub fn new() -> Self {
        Self {
            png: PngDecoder,
            raw: RawDecoder,
        }
    }
}

impl Decoder for KindDecoder {
    fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError> {
        match kind {
            AssetKind::Image => self.png.decode(kind, bytes),
            _ => self.raw.decode(kind, bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在内存中构造一个极小的 PNG，使测试无需依赖外部夹具文件。
    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
                ])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn a_png_is_decoded_to_rgba8() {
        let decoder = PngDecoder;
        let asset = decoder.decode(AssetKind::Image, &tiny_png()).unwrap();
        assert_eq!(asset.width, 2);
        assert_eq!(asset.height, 2);
        assert_eq!(asset.bytes.len(), 2 * 2 * 4);
        assert_eq!(&asset.bytes[0..4], &[255, 0, 0, 255]);
        assert_eq!(&asset.bytes[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn a_broken_png_reports_a_decode_error() {
        let decoder = PngDecoder;
        let error = decoder.decode(AssetKind::Image, b"not a png").unwrap_err();
        assert!(matches!(error, AssetError::Decode(_)));
    }

    #[test]
    fn the_kind_dispatcher_routes_images_to_the_png_decoder() {
        let decoder = KindDecoder::new();
        let asset = decoder.decode(AssetKind::Image, &tiny_png()).unwrap();
        assert_eq!(asset.width, 2);
        let raw = decoder.decode(AssetKind::Raw, b"payload").unwrap();
        assert_eq!(raw.bytes, b"payload");
        assert_eq!(raw.width, 0);
    }
}

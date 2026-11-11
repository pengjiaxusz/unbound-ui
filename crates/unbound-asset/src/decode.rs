//! Background decoders.
//!
//! Decoding is the expensive part of loading an asset: a 4 K PNG takes
//! milliseconds, a KTX2 transcode takes more. None of that may happen on the
//! thread that draws the frame.
//!
//! The [`Decoder`] trait is the seam: the pipeline knows nothing about PNGs,
//! it only knows how to turn bytes into a [`DecodedAsset`].

use crate::transfer::{AssetError, AssetKind, DecodedAsset};

/// Turns raw bytes into a decoded asset.
pub trait Decoder: Send + Sync {
    /// Decode one payload.
    ///
    /// # Errors
    /// Returns a human-readable description when the payload is malformed.
    fn decode(&self, kind: AssetKind, bytes: &[u8]) -> Result<DecodedAsset, AssetError>;
}

/// A decoder that keeps the payload as it is.
///
/// Used for assets the application interprets itself, and as the fallback for
/// an unknown kind.
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

/// A decoder for PNG images.
///
/// The output is always RGBA8, which is the format the bindless atlas
/// expects.
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
        // Normalise every input to RGBA8: the atlas is a single format, and
        // converting here keeps the shader free of per-format branches.
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

/// A decoder that dispatches on the asset kind.
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
    /// Build the dispatcher.
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

    /// Build a tiny PNG in memory so the test needs no fixture file.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255])
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

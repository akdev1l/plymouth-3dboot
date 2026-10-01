// SPDX-License-Identifier: GPL-3.0-or-later
//! PNG encoding and decoding of [`ColorBuffer`]s.

use std::io::Cursor;

use crate::color::Rgba8;
use crate::target::{ColorBuffer, SizeError};

/// Errors from PNG encoding or decoding.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PngError {
    /// The encoder rejected the image (for example a zero-sized buffer).
    #[error("PNG encoding failed: {0}")]
    Encode(String),
    /// The data is not a valid or supported PNG.
    #[error("PNG decoding failed: {0}")]
    Decode(String),
    /// The decoded image is larger than a [`ColorBuffer`] can hold.
    #[error(transparent)]
    Size(#[from] SizeError),
}

/// Encodes a colour buffer as an 8-bit RGBA PNG tagged as sRGB.
///
/// The output is deterministic: identical buffers give identical bytes.
///
/// # Errors
///
/// Returns [`PngError::Encode`] if the buffer has a zero dimension, which
/// PNG cannot represent.
pub fn encode(buffer: &ColorBuffer) -> Result<Vec<u8>, PngError> {
    let encode_err = |e: ::png::EncodingError| PngError::Encode(e.to_string());
    let mut out = Vec::new();
    let mut encoder = ::png::Encoder::new(&mut out, buffer.width(), buffer.height());
    encoder.set_color(::png::ColorType::Rgba);
    encoder.set_depth(::png::BitDepth::Eight);
    encoder.set_source_srgb(::png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(encode_err)?;
    writer
        .write_image_data(buffer.as_bytes())
        .map_err(encode_err)?;
    writer.finish().map_err(encode_err)?;
    Ok(out)
}

/// Decodes a PNG into a colour buffer.
///
/// Every PNG colour type is accepted. Grey and RGB images become opaque RGBA,
/// palettes are expanded and 16-bit channels are reduced to 8 bits. Sample
/// values are copied as-is (no colour management); for animated PNGs only
/// the default image is read.
///
/// # Errors
///
/// Returns [`PngError::Decode`] for malformed data and [`PngError::Size`]
/// if the image exceeds [`crate::target::MAX_DIMENSION`].
pub fn decode(data: &[u8]) -> Result<ColorBuffer, PngError> {
    let decode_err = |e: ::png::DecodingError| PngError::Decode(e.to_string());
    let mut decoder = ::png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(::png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(decode_err)?;
    let (width, height) = (reader.info().width, reader.info().height);
    // Check the size before allocating anything proportional to it.
    let mut buffer = ColorBuffer::new(width, height, Rgba8::TRANSPARENT)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| PngError::Decode("image too large".into()))?;
    let mut raw = vec![0; size];
    let info = reader.next_frame(&mut raw).map_err(decode_err)?;

    let channels = match info.color_type {
        ::png::ColorType::Grayscale => 1,
        ::png::ColorType::GrayscaleAlpha => 2,
        ::png::ColorType::Rgb => 3,
        ::png::ColorType::Rgba => 4,
        ::png::ColorType::Indexed => {
            return Err(PngError::Decode("palette was not expanded".into()));
        }
    };
    let width = width as usize;
    for (y, out_row) in buffer
        .pixels_mut()
        .chunks_exact_mut(width.max(1))
        .enumerate()
    {
        let in_row = &raw[y * info.line_size..][..width * channels];
        for (out, px) in out_row.iter_mut().zip(in_row.chunks_exact(channels)) {
            *out = match *px {
                [g] => Rgba8::new(g, g, g, 255),
                [g, a] => Rgba8::new(g, g, g, a),
                [r, g, b] => Rgba8::new(r, g, b, 255),
                [r, g, b, a] => Rgba8::new(r, g, b, a),
                _ => unreachable!("chunks have exactly `channels` elements"),
            };
        }
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern() -> ColorBuffer {
        let mut c = ColorBuffer::new(2, 2, Rgba8::TRANSPARENT).unwrap();
        *c.get_mut(0, 0).unwrap() = Rgba8::new(255, 0, 0, 255);
        *c.get_mut(1, 0).unwrap() = Rgba8::new(0, 255, 0, 128);
        *c.get_mut(0, 1).unwrap() = Rgba8::new(0, 0, 255, 0);
        *c.get_mut(1, 1).unwrap() = Rgba8::new(10, 20, 30, 40);
        c
    }

    /// Encodes raw samples with the png crate directly, for decoder tests.
    fn raw_png(
        width: u32,
        height: u32,
        color: ::png::ColorType,
        depth: ::png::BitDepth,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = ::png::Encoder::new(&mut out, width, height);
        e.set_color(color);
        e.set_depth(depth);
        let mut w = e.write_header().unwrap();
        w.write_image_data(data).unwrap();
        w.finish().unwrap();
        out
    }

    #[test]
    fn rgba_round_trips_exactly() {
        let c = pattern();
        assert_eq!(decode(&encode(&c).unwrap()).unwrap(), c);
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(encode(&pattern()).unwrap(), encode(&pattern()).unwrap());
    }

    #[test]
    fn encoded_png_is_tagged_srgb() {
        let bytes = encode(&pattern()).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"sRGB"));
    }

    #[test]
    fn rgb_and_grey_decode_as_opaque_rgba() {
        let rgb = raw_png(
            2,
            1,
            ::png::ColorType::Rgb,
            ::png::BitDepth::Eight,
            &[1, 2, 3, 4, 5, 6],
        );
        let c = decode(&rgb).unwrap();
        assert_eq!(
            c.pixels(),
            &[Rgba8::new(1, 2, 3, 255), Rgba8::new(4, 5, 6, 255)]
        );

        let grey = raw_png(
            1,
            2,
            ::png::ColorType::GrayscaleAlpha,
            ::png::BitDepth::Eight,
            &[7, 8, 9, 10],
        );
        let c = decode(&grey).unwrap();
        assert_eq!(
            c.pixels(),
            &[Rgba8::new(7, 7, 7, 8), Rgba8::new(9, 9, 9, 10)]
        );
    }

    #[test]
    fn sixteen_bit_decodes_to_high_byte() {
        let data = [0x12, 0x34, 0xab, 0xcd, 0xff, 0xff];
        let c = decode(&raw_png(
            1,
            1,
            ::png::ColorType::Rgb,
            ::png::BitDepth::Sixteen,
            &data,
        ))
        .unwrap();
        assert_eq!(c.get(0, 0), Some(Rgba8::new(0x12, 0xab, 0xff, 255)));
    }

    #[test]
    fn sub_byte_grey_rows_decode() {
        // 3x2 at 1 bit per pixel: rows are padded to whole bytes.
        let data = [0b1010_0000, 0b0110_0000];
        let c = decode(&raw_png(
            3,
            2,
            ::png::ColorType::Grayscale,
            ::png::BitDepth::One,
            &data,
        ))
        .unwrap();
        let v: Vec<u8> = c.pixels().iter().map(|p| p.r).collect();
        assert_eq!(v, [255, 0, 255, 0, 255, 255]);
    }

    #[test]
    fn garbage_is_a_decode_error() {
        assert!(matches!(decode(b"not a png"), Err(PngError::Decode(_))));
        let mut truncated = encode(&pattern()).unwrap();
        truncated.truncate(truncated.len() / 2);
        assert!(matches!(decode(&truncated), Err(PngError::Decode(_))));
    }

    #[test]
    fn oversized_image_is_a_size_error() {
        let wide = raw_png(
            16_385,
            1,
            ::png::ColorType::Grayscale,
            ::png::BitDepth::One,
            &[0; 2049],
        );
        assert!(matches!(decode(&wide), Err(PngError::Size(_))));
    }

    #[test]
    fn zero_sized_buffer_is_an_encode_error() {
        let c = ColorBuffer::new(0, 3, Rgba8::BLACK).unwrap();
        assert!(matches!(encode(&c), Err(PngError::Encode(_))));
    }
}

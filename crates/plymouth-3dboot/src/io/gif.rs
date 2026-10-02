// SPDX-License-Identifier: GPL-3.0-or-later
//! Animated GIF encoding (feature `gif`).

use ::gif::{DisposalMethod, Encoder, Frame, Repeat};

use crate::target::ColorBuffer;

/// Encoding a GIF failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GifError {
    /// There are no frames, or frames differ in size, or a frame is larger
    /// than GIF allows (65535 pixels per side).
    #[error("invalid frames: {0}")]
    Frames(String),
    /// The encoder failed.
    #[error("GIF encoding failed: {0}")]
    Encode(String),
}

/// Quantizer speed for colour reduction to 256 colours (1 = best, 30 =
/// fastest); 10 is the `gif` crate's recommended balance.
const QUANTIZER_SPEED: i32 = 10;

/// Encodes `frames` as a looping animated GIF shown at `fps` frames per
/// second.
///
/// Each frame is reduced to its own 256-colour palette; fully transparent
/// pixels stay transparent. GIF delays are whole centiseconds, so the frame
/// delay is `100 / fps` rounded (at least 1).
///
/// # Errors
///
/// Returns [`GifError`] for an empty or inconsistent frame list or a
/// non-positive `fps`.
pub fn encode(frames: &[ColorBuffer], fps: f64) -> Result<Vec<u8>, GifError> {
    let first = frames
        .first()
        .ok_or_else(|| GifError::Frames("no frames".into()))?;
    let size = (first.width(), first.height());
    if let Some(bad) = frames.iter().find(|f| (f.width(), f.height()) != size) {
        return Err(GifError::Frames(format!(
            "frame size {}x{} differs from {}x{}",
            bad.width(),
            bad.height(),
            size.0,
            size.1
        )));
    }
    let (Ok(width), Ok(height)) = (u16::try_from(size.0), u16::try_from(size.1)) else {
        return Err(GifError::Frames(format!(
            "{}x{} exceeds the GIF size limit",
            size.0, size.1
        )));
    };
    if !(fps.is_finite() && fps > 0.0) {
        return Err(GifError::Frames(format!("invalid frame rate {fps}")));
    }
    // Clamped to the representable range before converting.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let delay = (100.0 / fps).round().clamp(1.0, f64::from(u16::MAX)) as u16;
    let encode_err = |e: ::gif::EncodingError| GifError::Encode(e.to_string());
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, width, height, &[]).map_err(encode_err)?;
        encoder.set_repeat(Repeat::Infinite).map_err(encode_err)?;
        for f in frames {
            let mut rgba = f.as_bytes().to_vec();
            let mut frame = Frame::from_rgba_speed(width, height, &mut rgba, QUANTIZER_SPEED);
            frame.delay = delay;
            frame.dispose = DisposalMethod::Background;
            encoder.write_frame(&frame).map_err(encode_err)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    fn frame(c: Rgba8) -> ColorBuffer {
        let mut f = ColorBuffer::new(8, 4, Rgba8::BLACK).unwrap();
        *f.get_mut(1, 1).unwrap() = c;
        f
    }

    #[test]
    fn frame_count_delays_and_colours_survive_decoding() {
        let colours = [
            Rgba8::new(255, 0, 0, 255),
            Rgba8::new(0, 255, 0, 255),
            Rgba8::new(0, 0, 255, 255),
        ];
        let frames: Vec<_> = colours.iter().map(|&c| frame(c)).collect();
        let bytes = encode(&frames, 30.0).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");

        let mut options = ::gif::DecodeOptions::new();
        options.set_color_output(::gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(&bytes[..]).unwrap();
        let mut decoded = Vec::new();
        while let Some(f) = decoder.read_next_frame().unwrap() {
            assert_eq!(f.delay, 3, "100 / 30 fps rounds to 3 cs");
            let px = &f.buffer[(8 + 1) * 4..][..4];
            decoded.push([px[0], px[1], px[2], px[3]]);
        }
        assert_eq!(decoded, colours.map(Rgba8::to_array));
        assert_eq!(decoder.repeat(), Repeat::Infinite);
    }

    #[test]
    fn encoding_is_deterministic() {
        let frames = [frame(Rgba8::WHITE), frame(Rgba8::new(10, 20, 30, 255))];
        assert_eq!(
            encode(&frames, 10.0).unwrap(),
            encode(&frames, 10.0).unwrap()
        );
    }

    #[test]
    fn invalid_input_is_rejected() {
        assert!(matches!(encode(&[], 30.0), Err(GifError::Frames(_))));
        let mixed = [
            frame(Rgba8::WHITE),
            ColorBuffer::new(2, 2, Rgba8::BLACK).unwrap(),
        ];
        assert!(matches!(encode(&mixed, 30.0), Err(GifError::Frames(_))));
        assert!(matches!(
            encode(&[frame(Rgba8::WHITE)], 0.0),
            Err(GifError::Frames(_))
        ));
        let tiny = ColorBuffer::new(1, 1, Rgba8::BLACK).unwrap();
        assert!(
            encode(&[tiny], 1000.0).is_ok(),
            "delay clamps to at least 1 cs"
        );
    }
}

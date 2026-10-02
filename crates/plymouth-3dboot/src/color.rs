// SPDX-License-Identifier: GPL-3.0-or-later
//! Colour types and sRGB conversions.
//!
//! Shading happens in linear light ([`LinearRgba`]); framebuffers store
//! sRGB-encoded 8-bit values ([`Rgba8`]). Alpha is always linear and stored
//! straight (not premultiplied). Transfer functions use `libm` so results are
//! identical on every target.

use core::ops::{Add, Mul};

/// Converts one sRGB-encoded component in `[0, 1]` to linear light.
#[must_use]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        libm::powf((c + 0.055) / 1.055, 2.4)
    }
}

/// Converts one linear-light component in `[0, 1]` to sRGB encoding.
#[must_use]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * libm::powf(c, 1.0 / 2.4) - 0.055
    }
}

/// Linear value of each sRGB-encoded byte (computed once).
pub(crate) fn srgb8_decode_table() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            srgb_to_linear(f32::from(u8::try_from(i).unwrap_or(u8::MAX)) / 255.0)
        })
    })
}

/// Buckets of the encode index: `[0, 1]` is split into this many equal
/// parts.
const ENCODE_BUCKETS: usize = 1024;

/// Tables for exact sRGB encoding by lookup.
struct EncodeTables {
    /// `thresholds[k]`: linear value of sRGB `(k + 0.5) / 255`, separating
    /// byte `k` from `k + 1`.
    thresholds: [f32; 255],
    /// `start[b]`: the encoded byte of the lowest value in bucket `b`
    /// (thresholds at or below `b / ENCODE_BUCKETS`).
    start: [u8; ENCODE_BUCKETS + 1],
}

impl EncodeTables {
    fn get() -> &'static Self {
        static TABLES: std::sync::OnceLock<EncodeTables> = std::sync::OnceLock::new();
        TABLES.get_or_init(|| {
            #[allow(clippy::cast_precision_loss)]
            let thresholds: [f32; 255] =
                std::array::from_fn(|k| srgb_to_linear((k as f32 + 0.5) / 255.0));
            let start = std::array::from_fn(|b| {
                #[allow(clippy::cast_precision_loss)]
                let low = b as f32 / ENCODE_BUCKETS as f32;
                u8::try_from(thresholds.partition_point(|&t| t <= low)).unwrap_or(u8::MAX)
            });
            Self { thresholds, start }
        })
    }

    /// The nearest sRGB byte for `linear`: the number of thresholds at or
    /// below it, found by jumping to its bucket and stepping forward (a few
    /// steps at most, since thresholds are sparse in each bucket).
    fn encode(&self, linear: f32) -> u8 {
        if linear.is_nan() || linear <= 0.0 {
            return 0;
        }
        if linear >= 1.0 {
            return 255;
        }
        // In range, so the bucket index is within 0..ENCODE_BUCKETS.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let bucket = (linear * ENCODE_BUCKETS as f32) as usize;
        let mut k = usize::from(self.start[bucket.min(ENCODE_BUCKETS)]);
        while k < 255 && self.thresholds[k] <= linear {
            k += 1;
        }
        u8::try_from(k).unwrap_or(u8::MAX)
    }
}

/// Encodes a linear value in `[0, 1]` as the nearest sRGB byte (values
/// outside are clamped, NaN gives 0). Uses tables of rounding thresholds
/// instead of evaluating the transfer function, which is exact, fast and
/// identical on every target.
#[must_use]
pub fn encode_srgb8(linear: f32) -> u8 {
    EncodeTables::get().encode(linear)
}

/// Quantizes a `[0, 1]` value to `u8`, rounding to nearest (halves away from
/// zero). Out-of-range values are clamped, and NaN maps to 0.
#[must_use]
pub fn unorm8(c: f32) -> u8 {
    if c.is_nan() {
        return 0;
    }
    let x = c.clamp(0.0, 1.0) * 255.0;
    // Non-negative and at most 255: truncation is floor, and comparing the
    // fraction rounds exactly like `f32::round` without a libm call.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let floor = x as u8;
    if x - f32::from(floor) >= 0.5 {
        floor + 1
    } else {
        floor
    }
}

/// An sRGB-encoded 8-bit colour with straight alpha, in `R, G, B, A` byte
/// order (the framebuffer pixel format).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Rgba8 {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (linear coverage).
    pub a: u8,
}

impl Rgba8 {
    /// Opaque black.
    pub const BLACK: Self = Self::new(0, 0, 0, 255);
    /// Opaque white.
    pub const WHITE: Self = Self::new(255, 255, 255, 255);
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);

    /// Creates a colour from its components.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// The components as `[r, g, b, a]`.
    #[must_use]
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Converts to linear light.
    #[must_use]
    pub fn to_linear(self) -> LinearRgba {
        LinearRgba::from_srgb8(self)
    }
}

impl From<[u8; 4]> for Rgba8 {
    fn from([r, g, b, a]: [u8; 4]) -> Self {
        Self { r, g, b, a }
    }
}

impl From<Rgba8> for [u8; 4] {
    fn from(c: Rgba8) -> Self {
        c.to_array()
    }
}

/// A colour in linear light with straight (non-premultiplied) alpha.
///
/// Components are nominally in `[0, 1]` but may exceed it during shading;
/// they are clamped when encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinearRgba {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha.
    pub a: f32,
}

impl LinearRgba {
    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    /// Opaque white.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Creates a colour from linear components.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Creates an opaque colour from linear components.
    #[must_use]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    /// Decodes from sRGB-encoded components in `[0, 1]` (alpha passes through).
    #[must_use]
    pub fn from_srgb(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::new(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a)
    }

    /// Decodes an 8-bit sRGB colour.
    #[must_use]
    pub fn from_srgb8(c: Rgba8) -> Self {
        let f = |v: u8| f32::from(v) / 255.0;
        Self::from_srgb(f(c.r), f(c.g), f(c.b), f(c.a))
    }

    /// Encodes to 8-bit sRGB, clamping out-of-range components (see
    /// [`encode_srgb8`]).
    #[must_use]
    pub fn to_srgb8(self) -> Rgba8 {
        let tables = EncodeTables::get();
        Rgba8::new(
            tables.encode(self.r),
            tables.encode(self.g),
            tables.encode(self.b),
            unorm8(self.a),
        )
    }

    /// Linear interpolation: `self` at `t = 0`, `other` at `t = 1`.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other + self * -1.0) * t
    }

    /// Component-wise product (e.g. light colour × surface colour).
    #[must_use]
    pub fn modulate(self, other: Self) -> Self {
        Self::new(
            self.r * other.r,
            self.g * other.g,
            self.b * other.b,
            self.a * other.a,
        )
    }
}

impl Add for LinearRgba {
    type Output = Self;

    fn add(self, o: Self) -> Self {
        Self::new(self.r + o.r, self.g + o.g, self.b + o.b, self.a + o.a)
    }
}

impl Mul<f32> for LinearRgba {
    type Output = Self;

    fn mul(self, s: f32) -> Self {
        Self::new(self.r * s, self.g * s, self.b * s, self.a * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_srgb8_value_round_trips_through_linear() {
        for v in 0..=255u8 {
            let c = Rgba8::new(v, v, v, v);
            assert_eq!(c.to_linear().to_srgb8(), c, "value {v}");
        }
    }

    #[test]
    fn transfer_functions_match_reference_values() {
        assert!((srgb_to_linear(0.5) - 0.214_041).abs() < 1e-5);
        assert!((linear_to_srgb(0.5) - 0.735_357).abs() < 1e-5);
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        // Linear segment.
        assert!((srgb_to_linear(0.04) - 0.04 / 12.92).abs() < 1e-9);
    }

    #[test]
    fn table_encoding_matches_the_transfer_function() {
        let mut differing = 0;
        for i in 0..=100_000 {
            #[allow(clippy::cast_precision_loss)]
            let linear = i as f32 / 100_000.0;
            let direct = unorm8(linear_to_srgb(linear));
            let table = encode_srgb8(linear);
            assert!(table.abs_diff(direct) <= 1, "{linear}");
            differing += usize::from(table != direct);
        }
        // Only values within float rounding of a threshold can differ.
        assert!(differing < 10, "{differing} values differ");
        assert_eq!(encode_srgb8(-1.0), 0);
        assert_eq!(encode_srgb8(2.0), 255);
        assert_eq!(encode_srgb8(f32::NAN), 0);
    }

    #[test]
    fn bucketed_encoding_equals_threshold_search_and_rounding_is_exact() {
        let t = &EncodeTables::get().thresholds;
        for i in 0..=200_000 {
            #[allow(clippy::cast_precision_loss)]
            let linear = i as f32 / 200_000.0;
            let search = u8::try_from(t.partition_point(|&th| th <= linear)).unwrap();
            assert_eq!(encode_srgb8(linear), search, "{linear}");
        }
        // Exactly on thresholds and just below.
        for &th in t {
            assert_eq!(
                encode_srgb8(th),
                u8::try_from(t.partition_point(|&x| x <= th)).unwrap()
            );
            let below = f32::from_bits(th.to_bits() - 1);
            assert_eq!(
                encode_srgb8(below),
                u8::try_from(t.partition_point(|&x| x <= below)).unwrap()
            );
        }
        for i in 0..=100_000u32 {
            #[allow(clippy::cast_precision_loss)]
            let c = i as f32 / 100_000.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let reference = (c * 255.0).round() as u8;
            assert_eq!(unorm8(c), reference, "{c}");
        }
    }

    #[test]
    fn transfer_functions_are_monotonic() {
        let mut prev = -1.0;
        for i in 0..=1000 {
            #[allow(clippy::cast_precision_loss)]
            let v = linear_to_srgb(i as f32 / 1000.0);
            assert!(v > prev, "not increasing at {i}");
            prev = v;
        }
    }

    #[test]
    fn encoding_clamps_out_of_range_and_nan() {
        let c = LinearRgba::new(-0.5, 2.0, f32::NAN, 1.5).to_srgb8();
        assert_eq!(c, Rgba8::new(0, 255, 0, 255));
        assert_eq!(unorm8(-1.0), 0);
        assert_eq!(unorm8(f32::INFINITY), 255);
        assert_eq!(unorm8(0.5), 128);
    }

    #[test]
    fn readme_reference_colours_round_trip() {
        for c in [
            [6, 147, 48, 255],
            [2, 34, 169, 255],
            [255, 24, 19, 255],
            [255, 192, 1, 255],
        ] {
            let c = Rgba8::from(c);
            assert_eq!(c.to_linear().to_srgb8(), c);
        }
    }

    #[test]
    fn lerp_and_arithmetic() {
        let a = LinearRgba::new(0.0, 0.2, 0.4, 1.0);
        let b = LinearRgba::new(1.0, 0.6, 0.4, 0.0);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        let mid = a.lerp(b, 0.5);
        assert!(
            (mid.r - 0.5).abs() < 1e-6 && (mid.g - 0.4).abs() < 1e-6 && (mid.a - 0.5).abs() < 1e-6
        );
        assert_eq!(LinearRgba::WHITE.modulate(a), a);
        assert_eq!(LinearRgba::rgb(0.1, 0.2, 0.3).a, 1.0);
    }

    #[test]
    fn rgba8_byte_order_is_rgba() {
        assert_eq!(Rgba8::new(1, 2, 3, 4).to_array(), [1, 2, 3, 4]);
        assert_eq!(<[u8; 4]>::from(Rgba8::from([9, 8, 7, 6])), [9, 8, 7, 6]);
        assert_eq!(size_of::<Rgba8>(), 4);
    }
}

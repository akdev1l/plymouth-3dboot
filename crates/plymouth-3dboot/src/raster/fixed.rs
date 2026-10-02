// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed-point subpixel coordinates.

use crate::math::Vec2;

/// Fractional bits of a fixed-point coordinate (24.8 format).
pub const SUBPIXEL_BITS: u32 = 8;

/// Fixed-point units per pixel.
pub const SUBPIXEL_SCALE: i64 = 1 << SUBPIXEL_BITS;

/// Largest accepted magnitude of a window coordinate, in pixels.
///
/// This is a guard band four times the largest framebuffer
/// ([`crate::target::MAX_DIMENSION`]). The clipper keeps vertices inside it.
/// It bounds each edge-function product below 2^49 and every edge-function
/// value below 2^51, far from `i64` overflow.
pub const MAX_COORD: f32 = 65_536.0;

/// A point on the subpixel grid: pixel coordinates times [`SUBPIXEL_SCALE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FixedPoint {
    /// X in subpixel units.
    pub x: i64,
    /// Y in subpixel units.
    pub y: i64,
}

impl FixedPoint {
    /// Snaps a window-space position to the nearest subpixel grid point
    /// (ties away from zero).
    ///
    /// Returns `None` if a coordinate is not finite or exceeds
    /// [`MAX_COORD`] in magnitude.
    #[must_use]
    pub fn from_window(p: Vec2) -> Option<Self> {
        let snap = |v: f32| -> Option<i64> {
            if !v.is_finite() || v.abs() > MAX_COORD {
                return None;
            }
            // |v| <= 2^16, so |v * 256| <= 2^24 fits i64 exactly.
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let s = (v * SUBPIXEL_SCALE as f32).round() as i64;
            Some(s)
        };
        Some(Self {
            x: snap(p.x)?,
            y: snap(p.y)?,
        })
    }

    /// The centre of pixel `(x, y)`, i.e. `(x + 0.5, y + 0.5)`.
    #[must_use]
    pub fn pixel_center(x: i64, y: i64) -> Self {
        Self {
            x: x * SUBPIXEL_SCALE + SUBPIXEL_SCALE / 2,
            y: y * SUBPIXEL_SCALE + SUBPIXEL_SCALE / 2,
        }
    }

    /// Converts back to pixel coordinates.
    #[cfg(test)]
    #[must_use]
    pub fn to_window(self) -> Vec2 {
        #[allow(clippy::cast_precision_loss)]
        let s = SUBPIXEL_SCALE as f32;
        #[allow(clippy::cast_precision_loss)]
        Vec2::new(self.x as f32 / s, self.y as f32 / s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_nearest_subpixel() {
        let unit = 1.0 / 256.0;
        assert_eq!(
            FixedPoint::from_window(Vec2::new(1.0, 2.5)),
            Some(FixedPoint { x: 256, y: 640 })
        );
        // Just below and above half a subpixel.
        assert_eq!(
            FixedPoint::from_window(Vec2::new(0.49 * unit, 0.51 * unit)),
            Some(FixedPoint { x: 0, y: 1 })
        );
        // Ties go away from zero.
        assert_eq!(
            FixedPoint::from_window(Vec2::new(0.5 * unit, -0.5 * unit)),
            Some(FixedPoint { x: 1, y: -1 })
        );
        assert_eq!(
            FixedPoint::from_window(Vec2::new(-3.25, 0.0)),
            Some(FixedPoint { x: -832, y: 0 })
        );
    }

    #[test]
    fn rejects_non_finite_and_out_of_range() {
        assert!(FixedPoint::from_window(Vec2::new(f32::NAN, 0.0)).is_none());
        assert!(FixedPoint::from_window(Vec2::new(0.0, f32::INFINITY)).is_none());
        assert!(FixedPoint::from_window(Vec2::new(MAX_COORD * 1.01, 0.0)).is_none());
        assert!(FixedPoint::from_window(Vec2::new(0.0, -MAX_COORD * 1.01)).is_none());
        assert_eq!(
            FixedPoint::from_window(Vec2::new(MAX_COORD, -MAX_COORD)),
            Some(FixedPoint {
                x: 1 << 24,
                y: -(1 << 24)
            })
        );
    }

    #[test]
    fn pixel_centers_are_half_pixel_offsets() {
        assert_eq!(
            FixedPoint::pixel_center(0, 0),
            FixedPoint { x: 128, y: 128 }
        );
        assert_eq!(
            FixedPoint::pixel_center(3, -1),
            FixedPoint { x: 896, y: -128 }
        );
        assert_eq!(
            FixedPoint::pixel_center(3, 2).to_window(),
            Vec2::new(3.5, 2.5)
        );
    }
}

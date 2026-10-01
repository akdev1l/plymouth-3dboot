// SPDX-License-Identifier: GPL-3.0-or-later
//! Perspective divide and viewport mapping.

use super::ClipVertex;
use crate::math::{Vec2, Vec3, Viewport};

/// A vertex after the perspective divide, in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenVertex<V> {
    /// Window position: `x`, `y` in pixels (origin top-left, +y down) and
    /// depth `z` in `[0, 1]` (smaller is closer).
    pub position: Vec3,
    /// `1 / w` of the clip-space vertex, for perspective-correct
    /// interpolation.
    pub inv_w: f32,
    /// Attributes, unchanged.
    pub varyings: V,
}

impl<V> ScreenVertex<V> {
    /// Divides by `w` and maps NDC to `viewport`.
    ///
    /// `vertex` must have `w > 0`, which [`super::Clipper`] guarantees.
    #[must_use]
    pub fn from_clip(vertex: ClipVertex<V>, viewport: &Viewport) -> Self {
        let inv_w = 1.0 / vertex.position.w;
        let ndc = vertex.position.truncate() * inv_w;
        Self {
            position: viewport.ndc_to_window(ndc),
            inv_w,
            varyings: vertex.varyings,
        }
    }

    /// The window-space x and y.
    #[must_use]
    pub fn xy(&self) -> Vec2 {
        self.position.truncate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec4;
    use crate::pipeline::clip::GUARD_BAND;
    use crate::raster::fixed::MAX_COORD;
    use crate::target::MAX_DIMENSION;

    fn at(x: f32, y: f32, z: f32, w: f32) -> ClipVertex<()> {
        ClipVertex {
            position: Vec4::new(x, y, z, w),
            varyings: (),
        }
    }

    #[test]
    fn ndc_corners_map_to_framebuffer_corners() {
        let vp = Viewport::new(200, 100);
        let tl = ScreenVertex::from_clip(at(-2.0, 2.0, -2.0, 2.0), &vp);
        let br = ScreenVertex::from_clip(at(4.0, -4.0, 4.0, 4.0), &vp);
        assert_eq!(tl.position, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(br.position, Vec3::new(200.0, 100.0, 1.0));
        assert_eq!(tl.xy(), Vec2::ZERO);
    }

    #[test]
    fn depth_and_inverse_w() {
        let v = ScreenVertex::from_clip(at(0.0, 0.0, 1.0, 4.0), &Viewport::new(10, 10));
        assert_eq!(v.position, Vec3::new(5.0, 5.0, 0.625));
        assert_eq!(v.inv_w, 0.25);
    }

    #[test]
    fn guard_band_maps_inside_rasterizer_range_for_largest_viewport() {
        let d = MAX_DIMENSION;
        let vp = Viewport {
            x: d,
            y: d,
            width: d,
            height: d,
        };
        for (x, y) in [(GUARD_BAND, GUARD_BAND), (-GUARD_BAND, -GUARD_BAND)] {
            let v = ScreenVertex::from_clip(at(x, y, 0.0, 1.0), &vp);
            assert!(
                v.position.x.abs() <= MAX_COORD && v.position.y.abs() <= MAX_COORD,
                "{}",
                v.position
            );
        }
        let v =
            ScreenVertex::from_clip(at(-GUARD_BAND, GUARD_BAND, 0.0, 1.0), &Viewport::new(d, d));
        assert!(
            v.position.x.abs() <= MAX_COORD && v.position.y.abs() <= MAX_COORD,
            "{}",
            v.position
        );
    }
}

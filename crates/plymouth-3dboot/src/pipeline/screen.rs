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

/// Converts screen-space barycentric weights into perspective-correct
/// weights for attributes interpolated in 3D.
///
/// Attributes vary linearly in clip space, so across the screen
/// `attribute / w` and `1 / w` vary linearly. The corrected weights are
/// `λᵢ/wᵢ / Σⱼ λⱼ/wⱼ`. Depth (`z/w`) is already screen-linear and uses the
/// uncorrected weights.
#[must_use]
pub fn perspective_weights(screen: [f32; 3], inv_w: [f32; 3]) -> [f32; 3] {
    let w = [
        screen[0] * inv_w[0],
        screen[1] * inv_w[1],
        screen[2] * inv_w[2],
    ];
    let sum = w[0] + w[1] + w[2];
    if sum > 0.0 {
        w.map(|x| x / sum)
    } else {
        screen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec4;
    use crate::pipeline::clip::GUARD_BAND;
    use crate::raster::MAX_COORD;
    use crate::target::MAX_DIMENSION;

    fn at(x: f32, y: f32, z: f32, w: f32) -> ClipVertex<()> {
        ClipVertex {
            position: Vec4::new(x, y, z, w),
            varyings: (),
        }
    }

    #[test]
    fn perspective_weights_equal_screen_weights_when_w_is_constant() {
        let b = [0.2, 0.3, 0.5];
        let w = perspective_weights(b, [0.5, 0.5, 0.5]);
        assert!(w.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-7), "{w:?}");
    }

    #[test]
    fn perspective_weights_favour_nearer_vertices() {
        // Halfway on screen between a near (w = 1) and a far (w = 3) vertex,
        // the 3D point is three quarters of the way to the near one.
        let w = perspective_weights([0.5, 0.5, 0.0], [1.0, 1.0 / 3.0, 1.0]);
        assert!(
            (w[0] - 0.75).abs() < 1e-6 && (w[1] - 0.25).abs() < 1e-6 && w[2] == 0.0,
            "{w:?}"
        );
    }

    /// Rasterizes a floor quad receding from z = -1 to z = -9 and checks the
    /// interpolated attribute `u = (-z - 1) / 8` at every covered pixel
    /// against the exact ray/plane intersection.
    #[test]
    fn receding_plane_matches_analytic_values() {
        use crate::math::{Mat4, perspective};
        use crate::pipeline::Clipper;
        use crate::raster::{Rect, TriangleSetup};
        use std::f32::consts::FRAC_PI_2;

        const SIZE: u32 = 64;
        let proj: Mat4 = perspective(FRAC_PI_2, 1.0, 0.5, 20.0);
        let vp = Viewport::new(SIZE, SIZE);
        let corner =
            |x: f32, z: f32| ClipVertex::project(&proj, Vec3::new(x, -1.0, z), (-z - 1.0) / 8.0);
        let quad = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, -9.0),
            corner(-1.0, -9.0),
        ];

        let (mut max_err_correct, mut max_err_affine, mut pixels) = (0.0f32, 0.0f32, 0);
        let mut clipper = Clipper::new();
        for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
            clipper.clip(tri, |clipped| {
                let s = clipped.map(|v| ScreenVertex::from_clip(v, &vp));
                let setup = TriangleSetup::new(s.map(|v| v.xy())).expect("visible");
                setup.for_each_pixel(Rect::from_size(SIZE, SIZE), |f| {
                    let bary = f.barycentric();
                    let u = |w: [f32; 3]| {
                        w[0] * s[0].varyings + w[1] * s[1].varyings + w[2] * s[2].varyings
                    };
                    let correct = u(perspective_weights(bary, s.map(|v| v.inv_w)));
                    let affine = u(bary);
                    // Exact value: intersect the ray through the pixel centre
                    // with the plane y = -1 (tan(fov/2) = 1, aspect 1).
                    // The ray's x does not affect where it meets the plane.
                    let ndc_y = 1.0 - (f.y as f32 + 0.5) / SIZE as f32 * 2.0;
                    let t = -1.0 / ndc_y; // ray (ndc_x, ndc_y, -1) * t reaches y = -1
                    let exact = (t - 1.0) / 8.0;
                    max_err_correct = max_err_correct.max((correct - exact).abs());
                    max_err_affine = max_err_affine.max((affine - exact).abs());
                    pixels += 1;
                });
            });
        }
        eprintln!("pixels {pixels}, max error: correct {max_err_correct}, affine {max_err_affine}");
        assert!(pixels > 200, "only {pixels} pixels rasterized");
        assert!(
            max_err_correct < 0.01,
            "perspective-correct error {max_err_correct}"
        );
        assert!(
            max_err_affine > 0.1,
            "control: affine interpolation should be visibly wrong ({max_err_affine})"
        );
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

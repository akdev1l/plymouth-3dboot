// SPDX-License-Identifier: GPL-3.0-or-later
//! Math types and helpers.
//!
//! Vector, matrix and quaternion types come from [`glam`] (`f32`, column
//! vectors), configured with scalar math and `libm` so that results are
//! bit-identical on native and WebAssembly targets. Conventions (right-handed,
//! Y-up, OpenGL-style clip space, top-left window origin) are described in
//! `docs/conventions.md`.

pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Right-handed perspective projection into OpenGL-style clip space
/// (`-w ≤ z ≤ w`).
///
/// `fov_y` is the vertical field of view in radians; `aspect` is width/height.
/// Requires `0 < z_near < z_far`.
#[must_use]
pub fn perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) -> Mat4 {
    glam::camera::rh::proj::opengl::perspective(fov_y, aspect, z_near, z_far)
}

/// Right-handed orthographic projection into OpenGL-style clip space.
#[must_use]
pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, z_near: f32, z_far: f32) -> Mat4 {
    glam::camera::rh::proj::opengl::orthographic(left, right, bottom, top, z_near, z_far)
}

/// Right-handed view matrix for a camera at `eye` looking at `target`.
///
/// The camera looks down its local −Z axis with `up` roughly +Y.
#[must_use]
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    glam::camera::rh::view::look_at_mat4(eye, target, up)
}

/// A rectangle of the framebuffer that normalized device coordinates map to.
///
/// Window coordinates have their origin at the top-left corner, with +x to
/// the right and +y down; pixel `(x, y)` has its centre at
/// `(x + 0.5, y + 0.5)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Viewport {
    /// A viewport covering a whole `width × height` framebuffer.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Maps normalized device coordinates to window coordinates.
    ///
    /// NDC `x, y ∈ [-1, 1]` map to the viewport rectangle (NDC +y is up,
    /// window +y is down); NDC `z ∈ [-1, 1]` maps to depth `[0, 1]`.
    #[must_use]
    pub fn ndc_to_window(&self, ndc: Vec3) -> Vec3 {
        // Exact up to 2^24, which covers every viewport the renderer accepts
        // (each component at most MAX_DIMENSION).
        #[allow(clippy::cast_precision_loss)]
        let (x, y, w, h) = (
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        );
        Vec3::new(
            x + (ndc.x + 1.0) * 0.5 * w,
            y + (1.0 - ndc.y) * 0.5 * h,
            (ndc.z + 1.0) * 0.5,
        )
    }
}

/// An axis-aligned bounding box.
///
/// The [`Aabb::EMPTY`] box (min = +∞, max = −∞) is the identity for
/// [`Aabb::union`] and [`Aabb::extend`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    /// The empty box, containing no points.
    pub const EMPTY: Self = Self {
        min: Vec3::INFINITY,
        max: Vec3::NEG_INFINITY,
    };

    /// Creates a box from two corners; their components may be in any order.
    #[must_use]
    pub fn new(a: Vec3, b: Vec3) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// The smallest box containing every point (empty for no points).
    #[must_use]
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        points.into_iter().fold(Self::EMPTY, Self::extend)
    }

    /// Returns `true` if the box contains no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    /// The box grown to contain `point`.
    #[must_use]
    pub fn extend(self, point: Vec3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    /// The smallest box containing both boxes.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Returns `true` if `point` lies inside or on the boundary of the box.
    #[must_use]
    pub fn contains(&self, point: Vec3) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    /// Centre of the box. Meaningless for an empty box.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Edge lengths of the box. Meaningless for an empty box.
    #[must_use]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// The eight corners of the box.
    #[must_use]
    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, b.y, b.z),
            Vec3::new(b.x, b.y, b.z),
        ]
    }

    /// The smallest axis-aligned box containing this box transformed by the
    /// affine matrix `m`. An empty box stays empty.
    #[must_use]
    pub fn transformed(&self, m: &Mat4) -> Self {
        if self.is_empty() {
            return Self::EMPTY;
        }
        Self::from_points(self.corners().map(|c| m.transform_point3(c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::f32::consts::FRAC_PI_2;

    const EPS: f32 = 1e-5;

    fn ndc(m: &Mat4, p: Vec3) -> Vec3 {
        m.project_point3(p)
    }

    #[test]
    fn perspective_maps_near_and_far_planes_to_depth_0_and_1() {
        let proj = perspective(FRAC_PI_2, 1.0, 0.5, 100.0);
        let vp = Viewport::new(100, 100);
        let near = vp.ndc_to_window(ndc(&proj, Vec3::new(0.0, 0.0, -0.5)));
        let far = vp.ndc_to_window(ndc(&proj, Vec3::new(0.0, 0.0, -100.0)));
        assert!((near.z - 0.0).abs() < EPS, "near depth {}", near.z);
        assert!((far.z - 1.0).abs() < 1e-4, "far depth {}", far.z);
    }

    #[test]
    fn perspective_maps_frustum_edges_to_ndc_edges() {
        // 90° vertical fov, aspect 2: at distance d the frustum spans
        // y ∈ [-d, d] and x ∈ [-2d, 2d].
        let proj = perspective(FRAC_PI_2, 2.0, 1.0, 10.0);
        let p = ndc(&proj, Vec3::new(6.0, -3.0, -3.0));
        assert!(p.abs_diff_eq(Vec3::new(1.0, -1.0, p.z), EPS), "{p}");
    }

    #[test]
    fn perspective_puts_points_behind_camera_at_negative_w() {
        let proj = perspective(FRAC_PI_2, 1.0, 0.1, 10.0);
        let clip = proj * Vec4::new(0.0, 0.0, 1.0, 1.0);
        assert!(clip.w < 0.0);
    }

    #[test]
    fn orthographic_maps_box_to_ndc_cube() {
        let proj = orthographic(-2.0, 2.0, -1.0, 1.0, 1.0, 5.0);
        assert!(
            ndc(&proj, Vec3::new(-2.0, -1.0, -1.0)).abs_diff_eq(Vec3::new(-1.0, -1.0, -1.0), EPS)
        );
        assert!(ndc(&proj, Vec3::new(2.0, 1.0, -5.0)).abs_diff_eq(Vec3::new(1.0, 1.0, 1.0), EPS));
    }

    #[test]
    fn look_at_puts_target_on_negative_z_axis() {
        let view = look_at(Vec3::new(3.0, 4.0, 5.0), Vec3::new(3.0, 4.0, -2.0), Vec3::Y);
        let t = view.transform_point3(Vec3::new(3.0, 4.0, -2.0));
        assert!(t.abs_diff_eq(Vec3::new(0.0, 0.0, -7.0), EPS), "{t}");
        let up = view.transform_vector3(Vec3::Y);
        assert!(up.abs_diff_eq(Vec3::Y, EPS), "{up}");
    }

    #[test]
    fn viewport_maps_ndc_corners_with_top_left_origin() {
        let vp = Viewport::new(640, 480);
        assert_eq!(
            vp.ndc_to_window(Vec3::new(-1.0, 1.0, -1.0)),
            Vec3::new(0.0, 0.0, 0.0)
        );
        assert_eq!(
            vp.ndc_to_window(Vec3::new(1.0, -1.0, 1.0)),
            Vec3::new(640.0, 480.0, 1.0)
        );
        assert_eq!(vp.ndc_to_window(Vec3::ZERO), Vec3::new(320.0, 240.0, 0.5));
    }

    #[test]
    fn viewport_offset_is_applied() {
        let vp = Viewport {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(
            vp.ndc_to_window(Vec3::new(-1.0, 1.0, 0.0)),
            Vec3::new(10.0, 20.0, 0.5)
        );
        assert_eq!(
            vp.ndc_to_window(Vec3::new(1.0, -1.0, 0.0)),
            Vec3::new(110.0, 70.0, 0.5)
        );
    }

    #[test]
    fn empty_aabb_is_identity_for_union_and_contains_nothing() {
        let b = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(-1.0, 0.0, 5.0));
        assert!(Aabb::EMPTY.is_empty());
        assert!(!Aabb::EMPTY.contains(Vec3::ZERO));
        assert_eq!(Aabb::EMPTY.union(b), b);
        assert_eq!(b.union(Aabb::EMPTY), b);
        assert!(Aabb::from_points([]).is_empty());
        assert!(Aabb::EMPTY.transformed(&Mat4::IDENTITY).is_empty());
        assert_eq!(Aabb::default(), Aabb::EMPTY);
    }

    #[test]
    fn aabb_new_orders_corners() {
        let b = Aabb::new(Vec3::new(1.0, 0.0, 5.0), Vec3::new(-1.0, 2.0, 3.0));
        assert_eq!(b.min, Vec3::new(-1.0, 0.0, 3.0));
        assert_eq!(b.max, Vec3::new(1.0, 2.0, 5.0));
        assert_eq!(b.center(), Vec3::new(0.0, 1.0, 4.0));
        assert_eq!(b.size(), Vec3::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn aabb_corners_are_distinct_and_contained() {
        let b = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0));
        let corners = b.corners();
        for (i, c) in corners.iter().enumerate() {
            assert!(b.contains(*c));
            assert!(
                corners[i + 1..].iter().all(|o| o != c),
                "duplicate corner {c}"
            );
        }
    }

    fn vec3() -> impl Strategy<Value = Vec3> {
        (-100.0f32..100.0, -100.0f32..100.0, -100.0f32..100.0)
            .prop_map(|(x, y, z)| Vec3::new(x, y, z))
    }

    fn affine() -> impl Strategy<Value = Mat4> {
        (vec3(), vec3(), -3.0f32..3.0, 0.1f32..5.0).prop_map(|(t, axis, angle, s)| {
            let axis = axis.try_normalize().unwrap_or(Vec3::Y);
            Mat4::from_scale_rotation_translation(
                Vec3::new(s, s * 0.5, s * 2.0),
                Quat::from_axis_angle(axis, angle),
                t,
            )
        })
    }

    proptest! {
        #[test]
        fn from_points_contains_all_points(points in proptest::collection::vec(vec3(), 1..20)) {
            let b = Aabb::from_points(points.iter().copied());
            for p in &points {
                prop_assert!(b.contains(*p));
            }
        }

        #[test]
        fn transformed_aabb_contains_transformed_points(a in vec3(), b in vec3(), m in affine(), t in (0.0f32..1.0, 0.0f32..1.0, 0.0f32..1.0)) {
            let bx = Aabb::new(a, b);
            let out = bx.transformed(&m);
            // Any point inside the source box lands inside the result.
            let inner = bx.min + bx.size() * Vec3::new(t.0, t.1, t.2);
            let p = m.transform_point3(inner);
            let tol = Vec3::splat(1e-3 * (1.0 + p.abs().max_element()));
            prop_assert!(Aabb::new(out.min - tol, out.max + tol).contains(p), "{p} not in {out:?}");
            // The result is tight: every face is touched by a transformed corner.
            let tc: Vec<Vec3> = bx.corners().iter().map(|c| m.transform_point3(*c)).collect();
            for axis in 0..3 {
                prop_assert!(tc.iter().any(|c| c[axis] == out.min[axis]));
                prop_assert!(tc.iter().any(|c| c[axis] == out.max[axis]));
            }
        }
    }
}

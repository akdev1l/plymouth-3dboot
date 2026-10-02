// SPDX-License-Identifier: GPL-3.0-or-later
//! The 3D pipeline: vertex transformation, clipping, projection to the
//! window, and per-fragment interpolation and depth testing.

pub mod clip;
pub mod depth;
pub mod renderer;
pub mod screen;

pub use clip::Clipper;
pub use depth::{DepthFunc, DepthState};
pub use renderer::{
    DrawError, DrawStats, FragmentInput, MAX_THREADS, RenderState, Renderer, Shader,
};
pub use screen::{ScreenVertex, perspective_weights};

use crate::math::{Mat4, Vec3, Vec4};

/// A vertex in homogeneous clip space with its attributes ("varyings").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipVertex<V> {
    /// Clip-space position (OpenGL convention: visible when
    /// `-w ≤ x, y, z ≤ w`).
    pub position: Vec4,
    /// Attributes interpolated across the triangle.
    pub varyings: V,
}

impl<V> ClipVertex<V> {
    /// Transforms a model-space `position` by the combined
    /// model-view-projection matrix `mvp`.
    #[must_use]
    pub fn project(mvp: &Mat4, position: Vec3, varyings: V) -> Self {
        Self {
            position: *mvp * position.extend(1.0),
            varyings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{look_at, perspective};
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn project_applies_mvp_to_homogeneous_point() {
        // 90° fov, aspect 1, near 1, far 3. A point 2 units in front of the
        // camera: z_clip = (f+n)/(n-f)·z + 2fn/(n-f) = (-2)(-2) + (-3) = 1,
        // w = -z = 2, so NDC depth is 0.5.
        let proj = perspective(FRAC_PI_2, 1.0, 1.0, 3.0);
        let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let model = Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0));
        let v = ClipVertex::project(&(proj * view * model), Vec3::new(1.0, -0.5, 0.0), 7u8);
        assert!(
            v.position.abs_diff_eq(Vec4::new(1.0, -0.5, 1.0, 2.0), 1e-5),
            "{}",
            v.position
        );
        assert_eq!(v.varyings, 7);
    }

    #[test]
    fn project_with_identity_keeps_position_and_sets_w_to_one() {
        let v = ClipVertex::project(&Mat4::IDENTITY, Vec3::new(0.25, 0.5, -0.75), ());
        assert_eq!(v.position, Vec4::new(0.25, 0.5, -0.75, 1.0));
    }
}

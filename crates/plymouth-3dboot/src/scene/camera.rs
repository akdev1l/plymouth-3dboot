// SPDX-License-Identifier: GPL-3.0-or-later
//! Cameras: projection plus placement.

use crate::math::{Aabb, Mat4, Vec3, look_at, orthographic, perspective};

/// How a camera maps view space to clip space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Perspective projection.
    Perspective {
        /// Vertical field of view in radians, in `(0, π)`.
        fov_y: f32,
        /// Distance to the near plane (> 0).
        z_near: f32,
        /// Distance to the far plane (> `z_near`).
        z_far: f32,
    },
    /// Orthographic projection.
    Orthographic {
        /// Half the height of the view volume; the half width follows from
        /// the aspect ratio.
        half_height: f32,
        /// Distance to the near plane.
        z_near: f32,
        /// Distance to the far plane (> `z_near`).
        z_far: f32,
    },
}

impl Projection {
    /// The projection matrix for a viewport of the given `aspect` ratio
    /// (width / height).
    #[must_use]
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        match *self {
            Self::Perspective {
                fov_y,
                z_near,
                z_far,
            } => perspective(fov_y, aspect, z_near, z_far),
            Self::Orthographic {
                half_height,
                z_near,
                z_far,
            } => {
                let hw = half_height * aspect;
                orthographic(-hw, hw, -half_height, half_height, z_near, z_far)
            }
        }
    }
}

/// A camera: a projection and its placement in the world.
///
/// The camera looks down its local −Z axis with +Y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Projection.
    pub projection: Projection,
    /// Camera-to-world transform (the inverse of the view matrix).
    pub world: Mat4,
}

impl Camera {
    /// A camera at `eye` looking at `target`.
    #[must_use]
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3, projection: Projection) -> Self {
        Self {
            projection,
            world: look_at(eye, target, up).inverse(),
        }
    }

    /// The world-to-view matrix.
    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        self.world.inverse()
    }

    /// Projection × view for a viewport of the given aspect ratio.
    #[must_use]
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection.matrix(aspect) * self.view_matrix()
    }

    /// Camera position in world space.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.world.transform_point3(Vec3::ZERO)
    }

    /// The camera with its near and far planes moved out, if needed, so
    /// that nothing within `bounds` is clipped by them. The near plane of a
    /// perspective camera moves at most to a quarter of its distance, which
    /// keeps depth precision.
    #[must_use]
    pub fn enclosing(mut self, bounds: Aabb) -> Self {
        if bounds.is_empty() {
            return self;
        }
        let view = self.world.inverse();
        let depths = bounds.corners().map(|p| -view.transform_point3(p).z);
        let nearest = depths.iter().copied().fold(f32::INFINITY, f32::min);
        let farthest = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        match &mut self.projection {
            Projection::Perspective { z_near, z_far, .. } => {
                *z_far = z_far.max(farthest * 1.01);
                *z_near = z_near.min((nearest * 0.99).max(*z_near * 0.25));
            }
            Projection::Orthographic { z_near, z_far, .. } => {
                *z_far = z_far.max(farthest + (farthest - nearest).abs() * 0.01);
                *z_near = z_near.min(nearest - (farthest - nearest).abs() * 0.01);
            }
        }
        self
    }

    /// A camera that sees all of `bounds` from `direction` (pointing from the
    /// camera towards the scene), for a viewport of the given `aspect`.
    ///
    /// The bounding sphere of `bounds` is fitted into the view: `fov_y` is
    /// used for a perspective camera, or `None` gives an orthographic one.
    /// Near and far planes enclose the sphere. An empty box is treated as a
    /// unit box at the origin.
    #[must_use]
    pub fn framing(
        bounds: Aabb,
        direction: Vec3,
        up: Vec3,
        aspect: f32,
        fov_y: Option<f32>,
    ) -> Self {
        let bounds = if bounds.is_empty() {
            Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5))
        } else {
            bounds
        };
        let center = bounds.center();
        let radius = (bounds.size().length() * 0.5).max(1e-4);
        let dir = direction.try_normalize().unwrap_or(Vec3::NEG_Z);
        // Avoid a degenerate look-at when `up` is parallel to the direction.
        let up = if dir.cross(up).length_squared() < 1e-8 {
            dir.any_orthonormal_vector()
        } else {
            up
        };
        match fov_y {
            Some(fov_y) => {
                // The sphere must fit both the vertical and horizontal fov.
                let half_v = fov_y * 0.5;
                let half_h = libm::atanf(libm::tanf(half_v) * aspect);
                let distance = radius / libm::sinf(half_v.min(half_h));
                let eye = center - dir * distance;
                // distance >= radius; keep the near plane strictly positive
                // even when the sphere nearly touches the camera.
                let projection = Projection::Perspective {
                    fov_y,
                    z_near: ((distance - radius) * 0.9).max(distance * 1e-3),
                    z_far: (distance + radius) * 1.1,
                };
                Self::look_at(eye, center, up, projection)
            }
            None => {
                let distance = 2.0 * radius;
                let half_height = if aspect < 1.0 {
                    radius / aspect
                } else {
                    radius
                };
                let projection = Projection::Orthographic {
                    half_height,
                    z_near: distance - radius * 1.1,
                    z_far: distance + radius * 1.1,
                };
                Self::look_at(center - dir * distance, center, up, projection)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn assert_inside_ndc(camera: &Camera, aspect: f32, bounds: Aabb) {
        let vp = camera.view_projection(aspect);
        for c in bounds.corners() {
            let clip = vp * c.extend(1.0);
            assert!(clip.w > 0.0, "corner {c} behind the camera");
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.abs().max_element() <= 1.0 + 1e-4,
                "corner {c} outside: ndc {ndc}"
            );
        }
    }

    #[test]
    fn look_at_puts_target_at_screen_centre() {
        let cam = Camera::look_at(
            Vec3::new(3.0, 4.0, 5.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::Y,
            Projection::Perspective {
                fov_y: 1.0,
                z_near: 0.1,
                z_far: 100.0,
            },
        );
        let clip = cam.view_projection(1.5) * Vec3::ONE.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.truncate().abs().max_element() < 1e-5, "{ndc}");
        assert!(cam.position().abs_diff_eq(Vec3::new(3.0, 4.0, 5.0), 1e-5));
        assert!((cam.view_matrix() * cam.world).abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }

    #[test]
    fn orthographic_projection_matches_aspect() {
        let p = Projection::Orthographic {
            half_height: 2.0,
            z_near: 0.0,
            z_far: 10.0,
        };
        let ndc = p.matrix(2.0).project_point3(Vec3::new(4.0, 2.0, -10.0));
        assert!(ndc.abs_diff_eq(Vec3::new(1.0, 1.0, 1.0), 1e-6), "{ndc}");
    }

    #[test]
    fn framing_empty_bounds_is_well_defined() {
        let cam = Camera::framing(Aabb::EMPTY, Vec3::NEG_Z, Vec3::Y, 1.0, Some(1.0));
        assert!(cam.world.is_finite());
    }

    fn vec3(range: f32) -> impl Strategy<Value = Vec3> {
        (-range..range, -range..range, -range..range).prop_map(|(x, y, z)| Vec3::new(x, y, z))
    }

    proptest! {
        #[test]
        fn framed_bounds_project_inside_ndc(
            a in vec3(50.0),
            size in (0.01f32..30.0, 0.01f32..30.0, 0.01f32..30.0),
            dir in vec3(1.0),
            aspect in 0.3f32..3.0,
            fov in prop_oneof![Just(None), (0.2f32..2.5).prop_map(Some)],
        ) {
            prop_assume!(dir.length() > 0.1);
            let bounds = Aabb::new(a, a + Vec3::new(size.0, size.1, size.2));
            let cam = Camera::framing(bounds, dir, Vec3::Y, aspect, fov);
            assert_inside_ndc(&cam, aspect, bounds);
        }
    }

    #[test]
    fn enclosing_widens_near_and_far_only_as_needed() {
        let projection = Projection::Perspective {
            fov_y: 0.8,
            z_near: 4.0,
            z_far: 6.0,
        };
        let camera = Camera::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y, projection);
        let planes = |c: Camera| match c.projection {
            Projection::Perspective { z_near, z_far, .. } => (z_near, z_far),
            Projection::Orthographic { .. } => unreachable!(),
        };
        assert_eq!(
            planes(camera.enclosing(Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5)))),
            (4.0, 6.0)
        );
        let (near, far) = planes(camera.enclosing(Aabb::new(
            Vec3::new(-1.0, -1.0, -5.0),
            Vec3::new(1.0, 1.0, 3.0),
        )));
        assert!((near - 1.98).abs() < 1e-5, "{near}");
        assert!((far - 10.1).abs() < 1e-4, "{far}");
        // Behind the camera: the near plane stops at a quarter.
        let (near, _) =
            planes(camera.enclosing(Aabb::new(Vec3::splat(-1.0), Vec3::new(1.0, 1.0, 9.0))));
        assert_eq!(near, 1.0);
        assert_eq!(camera.enclosing(Aabb::EMPTY), camera);
    }
}

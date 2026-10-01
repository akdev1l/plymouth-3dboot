// SPDX-License-Identifier: GPL-3.0-or-later
//! Diffuse (Lambertian) shading.

use super::{Lighting, Transforms};
use crate::color::LinearRgba;
use crate::math::{Mat3, Mat4, Vec3};
use crate::pipeline::{ClipVertex, FragmentInput, Shader};
use crate::scene::{Material, Mesh};

/// Lambertian diffuse light reaching a surface with unit normal `n`:
/// ambient plus `max(0, n · l)` for each directional light.
pub(super) fn diffuse_light(n: Vec3, lighting: &Lighting) -> LinearRgba {
    lighting.lights.iter().fold(lighting.ambient, |acc, light| {
        let to_light = -light.direction.normalize_or_zero();
        acc + light.color * n.dot(to_light).max(0.0)
    })
}

/// The shading normal of a fragment: the interpolated normal, renormalized
/// and flipped for back faces (two-sided lighting).
pub(super) fn shading_normal(interpolated: Vec3, front_facing: bool) -> Vec3 {
    let n = interpolated.normalize_or_zero();
    if front_facing { n } else { -n }
}

/// Diffuse shading: `emissive + base_color × (ambient + Σ lights · max(0, n·l))`.
#[derive(Clone, Copy, Debug)]
pub struct LambertShader<'a> {
    mvp: Mat4,
    normal_matrix: Mat3,
    positions: &'a [Vec3],
    normals: &'a [Vec3],
    base: LinearRgba,
    emissive: LinearRgba,
    lighting: &'a Lighting,
}

impl<'a> LambertShader<'a> {
    /// A shader for `mesh` drawn with `mvp`, where `world` places the mesh in
    /// the world (normals are transformed by its inverse transpose).
    ///
    /// Returns `None` if the mesh has no normals.
    #[must_use]
    pub fn new(
        mvp: Mat4,
        world: Mat4,
        mesh: &'a Mesh,
        material: &Material,
        lighting: &'a Lighting,
    ) -> Option<Self> {
        Some(Self {
            mvp,
            normal_matrix: Transforms::normal_matrix(world),
            positions: mesh.positions(),
            normals: mesh.normals()?,
            base: material.base_color,
            emissive: material.emissive,
            lighting,
        })
    }
}

impl Shader for LambertShader<'_> {
    type Varyings = Vec3;

    fn vertex(&self, index: u32) -> ClipVertex<Vec3> {
        let i = index as usize;
        ClipVertex::project(
            &self.mvp,
            self.positions[i],
            self.normal_matrix * self.normals[i],
        )
    }

    fn fragment(&self, f: &FragmentInput<Vec3>) -> Option<LinearRgba> {
        let n = shading_normal(f.varyings, f.front_facing);
        let lit = self.emissive + self.base.modulate(diffuse_light(n, self.lighting));
        Some(LinearRgba {
            a: self.base.a,
            ..lit
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::DirectionalLight;

    fn lighting() -> Lighting {
        Lighting {
            ambient: LinearRgba::rgb(0.2, 0.2, 0.2),
            lights: vec![DirectionalLight {
                direction: Vec3::new(0.0, 0.0, -2.0),
                color: LinearRgba::rgb(1.0, 1.0, 1.0),
            }],
        }
    }

    fn shade(normal: Vec3, front_facing: bool, material: &Material) -> LinearRgba {
        let mesh = Mesh::new(vec![Vec3::ZERO], vec![])
            .unwrap()
            .with_normals(vec![normal])
            .unwrap();
        let l = lighting();
        let s = LambertShader::new(Mat4::IDENTITY, Mat4::IDENTITY, &mesh, material, &l).unwrap();
        s.fragment(&FragmentInput {
            x: 0,
            y: 0,
            depth: 0.5,
            front_facing,
            varyings: s.vertex(0).varyings,
        })
        .unwrap()
    }

    fn close(a: LinearRgba, b: LinearRgba) -> bool {
        [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a]
            .iter()
            .all(|d| d.abs() < 1e-5)
    }

    #[test]
    fn hand_computed_fragments() {
        let m = Material {
            base_color: LinearRgba::new(0.5, 0.4, 0.2, 0.8),
            ..Material::default()
        };
        // Facing the light: base × (0.2 + 1).
        assert!(close(
            shade(Vec3::Z, true, &m),
            LinearRgba::new(0.6, 0.48, 0.24, 0.8)
        ));
        // 60° from the light: n·l = 0.5.
        let n = Vec3::new(libm::sinf(std::f32::consts::FRAC_PI_3), 0.0, 0.5);
        assert!(close(
            shade(n, true, &m),
            LinearRgba::new(0.35, 0.28, 0.14, 0.8)
        ));
        // Facing away: ambient only.
        assert!(close(
            shade(Vec3::NEG_Z, true, &m),
            LinearRgba::new(0.1, 0.08, 0.04, 0.8)
        ));
        // Back faces are lit from the other side.
        assert!(close(
            shade(Vec3::NEG_Z, false, &m),
            LinearRgba::new(0.6, 0.48, 0.24, 0.8)
        ));
    }

    #[test]
    fn emissive_adds_and_unnormalized_normals_are_fine() {
        let m = Material {
            base_color: LinearRgba::rgb(0.5, 0.5, 0.5),
            emissive: LinearRgba::rgb(0.1, 0.0, 0.0),
            ..Material::default()
        };
        assert!(close(
            shade(Vec3::Z * 7.0, true, &m),
            LinearRgba::rgb(0.7, 0.6, 0.6)
        ));
    }

    #[test]
    fn normals_follow_non_uniform_scale_correctly() {
        // A plane tilted 45° in the xy-plane, then squashed in x by 0.5: the
        // inverse transpose keeps the normal perpendicular to the surface.
        let world = Mat4::from_scale(Vec3::new(0.5, 1.0, 1.0));
        let surface_dir = Vec3::new(1.0, -1.0, 0.0); // lies in the plane
        let normal = Vec3::new(1.0, 1.0, 0.0);
        let n = Transforms::normal_matrix(world) * normal;
        let t = world.transform_vector3(surface_dir);
        assert!(
            n.dot(t).abs() < 1e-6,
            "transformed normal {n} not perpendicular to {t}"
        );
    }

    #[test]
    fn requires_normals() {
        let mesh = Mesh::new(vec![Vec3::ZERO], vec![]).unwrap();
        assert!(
            LambertShader::new(
                Mat4::IDENTITY,
                Mat4::IDENTITY,
                &mesh,
                &Material::default(),
                &lighting()
            )
            .is_none()
        );
    }
}

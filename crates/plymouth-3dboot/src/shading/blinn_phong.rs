// SPDX-License-Identifier: GPL-3.0-or-later
//! Blinn–Phong shading: diffuse plus specular highlights.

use super::lambert::{diffuse_light, shading_normal};
use super::{Lighting, Transforms};
use crate::color::LinearRgba;
use crate::math::{Mat3, Mat4, Vec3};
use crate::pipeline::{ClipVertex, FragmentInput, Shader};
use crate::scene::{Material, Mesh};

/// Lambertian diffuse lighting plus, for each light reaching the surface,
/// `specular × max(0, n·h)^shininess`, where `h` is the half vector
/// between the light and view directions.
#[derive(Clone, Copy, Debug)]
pub struct BlinnPhongShader<'a> {
    mvp: Mat4,
    world: Mat4,
    normal_matrix: Mat3,
    eye: Vec3,
    positions: &'a [Vec3],
    normals: &'a [Vec3],
    base: LinearRgba,
    emissive: LinearRgba,
    specular: LinearRgba,
    shininess: f32,
    lighting: &'a Lighting,
}

impl<'a> BlinnPhongShader<'a> {
    /// A shader for `mesh` drawn with `mvp`, placed by `world`, viewed from
    /// the world-space `eye` position.
    ///
    /// Returns `None` if the mesh has no normals.
    #[must_use]
    pub fn new(
        mvp: Mat4,
        world: Mat4,
        eye: Vec3,
        mesh: &'a Mesh,
        material: &Material,
        lighting: &'a Lighting,
    ) -> Option<Self> {
        Some(Self {
            mvp,
            world,
            normal_matrix: Transforms::normal_matrix(world),
            eye,
            positions: mesh.positions(),
            normals: mesh.normals()?,
            base: material.base_color,
            emissive: material.emissive,
            specular: material.specular,
            shininess: material.shininess,
            lighting,
        })
    }
}

impl Shader for BlinnPhongShader<'_> {
    /// World-space normal and position.
    type Varyings = (Vec3, Vec3);

    fn vertex(&self, index: u32) -> ClipVertex<(Vec3, Vec3)> {
        let i = index as usize;
        let p = self.positions[i];
        ClipVertex::project(
            &self.mvp,
            p,
            (
                self.normal_matrix * self.normals[i],
                self.world.transform_point3(p),
            ),
        )
    }

    fn fragment(&self, f: &FragmentInput<(Vec3, Vec3)>) -> Option<LinearRgba> {
        let (normal, position) = f.varyings;
        let n = shading_normal(normal, f.front_facing);
        let to_eye = (self.eye - position).normalize_or_zero();
        let mut color = self.emissive + self.base.modulate(diffuse_light(n, self.lighting));
        for light in &self.lighting.lights {
            let to_light = -light.direction.normalize_or_zero();
            if n.dot(to_light) <= 0.0 {
                continue;
            }
            let h = (to_light + to_eye).normalize_or_zero();
            let strength = libm::powf(n.dot(h).max(0.0), self.shininess);
            color = color + light.color.modulate(self.specular) * strength;
        }
        Some(LinearRgba {
            a: self.base.a,
            ..color
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::DirectionalLight;

    fn lighting() -> Lighting {
        Lighting {
            ambient: LinearRgba::rgb(0.0, 0.0, 0.0),
            lights: vec![DirectionalLight {
                direction: Vec3::NEG_Z,
                color: LinearRgba::rgb(1.0, 1.0, 1.0),
            }],
        }
    }

    fn material() -> Material {
        Material {
            base_color: LinearRgba::rgb(0.5, 0.0, 0.0),
            specular: LinearRgba::rgb(0.25, 0.25, 0.25),
            shininess: 16.0,
            ..Material::default()
        }
    }

    /// Shades a surface at the origin with normal `n`, seen from `eye`.
    fn shade(n: Vec3, eye: Vec3) -> LinearRgba {
        let mesh = Mesh::new(vec![Vec3::ZERO], vec![])
            .unwrap()
            .with_normals(vec![n])
            .unwrap();
        let l = lighting();
        let s = BlinnPhongShader::new(Mat4::IDENTITY, Mat4::IDENTITY, eye, &mesh, &material(), &l)
            .unwrap();
        s.fragment(&FragmentInput {
            x: 0,
            y: 0,
            depth: 0.5,
            front_facing: true,
            varyings: s.vertex(0).varyings,
        })
        .unwrap()
    }

    #[test]
    fn mirror_direction_gives_full_highlight() {
        // Light and eye both along +Z: h = n, so the highlight is `specular`.
        let c = shade(Vec3::Z, Vec3::new(0.0, 0.0, 5.0));
        assert!(
            (c.r - 0.75).abs() < 1e-5 && (c.g - 0.25).abs() < 1e-5 && (c.b - 0.25).abs() < 1e-5,
            "{c:?}"
        );
    }

    #[test]
    fn highlight_falls_off_with_the_half_vector_angle() {
        // Eye at 90° from the light: h is 45° from n, cos^16(45°) = 2^-8.
        let c = shade(Vec3::Z, Vec3::new(5.0, 0.0, 0.0));
        let expected = 0.25 * libm::powf(std::f32::consts::FRAC_1_SQRT_2, 16.0);
        assert!((c.g - expected).abs() < 1e-6, "{c:?} vs {expected}");
        assert!((c.r - (0.5 + expected)).abs() < 1e-6);
    }

    #[test]
    fn no_highlight_where_the_light_does_not_reach() {
        let c = shade(Vec3::NEG_Z, Vec3::new(0.0, 0.0, -5.0));
        assert_eq!((c.r, c.g, c.b), (0.0, 0.0, 0.0));
    }
}

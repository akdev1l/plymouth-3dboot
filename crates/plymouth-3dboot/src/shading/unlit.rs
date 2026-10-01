// SPDX-License-Identifier: GPL-3.0-or-later
//! Unlit shading.

use crate::color::LinearRgba;
use crate::math::{Mat4, Vec3};
use crate::pipeline::{ClipVertex, FragmentInput, Shader};
use crate::scene::Material;

/// Shades every fragment with the material's base colour, ignoring lights.
///
/// This matches materials authored as fully self-illuminated (for example
/// the sample N64 logo).
#[derive(Clone, Copy, Debug)]
pub struct UnlitShader<'a> {
    mvp: Mat4,
    positions: &'a [Vec3],
    color: LinearRgba,
}

impl<'a> UnlitShader<'a> {
    /// A shader drawing `positions` (model space) with `mvp` and the
    /// material's base colour.
    #[must_use]
    pub fn new(mvp: Mat4, positions: &'a [Vec3], material: &Material) -> Self {
        Self {
            mvp,
            positions,
            color: material.base_color,
        }
    }
}

impl Shader for UnlitShader<'_> {
    type Varyings = ();

    fn vertex(&self, index: u32) -> ClipVertex<()> {
        ClipVertex::project(&self.mvp, self.positions[index as usize], ())
    }

    fn fragment(&self, _: &FragmentInput<()>) -> Option<LinearRgba> {
        Some(self.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_is_the_base_colour_regardless_of_facing() {
        let m = Material {
            emissive: LinearRgba::rgb(0.5, 0.5, 0.5),
            ..Material::with_color("c", LinearRgba::new(0.1, 0.2, 0.3, 0.4))
        };
        let s = UnlitShader::new(Mat4::IDENTITY, &[Vec3::ONE], &m);
        for front_facing in [true, false] {
            let f = FragmentInput {
                x: 0,
                y: 0,
                depth: 0.5,
                front_facing,
                varyings: (),
            };
            assert_eq!(s.fragment(&f), Some(LinearRgba::new(0.1, 0.2, 0.3, 0.4)));
        }
        assert_eq!(s.vertex(0).position, Vec3::ONE.extend(1.0));
    }
}

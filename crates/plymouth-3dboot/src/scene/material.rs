// SPDX-License-Identifier: GPL-3.0-or-later
//! Surface materials.

use crate::color::LinearRgba;

/// Index of a [`Material`] in a scene's material list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub usize);

/// Surface appearance parameters, in linear colour.
///
/// Shading models use the subset they need: unlit shading shows
/// `base_color` as is; lit models add `emissive`, diffuse lighting of
/// `base_color`, and (Blinn–Phong) `specular` highlights of `shininess`.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// Name from the source file (may be empty).
    pub name: String,
    /// Diffuse / base colour; alpha is opacity.
    pub base_color: LinearRgba,
    /// Light emitted by the surface itself (alpha ignored).
    pub emissive: LinearRgba,
    /// Specular reflectance (alpha ignored).
    pub specular: LinearRgba,
    /// Blinn–Phong specular exponent.
    pub shininess: f32,
    /// Whether back faces should be rendered (disables back-face culling).
    pub double_sided: bool,
}

impl Default for Material {
    /// Opaque white, no emission, no specular, single-sided.
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: LinearRgba::WHITE,
            emissive: LinearRgba::BLACK,
            specular: LinearRgba::BLACK,
            shininess: 0.0,
            double_sided: false,
        }
    }
}

impl Material {
    /// A default material with the given name and base colour.
    #[must_use]
    pub fn with_color(name: impl Into<String>, base_color: LinearRgba) -> Self {
        Self {
            name: name.into(),
            base_color,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_material_is_opaque_white_without_emission() {
        let m = Material::default();
        assert_eq!(m.base_color, LinearRgba::WHITE);
        assert_eq!(m.emissive, LinearRgba::BLACK);
        assert!(!m.double_sided);
        let c = Material::with_color("red", LinearRgba::rgb(1.0, 0.0, 0.0));
        assert_eq!((c.name.as_str(), c.base_color.g), ("red", 0.0));
    }
}

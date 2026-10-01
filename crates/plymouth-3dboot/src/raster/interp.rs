// SPDX-License-Identifier: GPL-3.0-or-later
//! Interpolation of per-vertex attributes ("varyings") across a triangle.

use crate::color::LinearRgba;
use crate::math::{Vec2, Vec3, Vec4};

/// A per-vertex attribute that can be blended with three weights.
///
/// Weights are barycentric (screen-linear, or perspective-corrected by the
/// pipeline) and sum to 1.
pub trait Interpolate: Copy {
    /// `w[0] * values[0] + w[1] * values[1] + w[2] * values[2]`.
    #[must_use]
    fn interpolate(values: [Self; 3], w: [f32; 3]) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(v: [Self; 3], w: [f32; 3]) -> Self {
        v[0] * w[0] + v[1] * w[1] + v[2] * w[2]
    }
}

macro_rules! impl_interpolate_vec {
    ($($t:ty),*) => {$(
        impl Interpolate for $t {
            fn interpolate(v: [Self; 3], w: [f32; 3]) -> Self {
                v[0] * w[0] + v[1] * w[1] + v[2] * w[2]
            }
        }
    )*};
}
impl_interpolate_vec!(Vec2, Vec3, Vec4, LinearRgba);

impl Interpolate for () {
    fn interpolate(_: [Self; 3], _: [f32; 3]) -> Self {}
}

macro_rules! impl_interpolate_tuple {
    ($(($($name:ident $idx:tt),+)),*) => {$(
        impl<$($name: Interpolate),+> Interpolate for ($($name,)+) {
            fn interpolate(v: [Self; 3], w: [f32; 3]) -> Self {
                ($($name::interpolate([v[0].$idx, v[1].$idx, v[2].$idx], w),)+)
            }
        }
    )*};
}
impl_interpolate_tuple!((A 0), (A 0, B 1), (A 0, B 1, C 2), (A 0, B 1, C 2, D 3));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_hot_weights_select_a_vertex() {
        let v = [Vec3::X, Vec3::Y, Vec3::Z];
        assert_eq!(Vec3::interpolate(v, [0.0, 1.0, 0.0]), Vec3::Y);
        assert_eq!(f32::interpolate([1.0, 2.0, 3.0], [0.0, 0.0, 1.0]), 3.0);
    }

    #[test]
    fn tuples_interpolate_componentwise() {
        let v = [
            (1.0f32, Vec2::ZERO, LinearRgba::BLACK),
            (3.0, Vec2::ONE, LinearRgba::WHITE),
            (5.0, Vec2::ONE, LinearRgba::WHITE),
        ];
        let (a, b, c) = <(f32, Vec2, LinearRgba)>::interpolate(v, [0.5, 0.25, 0.25]);
        assert_eq!(a, 2.5);
        assert_eq!(b, Vec2::splat(0.5));
        assert_eq!(c, LinearRgba::new(0.5, 0.5, 0.5, 1.0));
    }
}

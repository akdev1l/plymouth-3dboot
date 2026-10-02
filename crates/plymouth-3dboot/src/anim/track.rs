// SPDX-License-Identifier: GPL-3.0-or-later
//! Keyframe tracks: values over time with an interpolation mode.

use crate::math::{Mat4, Quat, Vec3, Vec4};

/// How values between keyframes are computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Interpolation {
    /// Hold each key's value until the next key.
    Step,
    /// Linear interpolation (spherical for rotations).
    #[default]
    Linear,
    /// Cubic Hermite spline with explicit in/out tangents per key (as in
    /// glTF): tangents are in value units per second.
    CubicSpline,
}

/// A value that can be animated.
pub trait Animatable: Copy {
    /// Interpolates linearly from `a` (`t = 0`) to `b` (`t = 1`).
    #[must_use]
    fn lerp(a: Self, b: Self, t: f32) -> Self;

    /// `a * wa + b * wb + c * wc + d * wd` (for Hermite splines).
    #[must_use]
    fn combine(terms: [(Self, f32); 4]) -> Self;

    /// Fixes up a value after spline evaluation (e.g. renormalizes
    /// quaternions). The default does nothing.
    #[must_use]
    fn finish(self) -> Self {
        self
    }
}

impl Animatable for f32 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }

    fn combine(terms: [(Self, f32); 4]) -> Self {
        terms.iter().map(|(v, w)| v * w).sum()
    }
}

impl Animatable for Vec3 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a.lerp(b, t)
    }

    fn combine(terms: [(Self, f32); 4]) -> Self {
        terms.iter().map(|(v, w)| *v * *w).sum()
    }
}

impl Animatable for Quat {
    /// Spherical interpolation along the shortest arc.
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        let b = if a.dot(b) < 0.0 { -b } else { b };
        slerp(a, b, t)
    }

    fn combine(terms: [(Self, f32); 4]) -> Self {
        let v: Vec4 = terms.iter().map(|(q, w)| Vec4::from(*q) * *w).sum();
        Self::from_vec4(v)
    }

    fn finish(self) -> Self {
        self.normalize()
    }
}

impl Animatable for Mat4 {
    /// Component-wise interpolation (adequate for small steps of affine
    /// matrices; exact for translations).
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a * (1.0 - t) + b * t
    }

    fn combine(terms: [(Self, f32); 4]) -> Self {
        terms.iter().fold(Self::ZERO, |acc, (m, w)| acc + *m * *w)
    }
}

/// Spherical linear interpolation of unit quaternions with `a · b >= 0`,
/// using `libm` for cross-target determinism.
fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let d = a.dot(b).clamp(-1.0, 1.0);
    if d > 0.9995 {
        // Nearly parallel: normalized lerp avoids dividing by ~0.
        return Quat::from_vec4(Vec4::from(a).lerp(Vec4::from(b), t)).normalize();
    }
    let theta = libm::acosf(d);
    let s = libm::sinf(theta);
    let wa = libm::sinf((1.0 - t) * theta) / s;
    let wb = libm::sinf(t * theta) / s;
    Quat::from_vec4(Vec4::from(a) * wa + Vec4::from(b) * wb).normalize()
}

/// A track's keyframes were invalid.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TrackError {
    /// A track needs at least one key.
    #[error("a track needs at least one key")]
    Empty,
    /// Key times must be finite and strictly increasing.
    #[error("key times must be finite and strictly increasing (at key {0})")]
    Times(usize),
    /// The number of values does not match the keys (cubic splines need
    /// three values per key: in-tangent, value, out-tangent).
    #[error("{values} values for {keys} keys with {interpolation:?} interpolation")]
    ValueCount {
        /// Number of keys.
        keys: usize,
        /// Number of values.
        values: usize,
        /// The interpolation mode.
        interpolation: Interpolation,
    },
}

/// Keyframed values of type `T`.
#[derive(Clone, Debug, PartialEq)]
pub struct Track<T> {
    times: Vec<f32>,
    values: Vec<T>,
    interpolation: Interpolation,
}

impl<T: Animatable> Track<T> {
    /// Creates a track. For [`Interpolation::CubicSpline`], `values` holds
    /// `[in_tangent, value, out_tangent]` for each key.
    ///
    /// # Errors
    ///
    /// Returns [`TrackError`] for empty tracks, non-increasing or
    /// non-finite times, or a mismatched value count.
    pub fn new(
        times: Vec<f32>,
        values: Vec<T>,
        interpolation: Interpolation,
    ) -> Result<Self, TrackError> {
        if times.is_empty() {
            return Err(TrackError::Empty);
        }
        if let Some(i) =
            (0..times.len()).find(|&i| !times[i].is_finite() || (i > 0 && times[i] <= times[i - 1]))
        {
            return Err(TrackError::Times(i));
        }
        let per_key = if interpolation == Interpolation::CubicSpline {
            3
        } else {
            1
        };
        if values.len() != times.len() * per_key {
            return Err(TrackError::ValueCount {
                keys: times.len(),
                values: values.len(),
                interpolation,
            });
        }
        Ok(Self {
            times,
            values,
            interpolation,
        })
    }

    /// A track holding one value forever.
    #[must_use]
    pub fn constant(value: T) -> Self {
        Self {
            times: vec![0.0],
            values: vec![value],
            interpolation: Interpolation::Step,
        }
    }

    /// Key times in seconds.
    #[must_use]
    pub fn times(&self) -> &[f32] {
        &self.times
    }

    /// Interpolation mode.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// Time of the last key.
    #[must_use]
    pub fn end_time(&self) -> f32 {
        self.times[self.times.len() - 1]
    }

    /// The value at key `i`.
    fn key(&self, i: usize) -> T {
        match self.interpolation {
            Interpolation::CubicSpline => self.values[i * 3 + 1],
            _ => self.values[i],
        }
    }

    /// The value at time `t` (seconds). Before the first key and after the
    /// last, the first or last value is held.
    #[must_use]
    pub fn sample(&self, t: f32) -> T {
        let n = self.times.len();
        if n == 1 || t <= self.times[0] || t.is_nan() {
            return self.key(0);
        }
        if t >= self.times[n - 1] {
            return self.key(n - 1);
        }
        // Index of the segment [times[i], times[i + 1]) containing t.
        let i = self.times.partition_point(|&k| k <= t) - 1;
        let (t0, t1) = (self.times[i], self.times[i + 1]);
        let dt = t1 - t0;
        let u = (t - t0) / dt;
        match self.interpolation {
            Interpolation::Step => self.key(i),
            Interpolation::Linear => T::lerp(self.key(i), self.key(i + 1), u),
            Interpolation::CubicSpline => {
                let (p0, m0) = (self.values[i * 3 + 1], self.values[i * 3 + 2]);
                let (p1, m1) = (self.values[(i + 1) * 3 + 1], self.values[(i + 1) * 3]);
                let (u2, u3) = (u * u, u * u * u);
                T::combine([
                    (p0, 2.0 * u3 - 3.0 * u2 + 1.0),
                    (m0, (u3 - 2.0 * u2 + u) * dt),
                    (p1, -2.0 * u3 + 3.0 * u2),
                    (m1, (u3 - u2) * dt),
                ])
                .finish()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::f32::consts::PI;

    fn linear(times: &[f32], values: &[f32]) -> Track<f32> {
        Track::new(times.to_vec(), values.to_vec(), Interpolation::Linear).unwrap()
    }

    #[test]
    fn clamps_outside_the_key_range_and_hits_keys_exactly() {
        let t = linear(&[1.0, 2.0, 4.0], &[10.0, 20.0, 0.0]);
        assert_eq!(t.sample(-5.0), 10.0);
        assert_eq!(t.sample(1.0), 10.0);
        assert_eq!(t.sample(2.0), 20.0);
        assert_eq!(t.sample(4.0), 0.0);
        assert_eq!(t.sample(100.0), 0.0);
        assert_eq!(t.sample(f32::NAN), 10.0);
        assert_eq!(t.end_time(), 4.0);
    }

    #[test]
    fn linear_midpoints() {
        let t = linear(&[1.0, 2.0, 4.0], &[10.0, 20.0, 0.0]);
        assert_eq!(t.sample(1.5), 15.0);
        assert_eq!(t.sample(3.0), 10.0);
    }

    #[test]
    fn step_holds_until_the_next_key() {
        let t = Track::new(
            vec![0.0, 1.0],
            vec![Vec3::ZERO, Vec3::ONE],
            Interpolation::Step,
        )
        .unwrap();
        assert_eq!(t.sample(0.999), Vec3::ZERO);
        assert_eq!(t.sample(1.0), Vec3::ONE);
        assert_eq!(Track::constant(Vec3::X).sample(7.0), Vec3::X);
    }

    #[test]
    fn cubic_spline_reproduces_a_cubic_polynomial() {
        // f(t) = t^3 - 2t, f'(t) = 3t^2 - 2: Hermite with exact derivatives
        // reproduces any cubic exactly.
        let f = |t: f32| t * t * t - 2.0 * t;
        let df = |t: f32| 3.0 * t * t - 2.0;
        let times = vec![-1.0, 0.5, 2.0];
        let values = times.iter().flat_map(|&t| [df(t), f(t), df(t)]).collect();
        let track = Track::new(times, values, Interpolation::CubicSpline).unwrap();
        for i in 0..=30 {
            let t = -1.0 + 3.0 * i as f32 / 30.0;
            assert!(
                (track.sample(t) - f(t)).abs() < 1e-4,
                "t = {t}: {} vs {}",
                track.sample(t),
                f(t)
            );
        }
    }

    #[test]
    fn quaternions_take_the_shortest_arc() {
        let a = Quat::from_rotation_y(0.0);
        // +170° stored as its negated (equivalent) quaternion.
        let b = -Quat::from_rotation_y(170f32.to_radians());
        let t = Track::new(vec![0.0, 1.0], vec![a, b], Interpolation::Linear).unwrap();
        let mid = t.sample(0.5);
        let expected = Quat::from_rotation_y(85f32.to_radians());
        assert!(mid.dot(expected).abs() > 0.99999, "{mid} vs {expected}");
        // Halfway through a 120° turn is 60°. (A 180° turn has no unique
        // shortest arc.)
        let q = Track::new(
            vec![0.0, 1.0],
            vec![Quat::IDENTITY, Quat::from_rotation_z(2.0 * PI / 3.0)],
            Interpolation::Linear,
        )
        .unwrap();
        assert!(q.sample(0.5).dot(Quat::from_rotation_z(PI / 3.0)).abs() > 0.99999);
    }

    #[test]
    fn invalid_tracks_are_rejected() {
        assert_eq!(
            Track::<f32>::new(vec![], vec![], Interpolation::Linear),
            Err(TrackError::Empty)
        );
        assert_eq!(
            Track::new(vec![0.0, 0.0], vec![1.0, 2.0], Interpolation::Linear),
            Err(TrackError::Times(1))
        );
        assert_eq!(
            Track::new(vec![1.0, 0.0], vec![1.0, 2.0], Interpolation::Linear),
            Err(TrackError::Times(1))
        );
        assert_eq!(
            Track::new(vec![f32::NAN], vec![1.0], Interpolation::Linear),
            Err(TrackError::Times(0))
        );
        assert!(matches!(
            Track::new(vec![0.0, 1.0], vec![1.0, 2.0], Interpolation::CubicSpline),
            Err(TrackError::ValueCount { .. })
        ));
    }

    #[test]
    fn matrices_interpolate_translations_exactly() {
        let a = Mat4::from_translation(Vec3::ZERO);
        let b = Mat4::from_translation(Vec3::new(2.0, 4.0, 6.0));
        let t = Track::new(vec![0.0, 2.0], vec![a, b], Interpolation::Linear).unwrap();
        assert_eq!(
            t.sample(1.0),
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    fn unit_quat() -> impl Strategy<Value = Quat> {
        (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0)
            .prop_filter_map("non-zero", |(x, y, z, w)| {
                Vec4::new(x, y, z, w).try_normalize().map(Quat::from_vec4)
            })
    }

    proptest! {
        /// Interpolated rotations stay unit length and never take the long
        /// way: the angle to either endpoint is at most the angle between
        /// them.
        #[test]
        fn quaternion_interpolation_is_unit_and_shortest(a in unit_quat(), b in unit_quat(), u in 0.0f32..1.0, cubic in any::<bool>()) {
            let track = if cubic {
                Track::new(vec![0.0, 1.0], vec![Quat::from_vec4(Vec4::ZERO), a, Quat::from_vec4(Vec4::ZERO), Quat::from_vec4(Vec4::ZERO), if a.dot(b) < 0.0 { -b } else { b }, Quat::from_vec4(Vec4::ZERO)], Interpolation::CubicSpline).unwrap()
            } else {
                Track::new(vec![0.0, 1.0], vec![a, b], Interpolation::Linear).unwrap()
            };
            let q = track.sample(u);
            prop_assert!((q.length() - 1.0).abs() < 1e-4);
            let angle = |x: Quat, y: Quat| 2.0 * libm::acosf(x.dot(y).abs().min(1.0));
            let total = angle(a, b);
            prop_assert!(angle(a, q) <= total + 1e-3 && angle(q, b) <= total + 1e-3, "{a} {b} {q}");
        }
    }
}

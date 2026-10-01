// SPDX-License-Identifier: GPL-3.0-or-later
//! Procedural clips, for models without authored animation.

use super::{Channel, Clip, Interpolation, Property, Track};
use crate::math::{Quat, Vec3};
use crate::scene::NodeId;

/// Keys per full turn of a turntable. Quarter turns keep every segment
/// well within the shortest-arc range of rotation interpolation.
const TURNTABLE_KEYS: u16 = 4;
/// Keys per bounce (cubic spline with exact tangents).
const BOUNCE_KEYS: u16 = 16;

impl Clip {
    /// Rotates `target` one full turn about `axis` (counter-clockwise looking
    /// down the axis) every `period` seconds, at constant speed. Loop it
    /// with [`super::WrapMode::Loop`]: the end pose equals the start pose.
    ///
    /// The rotation replaces the node's own rotation; to spin a whole model,
    /// target a dedicated parent node (see
    /// [`crate::scene::Scene::wrapped_in_root`]).
    #[must_use]
    pub fn turntable(target: NodeId, axis: Vec3, period: f32) -> Self {
        let axis = axis.try_normalize().unwrap_or(Vec3::Y);
        let period = period.max(f32::EPSILON);
        let n = f32::from(TURNTABLE_KEYS);
        let times = (0..=TURNTABLE_KEYS)
            .map(|k| period * f32::from(k) / n)
            .collect();
        let values = (0..=TURNTABLE_KEYS)
            .map(|k| Quat::from_axis_angle(axis, std::f32::consts::TAU * f32::from(k) / n))
            .collect();
        let track = Track::new(times, values, Interpolation::Linear).expect("increasing times");
        Self::new(
            "turntable",
            vec![Channel {
                target,
                property: Property::Rotation(track),
            }],
        )
    }

    /// Moves `target` from `base` up by `offset` and back once per `period`,
    /// following `|sin|` (fast near the ground, slow at the top, like a
    /// bouncing ball). Replaces the node's own translation.
    #[must_use]
    pub fn bounce(target: NodeId, base: Vec3, offset: Vec3, period: f32) -> Self {
        let period = period.max(f32::EPSILON);
        let n = f32::from(BOUNCE_KEYS);
        let w = std::f32::consts::PI / period;
        let mut times = Vec::new();
        let mut values = Vec::new();
        for k in 0..=BOUNCE_KEYS {
            let t = period * f32::from(k) / n;
            let phase = w * t;
            let height = libm::sinf(phase).abs();
            let slope = w * libm::cosf(phase);
            // |sin| has a kink at the ground contacts: the speed flips there.
            let (tan_in, tan_out) = match k {
                0 => (-w, w),
                k if k == BOUNCE_KEYS => (-w, w),
                _ => (slope, slope),
            };
            times.push(t);
            values.extend([offset * tan_in, base + offset * height, offset * tan_out]);
        }
        let track =
            Track::new(times, values, Interpolation::CubicSpline).expect("increasing times");
        Self::new(
            "bounce",
            vec![Channel {
                target,
                property: Property::Translation(track),
            }],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Pose, WrapMode};
    use crate::scene::{LocalTransform, Node, Scene};
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn turntable_turns_a_quarter_per_quarter_period() {
        let clip = Clip::turntable(NodeId(0), Vec3::Y * 2.0, 8.0);
        assert_eq!(clip.duration(), 8.0);
        let Property::Rotation(track) = &clip.channels[0].property else {
            panic!()
        };
        for (t, angle) in [
            (0.0, 0.0),
            (2.0, FRAC_PI_2),
            (3.0, 3.0 * FRAC_PI_2 / 2.0),
            (5.0, 5.0 * FRAC_PI_2 / 2.0),
        ] {
            let q = track.sample(t);
            assert!(
                q.dot(Quat::from_rotation_y(angle)).abs() > 0.99999,
                "t = {t}"
            );
        }
        // The loop is seamless: the end pose equals the start pose.
        let end = track.sample(clip.local_time(8.0, WrapMode::Loop));
        assert!(end.dot(Quat::IDENTITY).abs() > 0.99999);
        assert!(track.sample(8.0).dot(Quat::IDENTITY).abs() > 0.99999);
    }

    #[test]
    fn turntable_moves_points_at_constant_speed() {
        let mut scene = Scene::new();
        let n = scene
            .add_node(None, Node::new("n", LocalTransform::default()))
            .unwrap();
        let clip = Clip::turntable(n, Vec3::Y, 4.0);
        let p =
            |t: f32| Pose::evaluate(&scene, &clip, t).world(&scene)[0].transform_point3(Vec3::X);
        let mut prev = p(0.0);
        for i in 1..=40 {
            let cur = p(i as f32 * 0.1);
            let step = prev.distance(cur);
            assert!(
                (step - std::f32::consts::TAU / 40.0).abs() < 2e-3,
                "step {i}: {step}"
            );
            prev = cur;
        }
    }

    #[test]
    fn bounce_follows_abs_sine() {
        let clip = Clip::bounce(NodeId(0), Vec3::new(1.0, 0.0, 0.0), Vec3::Y * 2.0, 1.0);
        let Property::Translation(track) = &clip.channels[0].property else {
            panic!()
        };
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let expected = Vec3::new(1.0, 2.0 * libm::sinf(std::f32::consts::PI * t).abs(), 0.0);
            assert!(
                track.sample(t).abs_diff_eq(expected, 1e-3),
                "t = {t}: {}",
                track.sample(t)
            );
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Animation channels and clips.

use super::Track;
use crate::math::{Mat4, Quat, Vec3};
use crate::scene::NodeId;

/// Animation of one element of a node's transform stack
/// ([`crate::scene::LocalTransform::Stack`]), addressed by its `sid`.
#[derive(Clone, Debug, PartialEq)]
pub enum ElementTrack {
    /// The whole value of a `translate` or `scale` element.
    Vector(Track<Vec3>),
    /// The angle (radians) of a `rotate` element.
    Angle(Track<f32>),
    /// One component (0 = X, 1 = Y, 2 = Z) of a `translate` or `scale`
    /// element.
    Component {
        /// Component index.
        index: usize,
        /// Its values.
        track: Track<f32>,
    },
    /// The whole value of a `matrix` element.
    Matrix(Track<Mat4>),
}

impl ElementTrack {
    fn end_time(&self) -> f32 {
        match self {
            Self::Vector(t) => t.end_time(),
            Self::Angle(t) | Self::Component { track: t, .. } => t.end_time(),
            Self::Matrix(t) => t.end_time(),
        }
    }
}

/// What a channel animates.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    /// Translation of a node.
    Translation(Track<Vec3>),
    /// Rotation of a node.
    Rotation(Track<Quat>),
    /// Scale of a node.
    Scale(Track<Vec3>),
    /// The node's whole local matrix.
    Matrix(Track<Mat4>),
    /// One element of the node's transform stack.
    StackElement {
        /// The element's `sid`.
        sid: String,
        /// Its animation.
        track: ElementTrack,
    },
}

impl Property {
    /// Time of the last key.
    #[must_use]
    pub fn end_time(&self) -> f32 {
        match self {
            Self::Translation(t) | Self::Scale(t) => t.end_time(),
            Self::Rotation(t) => t.end_time(),
            Self::Matrix(t) => t.end_time(),
            Self::StackElement { track, .. } => track.end_time(),
        }
    }
}

/// One animated property of one node.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// The animated node.
    pub target: NodeId,
    /// The animated property.
    pub property: Property,
}

/// How time outside `[0, duration]` maps into the clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WrapMode {
    /// Hold the first/last pose.
    Clamp,
    /// Repeat from the start.
    #[default]
    Loop,
    /// Play forwards, then backwards, and so on.
    PingPong,
}

/// A named set of channels played together.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Name (may be empty).
    pub name: String,
    /// The animated properties.
    pub channels: Vec<Channel>,
    start: f32,
    duration: f32,
}

impl Clip {
    /// A clip whose duration is the time of its last key (0 without keys).
    #[must_use]
    pub fn new(name: impl Into<String>, channels: Vec<Channel>) -> Self {
        let duration = channels
            .iter()
            .map(|c| c.property.end_time())
            .fold(0.0, f32::max);
        Self {
            name: name.into(),
            channels,
            start: 0.0,
            duration,
        }
    }

    /// Restricts playback to the key times `[start, end]` (e.g. one clip of
    /// a longer animation): playback time 0 maps to `start`. Non-finite
    /// values give an empty range at 0.
    #[must_use]
    pub fn with_range(mut self, start: f32, end: f32) -> Self {
        if start.is_finite() && end.is_finite() {
            self.start = start;
            self.duration = (end - start).max(0.0);
        } else {
            self.start = 0.0;
            self.duration = 0.0;
        }
        self
    }

    /// Key time at which playback starts.
    #[must_use]
    pub fn start(&self) -> f32 {
        self.start
    }

    /// Overrides the duration (e.g. a loop that holds the last pose for a
    /// while). Negative or non-finite values are treated as 0.
    #[must_use]
    pub fn with_duration(mut self, duration: f32) -> Self {
        self.duration = if duration.is_finite() {
            duration.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// Length in seconds.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Maps playback time `t` (seconds, any value) to the key time in
    /// `[start, start + duration]` to sample. Wrapping is computed in `f64` so long-running
    /// playback keeps full precision.
    #[must_use]
    pub fn local_time(&self, t: f64, wrap: WrapMode) -> f32 {
        let d = f64::from(self.duration);
        if d <= 0.0 || !t.is_finite() {
            return self.start;
        }
        let local = match wrap {
            WrapMode::Clamp => t.clamp(0.0, d),
            WrapMode::Loop => t.rem_euclid(d),
            WrapMode::PingPong => {
                let p = t.rem_euclid(2.0 * d);
                if p > d { 2.0 * d - p } else { p }
            }
        };
        // `local` is within [0, d] and d fits in f32.
        #[allow(clippy::cast_possible_truncation)]
        let local = local as f32;
        self.start + local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Interpolation;

    fn clip(end: f32) -> Clip {
        let track = Track::new(
            vec![0.0, end],
            vec![Vec3::ZERO, Vec3::ONE],
            Interpolation::Linear,
        )
        .unwrap();
        Clip::new(
            "c",
            vec![Channel {
                target: NodeId(0),
                property: Property::Translation(track),
            }],
        )
    }

    #[test]
    fn duration_is_the_last_key_time() {
        let a = Track::new(vec![0.0, 2.0], vec![0.0, 1.0], Interpolation::Linear).unwrap();
        let b = Track::new(
            vec![0.5, 3.5],
            vec![Quat::IDENTITY, Quat::IDENTITY],
            Interpolation::Linear,
        )
        .unwrap();
        let c = Clip::new(
            "two",
            vec![
                Channel {
                    target: NodeId(0),
                    property: Property::StackElement {
                        sid: "rotateY".into(),
                        track: ElementTrack::Angle(a),
                    },
                },
                Channel {
                    target: NodeId(1),
                    property: Property::Rotation(b),
                },
            ],
        );
        assert_eq!(c.duration(), 3.5);
        assert_eq!(Clip::new("empty", vec![]).duration(), 0.0);
        assert_eq!(clip(2.0).with_duration(5.0).duration(), 5.0);
        assert_eq!(clip(2.0).with_duration(f32::NAN).duration(), 0.0);
    }

    #[test]
    fn wrap_modes() {
        let c = clip(2.0);
        let cases = [
            // (t, clamp, loop, ping-pong)
            (-0.5, 0.0, 1.5, 0.5),
            (0.0, 0.0, 0.0, 0.0),
            (1.25, 1.25, 1.25, 1.25),
            (2.0, 2.0, 0.0, 2.0),
            (3.0, 2.0, 1.0, 1.0),
            (4.5, 2.0, 0.5, 0.5),
        ];
        for (t, clamp, looped, ping) in cases {
            assert_eq!(c.local_time(t, WrapMode::Clamp), clamp, "clamp {t}");
            assert_eq!(c.local_time(t, WrapMode::Loop), looped, "loop {t}");
            assert_eq!(c.local_time(t, WrapMode::PingPong), ping, "ping-pong {t}");
        }
    }

    #[test]
    fn ranges_offset_and_limit_playback() {
        let c = clip(10.0).with_range(2.0, 5.0);
        assert_eq!((c.start(), c.duration()), (2.0, 3.0));
        assert_eq!(c.local_time(0.0, WrapMode::Loop), 2.0);
        assert_eq!(c.local_time(4.0, WrapMode::Loop), 3.0);
        assert_eq!(c.local_time(9.0, WrapMode::Clamp), 5.0);
        let bad = clip(1.0).with_range(f32::NAN, 1.0);
        assert_eq!((bad.start(), bad.duration()), (0.0, 0.0));
    }

    #[test]
    fn long_playback_keeps_precision() {
        // After a day of looping a 3.3333 s clip, f32 time alone would have
        // lost several milliseconds; wrapping in f64 does not.
        let c = clip(10.0 / 3.0);
        let t: f64 = 86_400.0 + 1.0;
        // The period is the clip duration as stored (an f32 key time).
        let expected = t.rem_euclid(f64::from(c.duration()));
        assert!((f64::from(c.local_time(t, WrapMode::Loop)) - expected).abs() < 1e-6);
    }

    #[test]
    fn degenerate_inputs() {
        assert_eq!(Clip::new("e", vec![]).local_time(5.0, WrapMode::Loop), 0.0);
        assert_eq!(clip(1.0).local_time(f64::INFINITY, WrapMode::Loop), 0.0);
    }
}

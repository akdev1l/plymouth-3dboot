// SPDX-License-Identifier: GPL-3.0-or-later
//! Keyframe animation.

pub mod builders;
pub mod clip;
pub mod pose;
pub mod track;

pub use clip::{Channel, Clip, ElementTrack, Property, WrapMode};
pub use pose::Pose;
pub use track::{Animatable, Interpolation, Track, TrackError};

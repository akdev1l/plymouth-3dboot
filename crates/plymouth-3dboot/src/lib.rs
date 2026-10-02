// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU rasterization of animated 3D models.
//!
//! The core crate is pure Rust: it has no platform dependencies and produces
//! identical output on native and WebAssembly targets. It performs no I/O of
//! its own, except reading files through [`io::FsResolver`] and writing
//! [`io::sequence::PngSequence`] files when a caller chooses to use them.

//!
//! # Example
//!
//! Load a model, spin it on a turntable and render a frame:
//!
//! ```
//! use plymouth_3dboot::{FrameSettings, Format, Model, WrapMode};
//! use plymouth_3dboot::io::MemResolver;
//! use plymouth_3dboot::math::Vec3;
//!
//! let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
//! let model = Model::from_source(Format::Obj, obj, &MemResolver::new())?
//!     .with_turntable(Vec3::Y, 4.0);
//! let mut renderer = model.renderer(Some(0), WrapMode::Loop, FrameSettings::new(64, 64))?;
//! let frame = renderer.render_at(1.0)?;
//! assert_eq!((frame.width(), frame.height()), (64, 64));
//! let png = plymouth_3dboot::io::png::encode(frame)?;
//! # let _ = png;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

pub mod anim;
pub mod color;
pub mod io;
pub mod math;
pub mod model;
pub mod pipeline;
pub mod raster;
pub mod render;
pub mod scene;
pub mod shading;
pub mod target;

pub use anim::{Clip, WrapMode};
pub use model::{Format, LoadError, Model};
pub use render::{AnimationRenderer, CameraSource, FrameSettings, RenderError};

/// Returns the version of this crate (`major.minor.patch`).
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_triple() {
        let parts: Vec<&str> = version().split('.').collect();
        assert_eq!(parts.len(), 3, "unexpected version {:?}", version());
        for part in parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "non-numeric part in {:?}",
                version()
            );
        }
    }
}

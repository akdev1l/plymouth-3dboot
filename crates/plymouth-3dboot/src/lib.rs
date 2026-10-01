// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU rasterization of animated 3D models.
//!
//! The core crate is pure Rust: it has no platform dependencies, performs no
//! I/O of its own and produces identical output on native and WebAssembly
//! targets.

#![forbid(unsafe_code)]

pub mod color;
pub mod io;
pub mod math;
pub mod pipeline;
pub mod raster;
pub mod target;

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

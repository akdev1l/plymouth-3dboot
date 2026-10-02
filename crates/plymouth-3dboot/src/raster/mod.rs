// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-space triangle rasterization.
//!
//! Vertices arrive in window coordinates (pixels, origin top-left, +y down;
//! see `docs/conventions.md`). They are snapped to a fixed-point subpixel
//! grid (24.8 fixed point) and tested against integer edge functions,
//! so coverage is exact: with the top-left fill rule, triangles sharing an
//! edge cover every pixel along it exactly once.

pub(crate) mod edge;
pub(crate) mod fixed;
pub mod interp;
pub mod triangle;

pub use fixed::MAX_COORD;
pub use interp::Interpolate;
pub use triangle::{CullMode, Fragment, Rect, TriangleSetup, Winding};

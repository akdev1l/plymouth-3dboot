// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-space triangle rasterization.
//!
//! Vertices arrive in window coordinates (pixels, origin top-left, +y down;
//! see `docs/conventions.md`). They are snapped to a fixed-point subpixel
//! grid ([`fixed`]) and tested against integer edge functions ([`edge`]),
//! so coverage is exact: with the top-left fill rule, triangles sharing an
//! edge cover every pixel along it exactly once.

pub mod edge;
pub mod fixed;

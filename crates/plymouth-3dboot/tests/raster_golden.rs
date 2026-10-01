// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden images for screen-space triangle rasterization (Phase 2).
//!
//! Compared exactly: the rasterizer is integer-based and the floating-point
//! math is target-independent, so native and wasm must agree bit for bit.

use plymouth_3dboot::color::{LinearRgba, Rgba8};
use plymouth_3dboot::math::Vec2;
use plymouth_3dboot::raster::{Rect, TriangleSetup};
use plymouth_3dboot::target::ColorBuffer;
use plymouth_3dboot_testutil::{Tolerance, golden};

const SIZE: u32 = 64;
const BACKGROUND: Rgba8 = Rgba8::new(24, 24, 32, 255);

/// Fills a triangle with per-vertex colours interpolated in linear light.
fn fill(target: &mut ColorBuffer, vertices: [Vec2; 3], colors: [LinearRgba; 3]) {
    let Some(tri) = TriangleSetup::new(vertices) else {
        return;
    };
    let scissor = Rect::from_size(target.width(), target.height());
    tri.for_each_pixel(scissor, |f| {
        *target.get_mut(f.x, f.y).expect("scissored") = f.interpolate(colors).to_srgb8();
    });
}

fn canvas() -> ColorBuffer {
    ColorBuffer::new(SIZE, SIZE, BACKGROUND).unwrap()
}

#[test]
fn rgb_triangle() {
    let mut c = canvas();
    fill(
        &mut c,
        [
            Vec2::new(32.0, 4.0),
            Vec2::new(60.0, 58.0),
            Vec2::new(4.0, 52.0),
        ],
        [
            LinearRgba::rgb(1.0, 0.0, 0.0),
            LinearRgba::rgb(0.0, 1.0, 0.0),
            LinearRgba::rgb(0.0, 0.0, 1.0),
        ],
    );
    golden!().assert("raster_rgb_triangle", &c, Tolerance::EXACT);
}

#[test]
fn overlapping_triangles() {
    let mut c = canvas();
    let flat = |r, g, b| [LinearRgba::rgb(r, g, b); 3];
    // A quad split along its diagonal: the shared edge must show no seam.
    fill(
        &mut c,
        [
            Vec2::new(6.0, 6.0),
            Vec2::new(40.0, 10.0),
            Vec2::new(36.0, 40.0),
        ],
        flat(0.8, 0.2, 0.1),
    );
    fill(
        &mut c,
        [
            Vec2::new(6.0, 6.0),
            Vec2::new(36.0, 40.0),
            Vec2::new(8.0, 34.0),
        ],
        flat(0.8, 0.2, 0.1),
    );
    // Drawn later, so it covers the quad where they overlap.
    fill(
        &mut c,
        [
            Vec2::new(24.0, 20.0),
            Vec2::new(62.25, 30.5),
            Vec2::new(30.0, 61.75),
        ],
        flat(0.1, 0.5, 0.9),
    );
    // Partially off-screen, opposite winding.
    fill(
        &mut c,
        [
            Vec2::new(-10.0, 50.0),
            Vec2::new(20.0, 70.0),
            Vec2::new(18.0, 44.0),
        ],
        flat(0.9, 0.8, 0.1),
    );
    // A thin sliver.
    fill(
        &mut c,
        [
            Vec2::new(44.0, 2.0),
            Vec2::new(63.0, 3.0),
            Vec2::new(44.0, 4.5),
        ],
        flat(1.0, 1.0, 1.0),
    );
    golden!().assert("raster_overlap", &c, Tolerance::EXACT);
}

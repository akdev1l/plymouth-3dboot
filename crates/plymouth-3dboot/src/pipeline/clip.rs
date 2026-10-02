// SPDX-License-Identifier: GPL-3.0-or-later
//! Triangle clipping in homogeneous clip space.
//!
//! Triangles are clipped against the near and far planes, a `w > 0` plane,
//! and a guard band of ±[`GUARD_BAND`]·w in x and y (Sutherland–Hodgman).
//! The guard band keeps window coordinates inside the rasterizer's range
//! ([`crate::raster::MAX_COORD`]) for every supported viewport,
//! while the viewport scissor discards off-screen pixels. Clipping against
//! the guard band rather than the screen edges means clip edges almost
//! never fall on visible pixels.

use super::ClipVertex;
use crate::math::Vec4;
use crate::raster::Interpolate;

/// Half-extent of the guard band in NDC units (the visible range is 1).
///
/// With viewports up to [`crate::target::MAX_DIMENSION`] pixels wide and
/// offset at most that much, window coordinates stay within
/// `[-1.5·D, 3.5·D]` ⊂ `[-65536, 65536]`.
pub const GUARD_BAND: f32 = 4.0;

/// Smallest `w` kept after clipping, which guards the perspective divide.
pub const MIN_W: f32 = 1e-5;

/// The clip planes, as signed distance functions (inside where ≥ 0).
const PLANES: [fn(Vec4) -> f32; 7] = [
    |p| p.w - MIN_W,
    |p| p.z + p.w,
    |p| p.w - p.z,
    |p| p.x + GUARD_BAND * p.w,
    |p| GUARD_BAND * p.w - p.x,
    |p| p.y + GUARD_BAND * p.w,
    |p| GUARD_BAND * p.w - p.y,
];

/// Clips triangles, reusing internal buffers between calls.
#[derive(Debug)]
pub struct Clipper<V> {
    current: Vec<ClipVertex<V>>,
    next: Vec<ClipVertex<V>>,
}

impl<V> Default for Clipper<V> {
    fn default() -> Self {
        Self {
            current: Vec::new(),
            next: Vec::new(),
        }
    }
}

/// The point where edge `inside → outside` crosses a plane, given the
/// signed distances of its endpoints.
///
/// Always interpolating from the inside endpoint makes the result
/// independent of the edge's direction, so triangles sharing an edge get
/// bit-identical clip vertices and no cracks.
fn intersect<V: Interpolate>(
    inside: &ClipVertex<V>,
    d_in: f32,
    outside: &ClipVertex<V>,
    d_out: f32,
) -> ClipVertex<V> {
    let t = d_in / (d_in - d_out);
    ClipVertex {
        position: inside.position + (outside.position - inside.position) * t,
        varyings: V::interpolate(
            [inside.varyings, outside.varyings, outside.varyings],
            [1.0 - t, t, 0.0],
        ),
    }
}

impl<V: Interpolate> Clipper<V> {
    /// Creates a clipper.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clips `triangle` and calls `emit` for each resulting triangle (a fan
    /// over the clipped polygon, preserving the winding). Emits nothing if
    /// the triangle is entirely outside.
    pub fn clip(&mut self, triangle: [ClipVertex<V>; 3], mut emit: impl FnMut([ClipVertex<V>; 3])) {
        let mut any_outside = false;
        for plane in PLANES {
            let d = triangle.map(|v| plane(v.position));
            if d.iter().all(|&x| x < 0.0) {
                return;
            }
            any_outside |= d.iter().any(|&x| x < 0.0);
        }
        if !any_outside {
            emit(triangle);
            return;
        }

        self.current.clear();
        self.current.extend_from_slice(&triangle);
        for plane in PLANES {
            if self.current.iter().all(|v| plane(v.position) >= 0.0) {
                continue;
            }
            self.next.clear();
            let n = self.current.len();
            for i in 0..n {
                let (a, b) = (&self.current[i], &self.current[(i + 1) % n]);
                let (da, db) = (plane(a.position), plane(b.position));
                match (da >= 0.0, db >= 0.0) {
                    (true, true) => self.next.push(*b),
                    (true, false) => self.next.push(intersect(a, da, b, db)),
                    (false, true) => {
                        self.next.push(intersect(b, db, a, da));
                        self.next.push(*b);
                    }
                    (false, false) => {}
                }
            }
            std::mem::swap(&mut self.current, &mut self.next);
            if self.current.len() < 3 {
                return;
            }
        }
        for i in 1..self.current.len() - 1 {
            emit([self.current[0], self.current[i], self.current[i + 1]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A clip vertex whose varyings are its own position, so interpolation
    /// can be checked exactly against the clipped position.
    fn cv(x: f32, y: f32, z: f32, w: f32) -> ClipVertex<Vec4> {
        let p = Vec4::new(x, y, z, w);
        ClipVertex {
            position: p,
            varyings: p,
        }
    }

    fn clip_all(tri: [ClipVertex<Vec4>; 3]) -> Vec<[ClipVertex<Vec4>; 3]> {
        let mut out = Vec::new();
        Clipper::new().clip(tri, |t| out.push(t));
        out
    }

    fn assert_valid(out: &[[ClipVertex<Vec4>; 3]]) {
        for v in out.iter().flatten() {
            let p = v.position;
            let eps = 1e-4 * (1.0 + p.abs().max_element());
            for plane in PLANES {
                assert!(plane(p) >= -eps, "vertex {p} outside a clip plane");
            }
            assert!(p.w > 0.0, "w must stay positive: {p}");
            assert!(
                v.varyings.abs_diff_eq(p, eps),
                "varyings {} != position {p}",
                v.varyings
            );
        }
    }

    #[test]
    fn inside_triangle_is_unchanged() {
        let tri = [
            cv(-0.5, -0.5, 0.0, 1.0),
            cv(0.5, -0.5, 0.2, 1.0),
            cv(0.0, 0.5, -0.3, 1.0),
        ];
        assert_eq!(clip_all(tri), vec![tri]);
    }

    #[test]
    fn triangle_inside_guard_band_but_off_screen_is_unchanged() {
        let tri = [
            cv(1.5, 0.0, 0.0, 1.0),
            cv(3.0, 0.0, 0.0, 1.0),
            cv(2.0, -3.5, 0.0, 1.0),
        ];
        assert_eq!(clip_all(tri), vec![tri]);
    }

    #[test]
    fn fully_outside_triangles_are_discarded() {
        // Behind the near plane.
        assert!(
            clip_all([
                cv(0.0, 0.0, -2.0, 1.0),
                cv(1.0, 0.0, -3.0, 1.0),
                cv(0.0, 1.0, -2.5, 1.0)
            ])
            .is_empty()
        );
        // Beyond the far plane.
        assert!(
            clip_all([
                cv(0.0, 0.0, 2.0, 1.0),
                cv(1.0, 0.0, 3.0, 1.0),
                cv(0.0, 1.0, 2.5, 1.0)
            ])
            .is_empty()
        );
        // Beyond the guard band.
        assert!(
            clip_all([
                cv(5.0, 0.0, 0.0, 1.0),
                cv(6.0, 0.0, 0.0, 1.0),
                cv(5.0, 1.0, 0.0, 1.0)
            ])
            .is_empty()
        );
        // Behind the eye (w < 0).
        assert!(
            clip_all([
                cv(0.0, 0.0, 0.5, -1.0),
                cv(1.0, 0.0, 0.5, -1.0),
                cv(0.0, 1.0, 0.5, -2.0)
            ])
            .is_empty()
        );
    }

    #[test]
    fn one_vertex_behind_near_gives_two_triangles() {
        let out = clip_all([
            cv(0.0, 0.0, -2.0, 1.0),
            cv(0.5, 0.0, 0.0, 1.0),
            cv(0.0, 0.5, 0.0, 1.0),
        ]);
        assert_eq!(out.len(), 2);
        assert_valid(&out);
        // Exactly two vertices lie on the near plane (z = -w).
        let on_plane: std::collections::BTreeSet<_> = out
            .iter()
            .flatten()
            .filter(|v| (v.position.z + v.position.w).abs() < 1e-6)
            .map(|v| (v.position.x.to_bits(), v.position.y.to_bits()))
            .collect();
        assert_eq!(on_plane.len(), 2);
    }

    #[test]
    fn two_vertices_behind_near_give_one_triangle() {
        let out = clip_all([
            cv(0.0, 0.0, 0.0, 1.0),
            cv(0.5, 0.0, -2.0, 1.0),
            cv(0.0, 0.5, -2.0, 1.0),
        ]);
        assert_eq!(out.len(), 1);
        assert_valid(&out);
        // The new vertices are at the midpoints (distances 1 and -1), in the
        // original cyclic order (the fan may start at any vertex).
        let mut p = out[0].map(|v| v.position);
        let start = p
            .iter()
            .position(|&q| q == Vec4::new(0.0, 0.0, 0.0, 1.0))
            .expect("kept vertex");
        p.rotate_left(start);
        assert!(
            p[1].abs_diff_eq(Vec4::new(0.25, 0.0, -1.0, 1.0), 1e-6),
            "{}",
            p[1]
        );
        assert!(
            p[2].abs_diff_eq(Vec4::new(0.0, 0.25, -1.0, 1.0), 1e-6),
            "{}",
            p[2]
        );
    }

    #[test]
    fn winding_is_preserved() {
        let tri = [
            cv(-0.5, -0.5, 0.0, 1.0),
            cv(8.0, -0.5, 0.0, 1.0),
            cv(-0.5, 8.0, 0.0, 1.0),
        ];
        let signed = |t: &[ClipVertex<Vec4>; 3]| {
            let [a, b, c] = t.map(|v| v.position.truncate().truncate() / v.position.w);
            (b - a).perp_dot(c - a)
        };
        let out = clip_all(tri);
        assert!(out.len() > 1);
        assert!(out.iter().all(|t| signed(t) > 0.0));
    }

    #[test]
    fn shared_edges_get_identical_clip_vertices() {
        // Two triangles share edge a-b, which crosses the near plane.
        let (a, b) = (cv(0.1, 0.2, -3.0, 1.0), cv(0.7, -0.4, 0.5, 1.3));
        let first = clip_all([a, b, cv(-0.6, 0.6, 0.2, 1.0)]);
        let second = clip_all([b, a, cv(0.9, 0.8, 0.1, 1.1)]);
        let near_pts = |out: &[[ClipVertex<Vec4>; 3]]| -> std::collections::BTreeSet<[u32; 4]> {
            out.iter()
                .flatten()
                .filter(|v| (v.position.z + v.position.w).abs() < 1e-5)
                .map(|v| v.position.to_array().map(f32::to_bits))
                .collect()
        };
        let common: Vec<_> = near_pts(&first)
            .intersection(&near_pts(&second))
            .copied()
            .collect();
        assert_eq!(
            common.len(),
            1,
            "the a-b crossing must be bit-identical in both triangles"
        );
    }

    fn clip_coord() -> impl Strategy<Value = Vec4> {
        (-8.0f32..8.0, -8.0f32..8.0, -8.0f32..8.0, -2.0f32..4.0)
            .prop_map(|(x, y, z, w)| Vec4::new(x, y, z, w))
    }

    proptest! {
        #[test]
        fn clipped_vertices_are_inside_with_correct_varyings(a in clip_coord(), b in clip_coord(), c in clip_coord()) {
            let tri = [a, b, c].map(|p| ClipVertex { position: p, varyings: p });
            let out = clip_all(tri);
            assert_valid(&out);
            prop_assert!(out.len() <= 8, "7 planes leave at most 10 vertices, i.e. 8 fan triangles");
        }
    }
}

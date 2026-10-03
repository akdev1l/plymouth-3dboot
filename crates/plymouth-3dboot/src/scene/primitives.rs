// SPDX-License-Identifier: GPL-3.0-or-later
//! Procedural meshes for tests, examples and placeholders.
//!
//! All primitives are centred on the origin, with counter-clockwise
//! outward-facing triangles, unit normals and texture coordinates.

use super::Mesh;
use crate::math::{Vec2, Vec3};

/// An axis-aligned cube with the given half extent. It has 24 vertices
/// (four per face, so normals are flat) and 12 triangles.
#[must_use]
pub fn cube(half_extent: f32) -> Mesh {
    // (normal, u, v) with u × v = normal: corners in (s, t) order below are
    // counter-clockwise seen from outside.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    let mut positions = Vec::with_capacity(24);
    let mut normals = Vec::with_capacity(24);
    let mut uvs = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (n, u, v) in faces {
        let base = u32::try_from(positions.len()).expect("24 vertices");
        for (s, t) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push((n + u * s + v * t) * half_extent);
            normals.push(n);
            uvs.push(Vec2::new((s + 1.0) * 0.5, (t + 1.0) * 0.5));
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, indices)
        .and_then(|m| m.with_normals(normals))
        .and_then(|m| m.with_uvs(uvs))
        .expect("cube is valid")
}

/// A square in the XZ plane (y = 0) with the given half extent, facing +Y:
/// 4 vertices and 2 triangles, counter-clockwise seen from above.
#[must_use]
pub fn plane(half_extent: f32) -> Mesh {
    let corners = [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)];
    let positions = corners
        .iter()
        .map(|&(x, z)| Vec3::new(x, 0.0, z) * half_extent)
        .collect();
    let uvs = corners
        .iter()
        .map(|&(x, z)| Vec2::new((x + 1.0) * 0.5, (1.0 - z) * 0.5))
        .collect();
    Mesh::new(positions, vec![0, 1, 2, 0, 2, 3])
        .and_then(|m| m.with_normals(vec![Vec3::Y; 4]))
        .and_then(|m| m.with_uvs(uvs))
        .expect("plane is valid")
}

/// A UV sphere with `segments` (≥ 3) divisions around the Y axis and
/// `rings` (≥ 2) from pole to pole. Normals are radial.
///
/// # Panics
///
/// Panics if `segments < 3` or `rings < 2`.
#[must_use]
pub fn uv_sphere(radius: f32, segments: u32, rings: u32) -> Mesh {
    assert!(
        segments >= 3 && rings >= 2,
        "sphere needs at least 3 segments and 2 rings"
    );
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    // Vertex grid (segments + 1) × (rings + 1); the seam and poles are
    // duplicated so each vertex has unique UVs.
    #[allow(clippy::cast_precision_loss)]
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let theta = v * std::f32::consts::PI; // 0 at +Y pole
        // Exact poles: sin(π) is not exactly 0 in f32.
        let (sin_t, cos_t) = match r {
            0 => (0.0, 1.0),
            r if r == rings => (0.0, -1.0),
            _ => (libm::sinf(theta), libm::cosf(theta)),
        };
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            // The seam column repeats column 0's position exactly.
            let phi = if s == segments {
                0.0
            } else {
                u * std::f32::consts::TAU
            };
            let n = Vec3::new(sin_t * libm::sinf(phi), cos_t, sin_t * libm::cosf(phi));
            positions.push(n * radius);
            normals.push(n);
            uvs.push(Vec2::new(u, v));
        }
    }
    let row = segments + 1;
    let mut indices = Vec::new();
    for r in 0..rings {
        for s in 0..segments {
            let (a, b) = (r * row + s, r * row + s + 1);
            let (c, d) = (a + row, b + row);
            // Going down the rings and around +phi (towards +x from +z),
            // (a, c, d) and (a, d, b) are counter-clockwise from outside.
            if r != 0 {
                indices.extend([a, c, b]);
            }
            if r != rings - 1 {
                indices.extend([b, c, d]);
            }
        }
    }
    Mesh::new(positions, indices)
        .and_then(|m| m.with_normals(normals))
        .and_then(|m| m.with_uvs(uvs))
        .expect("sphere is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Aabb;

    /// Every triangle's geometric normal points away from the origin.
    fn assert_outward_ccw(mesh: &Mesh) {
        let p = mesh.positions();
        for [a, b, c] in mesh.triangles() {
            let (a, b, c) = (p[a as usize], p[b as usize], p[c as usize]);
            let n = (b - a).cross(c - a);
            assert!(n.length() > 0.0, "degenerate triangle {a} {b} {c}");
            assert!(
                n.dot((a + b + c) / 3.0) > 0.0,
                "inward-facing triangle {a} {b} {c}"
            );
        }
    }

    #[test]
    fn cube_is_closed_outward_and_flat() {
        let m = cube(0.5);
        assert_eq!((m.positions().len(), m.triangle_count()), (24, 12));
        assert_eq!(m.bounds(), Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
        assert_outward_ccw(&m);
        // Vertex normals agree with the geometric face normals.
        let (p, n) = (m.positions(), m.normals().unwrap());
        for [a, b, c] in m.triangles() {
            let face = (p[b as usize] - p[a as usize])
                .cross(p[c as usize] - p[a as usize])
                .normalize();
            for i in [a, b, c] {
                assert!(n[i as usize].abs_diff_eq(face, 1e-6));
            }
        }
    }

    #[test]
    fn sphere_poles_and_seam_are_bit_identical() {
        let m = uv_sphere(1.0, 8, 4);
        let p = m.positions();
        let row = 9;
        for s in 0..9 {
            assert_eq!(p[s], Vec3::Y, "north pole");
            assert_eq!(p[4 * row + s], Vec3::NEG_Y, "south pole");
        }
        for r in 0..5 {
            assert_eq!(p[r * row], p[r * row + 8], "seam at ring {r}");
        }
    }

    #[test]
    fn sphere_is_outward_with_radial_unit_normals() {
        let m = uv_sphere(2.0, 12, 8);
        assert_outward_ccw(&m);
        // Two triangles per quad, except one per quad in the pole rows.
        assert_eq!(m.triangle_count(), 12 * 8 * 2 - 2 * 12);
        for (p, n) in m.positions().iter().zip(m.normals().unwrap()) {
            assert!((p.length() - 2.0).abs() < 1e-5);
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!(n.abs_diff_eq(*p / 2.0, 1e-5));
        }
        assert!(
            m.bounds().size().abs_diff_eq(Vec3::splat(4.0), 0.1),
            "{:?}",
            m.bounds()
        );
    }

    #[test]
    fn plane_faces_up() {
        let m = plane(2.0);
        assert_eq!((m.positions().len(), m.triangle_count()), (4, 2));
        assert_eq!(
            m.bounds(),
            Aabb::new(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0))
        );
        for [a, b, c] in m.triangles() {
            let p = |i: u32| m.positions()[i as usize];
            let n = (p(b) - p(a)).cross(p(c) - p(a));
            assert!(n.y > 0.0, "counter-clockwise seen from above");
        }
    }
}

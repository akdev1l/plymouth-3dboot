// SPDX-License-Identifier: GPL-3.0-or-later
//! Vertex normal generation.
//!
//! Smooth normals are angle-weighted: each adjacent face contributes its
//! unit normal weighted by its interior angle at the vertex. That makes the
//! result independent of how polygons were split into triangles.

use std::collections::HashMap;

use super::Mesh;
use crate::math::{Vec2, Vec3};

/// Fallback normal for vertices that touch only degenerate triangles.
pub const FALLBACK_NORMAL: Vec3 = Vec3::Y;

/// Unnormalized normal of triangle `[a, b, c]` (counter-clockwise triangles
/// get normals pointing towards the viewer).
fn raw_face_normal(p: &[Vec3], [a, b, c]: [u32; 3]) -> Vec3 {
    let (a, b, c) = (p[a as usize], p[b as usize], p[c as usize]);
    (b - a).cross(c - a)
}

/// Hash key of a position; -0.0 and 0.0 map to the same key.
fn position_key(p: Vec3) -> [u32; 3] {
    (p + Vec3::ZERO).to_array().map(f32::to_bits)
}

fn normalize_or_fallback(n: Vec3) -> Vec3 {
    n.try_normalize().unwrap_or(FALLBACK_NORMAL)
}

/// Interior angle (radians) of triangle `t` at its corner `k`, computed
/// with `libm` so results are identical on every target.
fn corner_angle(p: &[Vec3], t: [u32; 3], k: usize) -> f32 {
    let at = p[t[k] as usize];
    let (e1, e2) = (
        p[t[(k + 1) % 3] as usize] - at,
        p[t[(k + 2) % 3] as usize] - at,
    );
    match (e1.try_normalize(), e2.try_normalize()) {
        (Some(a), Some(b)) => libm::acosf(a.dot(b).clamp(-1.0, 1.0)),
        _ => 0.0,
    }
}

/// Unit normal of each triangle, in triangle order (degenerate triangles
/// get [`FALLBACK_NORMAL`], +Y).
#[must_use]
pub fn face_normals(mesh: &Mesh) -> Vec<Vec3> {
    mesh.triangles()
        .map(|t| normalize_or_fallback(raw_face_normal(mesh.positions(), t)))
        .collect()
}

/// Rebuilds `mesh` from per-corner attributes: corner `i` (index position
/// `i` in the index buffer) gets the original vertex's position and UV and
/// `normals[i]`. Identical (vertex, normal) pairs are merged.
fn rebuild(mesh: &Mesh, corner_normals: &[Vec3]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs: Option<Vec<Vec2>> = mesh.uvs().map(|_| Vec::new());
    let mut indices = Vec::with_capacity(mesh.indices().len());
    let mut seen: HashMap<(u32, [u32; 3]), u32> = HashMap::new();
    for (&vertex, &normal) in mesh.indices().iter().zip(corner_normals) {
        let key = (vertex, normal.to_array().map(f32::to_bits));
        let index = *seen.entry(key).or_insert_with(|| {
            let new = u32::try_from(positions.len())
                .expect("at most one vertex per corner, below u32::MAX");
            positions.push(mesh.positions()[vertex as usize]);
            normals.push(normal);
            if let (Some(out), Some(src)) = (uvs.as_mut(), mesh.uvs()) {
                out.push(src[vertex as usize]);
            }
            new
        });
        indices.push(index);
    }
    let mut out = Mesh::new(positions, indices)
        .and_then(|m| m.with_normals(normals))
        .and_then(|m| m.with_submeshes(mesh.submeshes().to_vec()))
        .expect("rebuilt mesh preserves validity");
    if let Some(uvs) = uvs {
        out = out.with_uvs(uvs).expect("one UV per vertex");
    }
    out
}

/// Flat shading: every triangle gets its own face normal, splitting shared
/// vertices as needed. UVs and submeshes are preserved.
#[must_use]
pub fn with_flat_normals(mesh: &Mesh) -> Mesh {
    let corner: Vec<Vec3> = face_normals(mesh)
        .into_iter()
        .flat_map(|n| [n; 3])
        .collect();
    rebuild(mesh, &corner)
}

/// Smooth shading over shared vertex indices (angle-weighted). Topology is
/// unchanged.
#[must_use]
pub fn with_smooth_normals(mesh: &Mesh) -> Mesh {
    let p = mesh.positions();
    let mut sums = vec![Vec3::ZERO; p.len()];
    for t in mesh.triangles() {
        let n = normalize_or_fallback(raw_face_normal(p, t));
        for (k, v) in t.into_iter().enumerate() {
            sums[v as usize] += n * corner_angle(p, t, k);
        }
    }
    let normals = sums.into_iter().map(normalize_or_fallback).collect();
    mesh.clone()
        .with_normals(normals)
        .expect("one normal per vertex")
}

/// Smooth shading with hard edges: at each corner, average (angle-weighted)
/// the normals of all faces that share the corner's *position* (not just
/// its vertex index) and whose face normals are within `crease_angle`
/// (radians) of this face's normal. Vertices are split where normals differ.
///
/// This removes seams in meshes that duplicate vertices (e.g. for UVs) and
/// keeps edges sharper than the crease angle hard.
#[must_use]
pub fn with_crease_normals(mesh: &Mesh, crease_angle: f32) -> Mesh {
    let p = mesh.positions();
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    let unit: Vec<Vec3> = tris
        .iter()
        .map(|&t| normalize_or_fallback(raw_face_normal(p, t)))
        .collect();
    // (triangle, angle-weighted normal) of every corner at each position.
    let mut by_position: HashMap<[u32; 3], Vec<(usize, Vec3)>> = HashMap::new();
    for (ti, &t) in tris.iter().enumerate() {
        for (k, &v) in t.iter().enumerate() {
            let key = position_key(p[v as usize]);
            by_position
                .entry(key)
                .or_default()
                .push((ti, unit[ti] * corner_angle(p, t, k)));
        }
    }
    let cos_limit = libm::cosf(crease_angle);
    let mut corner = Vec::with_capacity(tris.len() * 3);
    for (ti, t) in tris.iter().enumerate() {
        for &v in t {
            let key = position_key(p[v as usize]);
            let sum: Vec3 = by_position[&key]
                .iter()
                .filter(|(tj, _)| unit[ti].dot(unit[*tj]) >= cos_limit)
                .map(|(_, n)| *n)
                .sum();
            corner.push(normalize_or_fallback(sum));
        }
    }
    rebuild(mesh, &corner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::primitives::{cube, uv_sphere};
    use crate::scene::{MaterialId, Submesh};

    /// A ±1 cube with 8 shared vertices and 12 outward CCW triangles.
    fn shared_cube() -> Mesh {
        let p: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                )
            })
            .collect();
        #[rustfmt::skip]
        let idx = vec![
            1, 3, 7, 1, 7, 5, // +X
            0, 4, 6, 0, 6, 2, // -X
            2, 6, 7, 2, 7, 3, // +Y
            0, 1, 5, 0, 5, 4, // -Y
            4, 5, 7, 4, 7, 6, // +Z
            0, 2, 3, 0, 3, 1, // -Z
        ];
        let m = Mesh::new(p, idx).unwrap();
        // Sanity: outward winding.
        for (n, t) in face_normals(&m).iter().zip(m.triangles()) {
            let c: Vec3 = t.iter().map(|&v| m.positions()[v as usize]).sum::<Vec3>() / 3.0;
            assert!(n.dot(c) > 0.0);
        }
        m
    }

    fn distinct(normals: &[Vec3]) -> usize {
        let mut v: Vec<[u32; 3]> = normals
            .iter()
            .map(|n| n.to_array().map(f32::to_bits))
            .collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    #[test]
    fn flat_normals_split_cube_into_six_faces() {
        let m = with_flat_normals(&shared_cube());
        let n = m.normals().unwrap();
        assert_eq!(m.positions().len(), 24, "4 vertices per face after merging");
        assert_eq!(distinct(n), 6);
        assert!(n.iter().all(|v| (v.length() - 1.0).abs() < 1e-6));
        assert_eq!(m.triangle_count(), 12);
        // Each corner's normal is its face's normal.
        for (f, t) in face_normals(&m).iter().zip(m.triangles()) {
            for v in t {
                assert_eq!(n[v as usize], *f);
            }
        }
    }

    #[test]
    fn smooth_normals_on_shared_cube_point_along_diagonals() {
        let m = with_smooth_normals(&shared_cube());
        for (p, n) in m.positions().iter().zip(m.normals().unwrap()) {
            assert!(n.abs_diff_eq(p.normalize(), 1e-6), "{n} at {p}");
        }
        assert_eq!(m.positions().len(), 8);
    }

    #[test]
    fn crease_angle_selects_between_flat_and_smooth() {
        let hard = with_crease_normals(&shared_cube(), 30f32.to_radians());
        assert_eq!(hard.positions().len(), 24);
        assert_eq!(distinct(hard.normals().unwrap()), 6);
        let soft = with_crease_normals(&shared_cube(), 100f32.to_radians());
        assert_eq!(soft.positions().len(), 8);
        for (p, n) in soft.positions().iter().zip(soft.normals().unwrap()) {
            assert!(n.abs_diff_eq(p.normalize(), 1e-6));
        }
    }

    #[test]
    fn crease_normals_close_sphere_seams_and_poles() {
        // The UV sphere duplicates seam and pole vertices; index-based
        // smoothing cannot reach across them, position-based smoothing can.
        let sphere = uv_sphere(1.0, 16, 12);
        let m = with_crease_normals(&sphere, 60f32.to_radians());
        for (p, n) in m.positions().iter().zip(m.normals().unwrap()) {
            assert!(n.dot(p.normalize()) > 0.995, "normal {n} at {p}");
        }
        assert_eq!(m.uvs().map(<[Vec2]>::len), Some(m.positions().len()));
    }

    #[test]
    fn uvs_and_submeshes_are_preserved() {
        let c = cube(1.0)
            .with_submeshes(vec![
                Submesh {
                    material: MaterialId(0),
                    indices: 0..18,
                },
                Submesh {
                    material: MaterialId(1),
                    indices: 18..36,
                },
            ])
            .unwrap();
        for m in [with_flat_normals(&c), with_crease_normals(&c, 0.5)] {
            assert_eq!(m.submeshes(), c.submeshes());
            assert_eq!(m.uvs().unwrap().len(), m.positions().len());
            // Every corner keeps its original position and UV.
            for (new, old) in m.indices().iter().zip(c.indices()) {
                assert_eq!(m.positions()[*new as usize], c.positions()[*old as usize]);
                assert_eq!(
                    m.uvs().unwrap()[*new as usize],
                    c.uvs().unwrap()[*old as usize]
                );
            }
        }
    }

    #[test]
    fn degenerate_triangles_get_fallback_normal() {
        let m = Mesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0], vec![0, 1, 2]).unwrap();
        assert_eq!(face_normals(&m), vec![FALLBACK_NORMAL]);
        assert!(
            with_smooth_normals(&m)
                .normals()
                .unwrap()
                .iter()
                .all(|&n| n == FALLBACK_NORMAL)
        );
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Triangulation of planar polygons (as found in OBJ files).

use crate::math::Vec3;

/// Splits a simple polygon into triangles.
///
/// `points` are the polygon's vertices in order (in 3D, roughly planar).
/// The result lists triangles as indices into `points`, with the same
/// winding as the polygon. Convex polygons become a fan from vertex 0.
/// Concave polygons are ear-clipped. Fewer than three points give no
/// triangles.
///
/// Degenerate or self-intersecting input never panics: if ear clipping gets
/// stuck, the remainder is fanned.
#[must_use]
pub fn triangulate_polygon(points: &[Vec3]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let pts = project_to_plane(points);
    // Orientation of the polygon in its 2D projection.
    let area2: f64 = (0..n).map(|i| cross(pts[i], pts[(i + 1) % n])).sum();
    let sign = if area2 < 0.0 { -1.0 } else { 1.0 };
    let convex = (0..n).all(|i| sign * turn(pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]) >= 0.0);
    if convex {
        return (1..n - 1).map(|i| [0, i, i + 1]).collect();
    }
    ear_clip(&pts, sign)
}

type P2 = (f64, f64);

fn cross(a: P2, b: P2) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

/// Twice the signed area of triangle (a, b, c): positive for a left turn.
fn turn(a: P2, b: P2, c: P2) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Projects onto the coordinate plane most parallel to the polygon, chosen
/// from its Newell normal.
fn project_to_plane(points: &[Vec3]) -> Vec<P2> {
    let n = points.len();
    let mut normal = [0.0f64; 3];
    for i in 0..n {
        let (a, b) = (points[i].as_ref(), points[(i + 1) % n].as_ref());
        let (a, b) = (a.map(f64::from), b.map(f64::from));
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let axis = (0..3)
        .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
        .unwrap_or(2);
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    points
        .iter()
        .map(|p| (f64::from(p[u]), f64::from(p[v])))
        .collect()
}

fn ear_clip(pts: &[P2], sign: f64) -> Vec<[usize; 3]> {
    let mut remaining: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity(pts.len() - 2);
    while remaining.len() > 3 {
        let m = remaining.len();
        let ear = (0..m).find(|&k| {
            let (a, b, c) = (
                remaining[(k + m - 1) % m],
                remaining[k],
                remaining[(k + 1) % m],
            );
            if sign * turn(pts[a], pts[b], pts[c]) <= 0.0 {
                return false; // reflex or degenerate corner
            }
            // No other remaining vertex may lie inside or on the ear.
            remaining.iter().all(|&q| {
                q == a || q == b || q == c || {
                    let p = pts[q];
                    !(sign * turn(pts[a], pts[b], p) >= 0.0
                        && sign * turn(pts[b], pts[c], p) >= 0.0
                        && sign * turn(pts[c], pts[a], p) >= 0.0)
                }
            })
        });
        let Some(k) = ear else {
            // Degenerate or self-intersecting polygon: fan the rest.
            out.extend((1..m - 1).map(|i| [remaining[0], remaining[i], remaining[i + 1]]));
            return out;
        };
        out.push([
            remaining[(k + m - 1) % m],
            remaining[k],
            remaining[(k + 1) % m],
        ]);
        remaining.remove(k);
    }
    out.push([remaining[0], remaining[1], remaining[2]]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Quat, Vec2};
    use proptest::prelude::*;

    fn flat(points: &[(f32, f32)]) -> Vec<Vec3> {
        points.iter().map(|&(x, y)| Vec3::new(x, y, 0.0)).collect()
    }

    /// Signed area of a polygon in the XY plane.
    fn polygon_area(p: &[Vec3]) -> f32 {
        (0..p.len())
            .map(|i| p[i].truncate().perp_dot(p[(i + 1) % p.len()].truncate()))
            .sum::<f32>()
            / 2.0
    }

    fn triangle_area(p: &[Vec3], [a, b, c]: [usize; 3]) -> f32 {
        (p[b] - p[a]).truncate().perp_dot((p[c] - p[a]).truncate()) / 2.0
    }

    /// Area is preserved and every triangle has the polygon's orientation.
    fn check(p: &[Vec3]) {
        let tris = triangulate_polygon(p);
        assert_eq!(tris.len(), p.len() - 2);
        let total = polygon_area(p);
        let mut sum = 0.0;
        for &t in &tris {
            let a = triangle_area(p, t);
            assert!(
                a * total.signum() >= -1e-4,
                "inverted triangle {t:?} (area {a}) in {p:?}"
            );
            sum += a;
        }
        assert!(
            (sum - total).abs() <= 1e-3 * total.abs().max(1.0),
            "area {sum} != {total}"
        );
    }

    #[test]
    fn small_inputs() {
        assert!(triangulate_polygon(&flat(&[(0.0, 0.0), (1.0, 0.0)])).is_empty());
        assert_eq!(
            triangulate_polygon(&flat(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)])),
            vec![[0, 1, 2]]
        );
    }

    #[test]
    fn convex_polygons_become_fans() {
        let pentagon = flat(&[(0.0, 0.0), (2.0, 0.0), (3.0, 1.5), (1.0, 3.0), (-1.0, 1.5)]);
        assert_eq!(
            triangulate_polygon(&pentagon),
            vec![[0, 1, 2], [0, 2, 3], [0, 3, 4]]
        );
        check(&pentagon);
        // Clockwise input keeps clockwise triangles.
        let mut cw = pentagon;
        cw.reverse();
        check(&cw);
    }

    #[test]
    fn concave_polygons_are_ear_clipped() {
        // An L shape and an arrow, in both orientations.
        let l = flat(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 3.0),
            (0.0, 3.0),
        ]);
        let arrow = flat(&[(0.0, 0.0), (4.0, 2.0), (0.0, 4.0), (1.0, 2.0)]);
        for mut p in [l, arrow] {
            check(&p);
            p.reverse();
            check(&p);
        }
    }

    #[test]
    fn concave_quad_starting_at_its_reflex_vertex() {
        // Vertex 0 is reflex. The only valid split is the diagonal from it.
        let p = flat(&[(1.0, 2.0), (0.0, 0.0), (4.0, 2.0), (0.0, 4.0)]);
        check(&p);
    }

    #[test]
    fn polygons_in_any_plane() {
        let l = flat(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 3.0),
            (0.0, 3.0),
        ]);
        for axis in [Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.3).normalize()] {
            let rotated: Vec<Vec3> = l
                .iter()
                .map(|&q| Quat::from_axis_angle(axis, 1.1) * q)
                .collect();
            let tris = triangulate_polygon(&rotated);
            // Same triangles as in the plane: rotation preserves the structure.
            let flat_area: f32 = triangulate_polygon(&l)
                .iter()
                .map(|&t| triangle_area(&l, t))
                .sum();
            let area3d: f32 = tris
                .iter()
                .map(|&[a, b, c]| {
                    (rotated[b] - rotated[a])
                        .cross(rotated[c] - rotated[a])
                        .length()
                        / 2.0
                })
                .sum();
            assert!(
                (area3d - flat_area.abs()).abs() < 1e-4,
                "{area3d} vs {flat_area}"
            );
        }
    }

    #[test]
    fn degenerate_polygons_do_not_panic() {
        let collinear = flat(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]);
        assert_eq!(triangulate_polygon(&collinear).len(), 2);
        let bowtie = flat(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]);
        assert_eq!(triangulate_polygon(&bowtie).len(), 2);
        let repeated = flat(&[(0.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
        assert_eq!(triangulate_polygon(&repeated).len(), 2);
    }

    proptest! {
        /// Star-shaped polygons (sorted angles, random radii) are simple but
        /// often concave.
        #[test]
        fn star_polygons_preserve_area(radii in proptest::collection::vec(0.3f32..3.0, 3..14), reverse in any::<bool>()) {
            let n = radii.len();
            let mut p: Vec<Vec3> = radii
                .iter()
                .enumerate()
                .map(|(i, &r)| {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    (Vec2::new(libm::cosf(a), libm::sinf(a)) * r).extend(0.0)
                })
                .collect();
            if reverse {
                p.reverse();
            }
            check(&p);
        }
    }
}

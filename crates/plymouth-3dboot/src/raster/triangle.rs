// SPDX-License-Identifier: GPL-3.0-or-later
//! Triangle setup and pixel coverage.

use super::edge::EdgeFunction;
use super::fixed::{FixedPoint, SUBPIXEL_SCALE};
use crate::math::Vec2;

/// A rectangle of pixels `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    /// First column.
    pub x0: u32,
    /// First row.
    pub y0: u32,
    /// One past the last column.
    pub x1: u32,
    /// One past the last row.
    pub y1: u32,
}

impl Rect {
    /// The rectangle `[0, width) × [0, height)`.
    #[must_use]
    pub const fn from_size(width: u32, height: u32) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        }
    }

    /// Whether the rectangle contains no pixels.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// The overlap of two rectangles (possibly empty).
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }
}

/// The winding of a triangle's vertices as seen on screen (equivalently, in
/// NDC with +y up).
///
/// Window coordinates have +y down, so a triangle that winds
/// counter-clockwise on screen has a *negative* signed area
/// `(v1 - v0) × (v2 - v0)` when computed from window coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Winding {
    /// Counter-clockwise as seen by the viewer: front-facing by convention.
    CounterClockwise,
    /// Clockwise as seen by the viewer: back-facing by convention.
    Clockwise,
}

/// Which triangles to discard based on their [`Winding`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CullMode {
    /// Keep every triangle.
    #[default]
    None,
    /// Discard back faces (clockwise as seen by the viewer).
    Back,
    /// Discard front faces (counter-clockwise as seen by the viewer).
    Front,
}

impl CullMode {
    /// Whether a triangle with `winding` is discarded.
    #[must_use]
    pub fn culls(self, winding: Winding) -> bool {
        matches!(
            (self, winding),
            (Self::Back, Winding::Clockwise) | (Self::Front, Winding::CounterClockwise)
        )
    }
}

/// A covered pixel, as passed to [`TriangleSetup::for_each_pixel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Column.
    pub x: u32,
    /// Row.
    pub y: u32,
    /// Edge-function values at the pixel centre, indexed by the *opposite*
    /// vertex in the caller's vertex order. They are non-negative and sum
    /// to [`Fragment::double_area`].
    pub edge_values: [i64; 3],
    /// Twice the triangle's area in subpixel units squared (> 0).
    pub double_area: i64,
}

impl Fragment {
    /// Screen-space barycentric weights of the pixel centre, in the caller's
    /// vertex order. Each is in `[0, 1]` and together they sum to 1 (within
    /// rounding).
    #[must_use]
    pub fn barycentric(&self) -> [f32; 3] {
        // Edge values are below 2^52, so the f64 conversions are exact.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let w = self
            .edge_values
            .map(|e| (e as f64 / self.double_area as f64) as f32);
        w
    }

    /// Interpolates per-vertex `values` linearly in screen space.
    #[must_use]
    pub fn interpolate<T: super::Interpolate>(&self, values: [T; 3]) -> T {
        T::interpolate(values, self.barycentric())
    }
}

/// A triangle prepared for rasterization: snapped vertices, edge functions
/// and pixel bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleSetup {
    /// Edge functions in positive orientation; `edges[i]` is the edge
    /// opposite internal vertex `i`.
    edges: [EdgeFunction; 3],
    /// `order[i]` is the caller's index of internal vertex `i`.
    order: [usize; 3],
    /// Twice the triangle area in subpixel units squared (> 0).
    double_area: i64,
    /// Winding of the caller's vertex order, as seen by the viewer.
    winding: Winding,
    /// Pixel bounds `[min, max]` inclusive, possibly negative.
    min: (i64, i64),
    max: (i64, i64),
}

fn ceil_div(a: i64, b: i64) -> i64 {
    -((-a).div_euclid(b))
}

impl TriangleSetup {
    /// Prepares a triangle given in window coordinates.
    ///
    /// Either winding is accepted. Returns `None` for degenerate (zero-area
    /// after snapping) triangles and for vertices that are not finite or lie
    /// outside the guard band ([`super::fixed::MAX_COORD`]).
    #[must_use]
    pub fn new(vertices: [Vec2; 3]) -> Option<Self> {
        let p = [
            FixedPoint::from_window(vertices[0])?,
            FixedPoint::from_window(vertices[1])?,
            FixedPoint::from_window(vertices[2])?,
        ];
        let signed = EdgeFunction::new(p[0], p[1]).eval(p[2]);
        // A positive signed area in window space (+y down) is clockwise as
        // seen on screen.
        let (order, winding) = match signed.signum() {
            1 => ([0, 1, 2], Winding::Clockwise),
            -1 => ([0, 2, 1], Winding::CounterClockwise),
            _ => return None,
        };
        let v = order.map(|i| p[i]);
        let edges = [
            EdgeFunction::new(v[1], v[2]),
            EdgeFunction::new(v[2], v[0]),
            EdgeFunction::new(v[0], v[1]),
        ];
        // Pixel (x, y) is a candidate if its centre x + 0.5 lies within the
        // vertex extent: ceil((min - 0.5)) <= x <= floor((max - 0.5)).
        let half = SUBPIXEL_SCALE / 2;
        let (min_x, max_x) = (v.iter().map(|q| q.x).min()?, v.iter().map(|q| q.x).max()?);
        let (min_y, max_y) = (v.iter().map(|q| q.y).min()?, v.iter().map(|q| q.y).max()?);
        Some(Self {
            edges,
            order,
            double_area: signed.abs(),
            winding,
            min: (
                ceil_div(min_x - half, SUBPIXEL_SCALE),
                ceil_div(min_y - half, SUBPIXEL_SCALE),
            ),
            max: (
                (max_x - half).div_euclid(SUBPIXEL_SCALE),
                (max_y - half).div_euclid(SUBPIXEL_SCALE),
            ),
        })
    }

    /// Like [`TriangleSetup::new`], but also returns `None` if `cull`
    /// discards the triangle.
    #[must_use]
    pub fn new_culled(vertices: [Vec2; 3], cull: CullMode) -> Option<Self> {
        Self::new(vertices).filter(|t| !cull.culls(t.winding))
    }

    /// Winding of the vertices (in the order given), as seen by the viewer.
    #[must_use]
    pub fn winding(&self) -> Winding {
        self.winding
    }

    /// Twice the triangle's area, in subpixel units squared (always > 0).
    #[must_use]
    pub fn double_area(&self) -> i64 {
        self.double_area
    }

    /// Calls `f` once for every pixel inside `scissor` whose centre the
    /// triangle covers under the top-left fill rule, row by row.
    pub fn for_each_pixel(&self, scissor: Rect, mut f: impl FnMut(Fragment)) {
        // Intersect the candidate pixel range with the scissor in i64; an
        // empty or inverted scissor yields an empty range.
        let x0 = self.min.0.max(i64::from(scissor.x0));
        let y0 = self.min.1.max(i64::from(scissor.y0));
        let x1 = (self.max.0 + 1).min(i64::from(scissor.x1));
        let y1 = (self.max.1 + 1).min(i64::from(scissor.y1));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        // Within the scissor, hence non-negative and within u32.
        let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (
            u32::try_from(x0),
            u32::try_from(y0),
            u32::try_from(x1),
            u32::try_from(y1),
        ) else {
            return;
        };
        let start = FixedPoint::pixel_center(i64::from(x0), i64::from(y0));
        let mut row = self.edges.map(|e| e.eval(start));
        let step_x = self.edges.map(|e| e.step_x());
        let step_y = self.edges.map(|e| e.step_y());
        let bias = self.edges.map(|e| e.bias());
        for y in y0..y1 {
            let mut w = row;
            for x in x0..x1 {
                if w[0] + bias[0] > 0 && w[1] + bias[1] > 0 && w[2] + bias[2] > 0 {
                    let mut edge_values = [0; 3];
                    for (i, &value) in w.iter().enumerate() {
                        edge_values[self.order[i]] = value;
                    }
                    f(Fragment {
                        x,
                        y,
                        edge_values,
                        double_area: self.double_area,
                    });
                }
                for i in 0..3 {
                    w[i] += step_x[i];
                }
            }
            for i in 0..3 {
                row[i] += step_y[i];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::{BTreeMap, BTreeSet};

    const SCREEN: Rect = Rect::from_size(32, 32);

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    fn pixels(tri: [Vec2; 3], scissor: Rect) -> BTreeSet<(u32, u32)> {
        let mut out = BTreeSet::new();
        if let Some(t) = TriangleSetup::new(tri) {
            t.for_each_pixel(scissor, |f| {
                assert!(out.insert((f.x, f.y)), "pixel visited twice")
            });
        }
        out
    }

    fn set(points: &[(u32, u32)]) -> BTreeSet<(u32, u32)> {
        points.iter().copied().collect()
    }

    #[test]
    fn small_right_triangle_covers_exact_pixels() {
        // Centres (x+.5, y+.5) with x + y + 1 < 4; the diagonal x + y = 4 is a
        // bottom-right edge, so centres on it are excluded.
        let got = pixels([v(0.0, 0.0), v(4.0, 0.0), v(0.0, 4.0)], SCREEN);
        assert_eq!(got, set(&[(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2)]));
        // Winding does not matter.
        assert_eq!(pixels([v(0.0, 0.0), v(0.0, 4.0), v(4.0, 0.0)], SCREEN), got);
    }

    #[test]
    fn top_edge_through_centres_is_included_bottom_edge_excluded() {
        // A 4x2 rectangle whose top and bottom edges pass through pixel centres
        // (y = 0.5 and y = 2.5), split into two triangles.
        let (a, b, c, d) = (v(0.0, 0.5), v(4.0, 0.5), v(4.0, 2.5), v(0.0, 2.5));
        let mut both = pixels([a, b, c], SCREEN);
        let second = pixels([a, c, d], SCREEN);
        assert!(both.is_disjoint(&second));
        both.extend(second);
        let expected: BTreeSet<_> = (0..4).flat_map(|x| [(x, 0), (x, 1)]).collect();
        assert_eq!(
            both, expected,
            "rows 0 and 1 (top edge row included, bottom edge row 2 excluded)"
        );
    }

    #[test]
    fn left_edge_through_centres_is_included_right_edge_excluded() {
        let (a, b, c, d) = (v(0.5, 0.0), v(2.5, 0.0), v(2.5, 4.0), v(0.5, 4.0));
        let mut both = pixels([a, b, c], SCREEN);
        both.extend(pixels([a, c, d], SCREEN));
        let expected: BTreeSet<_> = (0..4).flat_map(|y| [(0, y), (1, y)]).collect();
        assert_eq!(both, expected);
    }

    #[test]
    fn winding_is_reported_as_seen_on_screen() {
        // On screen (+y down) this goes right along the top, then to the
        // bottom-left: clockwise as seen.
        let cw = [v(0.0, 0.0), v(10.0, 0.0), v(0.0, 10.0)];
        let ccw = [cw[0], cw[2], cw[1]];
        assert_eq!(
            TriangleSetup::new(cw).unwrap().winding(),
            Winding::Clockwise
        );
        assert_eq!(
            TriangleSetup::new(ccw).unwrap().winding(),
            Winding::CounterClockwise
        );
        // Rotating the vertex order keeps the winding.
        assert_eq!(
            TriangleSetup::new([cw[1], cw[2], cw[0]]).unwrap().winding(),
            Winding::Clockwise
        );
    }

    #[test]
    fn ndc_counter_clockwise_is_counter_clockwise_on_screen() {
        use crate::math::{Vec3, Viewport};
        // Counter-clockwise in NDC (+y up): bottom-left, bottom-right, top-right.
        let vp = Viewport::new(10, 10);
        let ndc = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        let window = ndc.map(|p| vp.ndc_to_window(p).truncate());
        assert_eq!(
            TriangleSetup::new(window).unwrap().winding(),
            Winding::CounterClockwise
        );
    }

    #[test]
    fn cull_modes() {
        use CullMode::{Back, Front, None};
        use Winding::{Clockwise, CounterClockwise};
        assert!(!None.culls(Clockwise) && !None.culls(CounterClockwise));
        assert!(Back.culls(Clockwise) && !Back.culls(CounterClockwise));
        assert!(Front.culls(CounterClockwise) && !Front.culls(Clockwise));
        assert_eq!(CullMode::default(), None);

        let cw = [v(0.0, 0.0), v(10.0, 0.0), v(0.0, 10.0)];
        let ccw = [cw[0], cw[2], cw[1]];
        assert!(TriangleSetup::new_culled(ccw, Back).is_some());
        assert!(TriangleSetup::new_culled(cw, Back).is_none());
        assert!(TriangleSetup::new_culled(ccw, Front).is_none());
        assert!(TriangleSetup::new_culled(cw, Front).is_some());
        assert!(TriangleSetup::new_culled(cw, None).is_some());
    }

    #[test]
    fn degenerate_triangles_are_rejected() {
        assert!(TriangleSetup::new([v(1.0, 1.0), v(5.0, 5.0), v(9.0, 9.0)]).is_none());
        assert!(TriangleSetup::new([v(1.0, 1.0), v(1.0, 1.0), v(9.0, 2.0)]).is_none());
        // Collapses to zero area after snapping.
        assert!(TriangleSetup::new([v(1.0, 1.0), v(1.0001, 1.0), v(1.0, 1.0001)]).is_none());
        assert!(TriangleSetup::new([v(f32::NAN, 1.0), v(2.0, 1.0), v(1.0, 2.0)]).is_none());
    }

    #[test]
    fn tiny_triangle_between_centres_covers_nothing() {
        assert!(pixels([v(0.6, 0.6), v(0.9, 0.6), v(0.6, 0.9)], SCREEN).is_empty());
    }

    #[test]
    fn offscreen_triangles_cover_nothing() {
        assert!(pixels([v(-10.0, 0.0), v(-1.0, 0.0), v(-10.0, 10.0)], SCREEN).is_empty());
        assert!(pixels([v(40.0, 0.0), v(50.0, 0.0), v(40.0, 10.0)], SCREEN).is_empty());
        assert!(pixels([v(0.0, -20.0), v(10.0, -20.0), v(0.0, -1.0)], SCREEN).is_empty());
        assert!(
            pixels(
                [v(0.0, 0.0), v(10.0, 0.0), v(0.0, 10.0)],
                Rect {
                    x0: 5,
                    y0: 5,
                    x1: 5,
                    y1: 9
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn empty_or_inverted_scissor_covers_nothing() {
        let tri = [v(0.0, 0.0), v(30.0, 0.0), v(0.0, 30.0)];
        assert!(
            pixels(
                tri,
                Rect {
                    x0: 10,
                    y0: 0,
                    x1: 5,
                    y1: 10
                }
            )
            .is_empty()
        );
        assert!(
            pixels(
                tri,
                Rect {
                    x0: 0,
                    y0: 9,
                    x1: 10,
                    y1: 2
                }
            )
            .is_empty()
        );
        // Disjoint rectangles intersect to an inverted, empty rectangle.
        let disjoint = Rect::from_size(4, 4).intersect(&Rect {
            x0: 8,
            y0: 8,
            x1: 12,
            y1: 12,
        });
        assert!(disjoint.is_empty());
        assert!(pixels(tri, disjoint).is_empty());
    }

    #[test]
    fn scissor_clips_coverage() {
        let tri = [v(-20.0, -20.0), v(60.0, 0.0), v(0.0, 60.0)];
        let all = pixels(tri, Rect::from_size(64, 64));
        let clip = Rect {
            x0: 3,
            y0: 4,
            x1: 17,
            y1: 9,
        };
        let clipped = pixels(tri, clip);
        let expected: BTreeSet<_> = all
            .iter()
            .copied()
            .filter(|&(x, y)| (3..17).contains(&x) && (4..9).contains(&y))
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(clipped, expected);
    }

    #[test]
    fn edge_values_are_nonnegative_and_sum_to_double_area() {
        let tri = [v(1.3, 2.1), v(20.7, 5.2), v(6.4, 18.9)];
        let t = TriangleSetup::new(tri).unwrap();
        let mut n = 0;
        t.for_each_pixel(SCREEN, |f| {
            assert!(f.edge_values.iter().all(|&e| e >= 0), "{f:?}");
            assert_eq!(f.edge_values.iter().sum::<i64>(), t.double_area());
            n += 1;
        });
        assert!(n > 50);
    }

    #[test]
    fn edge_values_follow_caller_vertex_order() {
        // edge_values[i] is largest at pixels next to vertex i, for either winding.
        let (p, q, r) = (v(1.0, 1.0), v(30.0, 1.0), v(1.0, 30.0));
        let near = [((1, 1), p), ((27, 1), q), ((1, 27), r)];
        for tri in [[p, q, r], [p, r, q], [r, q, p]] {
            let t = TriangleSetup::new(tri).unwrap();
            let mut seen = BTreeMap::new();
            t.for_each_pixel(SCREEN, |f| {
                seen.insert((f.x, f.y), f.edge_values);
            });
            for (pixel, vertex) in near {
                let w = seen[&pixel];
                let largest = (0..3).max_by_key(|&k| w[k]).unwrap();
                assert_eq!(tri[largest], vertex, "pixel {pixel:?} in {tri:?}: {w:?}");
            }
        }
    }

    #[test]
    fn barycentric_is_one_hot_at_a_covered_vertex() {
        // Vertex 0 sits exactly on the centre of pixel (0, 0), on the top and
        // left edges, so that pixel is covered with weights (1, 0, 0).
        let tri = [v(0.5, 0.5), v(10.5, 0.5), v(0.5, 10.5)];
        let mut found = None;
        TriangleSetup::new(tri)
            .unwrap()
            .for_each_pixel(SCREEN, |f| {
                if (f.x, f.y) == (0, 0) {
                    found = Some(f.barycentric());
                }
            });
        assert_eq!(found, Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn barycentric_is_a_third_at_the_centroid() {
        // Centroid (2.5, 2.5) is the centre of pixel (2, 2).
        let tri = [v(0.5, 0.5), v(6.5, 0.5), v(0.5, 6.5)];
        let mut found = None;
        TriangleSetup::new(tri)
            .unwrap()
            .for_each_pixel(SCREEN, |f| {
                if (f.x, f.y) == (2, 2) {
                    found = Some(f.barycentric());
                }
            });
        let w = found.expect("centroid pixel covered");
        assert!(w.iter().all(|&x| (x - 1.0 / 3.0).abs() < 1e-6), "{w:?}");
    }

    /// Points on a quarter-pixel grid, so that edges often pass exactly
    /// through pixel centres and the fill rule is exercised.
    fn grid_point() -> impl Strategy<Value = Vec2> {
        (0i32..=96, 0i32..=96).prop_map(|(x, y)| Vec2::new(x as f32 * 0.25, y as f32 * 0.25))
    }

    /// Pixel centres strictly inside a convex polygon (positive orientation).
    fn strictly_inside(poly: &[Vec2], x: u32, y: u32) -> bool {
        let p = FixedPoint::pixel_center(i64::from(x), i64::from(y));
        let q: Vec<_> = poly
            .iter()
            .map(|&v| FixedPoint::from_window(v).unwrap())
            .collect();
        (0..q.len()).all(|i| EdgeFunction::new(q[i], q[(i + 1) % q.len()]).eval(p) > 0)
    }

    /// Pixel centres inside or on the boundary of a convex polygon
    /// (positive orientation): nothing outside may ever be covered.
    fn inside_or_on(poly: &[Vec2], x: u32, y: u32) -> bool {
        let p = FixedPoint::pixel_center(i64::from(x), i64::from(y));
        let q: Vec<_> = poly
            .iter()
            .map(|&v| FixedPoint::from_window(v).unwrap())
            .collect();
        (0..q.len()).all(|i| EdgeFunction::new(q[i], q[(i + 1) % q.len()]).eval(p) >= 0)
    }

    fn coverage_counts(tris: &[[Vec2; 3]]) -> BTreeMap<(u32, u32), u32> {
        let mut counts = BTreeMap::new();
        for &tri in tris {
            if let Some(t) = TriangleSetup::new(tri) {
                t.for_each_pixel(SCREEN, |f| *counts.entry((f.x, f.y)).or_insert(0) += 1);
            }
        }
        counts
    }

    fn orientation(a: Vec2, b: Vec2, c: Vec2) -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }

    fn snap_quarter(p: Vec2) -> Vec2 {
        (p * 4.0).round() / 4.0
    }

    /// `n` points at increasing angles on an ellipse, snapped to the quarter
    /// grid, plus the ellipse centre. Points on an ellipse are in convex
    /// position. The centre is not always inside their hull (a gap can exceed
    /// π); tests that need that filter on orientation.
    fn ellipse_polygon(n: usize) -> impl Strategy<Value = (Vec2, Vec<Vec2>)> {
        (
            8.0f32..24.0,
            8.0f32..24.0,
            3.0f32..8.0,
            3.0f32..8.0,
            0.0f32..6.3,
            proptest::collection::vec(0.2f32..1.0, n),
        )
            .prop_map(|(cx, cy, rx, ry, phase, gaps)| {
                let total: f32 = gaps.iter().sum();
                let mut angle = phase;
                let mut points = Vec::with_capacity(gaps.len());
                for g in &gaps {
                    points.push(snap_quarter(Vec2::new(
                        cx + rx * libm::cosf(angle),
                        cy + ry * libm::sinf(angle),
                    )));
                    angle += g / total * std::f32::consts::TAU;
                }
                (snap_quarter(Vec2::new(cx, cy)), points)
            })
    }

    fn convex_and_consistent(poly: &[Vec2]) -> bool {
        let n = poly.len();
        let o: Vec<f32> = (0..n)
            .map(|i| orientation(poly[i], poly[(i + 1) % n], poly[(i + 2) % n]))
            .collect();
        o.iter().all(|&x| x > 0.0) || o.iter().all(|&x| x < 0.0)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// A convex quad split along either diagonal covers the same pixels,
        /// each exactly once, including every centre strictly inside it.
        #[test]
        fn quad_split_is_watertight_without_double_coverage((_, quad) in ellipse_polygon(4)) {
            // Snapping can rarely destroy strict convexity.
            prop_assume!(convex_and_consistent(&quad));
            let [a, b, c, d] = [quad[0], quad[1], quad[2], quad[3]];
            let split1 = coverage_counts(&[[a, b, c], [a, c, d]]);
            let split2 = coverage_counts(&[[a, b, d], [b, c, d]]);
            prop_assert!(split1.values().all(|&n| n == 1), "double coverage {split1:?}");
            prop_assert!(split2.values().all(|&n| n == 1), "double coverage {split2:?}");
            prop_assert_eq!(split1.keys().collect::<Vec<_>>(), split2.keys().collect::<Vec<_>>());
            let positive: Vec<Vec2> =
                if orientation(a, b, c) > 0.0 { quad.clone() } else { quad.iter().rev().copied().collect() };
            for y in 0..SCREEN.y1 {
                for x in 0..SCREEN.x1 {
                    if strictly_inside(&positive, x, y) {
                        prop_assert!(split1.contains_key(&(x, y)), "gap at ({x}, {y})");
                    }
                    if split1.contains_key(&(x, y)) {
                        prop_assert!(inside_or_on(&positive, x, y), "covered outside at ({x}, {y})");
                    }
                }
            }
        }

        /// A triangle fan around an interior point covers each pixel at most
        /// once, and every centre strictly inside the polygon.
        #[test]
        fn fan_never_double_covers((center, rim) in (3usize..10).prop_flat_map(ellipse_polygon)) {
            prop_assume!(convex_and_consistent(&rim));
            let tris: Vec<[Vec2; 3]> = (0..rim.len()).map(|i| [center, rim[i], rim[(i + 1) % rim.len()]]).collect();
            // The centre must be strictly inside, so all triangles wind alike.
            let o: Vec<f32> = tris.iter().map(|t| orientation(t[0], t[1], t[2])).collect();
            prop_assume!(o.iter().all(|&x| x > 0.0) || o.iter().all(|&x| x < 0.0));
            let counts = coverage_counts(&tris);
            prop_assert!(counts.values().all(|&n| n == 1), "double coverage {counts:?}");
            let positive: Vec<Vec2> = if o[0] > 0.0 { rim.clone() } else { rim.iter().rev().copied().collect() };
            for y in 0..SCREEN.y1 {
                for x in 0..SCREEN.x1 {
                    if strictly_inside(&positive, x, y) {
                        prop_assert!(counts.contains_key(&(x, y)), "gap at ({x}, {y})");
                    }
                    if counts.contains_key(&(x, y)) {
                        prop_assert!(inside_or_on(&positive, x, y), "covered outside at ({x}, {y})");
                    }
                }
            }
        }

        /// Weights are in [0, 1], sum to 1, and reproduce the pixel centre when
        /// interpolating the (snapped) vertex positions.
        #[test]
        fn barycentric_weights_are_affine(a in grid_point(), b in grid_point(), c in grid_point()) {
            let tri = [a, b, c];
            if let Some(t) = TriangleSetup::new(tri) {
                t.for_each_pixel(SCREEN, |f| {
                    let w = f.barycentric();
                    assert!(w.iter().all(|&x| (0.0..=1.0).contains(&x)), "{w:?}");
                    assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{w:?}");
                    let p = f.interpolate(tri);
                    let center = Vec2::new(f.x as f32 + 0.5, f.y as f32 + 0.5);
                    assert!((p - center).abs().max_element() < 1e-3, "{p} vs {center}");
                });
            }
        }

        /// Coverage is identical whatever the starting vertex or winding.
        #[test]
        fn coverage_is_invariant_under_vertex_permutation(a in grid_point(), b in grid_point(), c in grid_point()) {
            let base = pixels([a, b, c], SCREEN);
            for tri in [[b, c, a], [c, a, b], [a, c, b], [c, b, a], [b, a, c]] {
                prop_assert_eq!(&pixels(tri, SCREEN), &base);
            }
        }
    }
}

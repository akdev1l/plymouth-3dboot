// SPDX-License-Identifier: GPL-3.0-or-later
//! Integer edge functions and the top-left fill rule.

use super::fixed::{FixedPoint, SUBPIXEL_SCALE};

/// The edge function of a directed edge `p0 → p1`:
/// `E(p) = (p1 - p0) × (p - p0) = a·x + b·y + c`, in subpixel units squared.
///
/// In window coordinates (+y down), `E(p) > 0` for points to the right of
/// the edge when walking from `p0` to `p1` as seen on screen. A triangle
/// whose three edges are all positive inside has positive orientation; it
/// winds clockwise as seen on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeFunction {
    /// Coefficient of x (`p0.y - p1.y`).
    pub a: i64,
    /// Coefficient of y (`p1.x - p0.x`).
    pub b: i64,
    /// Constant term.
    pub c: i64,
}

impl EdgeFunction {
    /// The edge function of `p0 → p1`.
    #[must_use]
    pub fn new(p0: FixedPoint, p1: FixedPoint) -> Self {
        let a = p0.y - p1.y;
        let b = p1.x - p0.x;
        Self {
            a,
            b,
            c: -(a * p0.x + b * p0.y),
        }
    }

    /// Evaluates the edge function at `p`.
    #[must_use]
    pub fn eval(&self, p: FixedPoint) -> i64 {
        self.a * p.x + self.b * p.y + self.c
    }

    /// Change of [`EdgeFunction::eval`] per pixel step in +x.
    #[must_use]
    pub fn step_x(&self) -> i64 {
        self.a * SUBPIXEL_SCALE
    }

    /// Change of [`EdgeFunction::eval`] per pixel step in +y.
    #[must_use]
    pub fn step_y(&self) -> i64 {
        self.b * SUBPIXEL_SCALE
    }

    /// Whether this edge is a *top* or *left* edge of a triangle with
    /// positive orientation (interior where `E > 0`).
    ///
    /// A left edge has the interior towards +x, i.e. `a > 0`; a top edge
    /// is horizontal (`a == 0`) with the interior below, i.e. `b > 0`.
    #[must_use]
    pub fn is_top_left(&self) -> bool {
        self.a > 0 || (self.a == 0 && self.b > 0)
    }

    /// The fill-rule bias: `E(p) + bias > 0` exactly when `p` is covered,
    /// i.e. `E > 0`, or `E == 0` on a top-left edge.
    #[must_use]
    pub fn bias(&self) -> i64 {
        i64::from(self.is_top_left())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;
    use crate::raster::fixed::MAX_COORD;

    fn fp(x: f32, y: f32) -> FixedPoint {
        FixedPoint::from_window(Vec2::new(x, y)).unwrap()
    }

    #[test]
    fn sign_is_positive_to_the_right_on_screen() {
        // Walking down the screen (+y), "right as seen on screen" is -x.
        let e = EdgeFunction::new(fp(0.0, 0.0), fp(0.0, 10.0));
        assert!(e.eval(fp(-1.0, 5.0)) > 0);
        assert!(e.eval(fp(1.0, 5.0)) < 0);
        assert_eq!(e.eval(fp(0.0, 3.0)), 0);
    }

    #[test]
    fn steps_match_evaluation_differences() {
        let e = EdgeFunction::new(fp(1.25, 2.5), fp(7.0, -3.75));
        let p = FixedPoint::pixel_center(3, 4);
        let px = FixedPoint::pixel_center(4, 4);
        let py = FixedPoint::pixel_center(3, 5);
        assert_eq!(e.eval(px) - e.eval(p), e.step_x());
        assert_eq!(e.eval(py) - e.eval(p), e.step_y());
    }

    #[test]
    fn top_left_classification() {
        // (0,0) -> (10,0) -> (0,10) has positive orientation (E > 0 inside).
        // Its edges: the top (y = 0), the bottom-right diagonal, and the
        // left (x = 0, walking up the screen).
        let (a, b, c) = (fp(0.0, 0.0), fp(10.0, 0.0), fp(0.0, 10.0));
        let [top, diagonal, left] = [
            EdgeFunction::new(a, b),
            EdgeFunction::new(b, c),
            EdgeFunction::new(c, a),
        ];
        let centroid = fp(3.0, 3.0);
        for e in [top, diagonal, left] {
            assert!(e.eval(centroid) > 0, "test triangle orientation: {e:?}");
        }
        assert!(top.is_top_left(), "{top:?}");
        assert!(!diagonal.is_top_left(), "{diagonal:?}");
        assert!(left.is_top_left(), "{left:?}");
        assert_eq!((top.bias(), diagonal.bias(), left.bias()), (1, 0, 1));

        // The mirrored triangle (0,10) -> (10,10) -> (10,0): its horizontal
        // edge is a bottom edge and its vertical edge a right edge.
        let (a, b, c) = (fp(0.0, 10.0), fp(10.0, 0.0), fp(10.0, 10.0));
        let [diagonal, right, bottom] = [
            EdgeFunction::new(a, b),
            EdgeFunction::new(b, c),
            EdgeFunction::new(c, a),
        ];
        let centroid = fp(7.0, 7.0);
        for e in [diagonal, right, bottom] {
            assert!(e.eval(centroid) > 0, "test triangle orientation: {e:?}");
        }
        assert!(diagonal.is_top_left(), "{diagonal:?}");
        assert!(!right.is_top_left(), "{right:?}");
        assert!(!bottom.is_top_left(), "{bottom:?}");
    }

    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let m = MAX_COORD;
        let corners = [fp(-m, -m), fp(m, -m), fp(m, m), fp(-m, m)];
        for &p0 in &corners {
            for &p1 in &corners {
                let e = EdgeFunction::new(p0, p1);
                for &p in &corners {
                    // Debug builds panic on overflow, so evaluating is the test.
                    let v = e.eval(p);
                    assert!(v.unsigned_abs() < 1 << 51);
                }
            }
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Depth testing.

/// The comparison a fragment's depth must pass against the stored depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DepthFunc {
    /// Pass if strictly closer (the default).
    #[default]
    Less,
    /// Pass if closer or equal.
    LessEqual,
    /// Always pass (no depth test).
    Always,
}

impl DepthFunc {
    /// Whether a fragment at `depth` passes against `stored`.
    #[must_use]
    pub fn passes(self, depth: f32, stored: f32) -> bool {
        match self {
            Self::Less => depth < stored,
            Self::LessEqual => depth <= stored,
            Self::Always => true,
        }
    }
}

/// Depth-buffer configuration for a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DepthState {
    /// Comparison against the stored depth.
    pub func: DepthFunc,
    /// Whether passing fragments write their depth.
    pub write: bool,
}

impl Default for DepthState {
    fn default() -> Self {
        Self {
            func: DepthFunc::Less,
            write: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::math::Vec3;
    use crate::pipeline::ScreenVertex;
    use crate::raster::{Rect, TriangleSetup};
    use crate::target::Framebuffer;

    #[test]
    fn depth_functions() {
        assert!(DepthFunc::Less.passes(0.4, 0.5));
        assert!(!DepthFunc::Less.passes(0.5, 0.5));
        assert!(DepthFunc::LessEqual.passes(0.5, 0.5));
        assert!(!DepthFunc::LessEqual.passes(0.6, 0.5));
        assert!(DepthFunc::Always.passes(2.0, 0.0));
        assert_eq!(
            DepthState::default(),
            DepthState {
                func: DepthFunc::Less,
                write: true
            }
        );
    }

    /// Draws flat-coloured triangles given in window coordinates (x, y,
    /// depth), with depth interpolated linearly in screen space.
    fn draw(fb: &mut Framebuffer, state: DepthState, tris: &[([Vec3; 3], Rgba8)]) {
        let scissor = Rect::from_size(fb.width(), fb.height());
        for &(verts, color) in tris {
            let s = verts.map(|p| ScreenVertex {
                position: p,
                inv_w: 1.0,
                varyings: (),
            });
            let Some(setup) = TriangleSetup::new(s.map(|v| v.xy())) else {
                continue;
            };
            setup.for_each_pixel(scissor, |f| {
                let z = f.interpolate(s.map(|v| v.position.z));
                let stored = fb.depth.get_mut(f.x, f.y).expect("scissored");
                if state.func.passes(z, *stored) {
                    if state.write {
                        *stored = z;
                    }
                    *fb.color.get_mut(f.x, f.y).expect("scissored") = color;
                }
            });
        }
    }

    const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
    const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

    /// Two triangles that interpenetrate: A's depth falls left to right,
    /// B's rises, so each is in front over half of the overlap.
    fn interpenetrating() -> [([Vec3; 3], Rgba8); 2] {
        [
            (
                [
                    Vec3::new(2.0, 2.0, 0.9),
                    Vec3::new(30.0, 4.0, 0.1),
                    Vec3::new(4.0, 30.0, 0.9),
                ],
                RED,
            ),
            (
                [
                    Vec3::new(30.0, 30.0, 0.9),
                    Vec3::new(2.0, 6.0, 0.1),
                    Vec3::new(28.0, 2.0, 0.9),
                ],
                BLUE,
            ),
        ]
    }

    #[test]
    fn result_is_independent_of_draw_order() {
        let [a, b] = interpenetrating();
        let mut ab = Framebuffer::new(32, 32, Rgba8::BLACK).unwrap();
        let mut ba = ab.clone();
        draw(&mut ab, DepthState::default(), &[a, b]);
        draw(&mut ba, DepthState::default(), &[b, a]);
        assert_eq!(ab.color, ba.color);
        assert_eq!(ab.depth, ba.depth);
        // Both triangles are visible somewhere.
        assert!(ab.color.pixels().contains(&RED) && ab.color.pixels().contains(&BLUE));
    }

    #[test]
    fn closer_triangle_wins() {
        let near = (
            [
                Vec3::new(0.0, 0.0, 0.2),
                Vec3::new(16.0, 0.0, 0.2),
                Vec3::new(0.0, 16.0, 0.2),
            ],
            RED,
        );
        let far = (
            [
                Vec3::new(0.0, 0.0, 0.8),
                Vec3::new(16.0, 0.0, 0.8),
                Vec3::new(0.0, 16.0, 0.8),
            ],
            BLUE,
        );
        let mut fb = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        draw(&mut fb, DepthState::default(), &[near, far]);
        assert_eq!(fb.color.get(2, 2), Some(RED));
        assert_eq!(fb.depth.get(2, 2), Some(0.2));
    }

    #[test]
    fn equal_depth_follows_depth_function() {
        let tri = |c| {
            (
                [
                    Vec3::new(0.0, 0.0, 0.5),
                    Vec3::new(16.0, 0.0, 0.5),
                    Vec3::new(0.0, 16.0, 0.5),
                ],
                c,
            )
        };
        let mut less = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        draw(&mut less, DepthState::default(), &[tri(RED), tri(BLUE)]);
        assert_eq!(less.color.get(2, 2), Some(RED), "Less keeps the first");
        let mut less_equal = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        let le = DepthState {
            func: DepthFunc::LessEqual,
            write: true,
        };
        draw(&mut less_equal, le, &[tri(RED), tri(BLUE)]);
        assert_eq!(
            less_equal.color.get(2, 2),
            Some(BLUE),
            "LessEqual lets the second overwrite"
        );
    }

    #[test]
    fn fragments_beyond_cleared_depth_are_rejected() {
        // Depth exactly 1.0 (the far plane) fails `Less` against the clear value.
        let tri = (
            [
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(16.0, 0.0, 1.0),
                Vec3::new(0.0, 16.0, 1.0),
            ],
            RED,
        );
        let mut fb = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        draw(&mut fb, DepthState::default(), &[tri]);
        assert!(fb.color.pixels().iter().all(|&p| p == Rgba8::BLACK));
    }
}

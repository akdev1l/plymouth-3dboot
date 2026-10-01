// SPDX-License-Identifier: GPL-3.0-or-later
//! The draw call: shaders, render state and the triangle pipeline.

use super::{ClipVertex, Clipper, DepthState, ScreenVertex, perspective_weights};
use crate::color::LinearRgba;
use crate::math::Viewport;
use crate::raster::{CullMode, Interpolate, Rect, TriangleSetup, Winding};
use crate::target::{Framebuffer, MAX_DIMENSION};

/// Programmable vertex and fragment stages.
///
/// Implementations are plain Rust types. The renderer is generic over
/// them, so shading compiles to direct calls. Shaders own (or borrow) their
/// vertex data and *pull* vertices by index, so meshes with separate
/// attribute arrays need no interleaved copy.
pub trait Shader {
    /// Attributes passed from the vertex to the fragment stage, interpolated
    /// perspective-correctly.
    type Varyings: Interpolate;

    /// Transforms vertex `index` (always `< vertex_count` of the draw call)
    /// into clip space.
    fn vertex(&self, index: u32) -> ClipVertex<Self::Varyings>;

    /// Shades one fragment, returning its linear colour, or `None` to discard
    /// it (leaving colour and depth untouched).
    fn fragment(&self, fragment: &FragmentInput<Self::Varyings>) -> Option<LinearRgba>;
}

/// Inputs to [`Shader::fragment`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FragmentInput<V> {
    /// Pixel column.
    pub x: u32,
    /// Pixel row.
    pub y: u32,
    /// Window depth in `[0, 1]`.
    pub depth: f32,
    /// Whether the triangle faces the viewer (counter-clockwise on screen).
    pub front_facing: bool,
    /// Interpolated attributes.
    pub varyings: V,
}

/// Fixed-function state for a draw call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderState {
    /// Target rectangle for NDC; pixels outside it (or the framebuffer) are
    /// never touched.
    pub viewport: Viewport,
    /// Face culling.
    pub cull: CullMode,
    /// Depth testing.
    pub depth: DepthState,
}

impl RenderState {
    /// Defaults for `viewport`: no culling, `Less` depth test with writes.
    #[must_use]
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            cull: CullMode::None,
            depth: DepthState::default(),
        }
    }
}

/// Counters describing what a draw call did.
///
/// Counts after clipping refer to the (sub-)triangles produced by the
/// clipper, so one input triangle can count more than once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DrawStats {
    /// Input triangles.
    pub triangles: usize,
    /// Triangles (after clipping) that were degenerate once snapped to the
    /// subpixel grid.
    pub degenerate: usize,
    /// Triangles (after clipping) rejected by face culling.
    pub culled: usize,
    /// Triangles (after clipping) passed to the rasterizer.
    pub rasterized: usize,
    /// Fragments that passed the depth test and were shaded.
    pub fragments_shaded: usize,
    /// Fragments written (shaded and not discarded).
    pub fragments_written: usize,
}

impl std::ops::AddAssign for DrawStats {
    fn add_assign(&mut self, o: Self) {
        self.triangles += o.triangles;
        self.degenerate += o.degenerate;
        self.culled += o.culled;
        self.rasterized += o.rasterized;
        self.fragments_shaded += o.fragments_shaded;
        self.fragments_written += o.fragments_written;
    }
}

/// A draw call's input was invalid; nothing was drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DrawError {
    /// The index count is not a multiple of three.
    #[error("index count {0} is not a multiple of 3")]
    IndexCount(usize),
    /// A viewport offset or size exceeds [`MAX_DIMENSION`]. The clipper's
    /// guard band, and thus correct rasterization, relies on this limit.
    #[error("viewport {0:?} exceeds the maximum dimension {MAX_DIMENSION}")]
    Viewport(Viewport),
    /// An index refers past the end of the vertex array.
    #[error("index {index} out of range for {vertex_count} vertices")]
    IndexOutOfRange {
        /// The offending index.
        index: u32,
        /// Number of vertices supplied.
        vertex_count: usize,
    },
}

/// Executes draw calls.
///
/// The renderer is cheap to create. Keep one around so that future
/// versions can reuse internal scratch memory between draws.
#[derive(Debug, Default)]
pub struct Renderer {
    _private: (),
}

impl Renderer {
    /// Creates a renderer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws indexed triangles (`indices` in groups of three, referring to
    /// vertices `0..vertex_count` of `shader`) into `target`.
    ///
    /// Depth is tested with depth before shading ("early Z"). Fragment
    /// colours replace the target colour (no blending).
    ///
    /// # Errors
    ///
    /// Returns [`DrawError`] without drawing anything if `indices` are
    /// malformed or a viewport offset or size exceeds [`MAX_DIMENSION`].
    pub fn draw_indexed<S: Shader>(
        &mut self,
        target: &mut Framebuffer,
        state: &RenderState,
        shader: &S,
        vertex_count: usize,
        indices: &[u32],
    ) -> Result<DrawStats, DrawError> {
        let vp = state.viewport;
        if [vp.x, vp.y, vp.width, vp.height]
            .iter()
            .any(|&d| d > MAX_DIMENSION)
        {
            return Err(DrawError::Viewport(vp));
        }
        if !indices.len().is_multiple_of(3) {
            return Err(DrawError::IndexCount(indices.len()));
        }
        if let Some(&index) = indices.iter().find(|&&i| i as usize >= vertex_count) {
            return Err(DrawError::IndexOutOfRange {
                index,
                vertex_count,
            });
        }
        let Ok(vertex_count) = u32::try_from(vertex_count) else {
            // Every index is below vertex_count, so the draw is empty.
            return Ok(DrawStats::default());
        };

        // Vertex shading, once per vertex.
        let clip: Vec<ClipVertex<S::Varyings>> =
            (0..vertex_count).map(|i| shader.vertex(i)).collect();
        let scissor = Rect {
            x0: vp.x,
            y0: vp.y,
            x1: vp.x.saturating_add(vp.width),
            y1: vp.y.saturating_add(vp.height),
        }
        .intersect(&Rect::from_size(target.width(), target.height()));
        let mut clipper = Clipper::new();
        let mut stats = DrawStats {
            triangles: indices.len() / 3,
            ..DrawStats::default()
        };

        for &[a, b, c] in indices.as_chunks::<3>().0 {
            let input = [clip[a as usize], clip[b as usize], clip[c as usize]];
            clipper.clip(input, |clipped| {
                let screen = clipped.map(|v| ScreenVertex::from_clip(v, &vp));
                let Some(setup) = TriangleSetup::new(screen.map(|v| v.xy())) else {
                    stats.degenerate += 1;
                    return;
                };
                if state.cull.culls(setup.winding()) {
                    stats.culled += 1;
                    return;
                }
                stats.rasterized += 1;
                let front_facing = setup.winding() == Winding::CounterClockwise;
                let depths = screen.map(|v| v.position.z);
                let inv_w = screen.map(|v| v.inv_w);
                let varyings = screen.map(|v| v.varyings);
                setup.for_each_pixel(scissor, |f| {
                    let bary = f.barycentric();
                    let depth = f32::interpolate(depths, bary);
                    let Some(stored) = target.depth.get_mut(f.x, f.y) else {
                        return;
                    };
                    if !state.depth.func.passes(depth, *stored) {
                        return;
                    }
                    let input = FragmentInput {
                        x: f.x,
                        y: f.y,
                        depth,
                        front_facing,
                        varyings: S::Varyings::interpolate(
                            varyings,
                            perspective_weights(bary, inv_w),
                        ),
                    };
                    stats.fragments_shaded += 1;
                    let Some(color) = shader.fragment(&input) else {
                        return;
                    };
                    if state.depth.write {
                        *stored = depth;
                    }
                    if let Some(px) = target.color.get_mut(f.x, f.y) {
                        *px = color.to_srgb8();
                        stats.fragments_written += 1;
                    }
                });
            });
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::math::Vec4;

    /// Positions are already in clip space; varyings carry a colour.
    struct Passthrough(Vec<(Vec4, LinearRgba)>);

    impl Shader for Passthrough {
        type Varyings = LinearRgba;

        fn vertex(&self, i: u32) -> ClipVertex<LinearRgba> {
            let (position, varyings) = self.0[i as usize];
            ClipVertex { position, varyings }
        }

        fn fragment(&self, f: &FragmentInput<LinearRgba>) -> Option<LinearRgba> {
            Some(f.varyings)
        }
    }

    /// Discards fragments in odd columns.
    struct Stripes(Vec<Vec4>);

    impl Shader for Stripes {
        type Varyings = ();

        fn vertex(&self, i: u32) -> ClipVertex<()> {
            ClipVertex {
                position: self.0[i as usize],
                varyings: (),
            }
        }

        fn fragment(&self, f: &FragmentInput<()>) -> Option<LinearRgba> {
            f.x.is_multiple_of(2).then_some(LinearRgba::WHITE)
        }
    }

    const RED: LinearRgba = LinearRgba::rgb(1.0, 0.0, 0.0);
    const QUAD: [u32; 6] = [0, 1, 2, 0, 2, 3];

    fn quad_corners() -> [Vec4; 4] {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| Vec4::new(x, y, 0.0, 1.0))
    }

    fn red_quad() -> Passthrough {
        Passthrough(quad_corners().iter().map(|&p| (p, RED)).collect())
    }

    fn fb(w: u32, h: u32) -> Framebuffer {
        Framebuffer::new(w, h, Rgba8::BLACK).unwrap()
    }

    fn draw<S: Shader>(
        target: &mut Framebuffer,
        state: &RenderState,
        shader: &S,
        n: usize,
        indices: &[u32],
    ) -> Result<DrawStats, DrawError> {
        Renderer::new().draw_indexed(target, state, shader, n, indices)
    }

    #[test]
    fn full_screen_quad_covers_every_pixel_once() {
        let mut target = fb(8, 6);
        let stats = draw(
            &mut target,
            &RenderState::new(Viewport::new(8, 6)),
            &red_quad(),
            4,
            &QUAD,
        )
        .unwrap();
        assert!(
            target
                .color
                .pixels()
                .iter()
                .all(|&p| p == Rgba8::new(255, 0, 0, 255))
        );
        assert!(
            target
                .depth
                .values()
                .iter()
                .all(|&d| (d - 0.5).abs() < 1e-6)
        );
        assert_eq!(
            stats,
            DrawStats {
                triangles: 2,
                degenerate: 0,
                culled: 0,
                rasterized: 2,
                fragments_shaded: 48,
                fragments_written: 48
            }
        );
    }

    #[test]
    fn viewport_limits_drawing() {
        let mut target = fb(8, 8);
        let state = RenderState::new(Viewport {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        });
        let stats = draw(&mut target, &state, &red_quad(), 4, &QUAD).unwrap();
        assert_eq!(stats.fragments_written, 8);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (3..5).contains(&y);
                assert_eq!(
                    target.color.get(x, y) != Some(Rgba8::BLACK),
                    inside,
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn viewport_larger_than_framebuffer_is_clipped() {
        let mut target = fb(4, 4);
        let stats = draw(
            &mut target,
            &RenderState::new(Viewport::new(100, 100)),
            &red_quad(),
            4,
            &QUAD,
        )
        .unwrap();
        assert_eq!(stats.fragments_written, 16);
    }

    #[test]
    fn oversized_viewports_are_rejected() {
        let mut target = fb(16, 16);
        for vp in [
            Viewport::new(30_000, 30_000),
            Viewport {
                x: MAX_DIMENSION + 1,
                y: 0,
                width: 4,
                height: 4,
            },
        ] {
            assert_eq!(
                draw(&mut target, &RenderState::new(vp), &red_quad(), 4, &QUAD),
                Err(DrawError::Viewport(vp))
            );
        }
        // The largest allowed viewport still draws its on-screen part.
        let max = Viewport::new(MAX_DIMENSION, MAX_DIMENSION);
        let big = Passthrough(
            [(-1.0, 1.0), (3.9, 1.0), (-1.0, -1.0)]
                .map(|(x, y)| (Vec4::new(x, y, 0.0, 1.0), RED))
                .to_vec(),
        );
        let stats = draw(&mut target, &RenderState::new(max), &big, 3, &[0, 1, 2]).unwrap();
        assert_eq!(stats.fragments_written, 256, "{stats:?}");
    }

    #[test]
    fn degenerate_triangles_are_counted() {
        let mut target = fb(4, 4);
        let line = Stripes(vec![
            Vec4::new(0.0, 0.0, 0.0, 1.0),
            Vec4::new(0.5, 0.5, 0.0, 1.0),
            Vec4::new(1.0, 1.0, 0.0, 1.0),
        ]);
        let stats = draw(
            &mut target,
            &RenderState::new(Viewport::new(4, 4)),
            &line,
            3,
            &[0, 1, 2],
        )
        .unwrap();
        assert_eq!((stats.degenerate, stats.rasterized), (1, 0));
    }

    #[test]
    fn culling_counts_and_rejects_back_faces() {
        let mut target = fb(4, 4);
        let clockwise = [0, 2, 1, 0, 3, 2];
        let mut state = RenderState::new(Viewport::new(4, 4));
        state.cull = CullMode::Back;
        let stats = draw(&mut target, &state, &red_quad(), 4, &clockwise).unwrap();
        assert_eq!(
            (stats.culled, stats.rasterized, stats.fragments_written),
            (2, 0, 0)
        );
        state.cull = CullMode::Front;
        let stats = draw(&mut target, &state, &red_quad(), 4, &clockwise).unwrap();
        assert_eq!((stats.culled, stats.fragments_written), (0, 16));
    }

    #[test]
    fn front_facing_flag_matches_winding() {
        struct FacingColor;
        impl Shader for FacingColor {
            type Varyings = ();
            fn vertex(&self, i: u32) -> ClipVertex<()> {
                ClipVertex {
                    position: quad_corners()[i as usize],
                    varyings: (),
                }
            }
            fn fragment(&self, f: &FragmentInput<()>) -> Option<LinearRgba> {
                Some(if f.front_facing {
                    LinearRgba::rgb(0.0, 1.0, 0.0)
                } else {
                    RED
                })
            }
        }
        let mut target = fb(4, 4);
        let state = RenderState::new(Viewport::new(4, 4));
        // Counter-clockwise in NDC: front-facing.
        draw(&mut target, &state, &FacingColor, 4, &[0, 1, 2]).unwrap();
        assert_eq!(target.color.get(3, 1), Some(Rgba8::new(0, 255, 0, 255)));
        draw(&mut target, &state, &FacingColor, 4, &[0, 3, 2]).unwrap();
        assert_eq!(target.color.get(0, 2), Some(Rgba8::new(255, 0, 0, 255)));
    }

    #[test]
    fn discarded_fragments_leave_colour_and_depth() {
        let mut target = fb(4, 2);
        let stats = draw(
            &mut target,
            &RenderState::new(Viewport::new(4, 2)),
            &Stripes(quad_corners().to_vec()),
            4,
            &QUAD,
        )
        .unwrap();
        assert_eq!((stats.fragments_shaded, stats.fragments_written), (8, 4));
        assert_eq!(target.color.get(1, 0), Some(Rgba8::BLACK));
        assert_eq!(target.depth.get(1, 0), Some(1.0));
        assert_eq!(target.color.get(2, 0), Some(Rgba8::WHITE));
    }

    #[test]
    fn invalid_indices_are_rejected_before_drawing() {
        let mut target = fb(4, 4);
        let state = RenderState::new(Viewport::new(4, 4));
        assert_eq!(
            draw(&mut target, &state, &red_quad(), 4, &[0, 1]),
            Err(DrawError::IndexCount(2))
        );
        assert_eq!(
            draw(&mut target, &state, &red_quad(), 4, &[0, 1, 2, 0, 2, 4]),
            Err(DrawError::IndexOutOfRange {
                index: 4,
                vertex_count: 4
            })
        );
        assert!(
            target.color.pixels().iter().all(|&p| p == Rgba8::BLACK),
            "nothing drawn"
        );
    }

    #[test]
    fn varyings_are_interpolated_perspective_correctly() {
        // A quad whose right edge is 4x farther (w = 4): the attribute at the
        // horizontal screen centre is much closer to the left value.
        struct U;
        const VERTS: [(Vec4, f32); 4] = [
            (Vec4::new(-1.0, -1.0, 0.0, 1.0), 0.0),
            (Vec4::new(4.0, -4.0, 0.0, 4.0), 1.0),
            (Vec4::new(4.0, 4.0, 0.0, 4.0), 1.0),
            (Vec4::new(-1.0, 1.0, 0.0, 1.0), 0.0),
        ];
        impl Shader for U {
            type Varyings = f32;
            fn vertex(&self, i: u32) -> ClipVertex<f32> {
                let (position, varyings) = VERTS[i as usize];
                ClipVertex { position, varyings }
            }
            fn fragment(&self, f: &FragmentInput<f32>) -> Option<LinearRgba> {
                Some(LinearRgba::rgb(f.varyings, 0.0, 0.0))
            }
        }
        let mut target = Framebuffer::new(64, 1, Rgba8::BLACK).unwrap();
        draw(
            &mut target,
            &RenderState::new(Viewport::new(64, 1)),
            &U,
            4,
            &QUAD,
        )
        .unwrap();
        // Screen-space midpoint: u = (0.5/4) / (0.5/1 + 0.5/4) = 0.2.
        let mid = target.color.get(32, 0).unwrap().to_linear().r;
        assert!((mid - 0.2).abs() < 0.02, "u at screen centre = {mid}");
    }
}

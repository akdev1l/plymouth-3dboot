// SPDX-License-Identifier: GPL-3.0-or-later
//! The draw call: shaders, render state and the triangle pipeline.

use super::{ClipVertex, Clipper, DepthState, ScreenVertex, perspective_weights};
use crate::color::LinearRgba;
use crate::color::Rgba8;
use crate::math::Viewport;
use crate::raster::{CullMode, Interpolate, Rect, TriangleSetup, Winding};
use crate::target::{Framebuffer, MAX_DIMENSION};

/// Programmable vertex and fragment stages.
///
/// Implementations are plain Rust types. The renderer is generic over
/// them, so shading compiles to direct calls. Shaders own (or borrow) their
/// vertex data and *pull* vertices by index, so meshes with separate
/// attribute arrays need no interleaved copy. Shaders are shared between
/// rendering threads, hence `Sync`.
pub trait Shader: Sync {
    /// Attributes passed from the vertex to the fragment stage, interpolated
    /// perspective-correctly.
    type Varyings: Interpolate + Send + Sync;

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
    /// If set, only pixels within this rectangle (and the viewport) are
    /// drawn; the viewport still maps NDC.
    pub scissor: Option<Rect>,
}

impl RenderState {
    /// Defaults for `viewport`: no culling, `Less` depth test with writes,
    /// no scissor.
    #[must_use]
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            cull: CullMode::None,
            depth: DepthState::default(),
            scissor: None,
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
    /// Pixels the draw may have written: the union of the rasterized
    /// triangles' bounding boxes within the scissor (empty if none).
    pub bounds: Rect,
}

impl std::ops::AddAssign for DrawStats {
    fn add_assign(&mut self, o: Self) {
        self.triangles += o.triangles;
        self.degenerate += o.degenerate;
        self.culled += o.culled;
        self.rasterized += o.rasterized;
        self.fragments_shaded += o.fragments_shaded;
        self.fragments_written += o.fragments_written;
        self.bounds = self.bounds.union(&o.bounds);
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
/// The renderer is cheap to create; keep one around so that future
/// versions can reuse internal scratch memory between draws.
///
/// Rasterization can be split into horizontal bands (see
/// [`Renderer::with_threads`]), rendered in parallel with the `parallel`
/// feature. Each band processes triangles in submission order, so the
/// output is byte-identical for every thread count.
#[derive(Debug)]
pub struct Renderer {
    threads: usize,
    /// Worker threads (`threads` of them), when rendering in parallel.
    #[cfg(all(feature = "parallel", not(target_os = "emscripten")))]
    pool: Option<rayon::ThreadPool>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::with_threads(1)
    }
}

/// Largest [`Renderer::with_threads`] count.
pub const MAX_THREADS: usize = 64;

/// A triangle after clipping, setup and culling, ready to rasterize.
struct Prepared<V> {
    setup: TriangleSetup,
    depths: [f32; 3],
    inv_w: [f32; 3],
    varyings: [V; 3],
    front_facing: bool,
}

/// Rows `y0..y1` (all columns) of the target.
struct Band<'a> {
    y0: u32,
    y1: u32,
    width: usize,
    color: &'a mut [Rgba8],
    depth: &'a mut [f32],
}

/// Rasterizes and shades `prepared` triangles within `scissor` into `band`;
/// returns (fragments shaded, fragments written).
fn rasterize_band<S: Shader>(
    shader: &S,
    state: &RenderState,
    scissor: Rect,
    prepared: &[Prepared<S::Varyings>],
    band: Band<'_>,
) -> (usize, usize) {
    let rect = scissor.intersect(&Rect {
        x0: scissor.x0,
        y0: band.y0,
        x1: scissor.x1,
        y1: band.y1,
    });
    let (mut shaded, mut written) = (0, 0);
    // Flat and unlit shading give runs of one colour: encode it once.
    let mut last: Option<(LinearRgba, Rgba8)> = None;
    for tri in prepared {
        tri.setup.for_each_pixel(rect, |f| {
            let index = (f.y - band.y0) as usize * band.width + f.x as usize;
            let bary = f.barycentric();
            let depth = f32::interpolate(tri.depths, bary);
            let stored = &mut band.depth[index];
            if !state.depth.func.passes(depth, *stored) {
                return;
            }
            let input = FragmentInput {
                x: f.x,
                y: f.y,
                depth,
                front_facing: tri.front_facing,
                varyings: S::Varyings::interpolate(
                    tri.varyings,
                    perspective_weights(bary, tri.inv_w),
                ),
            };
            shaded += 1;
            let Some(color) = shader.fragment(&input) else {
                return;
            };
            if state.depth.write {
                *stored = depth;
            }
            band.color[index] = match last {
                Some((linear, encoded)) if linear == color => encoded,
                _ => {
                    let encoded = color.to_srgb8();
                    last = Some((color, encoded));
                    encoded
                }
            };
            written += 1;
        });
    }
    (shaded, written)
}

impl Renderer {
    /// Creates a single-threaded renderer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A renderer using `threads` threads (clamped to `1..=MAX_THREADS`).
    /// With more than one, each frame is split into `4 × threads` bands for
    /// load balance. With the `parallel` feature the renderer owns a pool of
    /// `threads` worker threads that render the bands concurrently (except
    /// on Emscripten, which has no threads, or if the threads cannot be
    /// spawned); otherwise the bands render one after another. Output is
    /// identical either way.
    #[must_use]
    pub fn with_threads(threads: usize) -> Self {
        let threads = threads.clamp(1, MAX_THREADS);
        Self {
            threads,
            #[cfg(all(feature = "parallel", not(target_os = "emscripten")))]
            pool: (threads > 1)
                .then(|| {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .thread_name(|i| format!("p3b-render-{i}"))
                        .build()
                        .ok()
                })
                .flatten(),
        }
    }

    /// Number of threads requested.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
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
        .intersect(&Rect::from_size(target.width(), target.height()))
        .intersect(&state.scissor.unwrap_or(Rect::from_size(u32::MAX, u32::MAX)));
        let mut clipper = Clipper::new();
        let mut stats = DrawStats {
            triangles: indices.len() / 3,
            ..DrawStats::default()
        };

        // Phase 1 (serial): clip, set up and cull every triangle.
        let mut prepared = Vec::with_capacity(indices.len() / 3);
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
                stats.bounds = stats.bounds.union(&setup.bounds(scissor));
                prepared.push(Prepared {
                    setup,
                    depths: screen.map(|v| v.position.z),
                    inv_w: screen.map(|v| v.inv_w),
                    varyings: screen.map(|v| v.varyings),
                    front_facing: setup.winding() == Winding::CounterClockwise,
                });
            });
        }
        if scissor.is_empty() || prepared.is_empty() {
            return Ok(stats);
        }

        // Phase 2: rasterize horizontal bands of the scissored rows.
        let width = target.width() as usize;
        let rows = (scissor.y1 - scissor.y0) as usize;
        // Several bands per thread balance the load when the geometry covers
        // only part of the frame.
        let bands = if self.threads > 1 {
            self.threads * 4
        } else {
            1
        };
        let band_rows = rows.div_ceil(bands.min(rows));
        let first = scissor.y0 as usize * width;
        let len = rows * width;
        let colors = target.color.pixels_mut()[first..first + len].chunks_mut(band_rows * width);
        let depths = target.depth.values_mut()[first..first + len].chunks_mut(band_rows * width);
        let bands = colors.zip(depths).enumerate().map(|(i, (color, depth))| {
            let y0 = scissor.y0 + u32::try_from(i * band_rows).unwrap_or(u32::MAX);
            let y1 = y0 + u32::try_from(color.len() / width).unwrap_or(0);
            Band {
                y0,
                y1,
                width,
                color,
                depth,
            }
        });
        let render = |band| rasterize_band(shader, state, scissor, &prepared, band);
        #[cfg(all(feature = "parallel", not(target_os = "emscripten")))]
        let (shaded, written) = if let Some(pool) = &self.pool {
            use rayon::prelude::*;
            let bands: Vec<_> = bands.collect();
            pool.install(|| {
                bands
                    .into_par_iter()
                    .map(render)
                    .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
            })
        } else {
            bands
                .map(render)
                .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
        };
        #[cfg(not(all(feature = "parallel", not(target_os = "emscripten"))))]
        let (shaded, written) = bands
            .map(render)
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        stats.fragments_shaded = shaded;
        stats.fragments_written = written;
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                fragments_written: 48,
                bounds: Rect::from_size(8, 6),
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

    #[test]
    fn banded_rendering_is_byte_identical() {
        // Overlapping, interpenetrating triangles across the whole target.
        let verts: Vec<(Vec4, LinearRgba)> = (0..24)
            .map(|i| {
                let f = i as f32;
                let p = Vec4::new(
                    libm::sinf(f * 1.7) * 1.2,
                    libm::cosf(f * 2.3) * 1.2,
                    libm::sinf(f * 0.9) * 0.9,
                    1.0,
                );
                (p, LinearRgba::rgb(f / 24.0, 1.0 - f / 24.0, 0.5))
            })
            .collect();
        let indices: Vec<u32> = (0..24).collect();
        let shader = Passthrough(verts);
        let render = |threads| {
            let mut target = fb(61, 47);
            let mut state = RenderState::new(Viewport {
                x: 3,
                y: 2,
                width: 55,
                height: 41,
            });
            state.cull = CullMode::None;
            let stats = Renderer::with_threads(threads)
                .draw_indexed(&mut target, &state, &shader, 24, &indices)
                .unwrap();
            (target, stats)
        };
        let (serial, serial_stats) = render(1);
        assert!(serial_stats.fragments_written > 500);
        for threads in [2, 3, 7, 64] {
            let (banded, stats) = render(threads);
            assert_eq!(banded, serial, "{threads} bands");
            assert_eq!(stats, serial_stats, "{threads} bands");
        }
        assert_eq!(Renderer::with_threads(0).threads(), 1);
        assert_eq!(Renderer::with_threads(1000).threads(), MAX_THREADS);
    }

    #[test]
    fn scissor_limits_drawing_without_moving_the_image() {
        let full = {
            let mut target = fb(8, 6);
            draw(
                &mut target,
                &RenderState::new(Viewport::new(8, 6)),
                &red_quad(),
                4,
                &QUAD,
            )
            .unwrap();
            target
        };
        let mut target = fb(8, 6);
        let mut state = RenderState::new(Viewport::new(8, 6));
        let scissor = Rect {
            x0: 2,
            y0: 1,
            x1: 5,
            y1: 9,
        };
        state.scissor = Some(scissor);
        let stats = draw(&mut target, &state, &red_quad(), 4, &QUAD).unwrap();
        assert_eq!(stats.fragments_written, 3 * 5);
        assert_eq!(
            stats.bounds,
            Rect {
                x0: 2,
                y0: 1,
                x1: 5,
                y1: 6
            }
        );
        for y in 0..6 {
            for x in 0..8 {
                let inside = (2..5).contains(&x) && y >= 1;
                let want = if inside {
                    full.color.get(x, y)
                } else {
                    fb(8, 6).color.get(x, y)
                };
                assert_eq!(target.color.get(x, y), want, "({x}, {y})");
            }
        }
    }
}

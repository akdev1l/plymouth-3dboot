// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden images for the 3D pipeline (Phase 3).

use plymouth_3dboot::color::{LinearRgba, Rgba8};
use plymouth_3dboot::math::{Mat4, Vec3, Viewport, look_at, perspective};
use plymouth_3dboot::pipeline::{ClipVertex, FragmentInput, RenderState, Renderer, Shader};
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::target::Framebuffer;
use plymouth_3dboot_testutil::{Tolerance, golden};

/// Transforms positions and passes per-vertex colours through.
struct VertexColor {
    mvp: Mat4,
    vertices: Vec<(Vec3, LinearRgba)>,
}

impl Shader for VertexColor {
    type Varyings = LinearRgba;

    fn vertex(&self, i: u32) -> ClipVertex<LinearRgba> {
        let (position, color) = self.vertices[i as usize];
        ClipVertex::project(&self.mvp, position, color)
    }

    fn fragment(&self, f: &FragmentInput<LinearRgba>) -> Option<LinearRgba> {
        Some(f.varyings)
    }
}

/// A unit cube (±1) with one colour per face and counter-clockwise
/// outward-facing triangles.
fn cube_mesh() -> (Vec<(Vec3, LinearRgba)>, Vec<u32>) {
    // (normal, u, v) with u × v = normal, so the corner order below is
    // counter-clockwise seen from outside.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z, LinearRgba::rgb(0.9, 0.1, 0.1)),
        (
            Vec3::NEG_X,
            Vec3::Z,
            Vec3::Y,
            LinearRgba::rgb(0.1, 0.9, 0.9),
        ),
        (Vec3::Y, Vec3::Z, Vec3::X, LinearRgba::rgb(0.1, 0.9, 0.1)),
        (
            Vec3::NEG_Y,
            Vec3::X,
            Vec3::Z,
            LinearRgba::rgb(0.9, 0.1, 0.9),
        ),
        (Vec3::Z, Vec3::X, Vec3::Y, LinearRgba::rgb(0.1, 0.1, 0.9)),
        (
            Vec3::NEG_Z,
            Vec3::Y,
            Vec3::X,
            LinearRgba::rgb(0.9, 0.9, 0.1),
        ),
    ];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (n, u, v, color) in faces {
        assert_eq!(u.cross(v), n);
        let base = u32::try_from(vertices.len()).unwrap();
        for (s, t) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            // Shade corners slightly differently to make interpolation visible.
            let shade = 0.75 + 0.125 * (s + t);
            vertices.push((n + u * s + v * t, color * shade));
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn render_cube(cull: CullMode) -> Framebuffer {
    const SIZE: u32 = 96;
    let (vertices, indices) = cube_mesh();
    let view = look_at(Vec3::new(2.6, 2.0, 3.4), Vec3::ZERO, Vec3::Y);
    let proj = perspective(50f32.to_radians(), 1.0, 0.1, 20.0);
    let model = Mat4::from_rotation_y(0.3);
    let shader = VertexColor {
        mvp: proj * view * model,
        vertices,
    };
    let mut target = Framebuffer::new(SIZE, SIZE, Rgba8::new(20, 20, 28, 255)).unwrap();
    let mut state = RenderState::new(Viewport::new(SIZE, SIZE));
    state.cull = cull;
    let stats = Renderer::new()
        .draw_indexed(
            &mut target,
            &state,
            &shader,
            shader.vertices.len(),
            &indices,
        )
        .unwrap();
    if cull == CullMode::Back {
        assert_eq!(
            (stats.triangles, stats.culled, stats.rasterized),
            (12, 6, 6),
            "three faces visible"
        );
    }
    target
}

#[test]
fn cube() {
    let culled = render_cube(CullMode::Back);
    golden!().assert("pipeline_cube", &culled.color, Tolerance::EXACT);
    // A closed convex mesh looks the same without culling (depth test only).
    assert_eq!(render_cube(CullMode::None).color, culled.color);
    // Culling front faces shows the inside of the far faces instead.
    assert_ne!(render_cube(CullMode::Front).color, culled.color);
}

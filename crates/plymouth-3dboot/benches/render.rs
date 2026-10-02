// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering benchmarks (`just bench`): triangle setup, fill rate and full
//! frames of the N64 logo. Native only.

#[cfg(not(target_os = "emscripten"))]
mod benches {
    use std::hint::black_box;

    use criterion::{Criterion, criterion_group};
    use plymouth_3dboot::color::{LinearRgba, Rgba8};
    use plymouth_3dboot::io::MemResolver;
    use plymouth_3dboot::math::{Vec2, Vec4, Viewport};
    use plymouth_3dboot::pipeline::{ClipVertex, FragmentInput, RenderState, Renderer, Shader};
    use plymouth_3dboot::raster::{Rect, TriangleSetup};
    use plymouth_3dboot::target::Framebuffer;
    use plymouth_3dboot::{Format, FrameSettings, Model, WrapMode};

    const OBJ: &str = include_str!("../../../tests/fixtures/n64_logo/n64_logo.obj");
    const MTL: &[u8] = include_bytes!("../../../tests/fixtures/n64_logo/n64_logo.mtl");

    fn n64() -> Model {
        let resolver = MemResolver::new().with("n64_logo.mtl", MTL).unwrap();
        Model::from_source(Format::Obj, OBJ, &resolver)
            .unwrap()
            .with_turntable(plymouth_3dboot::math::Vec3::Y, 6.0)
    }

    /// Clip-space positions with a constant colour.
    struct Flat(Vec<Vec4>);

    impl Shader for Flat {
        type Varyings = ();
        fn vertex(&self, i: u32) -> ClipVertex<()> {
            ClipVertex {
                position: self.0[i as usize],
                varyings: (),
            }
        }
        fn fragment(&self, _: &FragmentInput<()>) -> Option<LinearRgba> {
            Some(LinearRgba::WHITE)
        }
    }

    fn setup(c: &mut Criterion) {
        let tri = [
            Vec2::new(1.3, 2.7),
            Vec2::new(200.1, 30.4),
            Vec2::new(50.5, 180.9),
        ];
        c.bench_function("triangle_setup", |b| {
            b.iter(|| TriangleSetup::new(black_box(tri)))
        });
        let t = TriangleSetup::new(tri).unwrap();
        c.bench_function("coverage_256px_triangle_area", |b| {
            b.iter(|| {
                let mut n = 0u32;
                t.for_each_pixel(Rect::from_size(256, 256), |_| n += 1);
                n
            })
        });
    }

    fn fill(c: &mut Criterion) {
        let quad = Flat(
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(x, y)| Vec4::new(x, y, 0.0, 1.0))
                .to_vec(),
        );
        let mut target = Framebuffer::new(1920, 1080, Rgba8::BLACK).unwrap();
        let state = RenderState::new(Viewport::new(1920, 1080));
        let mut renderer = Renderer::new();
        c.bench_function("fill_1080p_quad", |b| {
            b.iter(|| {
                target.clear(Rgba8::BLACK);
                renderer
                    .draw_indexed(&mut target, &state, &quad, 4, &[0, 1, 2, 0, 2, 3])
                    .unwrap()
            })
        });
    }

    fn frames(c: &mut Criterion) {
        let model = n64();
        for (name, w, h, aa) in [
            ("n64_frame_640x480", 640, 480, 1),
            ("n64_frame_1080p", 1920, 1080, 1),
            ("n64_frame_640x480_aa2", 640, 480, 2),
        ] {
            let settings = FrameSettings {
                antialias: aa,
                ..FrameSettings::new(w, h)
            };
            let mut r = model.renderer(Some(0), WrapMode::Loop, settings).unwrap();
            let mut t = 0.0;
            c.bench_function(name, |b| {
                b.iter(|| {
                    t += 0.01;
                    r.render_at(t).unwrap().width()
                })
            });
        }
    }

    criterion_group!(all, setup, fill, frames);
}

#[cfg(not(target_os = "emscripten"))]
criterion::criterion_main!(benches::all);

#[cfg(target_os = "emscripten")]
fn main() {}

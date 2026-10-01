// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden images for the shading models (Phase 4.6).

use std::collections::BTreeSet;

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::math::{Mat4, Vec3, Viewport};
use plymouth_3dboot::pipeline::{RenderState, Renderer};
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::scene::primitives::uv_sphere;
use plymouth_3dboot::scene::{Camera, Material, Projection};
use plymouth_3dboot::shading::{DrawParams, Lighting, ShadingModel, draw_mesh};
use plymouth_3dboot::target::Framebuffer;
use plymouth_3dboot_testutil::{Tolerance, golden};

const SIZE: u32 = 96;
const BACKGROUND: Rgba8 = Rgba8::new(20, 20, 28, 255);

fn render_sphere(model: ShadingModel, material: &Material) -> Framebuffer {
    let sphere = uv_sphere(1.0, 32, 16);
    let camera = Camera::look_at(
        Vec3::new(0.0, 0.0, 3.5),
        Vec3::ZERO,
        Vec3::Y,
        Projection::Perspective {
            fov_y: 0.8,
            z_near: 0.5,
            z_far: 10.0,
        },
    );
    let lighting = Lighting::default();
    let materials = [material.clone()];
    let params = DrawParams {
        camera: &camera,
        lighting: &lighting,
        model,
        materials: &materials,
    };
    let mut state = RenderState::new(Viewport::new(SIZE, SIZE));
    state.cull = CullMode::Back;
    let mut target = Framebuffer::new(SIZE, SIZE, BACKGROUND).unwrap();
    draw_mesh(
        &mut Renderer::new(),
        &mut target,
        &state,
        &params,
        &sphere,
        Mat4::IDENTITY,
    )
    .unwrap();
    target
}

#[test]
fn unlit_sphere() {
    let material = Material::with_color("orange", Rgba8::new(240, 130, 20, 255).to_linear());
    let image = render_sphere(ShadingModel::Unlit, &material).color;
    let palette: BTreeSet<[u8; 4]> = image.pixels().iter().map(|p| p.to_array()).collect();
    assert_eq!(
        palette,
        BTreeSet::from([BACKGROUND.to_array(), [240, 130, 20, 255]]),
        "unlit output is exactly base colour + background"
    );
    golden!().assert("shading_unlit_sphere", &image, Tolerance::EXACT);
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden renders of the N64 logo sample (Phase 5.5).

use std::collections::BTreeSet;

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::io::FsResolver;
use plymouth_3dboot::io::obj::{ObjOptions, load_obj};
use plymouth_3dboot::math::{Vec3, Viewport};
use plymouth_3dboot::pipeline::{RenderState, Renderer};
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::scene::{Camera, Scene};
use plymouth_3dboot::shading::{DrawParams, Lighting, ShadingModel, draw_mesh};
use plymouth_3dboot::target::Framebuffer;
use plymouth_3dboot_testutil::{Tolerance, golden};

const SIZE: u32 = 256;
const BACKGROUND: Rgba8 = Rgba8::new(0, 0, 0, 255);
/// The colours listed in the model's Readme (sRGB).
const README_COLOURS: [[u8; 3]; 4] = [[6, 147, 48], [2, 34, 169], [255, 24, 19], [255, 192, 1]];
/// Direction from the camera towards the model: a three-quarter view from
/// the front right, slightly above.
pub const VIEW_DIRECTION: Vec3 = Vec3::new(-0.6, -0.45, -1.0);

fn fixture_dir() -> String {
    format!(
        "{}/../../tests/fixtures/n64_logo",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn load_obj_scene() -> Scene {
    let src = std::fs::read_to_string(format!("{}/n64_logo.obj", fixture_dir())).unwrap();
    load_obj(
        &src,
        &FsResolver::new(fixture_dir()),
        &ObjOptions::default(),
    )
    .unwrap()
    .scene
}

/// Renders every mesh node of `scene`, framed from `direction`.
fn render(scene: &Scene, direction: Vec3, model: ShadingModel) -> Framebuffer {
    let world = scene.world_matrices();
    let camera = Camera::framing(
        scene.bounds_with(&world),
        direction,
        Vec3::Y,
        1.0,
        Some(0.7),
    );
    let lighting = Lighting::default();
    let params = DrawParams {
        camera: &camera,
        lighting: &lighting,
        model,
        materials: &scene.materials,
    };
    let mut state = RenderState::new(Viewport::new(SIZE, SIZE));
    state.cull = CullMode::Back;
    let mut target = Framebuffer::new(SIZE, SIZE, BACKGROUND).unwrap();
    let mut renderer = Renderer::new();
    for (node, m) in scene.nodes().iter().zip(&world) {
        if let Some(mesh) = node.mesh {
            draw_mesh(
                &mut renderer,
                &mut target,
                &state,
                &params,
                &scene.meshes()[mesh.0],
                *m,
            )
            .unwrap();
        }
    }
    target
}

fn palette(image: &Framebuffer) -> BTreeSet<[u8; 4]> {
    image.color.pixels().iter().map(|p| p.to_array()).collect()
}

#[test]
fn obj_unlit() {
    let image = render(&load_obj_scene(), VIEW_DIRECTION, ShadingModel::Unlit);
    let expected: BTreeSet<[u8; 4]> = README_COLOURS
        .iter()
        .map(|&[r, g, b]| [r, g, b, 255])
        .chain([BACKGROUND.to_array()])
        .collect();
    assert_eq!(
        palette(&image),
        expected,
        "exactly the four Readme colours plus background"
    );
    golden!().assert("n64_obj_unlit", &image.color, Tolerance::EXACT);
}

#[test]
#[ignore = "exploration helper: writes candidate views to the target tmp dir"]
fn explore_views() {
    let scene = load_obj_scene();
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (i, d) in [
        VIEW_DIRECTION,
        Vec3::new(0.6, -0.45, -1.0),
        Vec3::new(-0.6, -0.45, 1.0),
        Vec3::new(0.6, -0.45, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let image = render(&scene, d, ShadingModel::Lambert);
        std::fs::write(
            out.join(format!("n64_view_{i}.png")),
            plymouth_3dboot::io::png::encode(&image.color).unwrap(),
        )
        .unwrap();
    }
}

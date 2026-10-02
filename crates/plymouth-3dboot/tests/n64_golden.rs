// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden renders of the N64 logo sample (Phase 5.5).

use std::collections::BTreeSet;

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::io::FsResolver;
use plymouth_3dboot::io::collada::{ColladaOptions, load_collada};
use plymouth_3dboot::io::obj::{ObjOptions, load_obj};
use plymouth_3dboot::math::{Vec3, Viewport};
use plymouth_3dboot::pipeline::{RenderState, Renderer};
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::scene::{Camera, Scene};
use plymouth_3dboot::shading::{DrawParams, Lighting, ShadingModel, draw_scene};
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

fn load_dae_scene() -> Scene {
    let text = std::fs::read_to_string(format!("{}/n64_logo.dae", fixture_dir())).unwrap();
    load_collada(
        &text,
        &FsResolver::new(fixture_dir()),
        &ColladaOptions::default(),
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
    draw_scene(
        &mut Renderer::new(),
        &mut target,
        &state,
        &params,
        scene,
        &world,
    )
    .unwrap();
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

/// Intersection over union of the non-background pixels of two renders.
fn silhouette_iou(a: &Framebuffer, b: &Framebuffer) -> f64 {
    let (mut inter, mut union) = (0u32, 0u32);
    for (p, q) in a.color.pixels().iter().zip(b.color.pixels()) {
        let (x, y) = (*p != BACKGROUND, *q != BACKGROUND);
        inter += u32::from(x && y);
        union += u32::from(x || y);
    }
    f64::from(inter) / f64::from(union.max(1))
}

#[test]
fn dae_unlit_matches_obj() {
    let dae = render(&load_dae_scene(), VIEW_DIRECTION, ShadingModel::Unlit);
    let obj = render(&load_obj_scene(), VIEW_DIRECTION, ShadingModel::Unlit);
    assert_eq!(palette(&dae), palette(&obj), "same colours");
    let iou = silhouette_iou(&dae, &obj);
    assert!(iou >= 0.95, "silhouette IoU {iou}");
    golden!().assert("n64_dae_unlit", &dae.color, Tolerance::EXACT);
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

/// Supersampling anti-aliasing softens the edges of the unlit logo.
#[test]
fn obj_unlit_antialiased() {
    use plymouth_3dboot::{FrameSettings, Model, WrapMode};
    let model = Model {
        scene: load_obj_scene(),
        clips: Vec::new(),
        warnings: Vec::new(),
    };
    let settings = FrameSettings {
        antialias: 4,
        ..FrameSettings::new(128, 128)
    };
    let mut renderer = model.renderer(None, WrapMode::Loop, settings).unwrap();
    let image = renderer.render_at(0.0).unwrap();
    let palette: BTreeSet<[u8; 4]> = image.pixels().iter().map(|p| p.to_array()).collect();
    assert!(
        palette.len() > 5,
        "edges blend the four colours and the background"
    );
    golden!().assert("n64_obj_unlit_aa4", image, Tolerance::EXACT);
}

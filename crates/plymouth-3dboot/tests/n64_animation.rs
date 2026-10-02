// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden frames of the N64 logo on a turntable (Phase 8.7).

use plymouth_3dboot::anim::{Clip, WrapMode};
use plymouth_3dboot::io::FsResolver;
use plymouth_3dboot::io::collada::{ColladaOptions, load_collada};
use plymouth_3dboot::io::obj::{ObjOptions, load_obj};
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::render::{AnimationRenderer, FrameSettings, frame_count, frame_time};
use plymouth_3dboot::scene::{LocalTransform, Node, Scene};
use plymouth_3dboot_testutil::{Tolerance, golden};

/// The DAE's scene range: 0 to 3.3333 s at 30 fps.
const PERIOD: f32 = 10.0 / 3.0;
const FPS: f64 = 30.0;

fn turntable_scene() -> (Scene, Clip) {
    let dir = format!(
        "{}/../../tests/fixtures/n64_logo",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(format!("{dir}/n64_logo.obj")).unwrap();
    let scene = load_obj(&src, &FsResolver::new(&dir), &ObjOptions::default())
        .unwrap()
        .scene;
    let (scene, root) = scene.wrapped_in_root(Node::new("turntable", LocalTransform::default()));
    (scene, Clip::turntable(root, Vec3::Y, PERIOD))
}

#[test]
fn turntable_frames() {
    let (scene, clip) = turntable_scene();
    assert_eq!(frame_count(f64::from(clip.duration()), FPS), 100);
    let mut renderer = AnimationRenderer::new(
        &scene,
        Some((&clip, WrapMode::Loop)),
        FrameSettings::new(128, 128),
    )
    .unwrap();
    let first = renderer.render_at(frame_time(0.0, 0, FPS)).unwrap().clone();
    for frame in [0, 25, 50, 75] {
        let image = renderer.render_at(frame_time(0.0, frame, FPS)).unwrap();
        golden!().assert(
            &format!("n64_turntable_{frame:03}"),
            image,
            Tolerance::EXACT,
        );
    }
    // The loop is seamless: frame 100 (one period later) shows frame 0.
    assert_eq!(
        renderer.render_at(frame_time(0.0, 100, FPS)).unwrap(),
        &first
    );
}

/// The spin authored in `n64_logo_spin.dae` reproduces the procedural
/// turntable: the same colours in nearly every pixel of every golden frame
/// (exact equality is not expected: the DAE and OBJ exports differ in the
/// last digits of their coordinates).
#[test]
fn collada_spin_matches_procedural_turntable() {
    let dir = format!(
        "{}/../../tests/fixtures/n64_logo",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(format!("{dir}/n64_logo_spin.dae")).unwrap();
    let model = load_collada(&text, &FsResolver::new(&dir), &ColladaOptions::default()).unwrap();
    assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    assert_eq!(model.clips.len(), 1);
    let clip = &model.clips[0];
    assert!((clip.duration() - PERIOD).abs() < 1e-5);
    let mut renderer = AnimationRenderer::new(
        &model.scene,
        Some((clip, WrapMode::Loop)),
        FrameSettings::new(128, 128),
    )
    .unwrap();
    let goldens = format!("{}/tests/golden", env!("CARGO_MANIFEST_DIR"));
    for frame in [0, 25, 50, 75] {
        let image = renderer.render_at(frame_time(0.0, frame, FPS)).unwrap();
        let golden = plymouth_3dboot::io::png::decode(
            &std::fs::read(format!("{goldens}/n64_turntable_{frame:03}.png")).unwrap(),
        )
        .unwrap();
        let same = image
            .pixels()
            .iter()
            .zip(golden.pixels())
            .filter(|(a, b)| a == b)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let agreement = same as f64 / image.pixels().len() as f64;
        assert!(
            agreement >= 0.999,
            "frame {frame}: only {agreement:.4} of pixels agree"
        );
    }
}

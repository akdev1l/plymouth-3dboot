// SPDX-License-Identifier: GPL-3.0-or-later
//! COLLADA scene loading (Phase 7.4).

use plymouth_3dboot::io::MemResolver;
use plymouth_3dboot::io::collada::{ColladaOptions, load_collada};
use plymouth_3dboot::io::obj::{ObjOptions, load_obj};
use plymouth_3dboot::math::{Mat4, Vec3};
use plymouth_3dboot::scene::LocalTransform;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/n64_logo");

fn n64_dae() -> String {
    std::fs::read_to_string(format!("{FIXTURES}/n64_logo.dae")).unwrap()
}

/// A single triangle under a node with the given transform elements, in a
/// document with the given asset block.
fn doc(asset: &str, transforms: &str) -> String {
    format!(
        r##"<COLLADA><asset>{asset}</asset>
<library_geometries><geometry id="g"><mesh>
  <source id="p"><float_array id="pa" count="9">0 0 0 1 0 0 0 1 0</float_array><technique_common><accessor source="#pa" count="3" stride="3"/></technique_common></source>
  <vertices id="v"><input semantic="POSITION" source="#p"/></vertices>
  <triangles count="1"><input semantic="VERTEX" source="#v" offset="0"/><p>0 1 2</p></triangles>
</mesh></geometry></library_geometries>
<library_visual_scenes><visual_scene id="vs"><node id="n" name="Tri">{transforms}<instance_geometry url="#g"/></node></visual_scene></library_visual_scenes>
<scene><instance_visual_scene url="#vs"/></scene></COLLADA>"##
    )
}

fn load(text: &str, options: ColladaOptions) -> plymouth_3dboot::io::collada::ColladaModel {
    load_collada(text, &MemResolver::new(), &options).unwrap()
}

/// World position of mesh vertex 1 (at local (1, 0, 0)).
fn world_vertex(model: &plymouth_3dboot::io::collada::ColladaModel) -> Vec3 {
    let world = model.scene.world_matrices();
    let (i, node) = model
        .scene
        .nodes()
        .iter()
        .enumerate()
        .find(|(_, n)| n.mesh.is_some())
        .unwrap();
    world[i].transform_point3(model.scene.meshes()[node.mesh.unwrap().0].positions()[1])
}

#[test]
fn transforms_apply_in_document_order() {
    let m = load(
        &doc(
            "",
            r#"<translate sid="t">0 0 5</translate><rotate sid="r">0 0 1 90</rotate><scale>2 2 2</scale>"#,
        ),
        ColladaOptions::default(),
    );
    let p = world_vertex(&m);
    assert!(p.abs_diff_eq(Vec3::new(0.0, 2.0, 5.0), 1e-5), "{p}");
    // Transform stacks keep their sids for animation.
    let tri = m.scene.find_node("Tri").unwrap();
    let LocalTransform::Stack(ops) = &m.scene.nodes()[tri.0].transform else {
        panic!("stack expected")
    };
    assert_eq!(
        ops.iter().map(|o| o.sid.as_str()).collect::<Vec<_>>(),
        ["t", "r", ""]
    );
}

#[test]
fn matrix_is_row_major() {
    let m = load(
        &doc("", "<matrix>1 0 0 7  0 1 0 8  0 0 1 9  0 0 0 1</matrix>"),
        ColladaOptions::default(),
    );
    assert!(world_vertex(&m).abs_diff_eq(Vec3::new(8.0, 8.0, 9.0), 1e-6));
}

#[test]
fn z_up_and_units_are_converted() {
    let text = doc(
        r#"<unit meter="0.01"/><up_axis>Z_UP</up_axis>"#,
        "<translate>0 0 100</translate>",
    );
    // Local (1,0,0) + (0,0,100) in centimetres, Z up -> (0.01, 1, 0) metres, Y up.
    let p = world_vertex(&load(&text, ColladaOptions::default()));
    assert!(p.abs_diff_eq(Vec3::new(0.01, 1.0, 0.0), 1e-6), "{p}");
    let raw = world_vertex(&load(
        &text,
        ColladaOptions {
            convert_up_axis: false,
            convert_units: false,
        },
    ));
    assert!(raw.abs_diff_eq(Vec3::new(1.0, 0.0, 100.0), 1e-6), "{raw}");
}

#[test]
fn x_up_is_converted() {
    let p = world_vertex(&load(
        &doc("<up_axis>X_UP</up_axis>", ""),
        ColladaOptions::default(),
    ));
    assert!(p.abs_diff_eq(Vec3::Y, 1e-6), "+X becomes +Y: {p}");
}

#[test]
fn unbound_materials_and_unsupported_instances_warn() {
    let text = doc("", "")
        .replace(
            r#"<triangles count="1">"#,
            r#"<triangles count="1" material="sym">"#,
        )
        .replace(
            "<instance_geometry",
            r##"<instance_camera url="#c"/><instance_geometry"##,
        );
    let m = load(&text, ColladaOptions::default());
    assert_eq!(m.scene.materials.len(), 1, "default material");
    assert_eq!(m.warnings.len(), 2, "{:?}", m.warnings);
}

#[test]
fn cyclic_instance_nodes_terminate() {
    let text = r##"<COLLADA><library_nodes><node id="loop"><instance_node url="#loop"/></node></library_nodes>
        <library_visual_scenes><visual_scene id="vs"><node><instance_node url="#loop"/></node></visual_scene></library_visual_scenes>
        <scene><instance_visual_scene url="#vs"/></scene></COLLADA>"##;
    let m = load(text, ColladaOptions::default());
    assert!(
        m.warnings.iter().any(|w| w.contains("cyclic")),
        "{:?}",
        m.warnings
    );
}

#[test]
fn exponential_instance_node_fan_out_is_rejected_quickly() {
    // Each level instances the next one twice: 2^40 nodes if expanded.
    let mut nodes = String::new();
    for level in 0..40 {
        nodes.push_str(&format!(r##"<node id="n{level}"><instance_node url="#n{next}"/><instance_node url="#n{next}"/></node>"##, next = level + 1));
    }
    nodes.push_str(r#"<node id="n40"/>"#);
    let text = format!(
        r##"<COLLADA><library_nodes>{nodes}</library_nodes><library_visual_scenes><visual_scene id="vs"><node><instance_node url="#n0"/></node></visual_scene></library_visual_scenes><scene><instance_visual_scene url="#vs"/></scene></COLLADA>"##
    );
    let start = std::time::Instant::now();
    let err = load_collada(&text, &MemResolver::new(), &ColladaOptions::default()).unwrap_err();
    assert!(
        matches!(err, plymouth_3dboot::io::collada::ColladaError::TooLarge(_)),
        "{err}"
    );
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn errors_are_reported() {
    assert!(
        load_collada(
            "<COLLADA><scene>",
            &MemResolver::new(),
            &ColladaOptions::default()
        )
        .is_err()
    );
    let dangling = doc("", "").replace(r##"url="#g""##, r##"url="#missing""##);
    assert!(load_collada(&dangling, &MemResolver::new(), &ColladaOptions::default()).is_err());
}

#[test]
fn n64_logo_scene() {
    let m = load(&n64_dae(), ColladaOptions::default());
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let s = &m.scene;
    assert_eq!(s.meshes().len(), 1);
    assert_eq!(s.meshes()[0].triangle_count(), 96);
    assert_eq!(s.materials.len(), 4);
    // collada root -> N64 -> N64_PIVOT -> geometry holder.
    let names: Vec<&str> = s.nodes().iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["collada", "N64", "N64_PIVOT", "N64_PIVOT#geometry"]);

    // Hand-computed world matrix of the geometry: up-axis and unit
    // conversion, then the N64 node's translate/rotate/scale and the pivot.
    let expected = Mat4::from_scale(Vec3::splat(0.0254))
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_translation(Vec3::new(-0.006047, 0.197258, 28.7307))
        * Mat4::from_axis_angle(Vec3::NEG_X, (-90f32).to_radians())
        * Mat4::from_scale(Vec3::new(1.18181, 0.896433, 1.16447))
        * Mat4::from_translation(Vec3::new(0.022241, -32.05, 10.756));
    let world = s.world_matrices();
    assert!(
        world[3].abs_diff_eq(expected, 1e-5),
        "{}\nvs\n{expected}",
        world[3]
    );
}

/// 3ds Max's OBJ export bakes node transforms and converts to Y-up, so the
/// DAE scene (after conversion) must match the OBJ positions in metres.
#[test]
fn n64_logo_dae_matches_obj_geometry() {
    let dae = load(&n64_dae(), ColladaOptions::default()).scene;
    let obj_src = std::fs::read_to_string(format!("{FIXTURES}/n64_logo.obj")).unwrap();
    let obj = load_obj(&obj_src, &MemResolver::new(), &ObjOptions::default())
        .unwrap()
        .scene;
    let (a, b) = (dae.bounds(), obj.bounds());
    let b_m = plymouth_3dboot::math::Aabb::new(b.min * 0.0254, b.max * 0.0254);
    assert!(
        a.min.abs_diff_eq(b_m.min, 2e-3) && a.max.abs_diff_eq(b_m.max, 2e-3),
        "DAE {a:?}\nOBJ {b_m:?}"
    );
    // Upright: tallest along +Y from the ground (y >= 0) like the OBJ.
    assert!(a.min.y.abs() < 2e-3);
}

// --- Animation (Phase 9) ---------------------------------------------------

use plymouth_3dboot::anim::Pose;

fn anim_fixture(name: &str) -> plymouth_3dboot::io::collada::ColladaModel {
    let path = format!(
        "{}/../../tests/fixtures/collada_anim/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    load(
        &std::fs::read_to_string(path).unwrap(),
        ColladaOptions::default(),
    )
}

/// World position of the first mesh vertex at local `p` under node `name`.
fn posed(model: &plymouth_3dboot::io::collada::ColladaModel, name: &str, t: f32, p: Vec3) -> Vec3 {
    let node = model.scene.find_node(name).unwrap();
    let world = Pose::evaluate(&model.scene, &model.clips[0], t).world(&model.scene);
    world[node.0].transform_point3(p)
}

#[test]
fn n64_logo_has_no_animation() {
    let m = load(&n64_dae(), ColladaOptions::default());
    assert!(m.clips.is_empty());
}

#[test]
fn rotate_angle_channel() {
    let m = anim_fixture("rotate_y.dae");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.clips.len(), 1);
    assert_eq!(m.clips[0].duration(), 4.0);
    for (t, expected) in [
        (0.0, Vec3::X),
        (1.0, Vec3::NEG_Z),
        (2.0, Vec3::NEG_X),
        (3.0, Vec3::Z),
        (0.5, Vec3::new(0.70710677, 0.0, -0.70710677)),
    ] {
        let p = posed(&m, "spinner", t, Vec3::X);
        assert!(p.abs_diff_eq(expected, 1e-5), "t = {t}: {p}");
    }
}

#[test]
fn matrix_channel_with_step_interpolation() {
    let m = anim_fixture("matrix_step.dae");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    for (t, x) in [(0.0, 0.0), (0.99, 0.0), (1.0, 2.0), (1.5, 2.0), (2.0, 0.0)] {
        assert!(
            posed(&m, "hopper", t, Vec3::ZERO).abs_diff_eq(Vec3::new(x, 0.0, 0.0), 1e-6),
            "t = {t}"
        );
    }
}

#[test]
fn component_and_vector_channels_in_nested_animations() {
    let m = anim_fixture("translate_x.dae");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.clips[0].duration(), 2.0);
    assert!(posed(&m, "slider", 0.5, Vec3::ZERO).abs_diff_eq(Vec3::new(1.5, 0.0, 0.0), 1e-6));
    // The child inherits the slide and adds its own lift.
    assert!(posed(&m, "child", 1.0, Vec3::ZERO).abs_diff_eq(Vec3::new(3.0, 2.0, 0.0), 1e-6));
}

#[test]
fn bad_channels_are_warnings() {
    let text = std::fs::read_to_string(format!(
        "{}/../../tests/fixtures/collada_anim/rotate_y.dae",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    for (from, to, expect) in [
        (
            "spinner/rotateY.ANGLE",
            "ghost/rotateY.ANGLE",
            "not in the scene",
        ),
        (
            "spinner/rotateY.ANGLE",
            "spinner/nosuch.ANGLE",
            "no transform with sid",
        ),
        (
            "spinner/rotateY.ANGLE",
            "spinner/translate.ANGLE",
            "unsupported target",
        ),
        (
            "spinner/rotateY.ANGLE",
            "spinner/a/b",
            "unsupported channel target",
        ),
    ] {
        let m = load(&text.replace(from, to), ColladaOptions::default());
        assert!(m.clips.is_empty());
        assert!(
            m.warnings.iter().any(|w| w.contains(expect)),
            "{to}: {:?}",
            m.warnings
        );
    }
    // Interpolations other than STEP/LINEAR are approximated (until BEZIER support).
    let m = load(
        &text.replace(
            "LINEAR LINEAR LINEAR LINEAR LINEAR",
            "CARDINAL CARDINAL CARDINAL CARDINAL CARDINAL",
        ),
        ColladaOptions::default(),
    );
    assert_eq!(m.clips.len(), 1);
    assert!(
        m.warnings.iter().any(|w| w.contains("approximated")),
        "{:?}",
        m.warnings
    );
}

#[test]
fn bezier_channels_become_cubic_splines() {
    let m = anim_fixture("translate_bezier.dae");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    // Ease-in-out with flat tangents: x(u) = 3 (3u^2 - 2u^3) on each half.
    for (t, x) in [
        (0.0, 0.0),
        (0.25, 0.46875),
        (0.5, 1.5),
        (1.0, 3.0),
        (1.5, 1.5),
        (1.75, 0.46875),
    ] {
        let p = posed(&m, "easer", t, Vec3::ZERO);
        assert!((p.x - x).abs() < 2e-5, "t = {t}: {} vs {x}", p.x);
    }
}

#[test]
fn animation_clips_select_channels_and_ranges() {
    let m = anim_fixture("clips.dae");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let names: Vec<&str> = m.clips.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["slide", "lift"]);
    let (slide, lift) = (&m.clips[0], &m.clips[1]);
    assert_eq!(
        (slide.channels.len(), slide.start(), slide.duration()),
        (1, 0.0, 1.0)
    );
    assert_eq!(
        (lift.channels.len(), lift.start(), lift.duration()),
        (1, 0.5, 1.5)
    );
    // Playing `lift` from 0 starts at key time 0.5 (a quarter of the lift).
    let child = m.scene.find_node("child").unwrap();
    let t = lift.local_time(0.0, plymouth_3dboot::anim::WrapMode::Loop);
    let world = Pose::evaluate(&m.scene, lift, t).world(&m.scene);
    assert!(
        world[child.0]
            .transform_point3(Vec3::ZERO)
            .abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6)
    );
}

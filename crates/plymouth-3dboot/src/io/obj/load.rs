// SPDX-License-Identifier: GPL-3.0-or-later
//! Conversion of parsed OBJ/MTL data into a [`Scene`].

use std::collections::HashMap;

use super::mtl::{MtlData, parse_mtl};
use super::parse::{ObjData, ObjError, parse_obj};
use crate::io::{ResolveError, ResourceResolver};
use crate::math::{Vec2, Vec3};
use crate::scene::triangulate::triangulate_polygon;
use crate::scene::{LocalTransform, Material, MaterialId, Mesh, MeshError, Node, Scene, Submesh};

/// How OBJ smoothing-group numbers are interpreted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SmoothingGroups {
    /// Groups are bit masks (as 3ds Max writes them): faces are smoothed
    /// together when their groups share a bit. For the common `s 1` /
    /// `s off` files this is the same as [`SmoothingGroups::Exact`].
    #[default]
    BitMask,
    /// Faces are smoothed together only when their group numbers are equal.
    Exact,
}

/// Options for [`load_obj`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjOptions {
    /// Smoothing-group interpretation.
    pub smoothing: SmoothingGroups,
}

/// A loaded model and the problems that did not prevent loading.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjModel {
    /// The scene: one root node holding one mesh.
    pub scene: Scene,
    /// Human-readable warnings (ignored directives, missing materials, ...).
    pub warnings: Vec<String>,
}

/// Loading an OBJ model failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ObjLoadError {
    /// The OBJ text is malformed.
    #[error("OBJ: {0}")]
    Obj(#[from] ObjError),
    /// A material library is malformed.
    #[error("material library {library}: {error}")]
    Mtl {
        /// Library name.
        library: String,
        /// Parse error.
        error: ObjError,
    },
    /// A material library exists but could not be read.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// A material library is not valid UTF-8.
    #[error("material library {0} is not valid UTF-8")]
    Utf8(String),
    /// The geometry could not form a valid mesh (e.g. too many vertices).
    #[error(transparent)]
    Mesh(#[from] MeshError),
}

/// Loads an OBJ model from its source text, fetching material libraries
/// through `resolver`.
///
/// Polygons are triangulated. Normals come from the file when present, and
/// are otherwise generated per smoothing group (group 0 gives flat faces).
/// Triangles are grouped into one submesh per material. Faces without
/// `usemtl`, or whose material is missing, use a default material. OBJ
/// coordinates are taken as-is (Y-up; no unit conversion).
///
/// # Errors
///
/// Returns [`ObjLoadError`] for malformed OBJ or MTL text, unreadable
/// libraries, or meshes that cannot be represented. A missing library or
/// material is only a warning.
pub fn load_obj(
    source: &str,
    resolver: &impl ResourceResolver,
    options: &ObjOptions,
) -> Result<ObjModel, ObjLoadError> {
    let data = parse_obj(source)?;
    let mut warnings: Vec<String> = data
        .warnings
        .iter()
        .map(|w| format!("OBJ line {}: {}", w.line, w.message))
        .collect();
    let libraries = load_libraries(&data.material_libraries, resolver, &mut warnings)?;
    let (materials, face_material) = resolve_materials(&data, &libraries, &mut warnings);
    let mesh = build_mesh(&data, &face_material, options)?;
    let mut scene = Scene::new();
    scene.materials = materials;
    let mesh = scene.add_mesh(mesh);
    scene
        .add_node(
            None,
            Node::new("obj", LocalTransform::default()).with_mesh(mesh),
        )
        .expect("mesh exists");
    Ok(ObjModel { scene, warnings })
}

fn load_libraries(
    names: &[String],
    resolver: &impl ResourceResolver,
    warnings: &mut Vec<String>,
) -> Result<Vec<MtlData>, ObjLoadError> {
    let mut out = Vec::new();
    for raw in names {
        // Exporters write names containing spaces; try the whole argument
        // first, then each whitespace-separated name.
        let candidates: Vec<&str> = std::iter::once(raw.as_str())
            .chain(
                raw.split_whitespace()
                    .filter(|_| raw.contains(char::is_whitespace)),
            )
            .collect();
        let mut found = false;
        for name in candidates {
            match resolver.resolve(name) {
                Ok(bytes) => {
                    let text = String::from_utf8(bytes)
                        .map_err(|_| ObjLoadError::Utf8(name.to_owned()))?;
                    let mtl = parse_mtl(&text).map_err(|error| ObjLoadError::Mtl {
                        library: name.to_owned(),
                        error,
                    })?;
                    warnings.extend(
                        mtl.warnings
                            .iter()
                            .map(|w| format!("{name} line {}: {}", w.line, w.message)),
                    );
                    out.push(mtl);
                    found = true;
                    if name == raw {
                        break;
                    }
                }
                Err(ResolveError::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if !found {
            warnings.push(format!("material library {raw:?} not found"));
        }
    }
    Ok(out)
}

/// Builds the material list and maps each face to a material index.
fn resolve_materials(
    data: &ObjData,
    libraries: &[MtlData],
    warnings: &mut Vec<String>,
) -> (Vec<Material>, Vec<usize>) {
    let mut materials: Vec<Material> = data
        .materials
        .iter()
        .map(|name| {
            libraries
                .iter()
                .flat_map(|l| &l.materials)
                .find(|m| &m.name == name)
                .map_or_else(
                    || {
                        warnings.push(format!(
                            "material {name:?} not found; using a default material"
                        ));
                        Material {
                            name: name.clone(),
                            ..Material::default()
                        }
                    },
                    super::mtl::MtlMaterial::to_material,
                )
        })
        .collect();
    let mut default_index = None;
    let face_material = data
        .faces
        .iter()
        .map(|f| {
            f.material.unwrap_or_else(|| {
                *default_index.get_or_insert_with(|| {
                    materials.push(Material::default());
                    materials.len() - 1
                })
            })
        })
        .collect();
    (materials, face_material)
}

fn shares_smoothing(a: u32, b: u32, mode: SmoothingGroups) -> bool {
    match mode {
        SmoothingGroups::BitMask => a & b != 0,
        SmoothingGroups::Exact => a == b && a != 0,
    }
}

/// Interior angle at `at` between the directions to `prev` and `next`.
fn angle(at: Vec3, prev: Vec3, next: Vec3) -> f32 {
    match ((prev - at).try_normalize(), (next - at).try_normalize()) {
        (Some(a), Some(b)) => libm::acosf(a.dot(b).clamp(-1.0, 1.0)),
        _ => 0.0,
    }
}

fn build_mesh(
    data: &ObjData,
    face_material: &[usize],
    options: &ObjOptions,
) -> Result<Mesh, ObjLoadError> {
    let pos = |i: u32| data.positions[i as usize];
    // Triangulate every face: (face index, three corner indices into the face).
    let mut triangles: Vec<(usize, [usize; 3])> = Vec::new();
    for (fi, face) in data.faces.iter().enumerate() {
        let points: Vec<Vec3> = face.vertices.iter().map(|v| pos(v.position)).collect();
        triangles.extend(triangulate_polygon(&points).into_iter().map(|t| (fi, t)));
    }
    // Unit normal of each triangle and, per position, the angle-weighted
    // contributions of the triangles touching it, tagged with their face.
    let tri_normal: Vec<Vec3> = triangles
        .iter()
        .map(|&(fi, [a, b, c])| {
            let v = &data.faces[fi].vertices;
            let (a, b, c) = (pos(v[a].position), pos(v[b].position), pos(v[c].position));
            (b - a).cross(c - a).try_normalize().unwrap_or(Vec3::Y)
        })
        .collect();
    let mut at_position: HashMap<u32, Vec<(usize, Vec3)>> = HashMap::new();
    for (ti, &(fi, corners)) in triangles.iter().enumerate() {
        let v = &data.faces[fi].vertices;
        for k in 0..3 {
            let (here, prev, next) = (
                v[corners[k]].position,
                v[corners[(k + 2) % 3]].position,
                v[corners[(k + 1) % 3]].position,
            );
            at_position
                .entry(here)
                .or_default()
                .push((fi, tri_normal[ti] * angle(pos(here), pos(prev), pos(next))));
        }
    }

    // Order triangles by material (stable), then emit deduplicated vertices.
    let mut order: Vec<usize> = (0..triangles.len()).collect();
    order.sort_by_key(|&ti| face_material[triangles[ti].0]);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let has_uvs = data
        .faces
        .iter()
        .all(|f| f.vertices.iter().all(|v| v.texcoord.is_some()))
        && !data.faces.is_empty();
    let mut indices = Vec::with_capacity(triangles.len() * 3);
    let mut seen: HashMap<(u32, Option<u32>, [u32; 3]), u32> = HashMap::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    for ti in order {
        let (fi, corners) = triangles[ti];
        let face = &data.faces[fi];
        let material = MaterialId(face_material[fi]);
        if submeshes.last().is_none_or(|s| s.material != material) {
            submeshes.push(Submesh {
                material,
                indices: indices.len()..indices.len(),
            });
        }
        for k in corners {
            let v = face.vertices[k];
            let normal = match v.normal {
                Some(n) => data.normals[n as usize].normalize_or(Vec3::Y),
                None if face.smoothing_group == 0 => {
                    // Flat: the angle-weighted normal over this face only.
                    let sum: Vec3 = at_position[&v.position]
                        .iter()
                        .filter(|(f, _)| *f == fi)
                        .map(|(_, n)| *n)
                        .sum();
                    sum.normalize_or(Vec3::Y)
                }
                None => {
                    let g = face.smoothing_group;
                    let sum: Vec3 = at_position[&v.position]
                        .iter()
                        .filter(|(f, _)| {
                            *f == fi
                                || shares_smoothing(
                                    g,
                                    data.faces[*f].smoothing_group,
                                    options.smoothing,
                                )
                        })
                        .map(|(_, n)| *n)
                        .sum();
                    sum.normalize_or(Vec3::Y)
                }
            };
            let texcoord = if has_uvs { v.texcoord } else { None };
            let key = (
                v.position,
                texcoord,
                (normal + Vec3::ZERO).to_array().map(f32::to_bits),
            );
            let index = match seen.get(&key) {
                Some(&i) => i,
                None => {
                    let i = u32::try_from(positions.len())
                        .map_err(|_| MeshError::TooManyVertices(positions.len()))?;
                    positions.push(pos(v.position));
                    normals.push(normal);
                    if let Some(t) = texcoord {
                        uvs.push(data.texcoords[t as usize]);
                    }
                    seen.insert(key, i);
                    i
                }
            };
            indices.push(index);
        }
        if let Some(last) = submeshes.last_mut() {
            last.indices.end = indices.len();
        }
    }
    let mut mesh = Mesh::new(positions, indices)?
        .with_normals(normals)?
        .with_submeshes(submeshes)?;
    if has_uvs {
        mesh = mesh.with_uvs(uvs)?;
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{FsResolver, MemResolver};
    use crate::math::Aabb;

    fn load(src: &str) -> ObjModel {
        load_obj(src, &MemResolver::new(), &ObjOptions::default()).unwrap()
    }

    fn mesh(m: &ObjModel) -> &Mesh {
        &m.scene.meshes()[0]
    }

    /// Two quads sharing an edge along x = 1 at a right angle.
    const HINGE: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 1 0 -1\nv 1 1 -1\n";

    #[test]
    fn quads_are_triangulated_with_flat_normals_without_smoothing() {
        let m = load(&format!("{HINGE}f 1 2 3 4\nf 2 5 6 3\n"));
        let mesh = mesh(&m);
        assert_eq!(mesh.triangle_count(), 4);
        // Shared edge vertices are split: two normals per shared position.
        assert_eq!(mesh.positions().len(), 8);
        let n = mesh.normals().unwrap();
        assert!(n.iter().all(|v| *v == Vec3::Z || *v == Vec3::X), "{n:?}");
    }

    #[test]
    fn smoothing_group_shares_normals_across_faces() {
        let m = load(&format!("{HINGE}s 1\nf 1 2 3 4\nf 2 5 6 3\n"));
        let mesh = mesh(&m);
        assert_eq!(mesh.positions().len(), 6, "shared edge vertices are welded");
        let diag = (Vec3::X + Vec3::Z).normalize();
        for (p, n) in mesh.positions().iter().zip(mesh.normals().unwrap()) {
            if p.x == 1.0 && p.z == 0.0 {
                assert!(n.abs_diff_eq(diag, 1e-6), "edge normal {n}");
            }
        }
    }

    #[test]
    fn smoothing_group_bitmask_versus_exact() {
        let src = format!("{HINGE}s 2\nf 1 2 3 4\ns 6\nf 2 5 6 3\n");
        assert_eq!(mesh(&load(&src)).positions().len(), 6, "2 & 6 share a bit");
        let exact = load_obj(
            &src,
            &MemResolver::new(),
            &ObjOptions {
                smoothing: SmoothingGroups::Exact,
            },
        )
        .unwrap();
        assert_eq!(mesh(&exact).positions().len(), 8, "2 != 6");
    }

    #[test]
    fn explicit_normals_and_uvs_are_kept() {
        let src =
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 2\nf 1/1/1 2/2/1 3/3/1\n";
        let mesh = mesh(&load(src)).clone();
        assert_eq!(mesh.normals().unwrap(), &[Vec3::Z; 3]);
        assert_eq!(mesh.uvs().unwrap(), &[Vec2::ZERO, Vec2::X, Vec2::Y]);
    }

    #[test]
    fn materials_become_submeshes_in_material_order() {
        let mtl = "newmtl red\nKd 1 0 0\nnewmtl blue\nKd 0 0 1\n";
        let resolver = MemResolver::new()
            .with("lib.mtl", mtl.as_bytes().to_vec())
            .unwrap();
        let src = format!(
            "{HINGE}mtllib lib.mtl\nusemtl blue\nf 1 2 3\nusemtl red\nf 1 3 4\nusemtl blue\nf 2 5 6\n"
        );
        let m = load_obj(&src, &resolver, &ObjOptions::default()).unwrap();
        let names: Vec<&str> = m.scene.materials.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["blue", "red"]);
        let subs = mesh(&m).submeshes();
        assert_eq!(
            subs,
            &[
                Submesh {
                    material: MaterialId(0),
                    indices: 0..6
                },
                Submesh {
                    material: MaterialId(1),
                    indices: 6..9
                }
            ]
        );
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn missing_library_and_material_are_warnings() {
        let src = format!("{HINGE}mtllib nowhere.mtl\nusemtl ghost\nf 1 2 3\nf 1 3 4\n");
        let m = load(&src);
        assert_eq!(m.scene.materials.len(), 1);
        assert_eq!(m.scene.materials[0].name, "ghost");
        assert!(
            m.warnings.iter().any(|w| w.contains("nowhere.mtl")),
            "{:?}",
            m.warnings
        );
        assert!(
            m.warnings.iter().any(|w| w.contains("ghost")),
            "{:?}",
            m.warnings
        );
    }

    #[test]
    fn faces_without_material_get_a_default_one() {
        let m = load(&format!("{HINGE}f 1 2 3\nusemtl x\nf 1 3 4\n"));
        assert_eq!(m.scene.materials.len(), 2, "x plus the default");
        assert_eq!(mesh(&m).submeshes().len(), 2);
    }

    #[test]
    fn malformed_library_is_an_error() {
        let resolver = MemResolver::new()
            .with("bad.mtl", b"Kd 1 1 1\n".to_vec())
            .unwrap();
        let err = load_obj(
            &format!("{HINGE}mtllib bad.mtl\nf 1 2 3\n"),
            &resolver,
            &ObjOptions::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, ObjLoadError::Mtl { ref library, .. } if library == "bad.mtl"),
            "{err}"
        );
    }

    #[test]
    fn n64_logo() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/n64_logo");
        let src = std::fs::read_to_string(format!("{dir}/n64_logo.obj")).unwrap();
        let m = load_obj(&src, &FsResolver::new(dir), &ObjOptions::default()).unwrap();
        let mesh = mesh(&m);
        assert_eq!(mesh.triangle_count(), 96);
        assert_eq!(m.scene.materials.len(), 4);
        assert_eq!(mesh.submeshes().len(), 4);
        let b = m.scene.bounds();
        let expected = Aabb::new(
            Vec3::new(-29.98, 0.0, -30.26),
            Vec3::new(29.97, 57.46, 29.87),
        );
        assert!(
            b.min.abs_diff_eq(expected.min, 0.01) && b.max.abs_diff_eq(expected.max, 0.01),
            "{b:?}"
        );
        assert!(
            mesh.normals()
                .unwrap()
                .iter()
                .all(|n| (n.length() - 1.0).abs() < 1e-5)
        );
        // Only the unsupported `g` line and MTL extras produce warnings.
        assert!(
            m.warnings.iter().all(|w| w.contains("ignored directive")),
            "{:?}",
            m.warnings
        );
    }
}

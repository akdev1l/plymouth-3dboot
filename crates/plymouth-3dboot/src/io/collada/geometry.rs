// SPDX-License-Identifier: GPL-3.0-or-later
//! `<geometry><mesh>`: sources, vertices and primitives.

use std::collections::HashMap;

use roxmltree::Node;

use super::ColladaError;
use super::xml::{Document, attr, child, children, numbers, require};
use crate::math::{Vec2, Vec3};
use crate::scene::triangulate::triangulate_polygon;
use crate::scene::{MaterialId, Mesh, MeshError, Submesh};

/// A geometry's mesh with material *symbols* still unresolved: submesh
/// `MaterialId(i)` refers to `symbols[i]`, to be bound per instance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub(crate) mesh: Mesh,
    pub(crate) symbols: Vec<String>,
}

/// One input of a primitive: where its index sits in each `<p>` tuple.
#[derive(Clone, Debug)]
struct Input {
    semantic: String,
    offset: usize,
    set: u32,
    source: String,
}

/// Reads an accessor's elements (the first `width` components of each).
fn read_source(doc: &Document<'_>, uri: &str, width: usize) -> Result<Vec<Vec<f32>>, ColladaError> {
    let source = doc.by_uri(uri)?;
    let accessor = require(doc, require(doc, source, "technique_common")?, "accessor")?;
    let array = doc.by_uri(attr(doc, accessor, "source")?)?;
    let values: Vec<f32> = numbers(doc, array)?;
    let parse = |name: &str, default: usize| -> Result<usize, ColladaError> {
        accessor.attribute(name).map_or(Ok(default), |v| {
            v.parse().map_err(|_| ColladaError::InvalidNumber {
                text: v.to_owned(),
                line: doc.line(accessor),
            })
        })
    };
    let count = parse("count", 0)?;
    let stride = parse("stride", 1)?;
    let offset = parse("offset", 0)?;
    if stride < width || offset + count.saturating_sub(1) * stride + width > values.len() {
        return Err(ColladaError::Count {
            expected: offset + count * stride,
            actual: values.len(),
            line: doc.line(accessor),
        });
    }
    Ok((0..count)
        .map(|i| values[offset + i * stride..][..width].to_vec())
        .collect())
}

fn vec3s(v: Vec<Vec<f32>>) -> Vec<Vec3> {
    v.into_iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect()
}

fn vec2s(v: Vec<Vec<f32>>) -> Vec<Vec2> {
    v.into_iter().map(|c| Vec2::new(c[0], c[1])).collect()
}

fn inputs(doc: &Document<'_>, primitive: Node<'_, '_>) -> Result<Vec<Input>, ColladaError> {
    children(primitive, "input")
        .map(|i| {
            Ok(Input {
                semantic: attr(doc, i, "semantic")?.to_owned(),
                offset: attr(doc, i, "offset")?.parse().map_err(|_| {
                    ColladaError::InvalidNumber {
                        text: i.attribute("offset").unwrap_or("").to_owned(),
                        line: doc.line(i),
                    }
                })?,
                set: i.attribute("set").and_then(|s| s.parse().ok()).unwrap_or(0),
                source: attr(doc, i, "source")?.to_owned(),
            })
        })
        .collect()
}

/// Per-vertex attribute streams of a mesh, indexed by their own indices.
#[derive(Default)]
struct Streams {
    positions: Vec<Vec3>,
    /// Normals bound in `<vertices>` (share the vertex index).
    vertex_normals: Option<Vec<Vec3>>,
    vertex_uvs: Option<Vec<Vec2>>,
}

/// A corner: indices into positions / normals / UVs (normals and UVs either
/// per-vertex from `<vertices>` or from their own primitive inputs).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Corner {
    vertex: u32,
    normal: Option<u32>,
    uv: Option<u32>,
}

/// Parses a `<geometry>` element. Returns `None` (with a warning) for
/// geometry types other than `<mesh>`.
pub(crate) fn parse_geometry(
    doc: &Document<'_>,
    geometry: Node<'_, '_>,
    warnings: &mut Vec<String>,
) -> Result<Option<Geometry>, ColladaError> {
    let Some(mesh) = child(geometry, "mesh") else {
        warnings.push(format!(
            "geometry {:?}: only <mesh> is supported",
            geometry.attribute("id").unwrap_or("")
        ));
        return Ok(None);
    };
    let vertices = require(doc, mesh, "vertices")?;
    let mut streams = Streams::default();
    for input in children(vertices, "input") {
        let source = attr(doc, input, "source")?;
        match attr(doc, input, "semantic")? {
            "POSITION" => streams.positions = vec3s(read_source(doc, source, 3)?),
            "NORMAL" => streams.vertex_normals = Some(vec3s(read_source(doc, source, 3)?)),
            "TEXCOORD" => streams.vertex_uvs = Some(vec2s(read_source(doc, source, 2)?)),
            _ => {}
        }
    }

    let mut normals: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut loaded: HashMap<String, (usize, &'static str)> = HashMap::new(); // source -> (base index, kind)
    let mut symbols: Vec<String> = Vec::new();
    let mut groups: Vec<(usize, Vec<[Corner; 3]>)> = Vec::new();

    for primitive in mesh.children().filter(|n| n.is_element()) {
        let kind = primitive.tag_name().name();
        if !matches!(kind, "triangles" | "polylist" | "polygons") {
            if !matches!(kind, "source" | "vertices" | "extra") {
                warnings.push(format!(
                    "line {}: <{kind}> primitives are not supported",
                    doc.line(primitive)
                ));
            }
            continue;
        }
        let ins = inputs(doc, primitive)?;
        let stride = ins.iter().map(|i| i.offset + 1).max().unwrap_or(1);
        let find = |semantic: &str| {
            ins.iter()
                .filter(|i| i.semantic == semantic)
                .min_by_key(|i| i.set)
        };
        let vertex_input = find("VERTEX").ok_or_else(|| ColladaError::Missing {
            element: "VERTEX input".into(),
            parent: kind.into(),
            line: doc.line(primitive),
        })?;
        // Own normal/UV streams (appended once per source, then offset).
        let mut own = |input: Option<&Input>,
                       width: usize,
                       kind: &'static str|
         -> Result<Option<(usize, usize)>, ColladaError> {
            let Some(input) = input else { return Ok(None) };
            if !loaded.contains_key(&input.source) {
                let data = read_source(doc, &input.source, width)?;
                let base = if kind == "normal" {
                    normals.len()
                } else {
                    uvs.len()
                };
                if kind == "normal" {
                    normals.extend(vec3s(data));
                } else {
                    uvs.extend(vec2s(data));
                }
                loaded.insert(input.source.clone(), (base, kind));
            }
            Ok(Some((input.offset, loaded[&input.source].0)))
        };
        let normal_input = own(find("NORMAL"), 3, "normal")?;
        let uv_input = own(find("TEXCOORD"), 2, "uv")?;

        let symbol = primitive.attribute("material").unwrap_or("").to_owned();
        let material = symbols
            .iter()
            .position(|s| *s == symbol)
            .unwrap_or_else(|| {
                symbols.push(symbol);
                symbols.len() - 1
            });

        // Polygons as lists of index tuples.
        let p_lists: Vec<Vec<u32>> = children(primitive, "p")
            .map(|p| numbers(doc, p))
            .collect::<Result<_, _>>()?;
        if child(primitive, "ph").is_some() {
            warnings.push(format!(
                "line {}: polygons with holes (<ph>) are skipped",
                doc.line(primitive)
            ));
        }
        let polygons: Vec<Vec<u32>> = match kind {
            "triangles" => p_lists
                .concat()
                .chunks(stride * 3)
                .map(<[u32]>::to_vec)
                .collect(),
            "polylist" => {
                let vcount: Vec<usize> = child(primitive, "vcount")
                    .map(|v| numbers(doc, v))
                    .transpose()?
                    .unwrap_or_default();
                let all = p_lists.concat();
                let mut start = 0;
                let mut out = Vec::with_capacity(vcount.len());
                for n in vcount {
                    let end = start + n * stride;
                    if end > all.len() {
                        return Err(ColladaError::Count {
                            expected: end,
                            actual: all.len(),
                            line: doc.line(primitive),
                        });
                    }
                    out.push(all[start..end].to_vec());
                    start = end;
                }
                out
            }
            _ => p_lists,
        };

        let mut tris = Vec::new();
        for poly in polygons {
            if poly.len() % stride != 0 || poly.len() < stride * 3 {
                return Err(ColladaError::Count {
                    expected: stride * 3,
                    actual: poly.len(),
                    line: doc.line(primitive),
                });
            }
            let corner = |k: usize| -> Corner {
                let t = &poly[k * stride..][..stride];
                let at = |o: (usize, usize)| t[o.0] + u32::try_from(o.1).unwrap_or(u32::MAX);
                Corner {
                    vertex: t[vertex_input.offset],
                    normal: normal_input.map(at),
                    uv: uv_input.map(at),
                }
            };
            let corners: Vec<Corner> = (0..poly.len() / stride).map(corner).collect();
            for c in &corners {
                let checks = [
                    ("vertex", Some(c.vertex), streams.positions.len()),
                    ("normal", c.normal, normals.len()),
                    ("texcoord", c.uv, uvs.len()),
                ];
                for (what, index, available) in checks {
                    if let Some(i) = index
                        && i as usize >= available
                    {
                        return Err(ColladaError::IndexOutOfRange {
                            what,
                            index: i,
                            available,
                            line: doc.line(primitive),
                        });
                    }
                }
            }
            let points: Vec<Vec3> = corners
                .iter()
                .map(|c| streams.positions[c.vertex as usize])
                .collect();
            tris.extend(
                triangulate_polygon(&points)
                    .into_iter()
                    .map(|[a, b, c]| [corners[a], corners[b], corners[c]]),
            );
        }
        groups.push((material, tris));
    }

    build(&streams, &normals, &uvs, groups, symbols)
        .map(Some)
        .map_err(|e| ColladaError::Invalid(e.to_string()))
}

/// De-indexes corners into unique vertices and builds the mesh.
fn build(
    streams: &Streams,
    normals: &[Vec3],
    uvs: &[Vec2],
    groups: Vec<(usize, Vec<[Corner; 3]>)>,
    symbols: Vec<String>,
) -> Result<Geometry, MeshError> {
    let has_normals = streams.vertex_normals.is_some()
        || groups
            .iter()
            .all(|(_, t)| t.iter().flatten().all(|c| c.normal.is_some()));
    let has_uvs = streams.vertex_uvs.is_some()
        || groups
            .iter()
            .all(|(_, t)| t.iter().flatten().all(|c| c.uv.is_some()));
    let any = groups.iter().any(|(_, t)| !t.is_empty());
    let (has_normals, has_uvs) = (has_normals && any, has_uvs && any);
    let mut positions = Vec::new();
    let mut out_normals = Vec::new();
    let mut out_uvs = Vec::new();
    let mut indices = Vec::new();
    let mut seen: HashMap<Corner, u32> = HashMap::new();
    let mut submeshes = Vec::new();
    let mut order: Vec<(usize, Vec<[Corner; 3]>)> = groups;
    order.sort_by_key(|(m, _)| *m);
    for (material, tris) in order {
        let start = indices.len();
        for c in tris.into_iter().flatten() {
            let key = Corner {
                vertex: c.vertex,
                normal: if has_normals { c.normal } else { None },
                uv: if has_uvs { c.uv } else { None },
            };
            let index = match seen.get(&key) {
                Some(&i) => i,
                None => {
                    let i = u32::try_from(positions.len())
                        .map_err(|_| MeshError::TooManyVertices(positions.len()))?;
                    let v = c.vertex as usize;
                    positions.push(streams.positions[v]);
                    if has_normals {
                        let n = c.normal.map_or_else(
                            || streams.vertex_normals.as_ref().map_or(Vec3::Y, |vn| vn[v]),
                            |n| normals[n as usize],
                        );
                        out_normals.push(n.normalize_or(Vec3::Y));
                    }
                    if has_uvs {
                        out_uvs.push(c.uv.map_or_else(
                            || streams.vertex_uvs.as_ref().map_or(Vec2::ZERO, |vu| vu[v]),
                            |u| uvs[u as usize],
                        ));
                    }
                    seen.insert(key, i);
                    i
                }
            };
            indices.push(index);
        }
        if indices.len() > start {
            submeshes.push(Submesh {
                material: MaterialId(material),
                indices: start..indices.len(),
            });
        }
    }
    let mut mesh = Mesh::new(positions, indices)?.with_submeshes(submeshes)?;
    if has_normals {
        mesh = mesh.with_normals(out_normals)?;
    }
    if has_uvs {
        mesh = mesh.with_uvs(out_uvs)?;
    }
    Ok(Geometry { mesh, symbols })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Result<Option<Geometry>, ColladaError> {
        let doc = Document::parse(xml)?;
        let g = doc.by_uri("#g")?;
        parse_geometry(&doc, g, &mut Vec::new())
    }

    fn wrap(mesh_body: &str) -> String {
        format!(
            r##"<COLLADA><library_geometries><geometry id="g"><mesh>
  <source id="pos"><float_array id="pos-a" count="12">0 0 0  1 0 0  1 1 0  0 1 0</float_array>
    <technique_common><accessor source="#pos-a" count="4" stride="3"/></technique_common></source>
  <source id="nrm"><float_array id="nrm-a" count="6">0 0 2  0 0 -1</float_array>
    <technique_common><accessor source="#nrm-a" count="2" stride="3"/></technique_common></source>
  <vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
  {mesh_body}
</mesh></geometry></library_geometries></COLLADA>"##
        )
    }

    #[test]
    fn triangles_with_separate_normal_indices() {
        let g = parse(&wrap(r##"<triangles material="m" count="2">
            <input semantic="VERTEX" source="#v" offset="0"/><input semantic="NORMAL" source="#nrm" offset="1"/>
            <p>0 0 1 0 2 0  0 0 2 0 3 1</p></triangles>"##)).unwrap().unwrap();
        assert_eq!(g.mesh.triangle_count(), 2);
        assert_eq!(g.symbols, ["m"]);
        // Vertex 0 and 2 are shared with the same normal; vertex 3 has its own.
        assert_eq!(g.mesh.positions().len(), 4);
        let n = g.mesh.normals().unwrap();
        assert!(
            n.iter().all(|v| (v.length() - 1.0).abs() < 1e-6),
            "normalized"
        );
        assert_eq!(n.iter().filter(|v| **v == Vec3::NEG_Z).count(), 1);
    }

    #[test]
    fn polylist_and_polygons_are_triangulated_per_material() {
        let g = parse(&wrap(r##"
            <polylist material="a" count="1"><input semantic="VERTEX" source="#v" offset="0"/><vcount>4</vcount><p>0 1 2 3</p></polylist>
            <polygons material="b" count="1"><input semantic="VERTEX" source="#v" offset="0"/><p>0 1 2</p></polygons>"##)).unwrap().unwrap();
        assert_eq!(g.symbols, ["a", "b"]);
        assert_eq!(
            g.mesh.submeshes(),
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
        assert!(g.mesh.normals().is_none(), "no normals in the file");
    }

    #[test]
    fn malformed_primitives_are_errors() {
        let bad_index = wrap(
            r##"<triangles count="1"><input semantic="VERTEX" source="#v" offset="0"/><p>0 1 9</p></triangles>"##,
        );
        assert!(matches!(
            parse(&bad_index),
            Err(ColladaError::IndexOutOfRange {
                what: "vertex",
                index: 9,
                available: 4,
                ..
            })
        ));
        let short = wrap(
            r##"<polylist count="1"><input semantic="VERTEX" source="#v" offset="0"/><vcount>5</vcount><p>0 1 2</p></polylist>"##,
        );
        assert!(matches!(parse(&short), Err(ColladaError::Count { .. })));
        let no_vertex = wrap(
            r##"<triangles count="1"><input semantic="NORMAL" source="#nrm" offset="0"/><p>0 0 0</p></triangles>"##,
        );
        assert!(matches!(
            parse(&no_vertex),
            Err(ColladaError::Missing { .. })
        ));
        let dangling = wrap(
            r##"<triangles count="1"><input semantic="VERTEX" source="#nowhere" offset="0"/><p>0 1 2</p></triangles>"##,
        );
        assert!(
            parse(&dangling).is_ok(),
            "the VERTEX input names <vertices>, which is resolved separately"
        );
    }

    #[test]
    fn non_mesh_geometry_is_skipped_with_a_warning() {
        let doc =
            Document::parse(r#"<COLLADA><geometry id="g"><spline/></geometry></COLLADA>"#).unwrap();
        let mut warnings = Vec::new();
        assert!(
            parse_geometry(&doc, doc.by_uri("#g").unwrap(), &mut warnings)
                .unwrap()
                .is_none()
        );
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn n64_logo_geometry() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/n64_logo/n64_logo.dae"
        ))
        .unwrap();
        let doc = Document::parse(&text).unwrap();
        let mut warnings = Vec::new();
        let g = parse_geometry(&doc, doc.by_uri("#N64-mesh").unwrap(), &mut warnings)
            .unwrap()
            .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(g.symbols.len(), 4);
        // Groups in file order: 30 / 50 / 8 / 8 triangles.
        let counts: Vec<usize> = g
            .mesh
            .submeshes()
            .iter()
            .map(|s| s.indices.len() / 3)
            .collect();
        assert_eq!(counts, [30, 50, 8, 8]);
        assert_eq!(g.mesh.triangle_count(), 96);
        // All 48 positions are used; per-corner normals split vertices.
        let mut distinct: Vec<[u32; 3]> = g
            .mesh
            .positions()
            .iter()
            .map(|p| p.to_array().map(f32::to_bits))
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 48);
        assert!(
            g.mesh
                .normals()
                .unwrap()
                .iter()
                .all(|n| (n.length() - 1.0).abs() < 1e-5)
        );
    }
}

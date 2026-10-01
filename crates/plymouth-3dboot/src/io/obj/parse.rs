// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront OBJ syntax.

use crate::math::{Vec2, Vec3};

/// One corner of a face: indices (0-based, resolved) into the position,
/// texture-coordinate and normal lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceVertex {
    /// Position index.
    pub position: u32,
    /// Texture-coordinate index, if given.
    pub texcoord: Option<u32>,
    /// Normal index, if given.
    pub normal: Option<u32>,
}

/// A polygon face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    /// Corners in order (at least three).
    pub vertices: Vec<FaceVertex>,
    /// Index into [`ObjData::materials`] of the active `usemtl`, if any.
    pub material: Option<usize>,
    /// Active smoothing group (0 = off).
    pub smoothing_group: u32,
    /// 1-based source line, for diagnostics.
    pub line: usize,
}

/// A directive that was ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// 1-based source line.
    pub line: usize,
    /// What was ignored.
    pub message: String,
}

/// The contents of an OBJ file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjData {
    /// `v` positions (a fourth `w` component is ignored).
    pub positions: Vec<Vec3>,
    /// `vt` texture coordinates (`u`, `v`; `w` ignored).
    pub texcoords: Vec<Vec2>,
    /// `vn` normals (not necessarily unit length).
    pub normals: Vec<Vec3>,
    /// `f` faces.
    pub faces: Vec<Face>,
    /// Raw arguments of `mtllib` lines.
    pub material_libraries: Vec<String>,
    /// Material names from `usemtl`, in order of first use.
    pub materials: Vec<String>,
    /// Ignored directives.
    pub warnings: Vec<Warning>,
}

/// What went wrong while parsing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ObjErrorKind {
    /// A number could not be parsed.
    #[error("invalid number {0:?}")]
    InvalidNumber(String),
    /// A directive has too few values.
    #[error("{0} needs more values")]
    MissingValue(&'static str),
    /// A face has fewer than three corners.
    #[error("face has fewer than 3 vertices")]
    DegenerateFace,
    /// A face corner is not of the form `v`, `v/vt`, `v//vn` or `v/vt/vn`.
    #[error("invalid face vertex {0:?}")]
    InvalidFaceVertex(String),
    /// A face refers to an element that does not exist.
    #[error("{kind} index {index} out of range ({count} defined)")]
    IndexOutOfRange {
        /// `"position"`, `"texcoord"` or `"normal"`.
        kind: &'static str,
        /// The index as written in the file.
        index: i64,
        /// Number of elements of that kind.
        count: usize,
    },
}

/// A parse error with its 1-based line number.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {kind}")]
pub struct ObjError {
    /// 1-based source line.
    pub line: usize,
    /// What went wrong.
    pub kind: ObjErrorKind,
}

fn number(token: Option<&str>, what: &'static str) -> Result<f32, ObjErrorKind> {
    let token = token.ok_or(ObjErrorKind::MissingValue(what))?;
    token
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| ObjErrorKind::InvalidNumber(token.to_owned()))
}

/// Resolves a 1-based (or negative, relative) OBJ index against the number
/// of elements defined so far. Positive indices may refer forward; they are
/// checked once the whole file is read.
fn index(raw: &str, count: usize, kind: &'static str) -> Result<u32, ObjErrorKind> {
    let v: i64 = raw
        .parse()
        .map_err(|_| ObjErrorKind::InvalidNumber(raw.to_owned()))?;
    let out_of_range = || ObjErrorKind::IndexOutOfRange {
        kind,
        index: v,
        count,
    };
    let resolved = match v {
        0 => return Err(out_of_range()),
        v if v > 0 => v - 1,
        v => i64::try_from(count).map_err(|_| out_of_range())? + v,
    };
    if resolved < 0 {
        return Err(out_of_range());
    }
    u32::try_from(resolved).map_err(|_| out_of_range())
}

fn face_vertex(token: &str, data: &ObjData) -> Result<FaceVertex, ObjErrorKind> {
    let mut parts = token.split('/');
    let invalid = || ObjErrorKind::InvalidFaceVertex(token.to_owned());
    let position = index(
        parts.next().filter(|s| !s.is_empty()).ok_or_else(invalid)?,
        data.positions.len(),
        "position",
    )?;
    let texcoord = match parts.next() {
        None | Some("") => None,
        Some(t) => Some(index(t, data.texcoords.len(), "texcoord")?),
    };
    let normal = match parts.next() {
        None | Some("") => None,
        Some(n) => Some(index(n, data.normals.len(), "normal")?),
    };
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(FaceVertex {
        position,
        texcoord,
        normal,
    })
}

/// Joins lines ending in `\` with the following line, keeping the line
/// number of the first.
fn logical_lines(source: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (i, raw) in source.lines().enumerate() {
        let (start, mut text) = pending.take().unwrap_or((i + 1, String::new()));
        let line = raw.trim_end();
        if let Some(stripped) = line.strip_suffix('\\') {
            text.push_str(stripped);
            text.push(' ');
            pending = Some((start, text));
        } else {
            text.push_str(line);
            out.push((start, text));
        }
    }
    out.extend(pending);
    out
}

/// Parses OBJ source text.
///
/// Supported: `v`, `vt`, `vn`, `f` (all index forms, negative indices),
/// `usemtl`, `mtllib`, `s`, and comments. `g`, `o`, and any other directive
/// are recorded as warnings and otherwise ignored. Both LF and CRLF line
/// endings are accepted.
///
/// # Errors
///
/// Returns the first [`ObjError`] (malformed numbers, too few values, faces
/// with fewer than three corners, or out-of-range indices).
pub fn parse_obj(source: &str) -> Result<ObjData, ObjError> {
    let mut data = ObjData::default();
    let mut material = None;
    let mut smoothing_group = 0;
    for (line, text) in logical_lines(source) {
        let err = |kind| ObjError { line, kind };
        let text = text.split('#').next().unwrap_or("").trim();
        let mut tokens = text.split_whitespace();
        let Some(keyword) = tokens.next() else {
            continue;
        };
        match keyword {
            "v" => {
                let p = [
                    number(tokens.next(), "v"),
                    number(tokens.next(), "v"),
                    number(tokens.next(), "v"),
                ];
                let [x, y, z] = p.map(|r| r.map_err(err));
                data.positions.push(Vec3::new(x?, y?, z?));
            }
            "vt" => {
                let u = number(tokens.next(), "vt").map_err(err)?;
                let v = tokens
                    .next()
                    .map_or(Ok(0.0), |t| number(Some(t), "vt"))
                    .map_err(err)?;
                data.texcoords.push(Vec2::new(u, v));
            }
            "vn" => {
                let p = [
                    number(tokens.next(), "vn"),
                    number(tokens.next(), "vn"),
                    number(tokens.next(), "vn"),
                ];
                let [x, y, z] = p.map(|r| r.map_err(err));
                data.normals.push(Vec3::new(x?, y?, z?));
            }
            "f" => {
                let vertices = tokens
                    .map(|t| face_vertex(t, &data))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?;
                if vertices.len() < 3 {
                    return Err(err(ObjErrorKind::DegenerateFace));
                }
                data.faces.push(Face {
                    vertices,
                    material,
                    smoothing_group,
                    line,
                });
            }
            "usemtl" => {
                let name = text["usemtl".len()..].trim();
                if name.is_empty() {
                    return Err(err(ObjErrorKind::MissingValue("usemtl")));
                }
                let id = data
                    .materials
                    .iter()
                    .position(|m| m == name)
                    .unwrap_or_else(|| {
                        data.materials.push(name.to_owned());
                        data.materials.len() - 1
                    });
                material = Some(id);
            }
            "mtllib" => {
                let libs = text["mtllib".len()..].trim();
                if libs.is_empty() {
                    return Err(err(ObjErrorKind::MissingValue("mtllib")));
                }
                data.material_libraries.push(libs.to_owned());
            }
            "s" => {
                smoothing_group = match tokens.next() {
                    Some("off") | None => 0,
                    Some("on") => 1,
                    Some(t) => t
                        .parse()
                        .map_err(|_| err(ObjErrorKind::InvalidNumber(t.to_owned())))?,
                };
            }
            other => data.warnings.push(Warning {
                line,
                message: format!("ignored directive {other:?}"),
            }),
        }
    }
    // Positive indices may refer forward; check them against the final counts.
    for face in &data.faces {
        for v in &face.vertices {
            let checks = [
                (Some(v.position), data.positions.len(), "position"),
                (v.texcoord, data.texcoords.len(), "texcoord"),
                (v.normal, data.normals.len(), "normal"),
            ];
            for (i, count, kind) in checks {
                if let Some(i) = i
                    && i as usize >= count
                {
                    let kind = ObjErrorKind::IndexOutOfRange {
                        kind,
                        index: i64::from(i) + 1,
                        count,
                    };
                    return Err(ObjError {
                        line: face.line,
                        kind,
                    });
                }
            }
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fv(position: u32, texcoord: Option<u32>, normal: Option<u32>) -> FaceVertex {
        FaceVertex {
            position,
            texcoord,
            normal,
        }
    }

    const TRI: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\n";

    #[test]
    fn all_face_vertex_forms() {
        let cases = [
            (
                "f 1 2 3",
                [fv(0, None, None), fv(1, None, None), fv(2, None, None)],
            ),
            (
                "f 1/1 2/2 3/3",
                [
                    fv(0, Some(0), None),
                    fv(1, Some(1), None),
                    fv(2, Some(2), None),
                ],
            ),
            (
                "f 1//1 2//1 3//1",
                [
                    fv(0, None, Some(0)),
                    fv(1, None, Some(0)),
                    fv(2, None, Some(0)),
                ],
            ),
            (
                "f 1/3/1 2/2/1 3/1/1",
                [
                    fv(0, Some(2), Some(0)),
                    fv(1, Some(1), Some(0)),
                    fv(2, Some(0), Some(0)),
                ],
            ),
        ];
        for (face, expected) in cases {
            let d = parse_obj(&format!("{TRI}{face}\n")).unwrap();
            assert_eq!(d.faces[0].vertices, expected, "{face}");
        }
    }

    #[test]
    fn negative_indices_are_relative_to_definitions_so_far() {
        let d = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\nv 5 5 5\nf -4 -1 -2\n").unwrap();
        assert_eq!(
            d.faces[0]
                .vertices
                .iter()
                .map(|v| v.position)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(
            d.faces[1]
                .vertices
                .iter()
                .map(|v| v.position)
                .collect::<Vec<_>>(),
            [0, 3, 2]
        );
    }

    #[test]
    fn values_comments_and_optional_components() {
        let d = parse_obj("# header\nv 1.5 -2e1 3 1.0 # w ignored\nvt 0.25\nvn 0 0 2\n\n   \n")
            .unwrap();
        assert_eq!(d.positions, [Vec3::new(1.5, -20.0, 3.0)]);
        assert_eq!(d.texcoords, [Vec2::new(0.25, 0.0)]);
        assert_eq!(d.normals, [Vec3::new(0.0, 0.0, 2.0)]);
    }

    #[test]
    fn materials_and_smoothing_groups_are_tracked() {
        let src = format!(
            "{TRI}mtllib my lib.mtl\nusemtl red\ns 1\nf 1 2 3\nusemtl blue\ns off\nf 1 2 3\nusemtl red\ns 4\nf 3 2 1\n"
        );
        let d = parse_obj(&src).unwrap();
        assert_eq!(d.material_libraries, ["my lib.mtl"]);
        assert_eq!(d.materials, ["red", "blue"]);
        let m: Vec<_> = d
            .faces
            .iter()
            .map(|f| (f.material, f.smoothing_group))
            .collect();
        assert_eq!(m, [(Some(0), 1), (Some(1), 0), (Some(0), 4)]);
    }

    #[test]
    fn ignored_directives_become_warnings() {
        let d = parse_obj(&format!(
            "{TRI}o thing\ng part\nl 1 2\ncstype bezier\nf 1 2 3\n"
        ))
        .unwrap();
        assert_eq!(d.faces.len(), 1);
        assert_eq!(
            d.warnings.iter().map(|w| w.line).collect::<Vec<_>>(),
            [8, 9, 10, 11]
        );
    }

    #[test]
    fn line_continuations_are_joined() {
        let d = parse_obj("v 1 \\\n 2 3\nv 0 0 0\n").unwrap();
        assert_eq!(d.positions, [Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO]);
    }

    #[test]
    fn errors_report_the_line() {
        let cases: [(&str, ObjError); 7] = [
            (
                "v 1 2\n",
                ObjError {
                    line: 1,
                    kind: ObjErrorKind::MissingValue("v"),
                },
            ),
            (
                "\nv 1 x 3\n",
                ObjError {
                    line: 2,
                    kind: ObjErrorKind::InvalidNumber("x".into()),
                },
            ),
            (
                "v 1 nan 3\n",
                ObjError {
                    line: 1,
                    kind: ObjErrorKind::InvalidNumber("nan".into()),
                },
            ),
            (
                "v 0 0 0\nf 1 1\n",
                ObjError {
                    line: 2,
                    kind: ObjErrorKind::DegenerateFace,
                },
            ),
            (
                "v 0 0 0\nf 0 1 1\n",
                ObjError {
                    line: 2,
                    kind: ObjErrorKind::IndexOutOfRange {
                        kind: "position",
                        index: 0,
                        count: 1,
                    },
                },
            ),
            (
                "v 0 0 0\nf 1 1 -2\n",
                ObjError {
                    line: 2,
                    kind: ObjErrorKind::IndexOutOfRange {
                        kind: "position",
                        index: -2,
                        count: 1,
                    },
                },
            ),
            (
                "v 0 0 0\nf 1/1/1/1 1 1\n",
                ObjError {
                    line: 2,
                    kind: ObjErrorKind::InvalidFaceVertex("1/1/1/1".into()),
                },
            ),
        ];
        for (src, expected) in cases {
            assert_eq!(parse_obj(src), Err(expected), "{src:?}");
        }
    }

    #[test]
    fn forward_references_are_checked_at_the_end() {
        assert!(parse_obj("f 1 2 3\nv 0 0 0\nv 1 0 0\nv 0 1 0\n").is_ok());
        let err = parse_obj("f 1 2 4\nv 0 0 0\nv 1 0 0\nv 0 1 0\n").unwrap_err();
        assert_eq!(
            err,
            ObjError {
                line: 1,
                kind: ObjErrorKind::IndexOutOfRange {
                    kind: "position",
                    index: 4,
                    count: 3
                }
            }
        );
        assert!(err.to_string().starts_with("line 1: position index 4"));
    }

    #[test]
    fn crlf_and_lf_parse_identically() {
        let lf = format!("{TRI}usemtl a\nf 1/1/1 2/2/1 3/3/1\n");
        assert_eq!(
            parse_obj(&lf.replace('\n', "\r\n")).unwrap(),
            parse_obj(&lf).unwrap()
        );
    }

    #[test]
    fn n64_logo_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/n64_logo/n64_logo.obj"
        );
        let src = std::fs::read_to_string(path).unwrap();
        let d = parse_obj(&src).unwrap();
        assert_eq!(d.positions.len(), 48);
        assert_eq!(d.faces.len(), 52);
        let quads = d.faces.iter().filter(|f| f.vertices.len() == 4).count();
        let tris = d.faces.iter().filter(|f| f.vertices.len() == 3).count();
        assert_eq!((quads, tris), (44, 8));
        assert_eq!(d.materials.len(), 4);
        assert_eq!(d.material_libraries, ["n64_logo.mtl"]);
        assert!(d.normals.is_empty() && d.texcoords.is_empty());
        assert!(d.faces.iter().any(|f| f.smoothing_group > 0));
        // Only the `g N64` group line is unsupported.
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
    }
}

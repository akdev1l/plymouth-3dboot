// SPDX-License-Identifier: GPL-3.0-or-later
//! `<library_animations>`: samplers and channels targeting transform-stack
//! elements.

use roxmltree::Node as XmlNode;

use super::ColladaError;
use super::scene::NodeMap;
use super::xml::{Document, attr, children, numbers, require};
use crate::anim::{Channel, ElementTrack, Interpolation, Property, Track};
use crate::math::{Mat4, Vec3};

/// The parsed address of a channel target such as `node/rotateY.ANGLE`,
/// `node/location.X`, `node/location(1)` or `node/transform`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target<'a> {
    node: &'a str,
    sid: &'a str,
    /// Member selector: `ANGLE`, `X`/`Y`/`Z`, or an array index.
    member: Option<Member>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Member {
    Angle,
    Component(usize),
    Other,
}

fn parse_target(target: &str) -> Option<Target<'_>> {
    let (node, rest) = target.split_once('/')?;
    // Deeper paths (node/sid/sub) are not supported.
    if rest.contains('/') {
        return None;
    }
    let (sid, member) = if let Some((sid, m)) = rest.split_once('.') {
        let member = match m {
            "ANGLE" => Member::Angle,
            "X" | "S" | "U" | "R" => Member::Component(0),
            "Y" | "T" | "V" | "G" => Member::Component(1),
            "Z" | "P" | "B" => Member::Component(2),
            _ => Member::Other,
        };
        (sid, Some(member))
    } else if let Some((sid, idx)) = rest.split_once('(') {
        let i = idx.trim_end_matches(')').parse().ok()?;
        (sid, Some(Member::Component(i)))
    } else {
        (rest, None)
    };
    Some(Target { node, sid, member })
}

/// A `<sampler>`'s inputs.
struct Sampler {
    times: Vec<f32>,
    /// Output values, `stride` per key.
    values: Vec<f32>,
    stride: usize,
    interpolation: Interpolation,
    /// For Bezier/Hermite: per key and component, the *value* coordinate of
    /// the incoming and outgoing control points (`stride` per key).
    controls: Option<(Vec<f32>, Vec<f32>)>,
}

/// Reads an accessor's raw values, count and stride.
fn read_accessor(doc: &Document<'_>, uri: &str) -> Result<(Vec<f32>, usize, usize), ColladaError> {
    let source = doc.by_uri(uri)?;
    let accessor = require(doc, require(doc, source, "technique_common")?, "accessor")?;
    let array = doc.by_uri(attr(doc, accessor, "source")?)?;
    let values: Vec<f32> = numbers(doc, array)?;
    let parse = |name: &str, default: usize| {
        accessor
            .attribute(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (count, stride, offset) = (
        parse("count", 0),
        parse("stride", 1).max(1),
        parse("offset", 0),
    );
    let needed = count
        .checked_mul(stride)
        .and_then(|n| n.checked_add(offset))
        .unwrap_or(usize::MAX);
    if needed > values.len() {
        return Err(ColladaError::Count {
            expected: needed,
            actual: values.len(),
            line: doc.line(accessor),
        });
    }
    Ok((values[offset..needed].to_vec(), count, stride))
}

fn interpolation_names(doc: &Document<'_>, uri: &str) -> Result<Vec<String>, ColladaError> {
    let source = doc.by_uri(uri)?;
    let array = require(doc, source, "Name_array")?;
    Ok(array
        .text()
        .unwrap_or("")
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect())
}

/// Reads a tangent source as control-point values: COLLADA 1.4.1 writes
/// (time, value) pairs per component (`2 × stride` per key); value-only
/// tangents (`stride` per key) are accepted too.
fn read_controls(
    doc: &Document<'_>,
    uri: &str,
    keys: usize,
    stride: usize,
) -> Result<Option<Vec<f32>>, ColladaError> {
    let (raw, count, tangent_stride) = read_accessor(doc, uri)?;
    if count != keys {
        return Ok(None);
    }
    Ok(if tangent_stride == 2 * stride {
        Some(
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|[_, value]| *value)
                .collect(),
        )
    } else if tangent_stride == stride {
        Some(raw)
    } else {
        None
    })
}

fn parse_sampler(
    doc: &Document<'_>,
    sampler: XmlNode<'_, '_>,
    warnings: &mut Vec<String>,
) -> Result<Sampler, ColladaError> {
    let input = |semantic: &str| {
        children(sampler, "input")
            .find(|i| i.attribute("semantic") == Some(semantic))
            .and_then(|i| i.attribute("source"))
    };
    let line = doc.line(sampler);
    let missing = |what: &str| ColladaError::Missing {
        element: format!("{what} input"),
        parent: "sampler".into(),
        line,
    };
    let (times, count, _) = read_accessor(doc, input("INPUT").ok_or_else(|| missing("INPUT"))?)?;
    let (values, out_count, stride) =
        read_accessor(doc, input("OUTPUT").ok_or_else(|| missing("OUTPUT"))?)?;
    if out_count != count {
        return Err(ColladaError::Count {
            expected: count,
            actual: out_count,
            line,
        });
    }
    let names = match input("INTERPOLATION") {
        Some(uri) => interpolation_names(doc, uri)?,
        None => Vec::new(),
    };
    let mut controls = None;
    let interpolation = match names.first().map(String::as_str) {
        Some("STEP") => Interpolation::Step,
        None | Some("LINEAR") => Interpolation::Linear,
        Some(curve @ ("BEZIER" | "HERMITE")) => {
            let tangents = match (input("IN_TANGENT"), input("OUT_TANGENT")) {
                (Some(i), Some(o)) => {
                    read_controls(doc, i, count, stride)?.zip(read_controls(doc, o, count, stride)?)
                }
                _ => None,
            };
            match tangents {
                Some(c) => {
                    if curve == "HERMITE" {
                        warnings.push(format!("line {line}: HERMITE tangents are interpreted like BEZIER control points"));
                    }
                    controls = Some(c);
                    Interpolation::CubicSpline
                }
                None => {
                    warnings.push(format!(
                        "line {line}: {curve} without usable tangents is approximated as LINEAR"
                    ));
                    Interpolation::Linear
                }
            }
        }
        Some(other) => {
            warnings.push(format!(
                "line {line}: {other} interpolation is approximated as LINEAR"
            ));
            Interpolation::Linear
        }
    };
    if names.iter().any(|n| *n != names[0]) {
        warnings.push(format!(
            "line {line}: mixed interpolation per key; using {interpolation:?} throughout"
        ));
    }
    Ok(Sampler {
        times,
        values,
        stride,
        interpolation,
        controls,
    })
}

impl Sampler {
    /// Component `c` of every key, as track values: plain values, or for
    /// cubic splines `[in_tangent, value, out_tangent]` per key, with the
    /// Bezier control points converted to Hermite tangents (value units
    /// per second): `out = 3 (c_out - v_i) / dt_i`, `in = 3 (v_i - c_in) / dt_{i-1}`.
    /// This is exact for control points at one third of each segment, as
    /// COLLADA exporters write them.
    fn column(&self, c: usize) -> Vec<f32> {
        let n = self.times.len();
        let v = |k: usize| self.values[k * self.stride + c];
        let Some((cin, cout)) = &self.controls else {
            return (0..n).map(v).collect();
        };
        let mut out = Vec::with_capacity(n * 3);
        for k in 0..n {
            let tan_in = if k > 0 {
                3.0 * (v(k) - cin[k * self.stride + c]) / (self.times[k] - self.times[k - 1])
            } else {
                0.0
            };
            let tan_out = if k + 1 < n {
                3.0 * (cout[k * self.stride + c] - v(k)) / (self.times[k + 1] - self.times[k])
            } else {
                0.0
            };
            out.extend([tan_in, v(k), tan_out]);
        }
        out
    }

    /// Number of track values per key (3 for cubic splines).
    fn per_key(&self) -> usize {
        if self.interpolation == Interpolation::CubicSpline {
            3
        } else {
            1
        }
    }
}

/// The element (`rotate`, `translate`, `scale`, `matrix`, ...) with `sid`
/// below the XML node with id `node`.
fn element_kind<'a>(doc: &'a Document<'_>, node: &str, sid: &str) -> Option<&'a str> {
    let node = doc.by_uri(node).ok()?;
    node.children()
        .find(|c| c.is_element() && c.attribute("sid") == Some(sid))
        .map(|c| c.tag_name().name())
}

/// Builds the element track for a sampler and target.
fn element_track(kind: &str, member: Option<Member>, s: &Sampler) -> Result<ElementTrack, String> {
    let track_err = |e: crate::anim::TrackError| e.to_string();
    let times = || s.times.clone();
    // Track values are per key (and per tangent for cubic splines), so a
    // multi-component value is assembled from the columns entry by entry.
    let entries = s.times.len() * s.per_key();
    let columns = |n: usize| (0..n).map(|c| s.column(c)).collect::<Vec<_>>();
    match (kind, member, s.stride) {
        ("rotate", Some(Member::Angle | Member::Component(3)), 1) | ("rotate", None, 4) => {
            let c = if s.stride == 4 { 3 } else { 0 };
            let radians = s.column(c).into_iter().map(f32::to_radians).collect();
            Track::new(times(), radians, s.interpolation)
                .map(ElementTrack::Angle)
                .map_err(track_err)
        }
        ("translate" | "scale", Some(Member::Component(index)), 1) if index < 3 => {
            Track::new(times(), s.column(0), s.interpolation)
                .map(|track| ElementTrack::Component { index, track })
                .map_err(track_err)
        }
        ("translate" | "scale", None, 3) => {
            let c = columns(3);
            let v = (0..entries)
                .map(|e| Vec3::new(c[0][e], c[1][e], c[2][e]))
                .collect();
            Track::new(times(), v, s.interpolation)
                .map(ElementTrack::Vector)
                .map_err(track_err)
        }
        ("matrix", None, 16) => {
            let c = columns(16);
            // COLLADA matrices are row-major.
            let m = (0..entries)
                .map(|e| Mat4::from_cols_array(&std::array::from_fn(|i| c[i][e])).transpose())
                .collect();
            Track::new(times(), m, s.interpolation)
                .map(ElementTrack::Matrix)
                .map_err(track_err)
        }
        _ => Err(format!(
            "unsupported target: <{kind}> member {member:?} with {} values per key",
            s.stride
        )),
    }
}

/// Parses every `<channel>` below `animation` (recursing into nested
/// animations) into channels for the scene nodes in `nodes`.
pub(crate) fn parse_animation(
    doc: &Document<'_>,
    animation: XmlNode<'_, '_>,
    nodes: &NodeMap,
    warnings: &mut Vec<String>,
) -> Result<Vec<Channel>, ColladaError> {
    let mut out = Vec::new();
    for channel in children(animation, "channel") {
        let target_text = attr(doc, channel, "target")?;
        let line = doc.line(channel);
        let Some(target) = parse_target(target_text) else {
            warnings.push(format!(
                "line {line}: unsupported channel target {target_text:?}"
            ));
            continue;
        };
        let Some(scene_nodes) = nodes.get(target.node) else {
            warnings.push(format!(
                "line {line}: channel target node {:?} is not in the scene",
                target.node
            ));
            continue;
        };
        let Some(kind) = element_kind(doc, target.node, target.sid) else {
            warnings.push(format!(
                "line {line}: node {:?} has no transform with sid {:?}",
                target.node, target.sid
            ));
            continue;
        };
        let sampler = parse_sampler(doc, doc.by_uri(attr(doc, channel, "source")?)?, warnings)?;
        match element_track(kind, target.member, &sampler) {
            Ok(track) => {
                for &node in scene_nodes {
                    out.push(Channel {
                        target: node,
                        property: Property::StackElement {
                            sid: target.sid.to_owned(),
                            track: track.clone(),
                        },
                    });
                }
            }
            Err(e) => warnings.push(format!("line {line}: {target_text}: {e}")),
        }
    }
    for nested in children(animation, "animation") {
        out.extend(parse_animation(doc, nested, nodes, warnings)?);
    }
    Ok(out)
}

/// All channels of `<library_animations>`.
pub(crate) fn parse_library(
    doc: &Document<'_>,
    nodes: &NodeMap,
    warnings: &mut Vec<String>,
) -> Result<Vec<Channel>, ColladaError> {
    let mut out = Vec::new();
    for library in children(doc.root(), "library_animations") {
        for animation in children(library, "animation") {
            out.extend(parse_animation(doc, animation, nodes, warnings)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_syntax() {
        let t = |s| parse_target(s);
        assert_eq!(
            t("n/rotateY.ANGLE"),
            Some(Target {
                node: "n",
                sid: "rotateY",
                member: Some(Member::Angle)
            })
        );
        assert_eq!(
            t("n/location.Y"),
            Some(Target {
                node: "n",
                sid: "location",
                member: Some(Member::Component(1))
            })
        );
        assert_eq!(
            t("n/location(2)"),
            Some(Target {
                node: "n",
                sid: "location",
                member: Some(Member::Component(2))
            })
        );
        assert_eq!(
            t("n/transform"),
            Some(Target {
                node: "n",
                sid: "transform",
                member: None
            })
        );
        assert_eq!(
            t("n/m.W"),
            Some(Target {
                node: "n",
                sid: "m",
                member: Some(Member::Other)
            })
        );
        assert_eq!(t("no-slash"), None);
        assert_eq!(t("n/a/b"), None);
        assert_eq!(t("n/m(x)"), None);
    }

    #[test]
    fn element_tracks_by_kind() {
        let s = |values: Vec<f32>, stride| Sampler {
            times: vec![0.0, 1.0],
            values,
            stride,
            interpolation: Interpolation::Linear,
            controls: None,
        };
        assert!(matches!(
            element_track("rotate", Some(Member::Angle), &s(vec![0.0, 180.0], 1)),
            Ok(ElementTrack::Angle(_))
        ));
        assert!(matches!(
            element_track(
                "rotate",
                None,
                &s(vec![0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 90.0], 4)
            ),
            Ok(ElementTrack::Angle(_))
        ));
        assert!(matches!(
            element_track(
                "translate",
                Some(Member::Component(2)),
                &s(vec![0.0, 1.0], 1)
            ),
            Ok(ElementTrack::Component { index: 2, .. })
        ));
        assert!(matches!(
            element_track("scale", None, &s(vec![1.0; 6], 3)),
            Ok(ElementTrack::Vector(_))
        ));
        assert!(element_track("translate", Some(Member::Angle), &s(vec![0.0, 1.0], 1)).is_err());
        assert!(element_track("skew", None, &s(vec![0.0, 1.0], 1)).is_err());
    }
}

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
}

/// Reads an accessor's raw values and stride.
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
    let needed = offset + count * stride;
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
    let missing = |what: &str| ColladaError::Missing {
        element: format!("{what} input"),
        parent: "sampler".into(),
        line: doc.line(sampler),
    };
    let (times, count, _) = read_accessor(doc, input("INPUT").ok_or_else(|| missing("INPUT"))?)?;
    let (values, out_count, stride) =
        read_accessor(doc, input("OUTPUT").ok_or_else(|| missing("OUTPUT"))?)?;
    if out_count != count {
        return Err(ColladaError::Count {
            expected: count,
            actual: out_count,
            line: doc.line(sampler),
        });
    }
    let names = match input("INTERPOLATION") {
        Some(uri) => interpolation_names(doc, uri)?,
        None => Vec::new(),
    };
    let interpolation = match names.first().map(String::as_str) {
        Some("STEP") => Interpolation::Step,
        None | Some("LINEAR") => Interpolation::Linear,
        Some(other) => {
            warnings.push(format!(
                "line {}: {other} interpolation is approximated as LINEAR",
                doc.line(sampler)
            ));
            Interpolation::Linear
        }
    };
    if names.iter().any(|n| *n != names[0]) {
        warnings.push(format!(
            "line {}: mixed interpolation per key; using {interpolation:?} throughout",
            doc.line(sampler)
        ));
    }
    Ok(Sampler {
        times,
        values,
        stride,
        interpolation,
    })
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
    let keys = s.times.len();
    let track_err = |e: crate::anim::TrackError| e.to_string();
    let column = |c: usize| {
        (0..keys)
            .map(|k| s.values[k * s.stride + c])
            .collect::<Vec<f32>>()
    };
    match (kind, member, s.stride) {
        ("rotate", Some(Member::Angle), 1) | ("rotate", Some(Member::Component(3)), 1) => {
            let radians = column(0).into_iter().map(f32::to_radians).collect();
            Track::new(s.times.clone(), radians, s.interpolation)
                .map(ElementTrack::Angle)
                .map_err(track_err)
        }
        ("rotate", None, 4) => {
            // Whole axis-angle values: only the angle is animated.
            let radians = column(3).into_iter().map(f32::to_radians).collect();
            Track::new(s.times.clone(), radians, s.interpolation)
                .map(ElementTrack::Angle)
                .map_err(track_err)
        }
        ("translate" | "scale", Some(Member::Component(index)), 1) if index < 3 => {
            Track::new(s.times.clone(), column(0), s.interpolation)
                .map(|track| ElementTrack::Component { index, track })
                .map_err(track_err)
        }
        ("translate" | "scale", None, 3) => {
            let v = (0..keys)
                .map(|k| Vec3::new(s.values[k * 3], s.values[k * 3 + 1], s.values[k * 3 + 2]))
                .collect();
            Track::new(s.times.clone(), v, s.interpolation)
                .map(ElementTrack::Vector)
                .map_err(track_err)
        }
        ("matrix", None, 16) => {
            // COLLADA matrices are row-major.
            let m = (0..keys)
                .map(|k| Mat4::from_cols_slice(&s.values[k * 16..][..16]).transpose())
                .collect();
            Track::new(s.times.clone(), m, s.interpolation)
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

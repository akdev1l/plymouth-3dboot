// SPDX-License-Identifier: GPL-3.0-or-later
//! Evaluating a clip at a point in time.

use super::{Clip, ElementTrack, Property};
use crate::math::{Mat4, Quat, Vec3};
use crate::scene::{LocalTransform, Scene, Transform, TransformOpKind};

/// The local transform of every node of a scene at one instant.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    locals: Vec<Mat4>,
}

/// Converts any local transform to TRS form (decomposing matrices), so that
/// translation/rotation/scale channels can be applied to it.
///
/// Matrices that do not decompose (zero scale along an axis) keep their
/// translation and column lengths as scale, with no rotation, instead of
/// producing NaNs. Shear cannot be represented and is dropped.
fn as_trs(t: &LocalTransform) -> Transform {
    match t {
        LocalTransform::Trs(t) => *t,
        other => {
            let m = other.to_matrix();
            let (scale, rotation, translation) = m.to_scale_rotation_translation();
            if scale.is_finite() && rotation.is_finite() && translation.is_finite() {
                Transform {
                    translation,
                    rotation,
                    scale,
                }
            } else {
                Transform {
                    translation: m.w_axis.truncate(),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(
                        m.x_axis.truncate().length(),
                        m.y_axis.truncate().length(),
                        m.z_axis.truncate().length(),
                    ),
                }
            }
        }
    }
}

/// Whether a property animates the node's translation/rotation/scale (as
/// opposed to its matrix or transform-stack elements).
fn is_trs(property: &Property) -> bool {
    matches!(
        property,
        Property::Translation(_) | Property::Rotation(_) | Property::Scale(_)
    )
}

fn apply(transform: &mut LocalTransform, property: &Property, t: f32) {
    match property {
        Property::Translation(track) => {
            *transform = LocalTransform::Trs(Transform {
                translation: track.sample(t),
                ..as_trs(transform)
            })
        }
        Property::Rotation(track) => {
            *transform = LocalTransform::Trs(Transform {
                rotation: track.sample(t),
                ..as_trs(transform)
            })
        }
        Property::Scale(track) => {
            *transform = LocalTransform::Trs(Transform {
                scale: track.sample(t),
                ..as_trs(transform)
            })
        }
        Property::Matrix(track) => *transform = LocalTransform::Matrix(track.sample(t)),
        Property::StackElement { sid, track } => {
            let LocalTransform::Stack(ops) = transform else {
                return;
            };
            let Some(op) = ops.iter_mut().find(|op| op.sid == *sid) else {
                return;
            };
            match (&mut op.kind, track) {
                (
                    TransformOpKind::Translate(v) | TransformOpKind::Scale(v),
                    ElementTrack::Vector(track),
                ) => *v = track.sample(t),
                (
                    TransformOpKind::Translate(v) | TransformOpKind::Scale(v),
                    ElementTrack::Component { index, track },
                ) if *index < 3 => {
                    v[*index] = track.sample(t);
                }
                (TransformOpKind::Rotate { angle, .. }, ElementTrack::Angle(track)) => {
                    *angle = track.sample(t)
                }
                (TransformOpKind::Matrix(m), ElementTrack::Matrix(track)) => *m = track.sample(t),
                // A track that does not fit the element is ignored.
                _ => {}
            }
        }
    }
}

impl Pose {
    /// The rest pose: every node's own transform.
    #[must_use]
    pub fn rest(scene: &Scene) -> Self {
        Self {
            locals: scene
                .nodes()
                .iter()
                .map(|n| n.transform.to_matrix())
                .collect(),
        }
    }

    /// The pose of `scene` with `clip` applied at clip-local time `t`
    /// (seconds; see [`Clip::local_time`]). Nodes without channels keep
    /// their own transform; channels whose target does not exist or does
    /// not fit (e.g. an unknown `sid`) are ignored.
    #[must_use]
    pub fn evaluate(scene: &Scene, clip: &Clip, t: f32) -> Self {
        let mut transforms: Vec<LocalTransform> =
            scene.nodes().iter().map(|n| n.transform.clone()).collect();
        // Matrix and stack-element channels first, TRS channels second: a
        // TRS channel turns the node into TRS form, after which stack
        // elements could no longer be addressed. This makes the result
        // independent of channel order.
        for trs_pass in [false, true] {
            for channel in clip
                .channels
                .iter()
                .filter(|c| is_trs(&c.property) == trs_pass)
            {
                if let Some(transform) = transforms.get_mut(channel.target.0) {
                    apply(transform, &channel.property, t);
                }
            }
        }
        Self {
            locals: transforms.iter().map(LocalTransform::to_matrix).collect(),
        }
    }

    /// Local matrices, indexed like [`Scene::nodes`].
    #[must_use]
    pub fn locals(&self) -> &[Mat4] {
        &self.locals
    }

    /// World matrices of `scene` in this pose.
    ///
    /// # Panics
    ///
    /// Panics if the pose was made for a scene with a different node count.
    #[must_use]
    pub fn world(&self, scene: &Scene) -> Vec<Mat4> {
        scene.world_matrices_with(&self.locals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Channel, Interpolation, Track};
    use crate::math::{Quat, Vec3};
    use crate::scene::{Node, NodeId, TransformOp};
    use std::f32::consts::FRAC_PI_2;

    fn trs(t: Vec3) -> LocalTransform {
        LocalTransform::Trs(Transform {
            translation: t,
            ..Transform::IDENTITY
        })
    }

    fn origin(world: &[Mat4], node: NodeId) -> Vec3 {
        world[node.0].transform_point3(Vec3::ZERO)
    }

    #[test]
    fn rotating_parent_with_translating_child() {
        let mut scene = Scene::new();
        let parent = scene
            .add_node(None, Node::new("parent", trs(Vec3::new(0.0, 1.0, 0.0))))
            .unwrap();
        let child = scene
            .add_node(Some(parent), Node::new("child", trs(Vec3::X)))
            .unwrap();
        let spin = Track::new(
            vec![0.0, 1.0],
            vec![Quat::IDENTITY, Quat::from_rotation_y(FRAC_PI_2)],
            Interpolation::Linear,
        )
        .unwrap();
        let slide = Track::new(
            vec![0.0, 1.0],
            vec![Vec3::new(1.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0)],
            Interpolation::Linear,
        )
        .unwrap();
        let clip = Clip::new(
            "c",
            vec![
                Channel {
                    target: parent,
                    property: Property::Rotation(spin),
                },
                Channel {
                    target: child,
                    property: Property::Translation(slide),
                },
            ],
        );
        for t in [0.0f32, 0.25, 0.5, 1.0] {
            let world = Pose::evaluate(&scene, &clip, t).world(&scene);
            // Child at x = 1 + 2t, rotated by 90t degrees about +Y, then lifted.
            let (r, angle) = (1.0 + 2.0 * t, FRAC_PI_2 * t);
            let expected = Vec3::new(r * libm::cosf(angle), 1.0, -r * libm::sinf(angle));
            assert!(
                origin(&world, child).abs_diff_eq(expected, 1e-5),
                "t = {t}: {} vs {expected}",
                origin(&world, child)
            );
            assert_eq!(
                origin(&world, parent),
                Vec3::Y,
                "translation kept from the rest pose"
            );
        }
    }

    #[test]
    fn nodes_without_channels_keep_their_rest_transform() {
        let mut scene = Scene::new();
        let a = scene
            .add_node(None, Node::new("a", trs(Vec3::new(5.0, 0.0, 0.0))))
            .unwrap();
        let pose = Pose::evaluate(&scene, &Clip::new("empty", vec![]), 1.0);
        assert_eq!(pose, Pose::rest(&scene));
        assert_eq!(origin(&pose.world(&scene), a), Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn stack_elements_are_animated_by_sid() {
        let mut scene = Scene::new();
        let stack = LocalTransform::Stack(vec![
            TransformOp {
                sid: "t".into(),
                kind: TransformOpKind::Translate(Vec3::new(0.0, 0.0, 0.0)),
            },
            TransformOp {
                sid: "rotateY".into(),
                kind: TransformOpKind::Rotate {
                    axis: Vec3::Y,
                    angle: 0.0,
                },
            },
        ]);
        let n = scene.add_node(None, Node::new("n", stack)).unwrap();
        let child = scene
            .add_node(Some(n), Node::new("c", trs(Vec3::X)))
            .unwrap();
        let angle =
            Track::new(vec![0.0, 1.0], vec![0.0, FRAC_PI_2], Interpolation::Linear).unwrap();
        let x = Track::new(vec![0.0, 1.0], vec![0.0, 4.0], Interpolation::Linear).unwrap();
        let clip = Clip::new(
            "c",
            vec![
                Channel {
                    target: n,
                    property: Property::StackElement {
                        sid: "rotateY".into(),
                        track: ElementTrack::Angle(angle),
                    },
                },
                Channel {
                    target: n,
                    property: Property::StackElement {
                        sid: "t".into(),
                        track: ElementTrack::Component { index: 0, track: x },
                    },
                },
                // Unknown sids and mismatched tracks are ignored.
                Channel {
                    target: n,
                    property: Property::StackElement {
                        sid: "nope".into(),
                        track: ElementTrack::Angle(Track::constant(9.0)),
                    },
                },
                Channel {
                    target: n,
                    property: Property::StackElement {
                        sid: "t".into(),
                        track: ElementTrack::Angle(Track::constant(9.0)),
                    },
                },
                Channel {
                    target: NodeId(99),
                    property: Property::Translation(Track::constant(Vec3::ONE)),
                },
            ],
        );
        let world = Pose::evaluate(&scene, &clip, 1.0).world(&scene);
        assert!(
            origin(&world, child).abs_diff_eq(Vec3::new(4.0, 0.0, -1.0), 1e-5),
            "{}",
            origin(&world, child)
        );
    }

    #[test]
    fn trs_channels_decompose_matrix_nodes() {
        let mut scene = Scene::new();
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_z(0.5),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let n = scene
            .add_node(None, Node::new("n", LocalTransform::Matrix(m)))
            .unwrap();
        let clip = Clip::new(
            "c",
            vec![Channel {
                target: n,
                property: Property::Translation(Track::constant(Vec3::ZERO)),
            }],
        );
        let local = Pose::evaluate(&scene, &clip, 0.0).locals()[n.0];
        let expected = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_z(0.5),
            Vec3::ZERO,
        );
        assert!(local.abs_diff_eq(expected, 1e-5));
    }

    #[test]
    fn degenerate_matrices_do_not_produce_nan() {
        let mut scene = Scene::new();
        let stack = LocalTransform::Stack(vec![
            TransformOp {
                sid: "t".into(),
                kind: TransformOpKind::Translate(Vec3::new(1.0, 2.0, 3.0)),
            },
            TransformOp {
                sid: "s".into(),
                kind: TransformOpKind::Scale(Vec3::ZERO),
            },
        ]);
        let n = scene.add_node(None, Node::new("n", stack)).unwrap();
        let clip = Clip::new(
            "c",
            vec![Channel {
                target: n,
                property: Property::Scale(Track::constant(Vec3::ONE)),
            }],
        );
        let local = Pose::evaluate(&scene, &clip, 0.0).locals()[n.0];
        assert!(local.is_finite(), "{local}");
        assert_eq!(local, Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn channel_order_does_not_matter() {
        let mut scene = Scene::new();
        let stack = LocalTransform::Stack(vec![TransformOp {
            sid: "t".into(),
            kind: TransformOpKind::Translate(Vec3::ZERO),
        }]);
        let n = scene.add_node(None, Node::new("n", stack)).unwrap();
        let element = Channel {
            target: n,
            property: Property::StackElement {
                sid: "t".into(),
                track: ElementTrack::Vector(Track::constant(Vec3::X)),
            },
        };
        let scale = Channel {
            target: n,
            property: Property::Scale(Track::constant(Vec3::splat(2.0))),
        };
        let a = Pose::evaluate(
            &scene,
            &Clip::new("a", vec![element.clone(), scale.clone()]),
            0.0,
        );
        let b = Pose::evaluate(&scene, &Clip::new("b", vec![scale, element]), 0.0);
        assert_eq!(a, b);
        assert_eq!(
            a.locals()[n.0].transform_point3(Vec3::ZERO),
            Vec3::X,
            "the element applied in both orders"
        );
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! The scene graph: nodes with local transforms, meshes and materials.

use super::{Camera, Material, Mesh, Projection};
use crate::math::{Aabb, Mat4, Quat, Vec3};

/// Index of a node in a [`Scene`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

/// Index of a mesh in a [`Scene`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshId(pub usize);

/// Translation, rotation and scale, applied as `T · R · S`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Translation.
    pub translation: Vec3,
    /// Rotation (unit quaternion).
    pub rotation: Quat,
    /// Scale along the local axes.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// No translation, rotation or scaling.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// The matrix `T · R · S`.
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// One element of a transform stack.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TransformOpKind {
    /// Translation.
    Translate(Vec3),
    /// Rotation by `angle` radians about `axis` (normalized when applied).
    Rotate {
        /// Rotation axis.
        axis: Vec3,
        /// Angle in radians (counter-clockwise looking down the axis).
        angle: f32,
    },
    /// Non-uniform scale.
    Scale(Vec3),
    /// An arbitrary affine matrix.
    Matrix(Mat4),
}

impl TransformOpKind {
    /// The operation as a matrix.
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        match *self {
            Self::Translate(t) => Mat4::from_translation(t),
            Self::Rotate { axis, angle } => axis
                .try_normalize()
                .map_or(Mat4::IDENTITY, |a| Mat4::from_axis_angle(a, angle)),
            Self::Scale(s) => Mat4::from_scale(s),
            Self::Matrix(m) => m,
        }
    }
}

/// A named element of a transform stack (COLLADA animations address the
/// elements by their `sid`).
#[derive(Clone, Debug, PartialEq)]
pub struct TransformOp {
    /// Scoped id (may be empty).
    pub sid: String,
    /// The operation.
    pub kind: TransformOpKind,
}

/// A node's transform relative to its parent.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LocalTransform {
    /// Decomposed translation, rotation and scale (animatable per channel).
    Trs(Transform),
    /// An arbitrary affine matrix.
    Matrix(Mat4),
    /// A sequence of operations applied in order, as in COLLADA:
    /// `ops[0] · ops[1] · … · point`.
    Stack(Vec<TransformOp>),
}

impl Default for LocalTransform {
    fn default() -> Self {
        Self::Trs(Transform::IDENTITY)
    }
}

impl LocalTransform {
    /// The transform as a matrix.
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        match self {
            Self::Trs(t) => t.to_matrix(),
            Self::Matrix(m) => *m,
            Self::Stack(ops) => ops
                .iter()
                .fold(Mat4::IDENTITY, |m, op| m * op.kind.to_matrix()),
        }
    }
}

/// A node of the scene graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// Name from the source file (may be empty).
    pub name: String,
    /// Transform relative to the parent (or the world, for roots).
    pub transform: LocalTransform,
    /// Mesh drawn at this node, if any.
    pub mesh: Option<MeshId>,
    /// A camera placed at this node (looking down its local −Z), if any.
    pub camera: Option<Projection>,
    /// Scenery, such as a floor: fixed in the world rather than part of the
    /// model. It applies to the node's descendants too. Camera framing
    /// ignores scenery, [`Scene::wrapped_in_root`] leaves it outside the new
    /// root, and renderers draw it as background that only needs redrawing
    /// where the moving model changed the image.
    pub scenery: bool,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

impl Node {
    /// A node with a name and transform, and no mesh.
    #[must_use]
    pub fn new(name: impl Into<String>, transform: LocalTransform) -> Self {
        Self {
            name: name.into(),
            transform,
            ..Self::default()
        }
    }

    /// Sets the mesh drawn at this node.
    #[must_use]
    pub fn with_mesh(mut self, mesh: MeshId) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// Places a camera at this node.
    #[must_use]
    pub fn with_camera(mut self, projection: Projection) -> Self {
        self.camera = Some(projection);
        self
    }

    /// Marks the node as scenery (see [`Node::scenery`]).
    #[must_use]
    pub fn as_scenery(mut self) -> Self {
        self.scenery = true;
        self
    }

    /// The parent node, or `None` for a root.
    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Child nodes, in insertion order.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}

/// A reference to something that does not exist in the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SceneError {
    /// A parent node id is out of range.
    #[error("parent node {0:?} does not exist")]
    InvalidParent(NodeId),
    /// A node refers to a mesh id that is out of range.
    #[error("mesh {0:?} does not exist")]
    InvalidMesh(MeshId),
}

/// Meshes, materials and a hierarchy of nodes.
///
/// Nodes are stored so that every parent precedes its children, which lets
/// world transforms be computed in a single forward pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    meshes: Vec<Mesh>,
    /// Materials, indexed by [`super::MaterialId`] from mesh submeshes.
    pub materials: Vec<Material>,
    nodes: Vec<Node>,
}

impl Scene {
    /// An empty scene.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a mesh and returns its id.
    pub fn add_mesh(&mut self, mesh: Mesh) -> MeshId {
        self.meshes.push(mesh);
        MeshId(self.meshes.len() - 1)
    }

    /// Adds `node` under `parent` (or as a root) and returns its id.
    ///
    /// # Errors
    ///
    /// Returns [`SceneError`] if `parent` or the node's mesh does not exist.
    pub fn add_node(
        &mut self,
        parent: Option<NodeId>,
        mut node: Node,
    ) -> Result<NodeId, SceneError> {
        if let Some(p) = parent
            && p.0 >= self.nodes.len()
        {
            return Err(SceneError::InvalidParent(p));
        }
        if let Some(m) = node.mesh
            && m.0 >= self.meshes.len()
        {
            return Err(SceneError::InvalidMesh(m));
        }
        let id = NodeId(self.nodes.len());
        node.parent = parent;
        node.children.clear();
        self.nodes.push(node);
        if let Some(p) = parent {
            self.nodes[p.0].children.push(id);
        }
        Ok(id)
    }

    /// A copy of the scene with a new node `root` as the parent of all
    /// current roots except scenery, which stays fixed in the world. Returns
    /// the new scene and the root's id (always `NodeId(0)`); every other
    /// node's id increases by one.
    ///
    /// Useful for animating a whole model, e.g. with
    /// [`crate::anim::Clip::turntable`].
    #[must_use]
    pub fn wrapped_in_root(&self, root: Node) -> (Self, NodeId) {
        let mut out = Self {
            meshes: self.meshes.clone(),
            materials: self.materials.clone(),
            nodes: Vec::with_capacity(self.nodes.len() + 1),
        };
        let root_id = out
            .add_node(
                None,
                Node {
                    parent: None,
                    children: Vec::new(),
                    ..root
                },
            )
            .expect("no references");
        for node in &self.nodes {
            let parent = match node.parent {
                Some(p) => Some(NodeId(p.0 + 1)),
                None if node.scenery => None,
                None => Some(root_id),
            };
            out.add_node(parent, node.clone())
                .expect("parents precede children");
        }
        (out, root_id)
    }

    /// All meshes.
    #[must_use]
    pub fn meshes(&self) -> &[Mesh] {
        &self.meshes
    }

    /// All nodes; parents precede their children.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Replaces a node's local transform. The hierarchy cannot be changed
    /// after insertion, which keeps parents ahead of their children.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range.
    pub fn set_transform(&mut self, id: NodeId, transform: LocalTransform) {
        self.nodes[id.0].transform = transform;
    }

    /// Ids of the root nodes.
    pub fn roots(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent.is_none())
            .map(|(i, _)| NodeId(i))
    }

    /// The first node named `name`.
    #[must_use]
    pub fn find_node(&self, name: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.name == name).map(NodeId)
    }

    /// World matrix of every node, indexed like [`Scene::nodes`].
    #[must_use]
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let locals: Vec<Mat4> = self.nodes.iter().map(|n| n.transform.to_matrix()).collect();
        self.world_matrices_with(&locals)
    }

    /// World matrices for the given per-node local matrices (for example an
    /// animated pose), indexed like [`Scene::nodes`].
    ///
    /// # Panics
    ///
    /// Panics if `locals` does not have one matrix per node.
    #[must_use]
    pub fn world_matrices_with(&self, locals: &[Mat4]) -> Vec<Mat4> {
        assert_eq!(locals.len(), self.nodes.len(), "one local matrix per node");
        let mut world: Vec<Mat4> = Vec::with_capacity(self.nodes.len());
        for (node, local) in self.nodes.iter().zip(locals) {
            let m = match node.parent {
                Some(p) => world[p.0] * *local,
                None => *local,
            };
            world.push(m);
        }
        world
    }

    /// The camera at `node` for the given world matrices (e.g. of an
    /// animated pose), or `None` if the node has no camera.
    ///
    /// Scale in the node's world matrix is removed, so a scaled parent
    /// (such as a unit conversion) moves the camera but does not distort
    /// the view.
    ///
    /// # Panics
    ///
    /// Panics if `node` or `world` does not match the scene.
    #[must_use]
    pub fn camera(&self, node: NodeId, world: &[Mat4]) -> Option<Camera> {
        let projection = self.nodes[node.0].camera?;
        let (_, rotation, translation) = world[node.0].to_scale_rotation_translation();
        Some(Camera {
            projection,
            world: Mat4::from_rotation_translation(rotation.normalize(), translation),
        })
    }

    /// Ids of the nodes that carry cameras, in node order.
    pub fn cameras(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.camera.is_some())
            .map(|(i, _)| NodeId(i))
    }

    /// World-space bounds of all mesh instances, given world matrices.
    #[must_use]
    pub fn bounds_with(&self, world: &[Mat4]) -> Aabb {
        self.nodes
            .iter()
            .zip(world)
            .filter_map(|(n, m)| n.mesh.map(|id| self.meshes[id.0].bounds().transformed(m)))
            .fold(Aabb::EMPTY, Aabb::union)
    }

    /// World-space bounds of all mesh instances at rest.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        self.bounds_with(&self.world_matrices())
    }

    /// Whether node `id` is scenery: marked so itself or below a node that
    /// is (see [`Node::scenery`]).
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range.
    #[must_use]
    pub fn is_scenery(&self, id: NodeId) -> bool {
        let mut node = Some(id);
        while let Some(n) = node {
            if self.nodes[n.0].scenery {
                return true;
            }
            node = self.nodes[n.0].parent;
        }
        false
    }

    /// World-space bounds of the mesh instances that are not scenery: the
    /// model itself, given world matrices.
    #[must_use]
    pub fn model_bounds_with(&self, world: &[Mat4]) -> Aabb {
        self.nodes
            .iter()
            .zip(world)
            .enumerate()
            .filter(|&(i, _)| !self.is_scenery(NodeId(i)))
            .filter_map(|(_, (n, m))| n.mesh.map(|id| self.meshes[id.0].bounds().transformed(m)))
            .fold(Aabb::EMPTY, Aabb::union)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::primitives::cube;
    use std::f32::consts::FRAC_PI_2;

    fn trs(t: Vec3, r: Quat, s: Vec3) -> LocalTransform {
        LocalTransform::Trs(Transform {
            translation: t,
            rotation: r,
            scale: s,
        })
    }

    #[test]
    fn transform_matrix_is_translate_rotate_scale() {
        let t = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_z(FRAC_PI_2),
            scale: Vec3::new(2.0, 1.0, 1.0),
        };
        // Scale x by 2, rotate +x to +y, then translate.
        let p = t.to_matrix().transform_point3(Vec3::X);
        assert!(p.abs_diff_eq(Vec3::new(1.0, 4.0, 3.0), 1e-6), "{p}");
        assert_eq!(Transform::default().to_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn child_world_matrix_composes_parent() {
        let mut s = Scene::new();
        let parent = s
            .add_node(
                None,
                Node::new(
                    "parent",
                    trs(
                        Vec3::new(10.0, 0.0, 0.0),
                        Quat::from_rotation_y(FRAC_PI_2),
                        Vec3::ONE,
                    ),
                ),
            )
            .unwrap();
        let child = s
            .add_node(
                Some(parent),
                Node::new(
                    "child",
                    trs(Vec3::new(0.0, 0.0, 1.0), Quat::IDENTITY, Vec3::splat(2.0)),
                ),
            )
            .unwrap();
        let world = s.world_matrices();
        // Child origin: rotate (0,0,1) about Y by 90° -> (1,0,0), then +10 in x.
        let origin = world[child.0].transform_point3(Vec3::ZERO);
        assert!(
            origin.abs_diff_eq(Vec3::new(11.0, 0.0, 0.0), 1e-5),
            "{origin}"
        );
        assert_eq!(
            world[child.0],
            world[parent.0] * s.nodes()[child.0].transform.to_matrix()
        );
        assert_eq!(s.nodes()[parent.0].children(), &[child]);
        assert_eq!(s.nodes()[child.0].parent(), Some(parent));
        assert_eq!(s.roots().collect::<Vec<_>>(), vec![parent]);
        assert_eq!(s.find_node("child"), Some(child));
    }

    #[test]
    fn deep_hierarchy_accumulates() {
        let mut s = Scene::new();
        let mut parent = None;
        for _ in 0..100 {
            parent = Some(
                s.add_node(
                    parent,
                    Node::new("", trs(Vec3::X, Quat::IDENTITY, Vec3::ONE)),
                )
                .unwrap(),
            );
        }
        let world = s.world_matrices();
        assert!(
            world[99]
                .transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-3)
        );
    }

    #[test]
    fn matrix_transforms_and_non_uniform_scale_compose() {
        let mut s = Scene::new();
        let r = Quat::from_rotation_x(0.7);
        let a = s
            .add_node(
                None,
                Node::new(
                    "a",
                    trs(Vec3::new(1.0, 2.0, 3.0), r, Vec3::new(1.0, 3.0, 0.5)),
                ),
            )
            .unwrap();
        let m = Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 1.0,
        ]);
        let b = s
            .add_node(Some(a), Node::new("b", LocalTransform::Matrix(m)))
            .unwrap();
        let expected = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
            * Mat4::from_quat(r)
            * Mat4::from_scale(Vec3::new(1.0, 3.0, 0.5))
            * m;
        assert!(s.world_matrices()[b.0].abs_diff_eq(expected, 1e-5));
    }

    #[test]
    fn transform_stack_applies_in_document_order() {
        let stack = LocalTransform::Stack(vec![
            TransformOp {
                sid: "translate".into(),
                kind: TransformOpKind::Translate(Vec3::new(1.0, 0.0, 0.0)),
            },
            TransformOp {
                sid: "rotateZ".into(),
                kind: TransformOpKind::Rotate {
                    axis: Vec3::Z * 3.0,
                    angle: FRAC_PI_2,
                },
            },
            TransformOp {
                sid: "scale".into(),
                kind: TransformOpKind::Scale(Vec3::new(2.0, 1.0, 1.0)),
            },
        ]);
        // Scale x by 2, rotate +x to +y, then translate.
        let p = stack.to_matrix().transform_point3(Vec3::X);
        assert!(p.abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 1e-6), "{p}");
        assert_eq!(
            LocalTransform::Stack(Vec::new()).to_matrix(),
            Mat4::IDENTITY
        );
        let zero_axis = TransformOpKind::Rotate {
            axis: Vec3::ZERO,
            angle: 1.0,
        };
        assert_eq!(
            zero_axis.to_matrix(),
            Mat4::IDENTITY,
            "degenerate axis is ignored"
        );
    }

    #[test]
    fn wrapping_in_a_root_preserves_world_transforms() {
        let mut s = Scene::new();
        let m = s.add_mesh(cube(1.0));
        let a = s
            .add_node(
                None,
                Node::new("a", trs(Vec3::X, Quat::IDENTITY, Vec3::ONE)),
            )
            .unwrap();
        s.add_node(
            Some(a),
            Node::new("b", trs(Vec3::Y, Quat::IDENTITY, Vec3::ONE)).with_mesh(m),
        )
        .unwrap();
        s.add_node(
            None,
            Node::new("c", trs(Vec3::Z, Quat::IDENTITY, Vec3::ONE)),
        )
        .unwrap();
        let (w, root) = s.wrapped_in_root(Node::new("root", LocalTransform::default()));
        assert_eq!(root, NodeId(0));
        assert_eq!(w.nodes().len(), 4);
        assert_eq!(w.roots().collect::<Vec<_>>(), vec![root]);
        assert_eq!(w.nodes()[0].children(), &[NodeId(1), NodeId(3)]);
        assert_eq!(w.nodes()[2].parent(), Some(NodeId(1)));
        assert_eq!(&w.world_matrices()[1..], &s.world_matrices()[..]);
        assert_eq!(w.bounds(), s.bounds());
    }

    #[test]
    fn orbiting_camera_node_keeps_its_distance_and_aim() {
        use crate::anim::{Clip, Pose};
        let mut s = Scene::new();
        let pivot = s
            .add_node(
                None,
                Node::new(
                    "pivot",
                    LocalTransform::Matrix(Mat4::from_scale(Vec3::splat(3.0))),
                ),
            )
            .unwrap();
        let projection = Projection::Perspective {
            fov_y: 1.0,
            z_near: 0.1,
            z_far: 100.0,
        };
        let cam = s
            .add_node(
                Some(pivot),
                Node::new(
                    "cam",
                    trs(Vec3::new(0.0, 1.0, 5.0), Quat::IDENTITY, Vec3::ONE),
                )
                .with_camera(projection),
            )
            .unwrap();
        assert_eq!(s.cameras().collect::<Vec<_>>(), vec![cam]);
        assert!(s.camera(pivot, &s.world_matrices()).is_none());
        let clip = Clip::turntable(pivot, Vec3::Y, 10.0);
        for i in 0..10 {
            let world = Pose::evaluate(&s, &clip, i as f32).world(&s);
            let camera = s.camera(cam, &world).unwrap();
            let p = camera.position();
            // The pivot's scale moves the camera to radius 15, height 3...
            assert!(
                (Vec3::new(p.x, 0.0, p.z).length() - 15.0).abs() < 1e-3 && (p.y - 3.0).abs() < 1e-4,
                "{p}"
            );
            // ...but the view itself is not scaled: the axis below the camera
            // projects to the horizontal centre line of the screen.
            let target = camera
                .view_projection(1.0)
                .project_point3(Vec3::new(0.0, 3.0, 0.0));
            assert!(target.x.abs() < 1e-4 && target.y.abs() < 1e-4, "{target}");
        }
    }

    #[test]
    fn invalid_references_are_rejected() {
        let mut s = Scene::new();
        assert_eq!(
            s.add_node(Some(NodeId(0)), Node::default()),
            Err(SceneError::InvalidParent(NodeId(0)))
        );
        assert_eq!(
            s.add_node(None, Node::default().with_mesh(MeshId(0))),
            Err(SceneError::InvalidMesh(MeshId(0)))
        );
        assert!(s.nodes().is_empty());
    }

    #[test]
    fn bounds_cover_transformed_mesh_instances() {
        let mut s = Scene::new();
        let m = s.add_mesh(cube(1.0));
        assert!(s.bounds().is_empty(), "no instances yet");
        let root = s
            .add_node(
                None,
                Node::new(
                    "root",
                    trs(Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE),
                ),
            )
            .unwrap();
        s.add_node(
            Some(root),
            Node::new("a", LocalTransform::default()).with_mesh(m),
        )
        .unwrap();
        s.add_node(
            Some(root),
            Node::new(
                "b",
                trs(Vec3::new(0.0, 10.0, 0.0), Quat::IDENTITY, Vec3::splat(2.0)),
            )
            .with_mesh(m),
        )
        .unwrap();
        assert_eq!(
            s.bounds(),
            Aabb::new(Vec3::new(3.0, -1.0, -2.0), Vec3::new(7.0, 12.0, 2.0))
        );
    }

    #[test]
    fn set_transform_and_pose_override() {
        let mut s = Scene::new();
        let a = s.add_node(None, Node::default()).unwrap();
        let b = s
            .add_node(
                Some(a),
                Node::new("b", trs(Vec3::Y, Quat::IDENTITY, Vec3::ONE)),
            )
            .unwrap();
        s.set_transform(a, trs(Vec3::X, Quat::IDENTITY, Vec3::ONE));
        assert_eq!(
            s.world_matrices()[b.0].transform_point3(Vec3::ZERO),
            Vec3::new(1.0, 1.0, 0.0)
        );
        let posed = s.world_matrices_with(&[Mat4::from_translation(Vec3::Z), Mat4::IDENTITY]);
        assert_eq!(posed[b.0].transform_point3(Vec3::ZERO), Vec3::Z);
        // The hierarchy is untouched by transform changes.
        assert_eq!(s.nodes()[b.0].parent(), Some(a));
    }

    #[test]
    fn scenery_is_inherited_excluded_from_model_bounds_and_not_wrapped() {
        let mut scene = Scene::new();
        let mesh = scene.add_mesh(cube(1.0));
        let model = scene
            .add_node(
                None,
                Node::new("model", LocalTransform::default()).with_mesh(mesh),
            )
            .unwrap();
        let far = trs(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);
        let floor = scene
            .add_node(None, Node::new("floor", far).as_scenery())
            .unwrap();
        let child = scene
            .add_node(
                Some(floor),
                Node::new("tile", LocalTransform::default()).with_mesh(mesh),
            )
            .unwrap();
        assert!(!scene.is_scenery(model));
        assert!(
            scene.is_scenery(floor) && scene.is_scenery(child),
            "inherited"
        );
        assert_eq!(
            scene.model_bounds_with(&scene.world_matrices()),
            Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0))
        );
        assert_eq!(scene.bounds().max.x, 11.0);

        let (wrapped, root) = scene.wrapped_in_root(Node::new("spin", LocalTransform::default()));
        assert_eq!(
            wrapped.nodes()[1].parent(),
            Some(root),
            "the model moves with the root"
        );
        assert_eq!(wrapped.nodes()[2].parent(), None, "scenery stays a root");
        assert_eq!(wrapped.nodes()[3].parent(), Some(NodeId(2)));
        assert!(wrapped.is_scenery(NodeId(3)));
    }
}

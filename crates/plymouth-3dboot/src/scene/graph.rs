// SPDX-License-Identifier: GPL-3.0-or-later
//! The scene graph: nodes with local transforms, meshes and materials.

use super::{Material, Mesh};
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

/// A node's transform relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocalTransform {
    /// Decomposed translation, rotation and scale (animatable per channel).
    Trs(Transform),
    /// An arbitrary affine matrix (e.g. baked COLLADA transform stacks).
    Matrix(Mat4),
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
}

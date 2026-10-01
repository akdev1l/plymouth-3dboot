// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene description: meshes, materials, nodes and cameras.

pub mod camera;
pub mod graph;
pub mod material;
pub mod mesh;
pub mod normals;
pub mod primitives;
pub mod triangulate;

pub use camera::{Camera, Projection};
pub use graph::{
    LocalTransform, MeshId, Node, NodeId, Scene, SceneError, Transform, TransformOp,
    TransformOpKind,
};
pub use material::{Material, MaterialId};
pub use mesh::{Mesh, MeshError, Submesh};

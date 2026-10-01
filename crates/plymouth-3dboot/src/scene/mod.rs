// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene description: meshes, materials, nodes and cameras.

pub mod material;
pub mod mesh;
pub mod normals;
pub mod primitives;
pub mod triangulate;

pub use material::{Material, MaterialId};
pub use mesh::{Mesh, MeshError, Submesh};

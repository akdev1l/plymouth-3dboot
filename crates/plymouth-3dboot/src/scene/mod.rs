// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene description: meshes, materials, nodes and cameras.

pub mod material;
pub mod mesh;
pub mod primitives;

pub use material::{Material, MaterialId};
pub use mesh::{Mesh, MeshError, Submesh};

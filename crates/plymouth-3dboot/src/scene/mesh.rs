// SPDX-License-Identifier: GPL-3.0-or-later
//! Indexed triangle meshes.

use std::ops::Range;

use super::MaterialId;
use crate::math::{Aabb, Vec2, Vec3};

/// A range of a mesh's triangles drawn with one material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submesh {
    /// Material of these triangles.
    pub material: MaterialId,
    /// Range into [`Mesh::indices`]; start and end are multiples of 3.
    pub indices: Range<usize>,
}

/// A mesh failed validation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MeshError {
    /// The index count is not a multiple of three.
    #[error("index count {0} is not a multiple of 3")]
    IndexCount(usize),
    /// An index refers past the last vertex.
    #[error("index {index} out of range for {vertex_count} vertices")]
    IndexOutOfRange {
        /// The offending index.
        index: u32,
        /// Number of vertices.
        vertex_count: usize,
    },
    /// A vertex attribute has a different length than the positions.
    #[error("{attribute} has {actual} entries but there are {expected} vertices")]
    AttributeLength {
        /// Attribute name.
        attribute: &'static str,
        /// Number of positions.
        expected: usize,
        /// Number of attribute values.
        actual: usize,
    },
    /// A submesh range is out of bounds or not aligned to whole triangles.
    #[error("submesh index range {range:?} is invalid for {index_count} indices")]
    SubmeshRange {
        /// The offending range.
        range: Range<usize>,
        /// Number of indices.
        index_count: usize,
    },
    /// A position is NaN or infinite.
    #[error("vertex {0} has a non-finite position")]
    NonFinitePosition(usize),
    /// The mesh has more vertices than `u32` indices can address.
    #[error("too many vertices ({0})")]
    TooManyVertices(usize),
}

/// An indexed triangle mesh with optional per-vertex normals and texture
/// coordinates, split into per-material submeshes.
///
/// Constructed through validating methods, so every index is in range and
/// every attribute has one value per vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    positions: Vec<Vec3>,
    normals: Option<Vec<Vec3>>,
    uvs: Option<Vec<Vec2>>,
    indices: Vec<u32>,
    submeshes: Vec<Submesh>,
}

impl Mesh {
    /// Creates a mesh from positions and triangle indices, with a single
    /// submesh using material 0.
    ///
    /// # Errors
    ///
    /// Returns a [`MeshError`] if indices are malformed or out of range, or a
    /// position is not finite.
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Result<Self, MeshError> {
        if u32::try_from(positions.len()).is_err() {
            return Err(MeshError::TooManyVertices(positions.len()));
        }
        if !indices.len().is_multiple_of(3) {
            return Err(MeshError::IndexCount(indices.len()));
        }
        if let Some(&index) = indices.iter().find(|&&i| i as usize >= positions.len()) {
            return Err(MeshError::IndexOutOfRange {
                index,
                vertex_count: positions.len(),
            });
        }
        if let Some(i) = positions.iter().position(|p| !p.is_finite()) {
            return Err(MeshError::NonFinitePosition(i));
        }
        let submeshes = vec![Submesh {
            material: MaterialId(0),
            indices: 0..indices.len(),
        }];
        Ok(Self {
            positions,
            normals: None,
            uvs: None,
            indices,
            submeshes,
        })
    }

    fn check_len(&self, attribute: &'static str, actual: usize) -> Result<(), MeshError> {
        if actual == self.positions.len() {
            Ok(())
        } else {
            Err(MeshError::AttributeLength {
                attribute,
                expected: self.positions.len(),
                actual,
            })
        }
    }

    /// Sets per-vertex normals.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::AttributeLength`] if there is not exactly one
    /// normal per vertex.
    pub fn with_normals(mut self, normals: Vec<Vec3>) -> Result<Self, MeshError> {
        self.check_len("normals", normals.len())?;
        self.normals = Some(normals);
        Ok(self)
    }

    /// Sets per-vertex texture coordinates.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::AttributeLength`] if there is not exactly one
    /// UV per vertex.
    pub fn with_uvs(mut self, uvs: Vec<Vec2>) -> Result<Self, MeshError> {
        self.check_len("uvs", uvs.len())?;
        self.uvs = Some(uvs);
        Ok(self)
    }

    /// Replaces the submesh list.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::SubmeshRange`] if a range is out of bounds,
    /// reversed, or does not start and end on whole triangles.
    pub fn with_submeshes(mut self, submeshes: Vec<Submesh>) -> Result<Self, MeshError> {
        let n = self.indices.len();
        if let Some(bad) = submeshes.iter().find(|s| {
            s.indices.start > s.indices.end
                || s.indices.end > n
                || !s.indices.start.is_multiple_of(3)
                || !s.indices.end.is_multiple_of(3)
        }) {
            return Err(MeshError::SubmeshRange {
                range: bad.indices.clone(),
                index_count: n,
            });
        }
        self.submeshes = submeshes;
        Ok(self)
    }

    /// Vertex positions.
    #[must_use]
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }

    /// Per-vertex normals, if present.
    #[must_use]
    pub fn normals(&self) -> Option<&[Vec3]> {
        self.normals.as_deref()
    }

    /// Per-vertex texture coordinates, if present.
    #[must_use]
    pub fn uvs(&self) -> Option<&[Vec2]> {
        self.uvs.as_deref()
    }

    /// Triangle indices, three per triangle.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Per-material index ranges.
    #[must_use]
    pub fn submeshes(&self) -> &[Submesh] {
        &self.submeshes
    }

    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Triangles as vertex-index triples.
    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        self.indices.as_chunks::<3>().0.iter().copied()
    }

    /// Bounding box of the vertices (empty for a mesh without vertices).
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        Aabb::from_points(self.positions.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        Mesh::new(p, vec![0, 1, 2, 0, 2, 3]).unwrap()
    }

    #[test]
    fn new_mesh_has_one_submesh_and_bounds() {
        let m = quad();
        assert_eq!(m.triangle_count(), 2);
        assert_eq!(
            m.submeshes(),
            &[Submesh {
                material: MaterialId(0),
                indices: 0..6
            }]
        );
        assert_eq!(m.bounds(), Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0)));
        assert_eq!(m.triangles().collect::<Vec<_>>(), [[0, 1, 2], [0, 2, 3]]);
        assert!(m.normals().is_none() && m.uvs().is_none());
    }

    #[test]
    fn invalid_indices_are_rejected() {
        assert_eq!(
            Mesh::new(vec![Vec3::ZERO; 3], vec![0, 1]),
            Err(MeshError::IndexCount(2))
        );
        assert_eq!(
            Mesh::new(vec![Vec3::ZERO; 3], vec![0, 1, 3]),
            Err(MeshError::IndexOutOfRange {
                index: 3,
                vertex_count: 3
            })
        );
        assert_eq!(
            Mesh::new(vec![Vec3::ZERO, Vec3::NAN, Vec3::ZERO], vec![0, 1, 2]),
            Err(MeshError::NonFinitePosition(1))
        );
    }

    #[test]
    fn attributes_must_match_vertex_count() {
        assert!(quad().with_normals(vec![Vec3::Z; 4]).is_ok());
        assert_eq!(
            quad().with_normals(vec![Vec3::Z; 3]),
            Err(MeshError::AttributeLength {
                attribute: "normals",
                expected: 4,
                actual: 3
            })
        );
        assert!(quad().with_uvs(vec![Vec2::ZERO; 5]).is_err());
        assert_eq!(
            quad().with_uvs(vec![Vec2::ONE; 4]).unwrap().uvs(),
            Some(&[Vec2::ONE; 4][..])
        );
    }

    #[test]
    fn submesh_ranges_are_validated() {
        let two = vec![
            Submesh {
                material: MaterialId(0),
                indices: 0..3,
            },
            Submesh {
                material: MaterialId(1),
                indices: 3..6,
            },
        ];
        assert_eq!(
            quad().with_submeshes(two.clone()).unwrap().submeshes(),
            &two[..]
        );
        #[allow(clippy::reversed_empty_ranges)]
        for bad in [0..4, 3..9, 6..3] {
            let err = quad().with_submeshes(vec![Submesh {
                material: MaterialId(0),
                indices: bad.clone(),
            }]);
            assert_eq!(
                err,
                Err(MeshError::SubmeshRange {
                    range: bad,
                    index_count: 6
                })
            );
        }
    }

    #[test]
    fn empty_mesh_is_valid_with_empty_bounds() {
        let m = Mesh::new(Vec::new(), Vec::new()).unwrap();
        assert_eq!(m.triangle_count(), 0);
        assert!(m.bounds().is_empty());
    }
}

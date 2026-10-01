// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront OBJ/MTL loading.

pub mod load;
pub mod mtl;
pub mod parse;

pub use load::{ObjLoadError, ObjModel, ObjOptions, SmoothingGroups, load_obj};
pub use mtl::{MtlData, MtlMaterial, parse_mtl};
pub use parse::{Face, FaceVertex, ObjData, ObjError, ObjErrorKind, Warning, parse_obj};

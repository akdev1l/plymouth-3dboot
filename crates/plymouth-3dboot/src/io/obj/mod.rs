// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront OBJ/MTL loading.

pub mod parse;

pub use parse::{Face, FaceVertex, ObjData, ObjError, ObjErrorKind, Warning, parse_obj};

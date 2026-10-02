// SPDX-License-Identifier: GPL-3.0-or-later
//! Encoders and decoders.
//!
//! Encoders and decoders work on in-memory bytes. Loaders fetch the files a
//! model refers to through a [`ResourceResolver`]; [`FsResolver`] is the only
//! place the core reads the filesystem, and [`sequence::PngSequence`] the
//! only place it writes; both only when a caller uses them.

pub mod collada;
pub mod obj;
pub mod png;
pub mod resolve;
pub mod sequence;

pub use resolve::{FsResolver, MemResolver, ResolveError, ResourceResolver};

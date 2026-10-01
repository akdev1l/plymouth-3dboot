// SPDX-License-Identifier: GPL-3.0-or-later
//! Encoders and decoders.
//!
//! Encoders and decoders work on in-memory bytes. Loaders fetch the files a
//! model refers to through a [`ResourceResolver`]; [`FsResolver`] is the only
//! place the core touches the filesystem, and only when a caller uses it.

pub mod png;
pub mod resolve;

pub use resolve::{FsResolver, MemResolver, ResolveError, ResourceResolver};

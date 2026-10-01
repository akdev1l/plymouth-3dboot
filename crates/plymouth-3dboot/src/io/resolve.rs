// SPDX-License-Identifier: GPL-3.0-or-later
//! Access to the files a model refers to (materials, textures, ...).
//!
//! Loaders never open files themselves: they ask a [`ResourceResolver`]
//! for the bytes of a name found in the model (e.g. an OBJ `mtllib`). That
//! keeps loading identical on native, WebAssembly (embedded assets) and C
//! callers that supply their own data.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// A resource could not be provided.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// No resource has this name.
    #[error("resource not found: {0}")]
    NotFound(String),
    /// The name is not allowed (absolute, or escaping the root with `..`).
    #[error("invalid resource name: {0}")]
    InvalidName(String),
    /// The resource exists but could not be read.
    #[error("cannot read resource {name}: {message}")]
    Io {
        /// Requested name.
        name: String,
        /// Underlying error message.
        message: String,
    },
}

/// Provides the bytes of resources referenced by name from a model file.
pub trait ResourceResolver {
    /// Returns the contents of the resource `name`, a relative path using
    /// `/` (or `\`) as separators.
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError`] if the resource is missing, the name is not
    /// allowed, or reading fails.
    fn resolve(&self, name: &str) -> Result<Vec<u8>, ResolveError>;
}

/// Splits `name` into normal path components, rejecting absolute paths and
/// parent-directory references.
fn components(name: &str) -> Result<Vec<String>, ResolveError> {
    let normalized = name.replace('\\', "/");
    let mut out = Vec::new();
    for c in Path::new(&normalized).components() {
        match c {
            Component::Normal(s) => out.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ResolveError::InvalidName(name.to_owned()));
            }
        }
    }
    if out.is_empty() || normalized.starts_with('/') {
        return Err(ResolveError::InvalidName(name.to_owned()));
    }
    Ok(out)
}

/// Resolves names from an in-memory map (e.g. assets embedded with
/// `include_bytes!`).
#[derive(Clone, Debug, Default)]
pub struct MemResolver {
    files: HashMap<String, Vec<u8>>,
}

impl MemResolver {
    /// An empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a resource.
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::InvalidName`] for absolute names or names
    /// containing `..`.
    pub fn insert(&mut self, name: &str, bytes: impl Into<Vec<u8>>) -> Result<(), ResolveError> {
        self.files.insert(components(name)?.join("/"), bytes.into());
        Ok(())
    }

    /// Builder-style [`MemResolver::insert`].
    ///
    /// # Errors
    ///
    /// See [`MemResolver::insert`].
    pub fn with(mut self, name: &str, bytes: impl Into<Vec<u8>>) -> Result<Self, ResolveError> {
        self.insert(name, bytes)?;
        Ok(self)
    }
}

impl ResourceResolver for MemResolver {
    fn resolve(&self, name: &str) -> Result<Vec<u8>, ResolveError> {
        let key = components(name)?.join("/");
        self.files
            .get(&key)
            .cloned()
            .ok_or(ResolveError::NotFound(name.to_owned()))
    }
}

/// Resolves names relative to a directory on the filesystem.
///
/// Names may not be absolute or contain `..`, so a model can only reach
/// files below `root`. (Symbolic links inside `root` are followed.)
#[derive(Clone, Debug)]
pub struct FsResolver {
    root: PathBuf,
}

impl FsResolver {
    /// A resolver for files below `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl ResourceResolver for FsResolver {
    fn resolve(&self, name: &str) -> Result<Vec<u8>, ResolveError> {
        let path = components(name)?
            .iter()
            .fold(self.root.clone(), |p, c| p.join(c));
        std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ResolveError::NotFound(name.to_owned()),
            _ => ResolveError::Io {
                name: name.to_owned(),
                message: e.to_string(),
            },
        })
    }
}

impl<R: ResourceResolver + ?Sized> ResourceResolver for &R {
    fn resolve(&self, name: &str) -> Result<Vec<u8>, ResolveError> {
        (**self).resolve(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_resolver_normalizes_names() {
        let r = MemResolver::new()
            .with("dir/model.mtl", b"abc".to_vec())
            .unwrap();
        assert_eq!(r.resolve("dir/model.mtl").unwrap(), b"abc");
        assert_eq!(r.resolve("./dir//model.mtl").unwrap(), b"abc");
        assert_eq!(r.resolve("dir\\model.mtl").unwrap(), b"abc");
        assert_eq!(
            r.resolve("model.mtl"),
            Err(ResolveError::NotFound("model.mtl".into()))
        );
    }

    #[test]
    fn traversal_and_absolute_names_are_rejected() {
        let r = MemResolver::new();
        for bad in [
            "../secret",
            "a/../../b",
            "/etc/passwd",
            "\\\\server\\share",
            "",
            ".",
        ] {
            assert!(
                matches!(r.resolve(bad), Err(ResolveError::InvalidName(_))),
                "{bad:?} accepted"
            );
        }
        assert!(MemResolver::new().insert("../x", b"".to_vec()).is_err());
        assert!(matches!(
            FsResolver::new("/").resolve("../etc/passwd"),
            Err(ResolveError::InvalidName(_))
        ));
    }

    #[test]
    fn fs_resolver_reads_below_root() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let r = FsResolver::new(&root);
        let mtl = r.resolve("n64_logo/n64_logo.mtl").unwrap();
        assert!(mtl.starts_with(b"# 3ds Max"));
        assert_eq!(
            r.resolve("n64_logo/missing.mtl"),
            Err(ResolveError::NotFound("n64_logo/missing.mtl".into()))
        );
        // A directory is not a readable resource.
        assert!(matches!(
            r.resolve("n64_logo"),
            Err(ResolveError::Io { .. })
        ));
        // Resolvers work through references too.
        fn via<R: ResourceResolver>(r: R) -> bool {
            r.resolve("n64_logo/Readme.txt").is_ok()
        }
        assert!(via(&r));
    }
}

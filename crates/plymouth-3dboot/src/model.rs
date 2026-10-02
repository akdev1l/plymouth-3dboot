// SPDX-License-Identifier: GPL-3.0-or-later
//! Loading models of any supported format.

use std::path::{Path, PathBuf};

use crate::anim::{Clip, WrapMode};
use crate::io::collada::{ColladaError, ColladaOptions, load_collada};
use crate::io::obj::{ObjLoadError, ObjOptions, load_obj};
use crate::io::{FsResolver, ResourceResolver};
use crate::math::Vec3;
use crate::render::{AnimationRenderer, FrameSettings, RenderError};
use crate::scene::{LocalTransform, Node, Scene};

/// A supported model file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// Wavefront OBJ (with MTL materials).
    Obj,
    /// COLLADA (`.dae`).
    Collada,
}

impl Format {
    /// The format for a file extension (case-insensitive, without the dot).
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "obj" => Some(Self::Obj),
            "dae" => Some(Self::Collada),
            _ => None,
        }
    }

    /// The format of a path, from its extension.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }
}

/// Loading a model failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The file extension is not a supported format.
    #[error("{0}: unsupported model format (expected .obj or .dae)")]
    UnsupportedFormat(PathBuf),
    /// The model file could not be read.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The model is not valid UTF-8 text.
    #[error("model is not valid UTF-8")]
    Utf8,
    /// The OBJ data is invalid.
    #[error(transparent)]
    Obj(#[from] ObjLoadError),
    /// The COLLADA data is invalid.
    #[error(transparent)]
    Collada(#[from] ColladaError),
}

/// A loaded model: its scene, animation clips and loader warnings.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// The scene.
    pub scene: Scene,
    /// Animation clips (possibly empty).
    pub clips: Vec<Clip>,
    /// Problems that did not prevent loading.
    pub warnings: Vec<String>,
}

impl Model {
    /// Loads a model from its source text; side files (materials) come from
    /// `resolver`. Format options are the defaults (COLLADA up axis and units
    /// are converted to Y-up metres).
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] if the data is invalid for the format.
    pub fn from_source(
        format: Format,
        source: &str,
        resolver: &impl ResourceResolver,
    ) -> Result<Self, LoadError> {
        Ok(match format {
            Format::Obj => {
                let m = load_obj(source, resolver, &ObjOptions::default())?;
                Self {
                    scene: m.scene,
                    clips: Vec::new(),
                    warnings: m.warnings,
                }
            }
            Format::Collada => {
                let m = load_collada(source, resolver, &ColladaOptions::default())?;
                Self {
                    scene: m.scene,
                    clips: m.clips,
                    warnings: m.warnings,
                }
            }
        })
    }

    /// Like [`Model::from_source`], for raw bytes (which must be UTF-8).
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Utf8`] for non-UTF-8 data, or the format's error.
    pub fn from_bytes(
        format: Format,
        bytes: &[u8],
        resolver: &impl ResourceResolver,
    ) -> Result<Self, LoadError> {
        let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Utf8)?;
        Self::from_source(format, text, resolver)
    }

    /// Loads a model file, choosing the format by extension and resolving
    /// side files relative to the file's directory.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] for unsupported extensions, unreadable files or
    /// invalid data.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let format =
            Format::from_path(path).ok_or_else(|| LoadError::UnsupportedFormat(path.to_owned()))?;
        let bytes = std::fs::read(path).map_err(|source| LoadError::Io {
            path: path.to_owned(),
            source,
        })?;
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self::from_bytes(format, &bytes, &FsResolver::new(dir))
    }

    /// Adds a clip spinning the whole model one turn about `axis` (through
    /// the origin) every `period` seconds, as the last clip. The scene gains
    /// a new root node (ids of existing nodes shift by one, and existing
    /// clips are retargeted accordingly).
    #[must_use]
    pub fn with_turntable(mut self, axis: Vec3, period: f32) -> Self {
        let (scene, root) = self
            .scene
            .wrapped_in_root(Node::new("turntable", LocalTransform::default()));
        for clip in &mut self.clips {
            for channel in &mut clip.channels {
                channel.target.0 += 1;
            }
        }
        self.scene = scene;
        self.clips.push(Clip::turntable(root, axis, period));
        self
    }

    /// A renderer for this model playing clip `clip` (if any) with `wrap`.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] for unsupported image sizes or camera nodes.
    pub fn renderer(
        &self,
        clip: Option<usize>,
        wrap: WrapMode,
        settings: FrameSettings,
    ) -> Result<AnimationRenderer<'_>, RenderError> {
        let clip = clip.and_then(|i| self.clips.get(i)).map(|c| (c, wrap));
        AnimationRenderer::new(&self.scene, clip, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemResolver;

    const TRI: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    #[test]
    fn formats_from_extensions() {
        assert_eq!(Format::from_extension("OBJ"), Some(Format::Obj));
        assert_eq!(
            Format::from_path(Path::new("a/b.Dae")),
            Some(Format::Collada)
        );
        assert_eq!(Format::from_path(Path::new("a/b.gltf")), None);
        assert_eq!(Format::from_path(Path::new("noext")), None);
    }

    #[test]
    fn from_bytes_checks_utf8() {
        assert!(matches!(
            Model::from_bytes(Format::Obj, &[0xff, 0xfe], &MemResolver::new()),
            Err(LoadError::Utf8)
        ));
        let m = Model::from_bytes(Format::Obj, TRI.as_bytes(), &MemResolver::new()).unwrap();
        assert_eq!(m.scene.meshes()[0].triangle_count(), 1);
        assert!(m.clips.is_empty());
    }

    #[test]
    fn load_reports_format_and_io_errors() {
        assert!(matches!(
            Model::load("model.gltf"),
            Err(LoadError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            Model::load("does/not/exist.obj"),
            Err(LoadError::Io { .. })
        ));
        let err = Model::load("does/not/exist.obj").unwrap_err();
        assert!(err.to_string().contains("exist.obj"));
    }

    #[test]
    fn turntable_retargets_existing_clips() {
        let dae = include_str!("../../../tests/fixtures/collada_anim/rotate_y.dae");
        let m = Model::from_source(Format::Collada, dae, &MemResolver::new()).unwrap();
        let before = m.clips[0].channels[0].target;
        let m = m.with_turntable(Vec3::Y, 2.0);
        assert_eq!(m.clips.len(), 2);
        assert_eq!(m.clips[0].channels[0].target.0, before.0 + 1);
        assert_eq!(
            m.scene.nodes()[m.clips[0].channels[0].target.0].name,
            "spinner"
        );
        assert_eq!(m.clips[1].channels[0].target.0, 0);
    }
}

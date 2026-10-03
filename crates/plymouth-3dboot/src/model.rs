// SPDX-License-Identifier: GPL-3.0-or-later
//! Loading models of any supported format.

use std::path::{Path, PathBuf};

use crate::anim::{Clip, WrapMode};
use crate::color::Rgba8;
use crate::io::collada::{ColladaError, ColladaOptions, load_collada};
use crate::io::obj::{ObjLoadError, ObjOptions, load_obj};
use crate::io::{FsResolver, ResourceResolver};
use crate::math::{Aabb, Quat, Vec3};
use crate::render::{AnimationRenderer, FrameSettings, RenderError};
use crate::scene::{
    LocalTransform, Material, MaterialId, Node, Scene, Submesh, Transform, primitives,
};

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

    /// Adds a floor: a square of colour `color` just below the model's
    /// lowest point over all its clips, centred under it, with a half
    /// extent of `size` times the model's horizontal radius (non-finite or
    /// non-positive sizes count as 1). Only clips present at this point
    /// count, so add the floor after any turntable. It is scenery (see
    /// [`Node::scenery`]): fixed while the model moves, ignored by camera
    /// framing, and cheap to keep on screen.
    #[must_use]
    pub fn with_floor(mut self, color: Rgba8, size: f32) -> Self {
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            1.0
        };
        let bounds = self
            .clips
            .iter()
            .map(|c| crate::render::animated_bounds(&self.scene, Some(c)))
            .fold(
                crate::render::animated_bounds(&self.scene, None),
                Aabb::union,
            );
        let bounds = if bounds.is_empty() {
            Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5))
        } else {
            bounds
        };
        let extent = bounds.size();
        let radius = (Vec3::new(extent.x, 0.0, extent.z).length() * 0.5).max(extent.y * 0.5);
        // A small gap keeps the floor from z-fighting with the model's base.
        let gap = (extent.y * 0.01).max(radius * 1e-3);
        let center = bounds.center();
        let material = MaterialId(self.scene.materials.len());
        self.scene
            .materials
            .push(Material::with_color("floor", color.to_linear()));
        let mesh = primitives::plane(1.0)
            .with_submeshes(vec![Submesh {
                material,
                indices: 0..6,
            }])
            .expect("the plane has 6 indices");
        let mesh = self.scene.add_mesh(mesh);
        let transform = LocalTransform::Trs(Transform {
            translation: Vec3::new(center.x, bounds.min.y - gap, center.z),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(radius * size, 1.0, radius * size),
        });
        self.scene
            .add_node(
                None,
                Node::new("floor", transform).with_mesh(mesh).as_scenery(),
            )
            .expect("a root node");
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

    #[test]
    fn floors_sit_under_the_model_stay_fixed_and_do_not_change_framing() {
        use crate::color::Rgba8;
        use crate::render::CameraSource;
        let base = Model::from_source(Format::Obj, TRI, &MemResolver::new()).unwrap();
        let green = Rgba8::new(0, 128, 0, 255);
        for floor_first in [false, true] {
            let model = if floor_first {
                base.clone()
                    .with_floor(green, 2.0)
                    .with_turntable(Vec3::Y, 4.0)
            } else {
                base.clone()
                    .with_turntable(Vec3::Y, 4.0)
                    .with_floor(green, 2.0)
            };
            let floor = model
                .scene
                .nodes()
                .iter()
                .position(|n| n.name == "floor")
                .unwrap();
            assert!(model.scene.nodes()[floor].scenery);
            assert_eq!(
                model.scene.nodes()[floor].parent(),
                None,
                "not spun by the turntable"
            );
            let world = model.scene.world_matrices();
            let mesh = model.scene.nodes()[floor].mesh.unwrap();
            let b = model.scene.meshes()[mesh.0]
                .bounds()
                .transformed(&world[floor]);
            // The triangle spans y in [0, 1]; spun about Y it covers a disc of
            // radius 1 around the origin.
            assert!(
                b.max.y < 0.0 && b.max.y > -0.05,
                "just below the model: {b:?}"
            );
            if floor_first {
                // Sized from the rest pose: x in [0, 1], centred at 0.5.
                assert!(b.min.x <= -0.5 && b.max.x >= 1.5, "{b:?}");
            } else {
                assert!(b.max.x >= 2.0 && b.max.z >= 2.0 && b.min.x <= -2.0, "{b:?}");
            }
            let mat = model.scene.meshes()[mesh.0].submeshes()[0].material;
            assert_eq!(model.scene.materials[mat.0].base_color, green.to_linear());

            // Framing (the camera's position) ignores the floor.
            let camera = |m: &Model| {
                let r = m
                    .renderer(Some(0), WrapMode::Loop, FrameSettings::new(32, 32))
                    .unwrap();
                match r.settings().camera {
                    CameraSource::Framed { .. } => r.framed_camera().unwrap().position(),
                    _ => unreachable!(),
                }
            };
            let without = base.clone().with_turntable(Vec3::Y, 4.0);
            assert!(camera(&model).abs_diff_eq(camera(&without), 1e-5));
        }
    }
}

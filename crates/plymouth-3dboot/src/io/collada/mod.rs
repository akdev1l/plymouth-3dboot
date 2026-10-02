// SPDX-License-Identifier: GPL-3.0-or-later
//! COLLADA (`.dae`) loading.
//!
//! A documented subset of COLLADA 1.4/1.5 is supported; unsupported
//! elements are skipped with a warning rather than failing the load.

mod animation;
mod geometry;
mod material;
mod scene;
mod xml;

use crate::anim::Clip;
use crate::io::ResourceResolver;
use crate::scene::Scene;

/// Options for [`load_collada`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColladaOptions {
    /// Rotate the scene so the document's `<up_axis>` becomes +Y.
    pub convert_up_axis: bool,
    /// Scale the scene by `<unit meter>` so that one unit is one metre.
    pub convert_units: bool,
}

impl Default for ColladaOptions {
    fn default() -> Self {
        Self {
            convert_up_axis: true,
            convert_units: true,
        }
    }
}

/// A loaded COLLADA document.
#[derive(Clone, Debug, PartialEq)]
pub struct ColladaModel {
    /// The scene. Its single root node (named `collada`) holds the up-axis
    /// and unit conversion; the document's nodes are below it.
    pub scene: Scene,
    /// Animation clips: one per `<animation_clip>` (playing its
    /// `start`..`end` range), or a single `default` clip with every channel
    /// when the document has animations but no clip library.
    pub clips: Vec<Clip>,
    /// Problems that did not prevent loading (unsupported features).
    pub warnings: Vec<String>,
}

/// Loads a COLLADA document.
///
/// Supported: `<mesh>` geometry (`triangles`, `polylist`, `polygons`),
/// common-profile materials (colours; textures are ignored), node
/// hierarchies with `translate`/`rotate`/`scale`/`matrix`/`lookat`
/// transforms (kept as transform stacks), `instance_node`, and material
/// binding, and transform animations (`<library_animations>` channels
/// targeting `rotate` angles, `translate`/`scale` values or components, and
/// `matrix` elements). Cameras, lights, controllers (skinning) and textures
/// produce warnings. `resolver` is reserved for external resources
/// (textures).
///
/// # Errors
///
/// Returns [`ColladaError`] for malformed XML or structurally invalid data.
pub fn load_collada(
    text: &str,
    _resolver: &impl ResourceResolver,
    options: &ColladaOptions,
) -> Result<ColladaModel, ColladaError> {
    let doc = xml::Document::parse(text)?;
    let (scene, nodes, mut warnings) = scene::build_scene(&doc, options)?;
    let clips = animation::parse_clips(&doc, &nodes, &mut warnings)?;
    Ok(ColladaModel {
        scene,
        clips,
        warnings,
    })
}

/// Loading a COLLADA document failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ColladaError {
    /// The document is not well-formed XML.
    #[error("XML: {0}")]
    Xml(String),
    /// A required element or attribute (`@name`) is missing.
    #[error("line {line}: <{parent}> is missing {element}")]
    Missing {
        /// Missing element name, or `@attribute`.
        element: String,
        /// The element it should be in.
        parent: String,
        /// Line of the parent element.
        line: u32,
    },
    /// The document would produce more nodes or vertices than allowed
    /// (protects against exponential `instance_node` expansion).
    #[error("document too large: {0}")]
    TooLarge(String),
    /// A `#id` reference does not match any element.
    #[error("unresolved reference {0}")]
    UnresolvedUri(String),
    /// A number could not be parsed.
    #[error("line {line}: invalid number {text:?}")]
    InvalidNumber {
        /// The offending text.
        text: String,
        /// Line of the element.
        line: u32,
    },
    /// The data is structurally invalid (e.g. a mesh cannot be built).
    #[error("{0}")]
    Invalid(String),
    /// A primitive refers past the end of an attribute array.
    #[error("line {line}: {what} index {index} out of range ({available} available)")]
    IndexOutOfRange {
        /// `"vertex"`, `"normal"` or `"texcoord"`.
        what: &'static str,
        /// The offending index.
        index: u32,
        /// Number of elements available.
        available: usize,
        /// Line of the primitive.
        line: u32,
    },
    /// An element has the wrong number of values.
    #[error("line {line}: expected {expected} values, found {actual}")]
    Count {
        /// Expected count.
        expected: usize,
        /// Actual count.
        actual: usize,
        /// Line of the element.
        line: u32,
    },
}

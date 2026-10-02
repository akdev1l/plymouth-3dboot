// SPDX-License-Identifier: GPL-3.0-or-later
//! `<visual_scene>`: node hierarchy, transforms and instances.

use std::collections::HashMap;

use roxmltree::Node as XmlNode;

use super::geometry::{Geometry, parse_geometry};
use super::material::parse_material;
use super::xml::{Document, attr, child, children, fixed_numbers};
use super::{ColladaError, ColladaOptions};
use crate::math::{Mat4, Vec3};
use crate::scene::{
    LocalTransform, MaterialId, Mesh, Node, NodeId, Scene, Submesh, TransformOp, TransformOpKind,
};

/// Deepest node nesting followed.
const MAX_DEPTH: usize = 64;
/// Most scene nodes a document may produce (`<instance_node>` can multiply
/// a small document exponentially).
pub(crate) const MAX_NODES: usize = 100_000;
/// Most vertices over all instanced meshes.
pub(crate) const MAX_VERTICES: usize = 10_000_000;

/// The document's up axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpAxis {
    X,
    Y,
    Z,
}

impl UpAxis {
    /// Rotation taking this up axis to +Y (right-handed).
    pub(crate) fn to_y_up(self) -> Mat4 {
        match self {
            // +X -> +Y: +90° about Z.
            Self::X => Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Self::Y => Mat4::IDENTITY,
            // +Z -> +Y: -90° about X.
            Self::Z => Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        }
    }
}

/// Reads `<asset>`: metres per unit and the up axis.
pub(crate) fn asset(doc: &Document<'_>) -> Result<(f32, UpAxis), ColladaError> {
    let Some(asset) = child(doc.root(), "asset") else {
        return Ok((1.0, UpAxis::Y));
    };
    let meter = match child(asset, "unit").and_then(|u| u.attribute("meter")) {
        Some(m) => m
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or_else(|| ColladaError::InvalidNumber {
                text: m.to_owned(),
                line: doc.line(asset),
            })?,
        None => 1.0,
    };
    let up = match child(asset, "up_axis")
        .and_then(|u| u.text())
        .map(str::trim)
    {
        Some("X_UP") => UpAxis::X,
        Some("Z_UP") => UpAxis::Z,
        _ => UpAxis::Y,
    };
    Ok((meter, up))
}

/// The transform stack of a `<node>` (elements in document order).
fn transform_stack(
    doc: &Document<'_>,
    node: XmlNode<'_, '_>,
    warnings: &mut Vec<String>,
) -> Result<Vec<TransformOp>, ColladaError> {
    let mut ops = Vec::new();
    for e in node.children().filter(XmlNode::is_element) {
        let sid = e.attribute("sid").unwrap_or("").to_owned();
        let kind = match e.tag_name().name() {
            "translate" => TransformOpKind::Translate(Vec3::from(fixed_numbers::<3>(doc, e)?)),
            "scale" => TransformOpKind::Scale(Vec3::from(fixed_numbers::<3>(doc, e)?)),
            "rotate" => {
                let [x, y, z, degrees] = fixed_numbers::<4>(doc, e)?;
                TransformOpKind::Rotate {
                    axis: Vec3::new(x, y, z),
                    angle: degrees.to_radians(),
                }
            }
            // COLLADA matrices are written row-major.
            "matrix" => TransformOpKind::Matrix(
                Mat4::from_cols_array(&fixed_numbers::<16>(doc, e)?).transpose(),
            ),
            "lookat" => {
                let [ex, ey, ez, tx, ty, tz, ux, uy, uz] = fixed_numbers::<9>(doc, e)?;
                let view = crate::math::look_at(
                    Vec3::new(ex, ey, ez),
                    Vec3::new(tx, ty, tz),
                    Vec3::new(ux, uy, uz),
                );
                TransformOpKind::Matrix(view.inverse())
            }
            "skew" => {
                warnings.push(format!(
                    "line {}: <skew> is not supported and was ignored",
                    doc.line(e)
                ));
                continue;
            }
            _ => continue,
        };
        ops.push(TransformOp { sid, kind });
    }
    Ok(ops)
}

/// Caches parsed geometries and materials while building the scene.
struct Builder<'d, 'input> {
    doc: &'d Document<'input>,
    scene: Scene,
    nodes: NodeMap,
    /// XML nodes being expanded (to reject `instance_node` cycles).
    expanding: Vec<roxmltree::NodeId>,
    /// Vertices of all mesh instances so far.
    vertices: usize,
    geometries: HashMap<String, Option<Geometry>>,
    materials: HashMap<String, usize>,
    default_material: Option<usize>,
    warnings: Vec<String>,
}

impl Builder<'_, '_> {
    fn material(&mut self, uri: &str) -> Result<usize, ColladaError> {
        if let Some(&i) = self.materials.get(uri) {
            return Ok(i);
        }
        let node = self.doc.by_uri(uri)?;
        let material = parse_material(self.doc, node, &mut self.warnings)?;
        self.scene.materials.push(material);
        let i = self.scene.materials.len() - 1;
        self.materials.insert(uri.to_owned(), i);
        Ok(i)
    }

    fn default_material(&mut self) -> usize {
        *self.default_material.get_or_insert_with(|| {
            self.scene.materials.push(crate::scene::Material::default());
            self.scene.materials.len() - 1
        })
    }

    /// The mesh of an `<instance_geometry>`, with its material symbols bound.
    fn instance_geometry(
        &mut self,
        instance: XmlNode<'_, '_>,
    ) -> Result<Option<Mesh>, ColladaError> {
        let url = attr(self.doc, instance, "url")?;
        if !self.geometries.contains_key(url) {
            let parsed = parse_geometry(self.doc, self.doc.by_uri(url)?, &mut self.warnings)?;
            self.geometries.insert(url.to_owned(), parsed);
        }
        let Some(geometry) = self.geometries[url].clone() else {
            return Ok(None);
        };
        let bindings: HashMap<String, String> = instance
            .descendants()
            .filter(|n| n.tag_name().name() == "instance_material")
            .filter_map(|n| {
                Some((
                    n.attribute("symbol")?.to_owned(),
                    n.attribute("target")?.to_owned(),
                ))
            })
            .collect();
        let mut submeshes = Vec::new();
        for s in geometry.mesh.submeshes() {
            let symbol = &geometry.symbols[s.material.0];
            let material = match bindings.get(symbol) {
                Some(target) => self.material(target)?,
                None => {
                    if !symbol.is_empty() {
                        self.warnings.push(format!("material symbol {symbol:?} of {url} is not bound; using a default material"));
                    }
                    self.default_material()
                }
            };
            submeshes.push(Submesh {
                material: MaterialId(material),
                indices: s.indices.clone(),
            });
        }
        let mesh = geometry
            .mesh
            .with_submeshes(submeshes)
            .map_err(|e| ColladaError::Invalid(e.to_string()))?;
        Ok(Some(mesh))
    }

    /// Adds `node` (and its subtree) under `parent`.
    fn node(
        &mut self,
        node: XmlNode<'_, '_>,
        parent: Option<NodeId>,
        depth: usize,
    ) -> Result<(), ColladaError> {
        if self.expanding.contains(&node.id()) {
            self.warnings.push(format!(
                "line {}: cyclic instance_node reference; skipped",
                self.doc.line(node)
            ));
            return Ok(());
        }
        if self.scene.nodes().len() >= MAX_NODES {
            return Err(ColladaError::TooLarge(format!(
                "more than {MAX_NODES} nodes"
            )));
        }
        self.expanding.push(node.id());
        let result = self.node_contents(node, parent, depth);
        self.expanding.pop();
        result
    }

    fn node_contents(
        &mut self,
        node: XmlNode<'_, '_>,
        parent: Option<NodeId>,
        depth: usize,
    ) -> Result<(), ColladaError> {
        if depth > MAX_DEPTH {
            self.warnings.push(format!(
                "line {}: node nesting deeper than {MAX_DEPTH}; skipped",
                self.doc.line(node)
            ));
            return Ok(());
        }
        let name = node
            .attribute("name")
            .or_else(|| node.attribute("id"))
            .unwrap_or("")
            .to_owned();
        let ops = transform_stack(self.doc, node, &mut self.warnings)?;
        let id = self
            .scene
            .add_node(parent, Node::new(name.clone(), LocalTransform::Stack(ops)))
            .map_err(|e| ColladaError::Invalid(e.to_string()))?;
        if let Some(xml_id) = node.attribute("id") {
            self.nodes.entry(xml_id.to_owned()).or_default().push(id);
        }
        for e in node.children().filter(XmlNode::is_element) {
            match e.tag_name().name() {
                "instance_geometry" => {
                    if let Some(mesh) = self.instance_geometry(e)? {
                        self.vertices = self.vertices.saturating_add(mesh.positions().len());
                        if self.vertices > MAX_VERTICES {
                            return Err(ColladaError::TooLarge(format!(
                                "more than {MAX_VERTICES} instanced vertices"
                            )));
                        }
                        let mesh = self.scene.add_mesh(mesh);
                        let holder =
                            Node::new(format!("{name}#geometry"), LocalTransform::default())
                                .with_mesh(mesh);
                        self.scene
                            .add_node(Some(id), holder)
                            .map_err(|e| ColladaError::Invalid(e.to_string()))?;
                    }
                }
                "node" => self.node(e, Some(id), depth + 1)?,
                "instance_node" => {
                    let target = self.doc.by_uri(attr(self.doc, e, "url")?)?;
                    self.node(target, Some(id), depth + 1)?;
                }
                "instance_camera" | "instance_light" | "instance_controller" => {
                    self.warnings.push(format!(
                        "line {}: <{}> is not supported",
                        self.doc.line(e),
                        e.tag_name().name()
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Builds the scene from the document's `<scene>` (or its first visual
/// scene), wrapped in a root node converting units and up axis.
/// Scene nodes created for each XML `<node id>` (an `<instance_node>` can
/// instantiate the same XML node several times).
pub(crate) type NodeMap = HashMap<String, Vec<NodeId>>;

pub(crate) fn build_scene(
    doc: &Document<'_>,
    options: &ColladaOptions,
) -> Result<(Scene, NodeMap, Vec<String>), ColladaError> {
    let mut b = Builder {
        doc,
        scene: Scene::new(),
        nodes: HashMap::new(),
        expanding: Vec::new(),
        vertices: 0,
        geometries: HashMap::new(),
        materials: HashMap::new(),
        default_material: None,
        warnings: Vec::new(),
    };
    let (meter, up) = asset(doc)?;
    let mut root_ops = Vec::new();
    if options.convert_up_axis && up != UpAxis::Y {
        root_ops.push(TransformOp {
            sid: "up_axis".into(),
            kind: TransformOpKind::Matrix(up.to_y_up()),
        });
    }
    if options.convert_units && (meter - 1.0).abs() > f32::EPSILON {
        root_ops.push(TransformOp {
            sid: "unit".into(),
            kind: TransformOpKind::Scale(Vec3::splat(meter)),
        });
    }
    let root = b
        .scene
        .add_node(None, Node::new("collada", LocalTransform::Stack(root_ops)))
        .map_err(|e| ColladaError::Invalid(e.to_string()))?;

    let visual_scene =
        match child(doc.root(), "scene").and_then(|s| child(s, "instance_visual_scene")) {
            Some(instance) => Some(doc.by_uri(attr(doc, instance, "url")?)?),
            None => children(doc.root(), "library_visual_scenes")
                .flat_map(|l| children(l, "visual_scene"))
                .next(),
        };
    match visual_scene {
        Some(vs) => {
            for node in children(vs, "node") {
                b.node(node, Some(root), 0)?;
            }
        }
        None => b.warnings.push("document has no visual scene".into()),
    }
    Ok((b.scene, b.nodes, b.warnings))
}

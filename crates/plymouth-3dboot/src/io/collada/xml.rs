// SPDX-License-Identifier: GPL-3.0-or-later
//! XML access for COLLADA: id lookup, children by local name, and number
//! arrays. Element names are matched by local name, ignoring namespaces
//! (COLLADA 1.4 and 1.5 use different ones).

use std::collections::HashMap;
use std::str::FromStr;

use roxmltree::{Node, NodeId};

use super::ColladaError;

/// A parsed document with an index of `id` attributes.
pub(crate) struct Document<'input> {
    doc: roxmltree::Document<'input>,
    ids: HashMap<String, NodeId>,
}

impl<'input> Document<'input> {
    /// Parses `text` and indexes every element's `id`.
    pub(crate) fn parse(text: &'input str) -> Result<Self, ColladaError> {
        let doc = roxmltree::Document::parse(text).map_err(|e| ColladaError::Xml(e.to_string()))?;
        let ids = doc
            .descendants()
            .filter_map(|n| n.attribute("id").map(|id| (id.to_owned(), n.id())))
            .collect();
        Ok(Self { doc, ids })
    }

    /// The root element.
    pub(crate) fn root(&self) -> Node<'_, 'input> {
        self.doc.root_element()
    }

    /// The element referred to by a URI fragment (`#id`) or bare id.
    pub(crate) fn by_uri(&self, uri: &str) -> Result<Node<'_, 'input>, ColladaError> {
        let id = uri.strip_prefix('#').unwrap_or(uri);
        self.ids
            .get(id)
            .and_then(|&n| self.doc.get_node(n))
            .ok_or_else(|| ColladaError::UnresolvedUri(uri.to_owned()))
    }

    /// 1-based line of `node` in the source, for error messages.
    pub(crate) fn line(&self, node: Node<'_, '_>) -> u32 {
        self.doc.text_pos_at(node.range().start).row
    }
}

/// Element children of `node` with local name `name`.
pub(crate) fn children<'a, 'input: 'a>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

/// The first element child of `node` named `name`.
pub(crate) fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Like [`child`], but missing children are an error.
pub(crate) fn require<'a, 'input>(
    doc: &Document<'_>,
    node: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, ColladaError> {
    child(node, name).ok_or_else(|| ColladaError::Missing {
        element: name.to_owned(),
        parent: node.tag_name().name().to_owned(),
        line: doc.line(node),
    })
}

/// A required attribute.
pub(crate) fn attr<'a>(
    doc: &Document<'_>,
    node: Node<'a, '_>,
    name: &str,
) -> Result<&'a str, ColladaError> {
    node.attribute(name).ok_or_else(|| ColladaError::Missing {
        element: format!("@{name}"),
        parent: node.tag_name().name().to_owned(),
        line: doc.line(node),
    })
}

/// Whitespace-separated numbers from an element's text.
pub(crate) fn numbers<T: FromStr>(
    doc: &Document<'_>,
    node: Node<'_, '_>,
) -> Result<Vec<T>, ColladaError> {
    node.text()
        .unwrap_or("")
        .split_ascii_whitespace()
        .map(|t| {
            t.parse().map_err(|_| ColladaError::InvalidNumber {
                text: t.to_owned(),
                line: doc.line(node),
            })
        })
        .collect()
}

/// Exactly `N` numbers from an element's text.
pub(crate) fn fixed_numbers<const N: usize>(
    doc: &Document<'_>,
    node: Node<'_, '_>,
) -> Result<[f32; N], ColladaError> {
    let v: Vec<f32> = numbers(doc, node)?;
    v.try_into().map_err(|v: Vec<f32>| ColladaError::Count {
        expected: N,
        actual: v.len(),
        line: doc.line(node),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <library_geometries>
    <geometry id="geo">
      <float_array id="arr" count="4">1 -2.5e1
        3.0 4E-1</float_array>
      <p>0 1 2</p>
      <bad>1 x 3</bad>
    </geometry>
  </library_geometries>
</COLLADA>"#;

    #[test]
    fn resolves_uris_and_bare_ids() {
        let doc = Document::parse(XML).unwrap();
        assert_eq!(doc.by_uri("#geo").unwrap().tag_name().name(), "geometry");
        assert_eq!(doc.by_uri("arr").unwrap().attribute("count"), Some("4"));
        assert!(matches!(doc.by_uri("#nope"), Err(ColladaError::UnresolvedUri(u)) if u == "#nope"));
    }

    #[test]
    fn finds_children_by_local_name_ignoring_namespace() {
        let doc = Document::parse(XML).unwrap();
        let geo = doc.by_uri("#geo").unwrap();
        assert!(child(geo, "p").is_some());
        assert_eq!(children(doc.root(), "library_geometries").count(), 1);
        let err = require(&doc, geo, "mesh").unwrap_err();
        assert_eq!(
            err,
            ColladaError::Missing {
                element: "mesh".into(),
                parent: "geometry".into(),
                line: 4
            }
        );
        assert!(attr(&doc, geo, "name").is_err());
    }

    #[test]
    fn parses_number_arrays_including_scientific_notation() {
        let doc = Document::parse(XML).unwrap();
        let geo = doc.by_uri("#geo").unwrap();
        assert_eq!(
            numbers::<f32>(&doc, doc.by_uri("#arr").unwrap()).unwrap(),
            [1.0, -25.0, 3.0, 0.4]
        );
        assert_eq!(
            numbers::<u32>(&doc, child(geo, "p").unwrap()).unwrap(),
            [0, 1, 2]
        );
        let bad = numbers::<f32>(&doc, child(geo, "bad").unwrap()).unwrap_err();
        assert_eq!(
            bad,
            ColladaError::InvalidNumber {
                text: "x".into(),
                line: 8
            }
        );
        assert_eq!(
            fixed_numbers::<4>(&doc, doc.by_uri("#arr").unwrap()).unwrap(),
            [1.0, -25.0, 3.0, 0.4]
        );
        assert!(matches!(
            fixed_numbers::<3>(&doc, doc.by_uri("#arr").unwrap()),
            Err(ColladaError::Count {
                expected: 3,
                actual: 4,
                ..
            })
        ));
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(matches!(
            Document::parse("<COLLADA><a></COLLADA>"),
            Err(ColladaError::Xml(_))
        ));
    }
}

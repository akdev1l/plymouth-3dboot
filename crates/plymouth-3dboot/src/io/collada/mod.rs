// SPDX-License-Identifier: GPL-3.0-or-later
//! COLLADA (`.dae`) loading.
//!
//! A documented subset of COLLADA 1.4/1.5 is supported; unsupported
//! elements are skipped with a warning rather than failing the load.

// Used by the geometry/material/scene loaders added next.
#[allow(dead_code)]
mod xml;

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

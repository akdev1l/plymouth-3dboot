// SPDX-License-Identifier: GPL-3.0-or-later
//! The model to display.

use plymouth_3dboot::io::MemResolver;
use plymouth_3dboot::{Format, LoadError, Model};

/// The N64 logo sample, embedded so the viewer works without files (e.g. in
/// the browser).
///
/// # Errors
///
/// Returns an error if the embedded model fails to load.
pub fn embedded_n64() -> Result<Model, LoadError> {
    const OBJ: &str = include_str!("../../../tests/fixtures/n64_logo/n64_logo.obj");
    const MTL: &[u8] = include_bytes!("../../../tests/fixtures/n64_logo/n64_logo.mtl");
    let resolver = MemResolver::new()
        .with("n64_logo.mtl", MTL)
        .expect("valid name");
    Model::from_source(Format::Obj, OBJ, &resolver)
}

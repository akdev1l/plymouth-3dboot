// SPDX-License-Identifier: GPL-3.0-or-later
//! Loading the model to display.

use std::path::Path;

use plymouth_3dboot::io::obj::{ObjOptions, load_obj};
use plymouth_3dboot::io::{FsResolver, MemResolver};
use plymouth_3dboot::scene::Scene;

/// The N64 logo sample, embedded so the viewer works without files (e.g. in
/// the browser).
///
/// # Errors
///
/// Returns a message if the embedded model fails to load.
pub fn embedded_n64() -> Result<Scene, String> {
    const OBJ: &str = include_str!("../../../tests/fixtures/n64_logo/n64_logo.obj");
    const MTL: &[u8] = include_bytes!("../../../tests/fixtures/n64_logo/n64_logo.mtl");
    let resolver = MemResolver::new()
        .with("n64_logo.mtl", MTL)
        .map_err(|e| e.to_string())?;
    load_obj(OBJ, &resolver, &ObjOptions::default())
        .map(|m| m.scene)
        .map_err(|e| e.to_string())
}

/// Loads a model file, choosing the format by extension (`.obj`).
///
/// # Errors
///
/// Returns a message for unreadable files, unsupported formats or parse
/// errors.
pub fn load_file(path: &Path) -> Result<Scene, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("obj") => {
            let source =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let dir = path.parent().unwrap_or_else(|| Path::new("."));
            let model = load_obj(&source, &FsResolver::new(dir), &ObjOptions::default())
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &model.warnings {
                eprintln!("warning: {w}");
            }
            Ok(model.scene)
        }
        _ => Err(format!(
            "{}: unsupported model format (expected .obj)",
            path.display()
        )),
    }
}

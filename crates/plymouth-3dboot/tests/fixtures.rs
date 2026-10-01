// SPDX-License-Identifier: GPL-3.0-or-later
//! Integrity checks for the vendored test fixtures, so a checkout that
//! rewrites line endings or loses a file fails loudly and early.

use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn read(name: &str) -> Vec<u8> {
    let path = fixture(name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e} (on wasm, run through `just test-wasm`, which links \
             test binaries with NODERAWFS for host filesystem access)",
            path.display()
        )
    })
}

#[test]
fn n64_logo_files_are_present() {
    for name in [
        "n64_logo/n64_logo.obj",
        "n64_logo/n64_logo.mtl",
        "n64_logo/n64_logo.dae",
        "n64_logo/Readme.txt",
    ] {
        assert!(!read(name).is_empty(), "{name} is empty");
    }
}

#[test]
fn n64_logo_obj_and_mtl_keep_crlf_line_endings() {
    for name in ["n64_logo/n64_logo.obj", "n64_logo/n64_logo.mtl"] {
        let bytes = read(name);
        let lf = bytes.iter().filter(|&&b| b == b'\n').count();
        let crlf = bytes.windows(2).filter(|w| w == b"\r\n").count();
        assert!(lf > 0, "{name} has no lines");
        assert_eq!(crlf, lf, "{name} must use CRLF line endings throughout");
    }
}

#[test]
fn n64_logo_obj_references_vendored_mtl() {
    let obj = String::from_utf8(read("n64_logo/n64_logo.obj")).expect("OBJ is UTF-8");
    let mtllib: Vec<&str> = obj
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix("mtllib "))
        .collect();
    assert_eq!(mtllib, ["n64_logo.mtl"]);
    assert!(fixture("n64_logo/n64_logo.mtl").is_file());
}

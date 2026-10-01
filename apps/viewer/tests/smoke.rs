// SPDX-License-Identifier: GPL-3.0-or-later
//! Runs the viewer binary headless for a few frames.

#[cfg(not(target_os = "emscripten"))]
#[test]
fn viewer_runs_headless_and_exits() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/n64_logo/n64_logo.obj"
    );
    for args in [
        vec!["--frames", "3"],
        vec![fixture, "--shading", "blinn-phong", "--frames", "2"],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_plymouth-3dboot-viewer"))
            .args(&args)
            .env("SDL_VIDEO_DRIVER", "dummy")
            .env("SDL_RENDER_DRIVER", "software")
            .output()
            .expect("viewer starts");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[cfg(not(target_os = "emscripten"))]
#[test]
fn viewer_reports_bad_input() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_plymouth-3dboot-viewer"))
        .args(["missing.obj"])
        .env("SDL_VIDEO_DRIVER", "dummy")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing.obj"));
}

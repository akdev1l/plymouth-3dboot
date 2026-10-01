// SPDX-License-Identifier: GPL-3.0-or-later
//! Browser smoke test for the web viewer (`just browser-smoke`).
//!
//! Serves `target/web` (built by `just viewer-web`) from a minimal HTTP
//! server, loads it in a headless Chromium named by `PLYMOUTH_BROWSER`, and
//! checks that the page initialized and that the canvas shows the N64 logo
//! colours. Skipped when `PLYMOUTH_BROWSER` is not set.

#![cfg(not(target_os = "emscripten"))]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use plymouth_3dboot::color::Rgba8;

/// The N64 logo's colours (from its Readme).
const README_COLOURS: [Rgba8; 4] = [
    Rgba8::new(6, 147, 48, 255),
    Rgba8::new(2, 34, 169, 255),
    Rgba8::new(255, 24, 19, 255),
    Rgba8::new(255, 192, 1, 255),
];

/// Serves files from `root` on an ephemeral port; returns the base URL.
fn serve(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            // Drain headers.
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let name = if path == "/" {
                "index.html"
            } else {
                path.trim_start_matches('/')
            };
            let file = root.join(name);
            let mut stream = &stream;
            let (status, body) = if name.contains("..") {
                ("404 Not Found", Vec::new())
            } else {
                std::fs::read(&file).map_or(("404 Not Found", Vec::new()), |b| ("200 OK", b))
            };
            let mime = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html",
                Some("js") => "text/javascript",
                Some("wasm") => "application/wasm",
                _ => "application/octet-stream",
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream
                .write_all(header.as_bytes())
                .and_then(|()| stream.write_all(&body));
        }
    });
    format!("http://{addr}/")
}

fn browser(program: &str, args: &[&str]) -> std::process::Output {
    let base = [
        "--no-sandbox",
        "--window-size=700,560",
        "--virtual-time-budget=1500",
        "--enable-unsafe-swiftshader",
        "--use-angle=swiftshader",
    ];
    Command::new(program)
        .args(base)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {program}: {e}"))
}

#[test]
fn web_viewer_renders_the_n64_logo() {
    let Ok(program) = std::env::var("PLYMOUTH_BROWSER") else {
        eprintln!(
            "skipped: set PLYMOUTH_BROWSER to a headless Chromium (see `just browser-smoke`)"
        );
        return;
    };
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/web");
    assert!(
        web.join("index.html").is_file(),
        "{} missing: run `just viewer-web` first",
        web.display()
    );
    let url = serve(web);

    // The runtime initialized (index.html sets data-ready).
    let dom = browser(&program, &["--dump-dom", &url]);
    let dom = String::from_utf8_lossy(&dom.stdout);
    assert!(
        dom.contains("data-ready=\"true\""),
        "page did not initialize:\n{dom}"
    );

    // The canvas shows the logo in its exact colours.
    let shot = std::env::temp_dir().join(format!("p3b-browser-{}.png", std::process::id()));
    let out = browser(
        &program,
        &[&format!("--screenshot={}", shot.display()), &url],
    );
    assert!(
        shot.is_file(),
        "no screenshot: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let image = plymouth_3dboot::io::png::decode(&std::fs::read(&shot).unwrap()).unwrap();
    let _ = std::fs::remove_file(&shot);
    for colour in README_COLOURS {
        let count = image.pixels().iter().filter(|&&p| p == colour).count();
        assert!(
            count > 500,
            "colour {colour:?} appears in only {count} pixels"
        );
    }
}

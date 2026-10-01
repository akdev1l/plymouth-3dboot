// SPDX-License-Identifier: GPL-3.0-or-later
//! Headless SDL tests (dummy video driver, software renderer).
//!
//! SDL must be initialized on the main thread, so this file has its own
//! `main` (`harness = false` in Cargo.toml) and runs its checks in order.
//! It answers the libtest `--list` protocol that cargo-nextest uses to
//! discover tests, presenting itself as a single test.

/// Handles `--list` (test discovery); returns `true` if the process should
/// exit without running.
fn list_requested() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "--list") {
        return false;
    }
    if !args.iter().any(|a| a == "--ignored") {
        println!("presenter: test");
    }
    true
}

#[cfg(not(target_os = "emscripten"))]
fn main() {
    if list_requested() {
        return;
    }
    run();
}

#[cfg(not(target_os = "emscripten"))]
fn run() {
    use plymouth_3dboot::color::Rgba8;
    use plymouth_3dboot::target::ColorBuffer;
    use plymouth_3dboot_sdl::{Presenter, WindowConfig};

    sdl3::hint::set("SDL_VIDEO_DRIVER", "dummy");
    sdl3::hint::set("SDL_RENDER_DRIVER", "software");

    let config = WindowConfig {
        title: "headless".into(),
        width: 8,
        height: 4,
        resizable: false,
    };
    let mut presenter = Presenter::new(&config).expect("SDL with the dummy driver");
    assert_eq!(presenter.renderer_name(), "software");
    assert_eq!(presenter.output_size().unwrap(), (8, 4));

    // A frame with distinct values in every channel survives the round trip
    // exactly, which also proves the R, G, B, A byte order.
    let mut frame = ColorBuffer::new(8, 4, Rgba8::BLACK).unwrap();
    for (i, px) in frame.pixels_mut().iter_mut().enumerate() {
        let i = u8::try_from(i).unwrap();
        *px = Rgba8::new(i * 7, 255 - i * 3, i * 5 + 1, 255);
    }
    *frame.get_mut(0, 0).unwrap() = Rgba8::new(255, 0, 0, 255);
    presenter.draw(&frame).unwrap();
    let back = presenter.read_back().unwrap();
    assert_eq!(
        back.get(0, 0),
        Some(Rgba8::new(255, 0, 0, 255)),
        "red stays red"
    );
    assert_eq!(back, frame, "read-back differs from the presented frame");
    presenter.present();
    println!("ok: exact round trip and byte order");

    // A frame of a different size recreates the texture; it is letterboxed.
    let small = ColorBuffer::new(2, 2, Rgba8::WHITE).unwrap();
    presenter.set_clear_color(Rgba8::new(0, 0, 255, 255));
    presenter.draw(&small).unwrap();
    let back = presenter.read_back().unwrap();
    assert_eq!(back.get(0, 0), Some(Rgba8::new(0, 0, 255, 255)), "left bar");
    assert_eq!(
        back.get(7, 3),
        Some(Rgba8::new(0, 0, 255, 255)),
        "right bar"
    );
    assert_eq!(
        back.get(4, 2),
        Some(Rgba8::WHITE),
        "scaled frame in the middle"
    );
    presenter.present();
    println!("ok: resize and letterbox");

    // Going back to the first size works too, and empty frames just clear.
    presenter.show(&frame).unwrap();
    presenter
        .show(&ColorBuffer::new(0, 0, Rgba8::BLACK).unwrap())
        .unwrap();
    println!("ok: texture reuse and empty frames");
}

#[cfg(target_os = "emscripten")]
fn main() {
    if list_requested() {
        return;
    }
    println!("skipped: SDL video needs a browser canvas on emscripten");
}

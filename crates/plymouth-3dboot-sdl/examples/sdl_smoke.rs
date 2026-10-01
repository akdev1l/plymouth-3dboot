// SPDX-License-Identifier: GPL-3.0-or-later
//! Toolchain smoke test: open a window and present CPU-rendered frames with
//! the crate's `Presenter` and frame loop.
//!
//! Natively it presents a few frames and exits, so it also works headless
//! (`SDL_VIDEODRIVER=dummy`). On Emscripten it keeps running in the
//! browser's main loop.

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::target::ColorBuffer;
use plymouth_3dboot_sdl::{App, Control, InputEvent, Presenter, RunOptions, WindowConfig, run};

/// Fills the frame with a colour that changes over time.
struct Pulse {
    time: f64,
    frame: ColorBuffer,
}

impl App for Pulse {
    fn update(&mut self, events: &[InputEvent], dt: f64) -> Control {
        self.time += dt;
        if events.contains(&InputEvent::Quit) {
            Control::Quit
        } else {
            Control::Continue
        }
    }

    fn render(&mut self, _size: (u32, u32)) -> &ColorBuffer {
        // A full red-to-yellow cycle every two seconds.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let shade = ((self.time * 128.0) % 256.0) as u8;
        self.frame.clear(Rgba8::new(255, shade, 0, 255));
        &self.frame
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let presenter = Presenter::new(&WindowConfig {
        title: "plymouth-3dboot smoke".into(),
        width: 64,
        height: 64,
        resizable: false,
    })?;
    println!("renderer: {}", presenter.renderer_name());
    let app = Pulse {
        time: 0.0,
        frame: ColorBuffer::new(64, 64, Rgba8::BLACK)?,
    };
    let max_frames = if cfg!(target_os = "emscripten") {
        None
    } else {
        Some(3)
    };
    if let Some(app) = run(
        presenter,
        app,
        RunOptions {
            max_frames,
            ..RunOptions::default()
        },
    )? {
        println!("presented frames for {:.3} s", app.time);
    }
    Ok(())
}

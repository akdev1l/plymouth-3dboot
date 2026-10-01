// SPDX-License-Identifier: GPL-3.0-or-later
//! Toolchain smoke test: open an SDL3 window and present a CPU-filled
//! RGBA buffer through a streaming texture.
//!
//! Natively it presents a few frames and exits, so it also works headless
//! (`SDL_VIDEODRIVER=dummy`). On Emscripten it runs from the browser's main
//! loop.

use sdl3::pixels::PixelFormat;
use sdl3::render::WindowCanvas;
use sdl3_sys::pixels::SDL_PixelFormat;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
#[cfg(not(target_os = "emscripten"))]
const FRAMES: u32 = 3;

struct State {
    canvas: WindowCanvas,
    pixels: Vec<u8>,
    frame: u32,
}

impl State {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let sdl = sdl3::init()?;
        let video = sdl.video()?;
        let window = video
            .window("plymouth-3dboot smoke", WIDTH, HEIGHT)
            .build()?;
        let canvas = window.into_canvas();
        println!("renderer: {}", canvas.renderer_name);
        Ok(Self {
            canvas,
            pixels: vec![0; (WIDTH * HEIGHT * 4) as usize],
            frame: 0,
        })
    }

    /// Renders and presents one frame.
    fn step(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let shade = u8::try_from(self.frame % 256)?;
        for px in self.pixels.as_chunks_mut::<4>().0 {
            *px = [255, shade, 0, 255];
        }
        // RGBA32 is byte-order R,G,B,A on every host endianness.
        let format = PixelFormat::try_from(SDL_PixelFormat::RGBA32)?;
        let creator = self.canvas.texture_creator();
        let mut texture = creator.create_texture_streaming(format, WIDTH, HEIGHT)?;
        texture.update(None, &self.pixels, (WIDTH * 4) as usize)?;
        self.canvas.copy(&texture, None, None)?;
        self.canvas.present();
        self.frame += 1;
        Ok(())
    }
}

#[cfg(not(target_os = "emscripten"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = State::new()?;
    while state.frame < FRAMES {
        state.step()?;
    }
    println!("presented {} frames", state.frame);
    Ok(())
}

#[cfg(target_os = "emscripten")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::ffi::{c_int, c_void};

    unsafe extern "C" {
        fn emscripten_set_main_loop_arg(
            func: extern "C" fn(*mut c_void),
            arg: *mut c_void,
            fps: c_int,
            simulate_infinite_loop: c_int,
        );
    }

    extern "C" fn tick(arg: *mut c_void) {
        // SAFETY: `arg` is the leaked `Box<State>` registered below; the
        // browser main loop calls this on a single thread, never reentrantly.
        let state = unsafe { &mut *arg.cast::<State>() };
        if let Err(err) = state.step() {
            eprintln!("frame failed: {err}");
        }
    }

    let state = Box::into_raw(Box::new(State::new()?));
    // SAFETY: `tick` matches the expected callback ABI and `state` stays
    // valid forever (it is intentionally leaked to the browser main loop).
    unsafe { emscripten_set_main_loop_arg(tick, state.cast(), 0, 1) };
    Ok(())
}

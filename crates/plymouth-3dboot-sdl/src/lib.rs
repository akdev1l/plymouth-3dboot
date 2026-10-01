// SPDX-License-Identifier: GPL-3.0-or-later
//! SDL3 presentation and main loop for `plymouth-3dboot`.
//!
//! Runs natively (system SDL3) and on `wasm32-unknown-emscripten`
//! (Emscripten's SDL3 port). Only the SDL 3.2 API is used (see
//! `docs/toolchain.md`).

#[cfg(target_os = "emscripten")]
mod emscripten;
pub mod input;
pub mod main_loop;
pub mod presenter;

pub use input::{InputEvent, map_event};
pub use main_loop::{App, Control, RunOptions, Runner, run};
pub use presenter::{Presenter, SdlError, WindowConfig};

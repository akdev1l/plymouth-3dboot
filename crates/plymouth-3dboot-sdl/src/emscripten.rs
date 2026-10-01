// SPDX-License-Identifier: GPL-3.0-or-later
//! The Emscripten main loop (the only `unsafe` FFI in this crate).

use std::ffi::{c_int, c_void};

unsafe extern "C" {
    fn emscripten_set_main_loop_arg(
        func: extern "C" fn(*mut c_void),
        arg: *mut c_void,
        fps: c_int,
        simulate_infinite_loop: c_int,
    );
    fn emscripten_cancel_main_loop();
}

type Frame = Box<dyn FnMut() -> bool>;

extern "C" fn trampoline(arg: *mut c_void) {
    // SAFETY: `arg` is the `Box<Frame>` leaked in `set_main_loop`; the
    // browser calls this on the main thread, one frame at a time, and the
    // pointer stays valid because the box is never freed.
    let frame = unsafe { &mut *arg.cast::<Frame>() };
    if !frame() {
        // SAFETY: called from within the main-loop callback, as required.
        unsafe { emscripten_cancel_main_loop() };
    }
}

/// Calls `frame` once per browser animation frame until it returns `false`.
///
/// Returns immediately (`simulate_infinite_loop = 0`): `main` may return
/// while the runtime stays alive. Throwing through Rust frames, as
/// `simulate_infinite_loop = 1` does, is unsound with wasm exceptions.
pub(crate) fn set_main_loop(frame: impl FnMut() -> bool + 'static) {
    let boxed: Box<Frame> = Box::new(Box::new(frame));
    let arg = Box::into_raw(boxed).cast::<c_void>();
    // SAFETY: `trampoline` has the expected C ABI and `arg` points to a
    // leaked `Box<Frame>` that lives for the rest of the program.
    unsafe { emscripten_set_main_loop_arg(trampoline, arg, 0, 0) };
}

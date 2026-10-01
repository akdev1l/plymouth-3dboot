<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Toolchain

Everything is pinned in [`Containerfile`](../Containerfile) and
[`rust-toolchain.toml`](../rust-toolchain.toml). Run any command inside that
environment with `scripts/dev.sh <command>`.

## Version matrix

| Component | Version | Notes |
|---|---|---|
| Base image | `rust:1.98.1-slim-trixie` (pinned by digest) | Debian trixie |
| rustc / cargo | 1.98.1 (LLVM 22.1.8) | |
| Emscripten (emsdk) | 6.0.10 | Uses an LLVM newer than rustc's, as required to link rustc's wasm objects |
| node (from emsdk) | 24.19.0 | Runs wasm test binaries |
| SDL3, native | 3.2.10 (Debian `libsdl3-dev`) | Found through pkg-config |
| SDL3, Emscripten port | 3.4.2 (`embuilder build sdl3`) | Prebuilt in the image; always statically linked |
| `sdl3` crate | 0.20.0 | Safe wrapper |
| `sdl3-sys` crate | 0.7.1 (bindings generated from SDL 3.4.16 headers) | |
| just / nextest / deny / llvm-cov / cargo-c | 1.58.0 / 0.9.146 / 0.20.2 / 0.9.1 / 0.10.25 | `cargo install --locked` |

### SDL API level

The bindings cover SDL 3.4, but the native library is 3.2.10. **Use only
the SDL 3.2 API.** Calling a 3.4-only function links on the web but fails to
link natively, which `just check` catches.

## WebAssembly (`wasm32-unknown-emscripten`)

- Configured in [`.cargo/config.toml`](../.cargo/config.toml): the runner is
  `node`, and links use `-sALLOW_MEMORY_GROWTH=1`.
- Panics use rustc's default strategy for this target. Unwinding,
  `#[should_panic]`, `catch_unwind`, filesystem access under `NODERAWFS` and
  heap growth past 256 MiB were all verified under node. No extra flags are
  needed.
- libtest runs tests serially on this target (there are no threads).
- Test binaries also link with `-sNODERAWFS=1`, which gives direct
  host-filesystem access for fixtures and golden images. It is passed only by
  `just test-wasm`, through `--config` and a separate `target/wasm-test`
  directory, because NODERAWFS breaks browser builds.
- **Known noise:** rustc 1.98 passes `-sWASM_BIGINT`, which emsdk 6 reports as
  deprecated. `-Wno-deprecated` is set on wasm links to silence it. Remove it
  once rustc stops passing the flag.
- `sdl3-sys` links `-lSDL3`, and emcc resolves that to its SDL3 port with no
  extra flags.

## SDL smoke test

`crates/plymouth-3dboot-sdl/examples/sdl_smoke.rs` opens a window and presents
a CPU-filled RGBA buffer through a streaming texture (`SDL_PIXELFORMAT_RGBA32`):

```sh
# Native, headless (software renderer)
scripts/dev.sh env SDL_VIDEODRIVER=dummy cargo run -p plymouth-3dboot-sdl --example sdl_smoke
# Web (build only; running it needs a browser, see Phase 6)
scripts/dev.sh cargo build -p plymouth-3dboot-sdl --example sdl_smoke --target wasm32-unknown-emscripten
```

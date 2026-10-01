<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# plymouth-3dboot

A CPU rasterizer for animated 3D models, written in Rust, for use in a
[Plymouth](https://gitlab.freedesktop.org/plymouth/plymouth) boot-splash theme.

- **Core library** (`crates/plymouth-3dboot`): pure Rust with no `unsafe` and
  no platform dependencies. It loads models (OBJ/MTL and COLLADA), evaluates
  animation at any time `t`, and rasterizes deterministically into an RGBA
  framebuffer.
- **SDL3 presenter** (`crates/plymouth-3dboot-sdl`): shows frames in a window,
  natively or in the browser through Emscripten.
- **Viewer** (`apps/viewer`): an interactive model viewer for both targets.
- **Test support** (`crates/plymouth-3dboot-testutil`): golden-image comparison, for development only.
- **C API and Plymouth plugin**: planned (Phases 11–12 of the plan).

Targets: `x86_64-unknown-linux-gnu` and `wasm32-unknown-emscripten`.

Status: early development. See [`PLAN.md`](PLAN.md) for the phased
execution plan.

## Development environment

Everything runs in a pinned container (podman), defined in
[`Containerfile`](Containerfile):

```sh
scripts/dev.sh just check    # full quality gate (builds the image on first use)
scripts/dev.sh just --list   # all recipes
scripts/dev.sh               # interactive shell
```

### Viewer

`apps/viewer` shows a model (the embedded N64 logo by default):

```sh
cargo run -p plymouth-3dboot-viewer -- [MODEL.obj|MODEL.dae] [--shading unlit|lambert|blinn-phong] \
    [--turntable SECONDS | --still]
```

Run it on the host for a window; this needs SDL3 development files, for
example `dnf install SDL3-devel`. The container has no display, so
`scripts/dev.sh just viewer --frames 3` only runs it headless. Controls:
- drag with the left mouse button to orbit;
- scroll to zoom;
- Space pauses;
- `+`/`-` change the speed;
- R resets the view;
- Escape or Q quits.

Pinned versions and WebAssembly notes are in [`docs/toolchain.md`](docs/toolchain.md).
Coordinate, pixel and colour conventions are in [`docs/conventions.md`](docs/conventions.md).

## Change protocol

1. Before a change, `git status` is clean and `scripts/dev.sh just check` passes.
2. Write the tests with the change, first where practical.
3. After the change, `just check` passes and only the intended files changed.
4. Commit with a [Conventional Commit](https://www.conventionalcommits.org/) message, one logical change per commit.

## Licence

GPL-3.0-or-later. See [`LICENSE`](LICENSE). Test fixtures carry their own
provenance in [`tests/fixtures/README.md`](tests/fixtures/README.md).

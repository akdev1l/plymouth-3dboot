<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Toolchain

Everything is pinned in [`Containerfile`](../Containerfile) and
[`rust-toolchain.toml`](../rust-toolchain.toml). Run any command inside that
environment with `scripts/dev.sh <command>`.

`scripts/dev.sh` requires podman 4.3 or later (for `--userns=keep-id:uid=…`).
It tags the image with a hash of both files, rebuilds it when either changes,
and prunes superseded images and Emscripten cache volumes.

## Fedora container

[`Containerfile.fedora`](../Containerfile.fedora) (`fedora:44`, pinned by
digest) builds and tests the C library and the Plymouth plugin against
Fedora's toolchain and Plymouth, and builds the RPMs. Select it with
`DEV_CONTAINER=fedora`:

```sh
DEV_CONTAINER=fedora scripts/dev.sh just test-c test-plymouth
DEV_CONTAINER=fedora scripts/dev.sh just rpm     # RPMs in dist/rpm/
```

It uses Fedora's packages only: rust/cargo 1.98.1 (no rustup, so
`rust-toolchain.toml` does not apply), cargo-c 0.10.24, Plymouth 24.004.60
with its label plugin, meson, rpm-build, cargo-rpm-macros and
cargo-vendor-filterer. Fedora keeps only the latest update of each package,
so packages follow Fedora 44 updates rather than exact versions. The
container mounts its own volume on `target/`, so its build outputs never mix
with the Debian container's. It has no Emscripten or SDL: `just check` stays
in the Debian container.

## Continuous integration (GitHub Actions)

- [`builder-image.yml`](../.github/workflows/builder-image.yml) builds
  `Containerfile.fedora` and pushes it to
  `ghcr.io/<owner>/plymouth-3dboot-builder-fedora`, tagged with the content
  hash from `DEV_CONTAINER=fedora scripts/dev.sh --image` and with `latest`.
  It runs when `Containerfile.fedora`, `rust-toolchain.toml` or the workflow
  change on `main`, and on demand (*Run workflow*).
- [`rpm.yml`](../.github/workflows/rpm.yml) runs `just rpm` through
  `scripts/dev.sh`, as locally, on pull requests (the PR check, *Build
  RPMs*), on pushes to `main` and on demand, and uploads the RPMs as the
  `rpms` artifact. It pulls the builder image for the commit's hash; if
  there is none (a pull request that changes `Containerfile.fedora`),
  `scripts/dev.sh` builds the image in the job.

- [`release.yml`](../.github/workflows/release.yml) runs release-please on
  every push to `main` ([`release-please-config.json`](../release-please-config.json),
  [`.release-please-manifest.json`](../.release-please-manifest.json)). It
  keeps a release pull request open that updates `CHANGELOG.md` from the
  conventional commits and bumps the version in `Cargo.toml`, `Cargo.lock`
  (the workspace crates, which have no `source`), `plymouth/meson.build`
  and the RPM spec (marked with `x-release-please-version` comments).
  Merging it tags `vX.Y.Z`, creates the GitHub release and attaches the RPMs
  built at that commit. Before 1.0, `feat` commits bump the minor version
  and breaking changes too (`bump-minor-pre-major`); the first release is
  0.1.0.

The image package is created on the first run of `builder-image.yml`.
Pull requests from forks can only pull it if the package is public
(package settings on GitHub); otherwise their jobs build the image
themselves, which works but takes longer. To make the check required, add
*Build RPMs* to the branch protection rules of `main`.

Pull requests opened with the default `GITHUB_TOKEN`, like release-please's,
do not trigger other workflows, so the *Build RPMs* check does not run on
the release pull request. If that check is required, give release-please a
personal access token or GitHub App token (the action's `token` input), or
merge the release pull request as an administrator.

## Version matrix

| Component | Version | Notes |
|---|---|---|
| Base image | `rust:1.98.1-slim-trixie` (pinned by digest) | Debian trixie |
| Debian packages | snapshot.debian.org `20261001T000000Z`, exact versions | Rebuilds are reproducible or fail loudly |
| rustc / cargo | 1.98.1 (LLVM 22.1.8) | |
| Emscripten (emsdk) | 6.0.10 (commit `a2b9277`, verified at build) | Uses an LLVM newer than rustc's, as required to link rustc's wasm objects |
| node (from emsdk) | 24.19.0 | Runs wasm test binaries |
| SDL3, native | 3.2.10 (Debian `libsdl3-dev`) | Found through pkg-config |
| SDL3, Emscripten port | 3.4.2 (`embuilder build sdl3`) | Prebuilt in the image; always statically linked |
| `sdl3` crate | 0.20.0 | Safe wrapper |
| `sdl3-sys` crate | 0.7.1 (bindings generated from SDL 3.4.16 headers) | |
| just / nextest / deny / llvm-cov / cargo-c | 1.58.0 / 0.9.146 / 0.20.2 / 0.9.1 / 0.10.25 | `cargo install --locked` |

### SDL API level

The bindings cover SDL 3.4.16, but the native library is 3.2.10 and the
Emscripten port is 3.4.2. **Use only the SDL 3.2 API.** A newer function
fails to link on at least one target. `just check` builds both, so it
catches this.

## WebAssembly (`wasm32-unknown-emscripten`)

- Configured in [`.cargo/config.toml`](../.cargo/config.toml): the runner is
  `node`, and links use `-sALLOW_MEMORY_GROWTH=1` and `-sSTACK_SIZE=1MB`.
  Emscripten's default 64 KiB stack overflows in debug builds: PNG encoding
  alone needs more.
- Panics use rustc's default strategy for this target (native wasm
  exceptions). Unwinding, `#[should_panic]`, `catch_unwind`, filesystem access
  under `NODERAWFS` and heap growth past 256 MiB were all verified under node.
  No panic-related flags are needed.
- libtest runs tests serially on this target (there are no threads).
- Doctests run natively only. Cargo would cross-compile them, but rustdoc
  does not receive the wasm link flags (such as the stack size) from
  `target.rustflags`.
- Test binaries also link with `-sNODERAWFS=1`, which gives direct
  host-filesystem access for fixtures and golden images. It is passed only by
  `just test-wasm`, through `--config` and a separate `target/wasm-test`
  directory, because NODERAWFS breaks browser builds.
- **Known noise:** rustc 1.98 passes `-sWASM_BIGINT`, which emsdk 6 reports as
  deprecated. `-Wno-deprecated` is set on wasm links to silence it. Remove it
  once rustc stops passing the flag.
- `sdl3-sys` links `-lSDL3`, and emcc resolves that to its SDL3 port with no
  extra flags.
- Emscripten main loops are registered with `simulate_infinite_loop = 0`.
  `main` returns, and the runtime stays alive. The alternative throws a JS
  exception through Rust frames, which is unsound with wasm exceptions.

## SDL smoke test

`crates/plymouth-3dboot-sdl/examples/sdl_smoke.rs` opens a window and presents
CPU-filled frames with the crate's `Presenter` (streaming `SDL_PIXELFORMAT_RGBA32`
texture) and frame loop:

```sh
# Native, headless (software renderer)
scripts/dev.sh env SDL_VIDEODRIVER=dummy cargo run -p plymouth-3dboot-sdl --example sdl_smoke
# Web (build only; running it needs a browser, see Phase 6)
scripts/dev.sh cargo build -p plymouth-3dboot-sdl --example sdl_smoke --target wasm32-unknown-emscripten
```

## Web viewer

`just viewer-web` builds the viewer for the browser into `target/web/`:
`index.html` (from `apps/viewer/web/`), `plymouth-3dboot-viewer.js` and
`plymouth_3dboot_viewer.wasm`. The JS loader expects the `.wasm` under its
crate name, with underscores. Serve the directory over HTTP, for example
`python3 -m http.server -d target/web`; `file://` URLs cannot load wasm.

Sizes at the time of writing (release, embedded N64 model): `.wasm`
1.2 MB, `.js` 180 KB. `just check` fails if the `.wasm` exceeds 4 MiB.

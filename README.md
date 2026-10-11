<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# plymouth-3dboot

[![RPM](https://github.com/akdev1l/plymouth-3dboot/actions/workflows/rpm.yml/badge.svg?branch=main&event=push)](https://github.com/akdev1l/plymouth-3dboot/actions/workflows/rpm.yml?query=branch%3Amain)
[![Builder image](https://github.com/akdev1l/plymouth-3dboot/actions/workflows/builder-image.yml/badge.svg?branch=main)](https://github.com/akdev1l/plymouth-3dboot/actions/workflows/builder-image.yml?query=branch%3Amain)

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
- **C API** (`crates/plymouth-3dboot-capi`): the library as
  `libplymouth_3dboot` with a C header ([docs/c-api.md](docs/c-api.md)).
- **Plymouth plugin and theme** (`plymouth/`): a splash plugin that renders
  a model at boot, and a demo theme with the spinning N64 logo
  ([docs/plymouth.md](docs/plymouth.md)).

Targets: `x86_64-unknown-linux-gnu` and `wasm32-unknown-emscripten`.

See [`PLAN.md`](PLAN.md) for the phased execution plan.

## Installing on Fedora

Each [release](https://github.com/akdev1l/plymouth-3dboot/releases) has RPMs
for Fedora 44, 45 and rawhide (x86_64). Three packages are needed: the
library (`plymouth-3dboot`), the plugin (`plymouth-plugin-3dboot`) and the
theme (`plymouth-theme-3dboot-n64`). The `-devel`, `-debuginfo`,
`-debugsource` and `.src.rpm` files are not.

Download the three for your Fedora version, either from the release page or
with the [GitHub CLI](https://cli.github.com/):

```sh
fc="fc$(rpm -E %fedora)"    # fc44, fc45, or fc46 on rawhide
gh release download --repo akdev1l/plymouth-3dboot --dir plymouth-3dboot-rpms \
    --pattern "plymouth-3dboot-[0-9]*.${fc}.x86_64.rpm" \
    --pattern "plymouth-plugin-3dboot-[0-9]*.${fc}.x86_64.rpm" \
    --pattern "plymouth-theme-3dboot-n64-[0-9]*.${fc}.noarch.rpm"
```

Then install them and select the theme. On Fedora Workstation and other
mutable variants:

```sh
sudo dnf install ./plymouth-3dboot-rpms/*.rpm
sudo plymouth-set-default-theme -R 3dboot-n64    # -R rebuilds the initramfs
```

On Fedora Atomic (Silverblue, Kinoite, uBlue images), layer the packages and
have the initramfs regenerated locally, then reboot:

```sh
sudo rpm-ostree install ./plymouth-3dboot-rpms/*.rpm
printf '[Daemon]\nTheme=3dboot-n64\n' | sudo tee /etc/plymouth/plymouthd.conf
sudo rpm-ostree initramfs --enable
systemctl reboot
```

To go back, select the previous theme again (`bgrt` is Fedora's default) in
the same way; on Atomic, `sudo rpm-ostree rollback` undoes the whole change.
[docs/plymouth.md](docs/plymouth.md#installing) has the details: updating,
removing, trying the theme without rebooting, and the theme's options.

## Development environment

Everything runs in a pinned container (podman), defined in
[`Containerfile`](Containerfile). [`Containerfile.fedora`](Containerfile.fedora)
builds the Fedora RPMs ([packaging/](packaging/plymouth-3dboot.spec)):

```sh
scripts/dev.sh just check    # full quality gate (builds the image on first use)
scripts/dev.sh just --list   # all recipes
scripts/dev.sh               # interactive shell
DEV_CONTAINER=fedora scripts/dev.sh just rpm   # Fedora RPMs in dist/rpm/ (also built by CI)
```

### Viewer

`apps/viewer` shows a model (the embedded N64 logo by default), playing its
COLLADA animation or else spinning it on a turntable:

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

### Offline rendering

```sh
cargo run -p plymouth-3dboot --example animate -- MODEL --out spin.gif   # or --out frames/
```

The example renders a model's animation (or a turntable) to an animated GIF
or to numbered PNG frames. See the file header for options.

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

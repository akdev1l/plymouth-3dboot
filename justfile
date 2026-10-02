# SPDX-License-Identifier: GPL-3.0-or-later
#
# Task runner. Run inside the dev container: `scripts/dev.sh just <recipe>`.

set positional-arguments

wasm_target := "wasm32-unknown-emscripten"
# Size budget for the release web viewer's .wasm (docs/toolchain.md).
web_wasm_budget := "4194304"
# Test binaries need host filesystem access under node (see docs/toolchain.md).
wasm_test_flags := "--target " + wasm_target + " --target-dir target/wasm-test --config 'target." + wasm_target + ".rustflags=[\"-Clink-arg=-sNODERAWFS=1\"]'"

# List recipes.
default:
    @just --list

# Full quality gate; must pass before and after every change.
check: fmt-check clippy test doc build-wasm test-wasm smoke-sdl smoke-example capi browser-smoke deny

# Format all code.
fmt:
    cargo fmt --all

# Verify formatting.
fmt-check:
    cargo fmt --all --check

# Lint native and wasm builds, warnings are errors.
clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo clippy --locked --workspace --all-targets --target {{wasm_target}} -- -D warnings
    cargo clippy --locked -p plymouth-3dboot --no-default-features --all-targets -- -D warnings

# Native tests (nextest) and doctests.
test *args:
    cargo nextest run --locked --workspace --no-tests=pass "$@"
    cargo test --locked --workspace --doc

# Tests compiled to wasm and run under node.
test-wasm *args:
    # Doctests run natively (`just test`): rustdoc does not get the wasm link
    # flags (stack size) from target.rustflags.
    cargo test --locked --workspace --lib --bins --tests {{wasm_test_flags}} "$@"

# Build every target for wasm.
build-wasm:
    cargo build --locked --workspace --all-targets --target {{wasm_target}}

# Build API docs; warnings are errors.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps

# Run the SDL smoke example headless.
smoke-sdl:
    SDL_VIDEODRIVER=dummy cargo run --locked -p plymouth-3dboot-sdl --example sdl_smoke

# Run the native viewer (headless in the container: add `--frames N`;
# for a window, run it on the host with SDL3 installed).
viewer *args:
    cargo run --locked -p plymouth-3dboot-viewer -- "$@"

# Build the web viewer into target/web (serve that directory over HTTP).
viewer-web:
    cargo build --locked --release -p plymouth-3dboot-viewer --target {{wasm_target}}
    rm -rf target/web && mkdir -p target/web
    cp apps/viewer/web/index.html target/web/
    cp target/{{wasm_target}}/release/plymouth-3dboot-viewer.js target/{{wasm_target}}/release/plymouth_3dboot_viewer.wasm target/web/
    @ls -l target/web
    @size=$(stat -c %s target/web/plymouth_3dboot_viewer.wasm); \
        if [ "$size" -gt {{web_wasm_budget}} ]; then echo "wasm is $size bytes, over the {{web_wasm_budget}} budget"; exit 1; fi

# Load the web viewer in headless Chromium and check what it renders.
browser-smoke: viewer-web
    PLYMOUTH_BROWSER=chromium-headless-shell cargo nextest run --locked -p plymouth-3dboot-viewer --test browser --no-capture

# Render the animated N64 sample offline to a GIF and PNG frames.
smoke-example:
    rm -rf target/tmp/animate && mkdir -p target/tmp/animate
    cargo run --locked -q -p plymouth-3dboot --example animate -- tests/fixtures/n64_logo/n64_logo_spin.dae --out target/tmp/animate/spin.gif --frames 4 --size 64x64
    cargo run --locked -q -p plymouth-3dboot --example animate -- tests/fixtures/n64_logo/n64_logo.obj --out target/tmp/animate/frames --frames 2 --size 32x32 --shading lambert
    test -s target/tmp/animate/spin.gif && test -s target/tmp/animate/frames/frame0001.png

# Build the C library (cargo-c) and check its artifacts and exports.
capi:
    cargo cbuild --locked -p plymouth-3dboot-capi --target-dir target/capi
    scripts/check-capi.sh target/capi/x86_64-unknown-linux-gnu/debug

# Licence, advisory, ban and source policy (deny.toml).
deny:
    cargo deny --locked check

# Re-run tests, regenerating golden images (review the diffs before committing).
golden-update:
    UPDATE_GOLDEN=1 cargo nextest run --locked --workspace --no-tests=pass

# Line coverage summary for the workspace.
cov:
    cargo llvm-cov nextest --locked --workspace --no-tests=pass

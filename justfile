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
check: fmt-check clippy test doc build-wasm test-wasm smoke-sdl viewer-web deny

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

# Native tests (nextest) and doctests.
test *args:
    cargo nextest run --locked --workspace --no-tests=pass "$@"
    cargo test --locked --workspace --doc

# Tests compiled to wasm and run under node.
test-wasm *args:
    cargo test --locked --workspace {{wasm_test_flags}} "$@"

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

# Licence, advisory, ban and source policy (deny.toml).
deny:
    cargo deny --locked check

# Re-run tests, regenerating golden images (review the diffs before committing).
golden-update:
    UPDATE_GOLDEN=1 cargo nextest run --locked --workspace --no-tests=pass

# Line coverage summary for the workspace.
cov:
    cargo llvm-cov nextest --locked --workspace --no-tests=pass

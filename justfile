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
check: fmt-check clippy test doc build-wasm test-wasm smoke-sdl smoke-example capi test-c test-plymouth plymouth-install budgets browser-smoke deny

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

# Build and run the C API tests (shared and static) under valgrind.
test-c: capi
    scripts/test-c.sh target/capi/x86_64-unknown-linux-gnu/debug

# Build the Plymouth splash plugin (meson) against the freshly built C library.
plymouth: capi
    rm -rf target/plymouth
    PKG_CONFIG_PATH="$PWD/target/capi/x86_64-unknown-linux-gnu/debug:${PKG_CONFIG_PATH:-}" \
        meson setup target/plymouth plymouth >/dev/null
    meson compile -C target/plymouth
    @nm -D --defined-only target/plymouth/plymouth-3dboot.so | awk '$2 == "T" {print $3}' | grep -qx ply_boot_splash_plugin_get_interface
    @echo "plugin OK: exports ply_boot_splash_plugin_get_interface"

# Install the plugin and demo theme into a scratch root and check the result.
plymouth-install: plymouth
    rm -rf target/plymouth-root
    meson install -C target/plymouth --destdir "$PWD/target/plymouth-root" >/dev/null
    test -f target/plymouth-root/usr/lib/x86_64-linux-gnu/plymouth/plymouth-3dboot.so
    test -f target/plymouth-root/usr/share/plymouth/themes/3dboot-n64/n64_logo_spin.dae
    grep -qx 'ModelFile=/usr/share/plymouth/themes/3dboot-n64/n64_logo_spin.dae' target/plymouth-root/usr/share/plymouth/themes/3dboot-n64/3dboot-n64.plymouth
    @echo "plugin and theme install OK"

# Run the plugin's headless harness under valgrind.
test-plymouth: plymouth
    # The build directory has no soname link (cargo cinstall creates it).
    ln -sf libplymouth_3dboot.so target/capi/x86_64-unknown-linux-gnu/debug/libplymouth_3dboot.so.0
    meson test -C target/plymouth --print-errorlogs \
        --wrapper "valgrind --quiet --error-exitcode=1 --leak-check=full --errors-for-leak-kinds=definite,indirect --suppressions=$PWD/plymouth/tests/valgrind.supp"


# Check size and speed budgets of the release C library (docs/perf.md).
budgets:
    cargo cbuild --locked --release -p plymouth-3dboot-capi --target-dir target/capi
    rm -rf target/tmp/budget && mkdir -p target/tmp/budget
    cp target/capi/x86_64-unknown-linux-gnu/release/libplymouth_3dboot.so target/tmp/budget/
    ln -sf libplymouth_3dboot.so target/tmp/budget/libplymouth_3dboot.so.0
    cc -O2 -std=c11 crates/plymouth-3dboot-capi/examples/budget.c -I target/capi/x86_64-unknown-linux-gnu/release/include \
        -L target/tmp/budget -lplymouth_3dboot -Wl,-rpath,"$PWD/target/tmp/budget" -o target/tmp/budget/budget
    strip -o target/tmp/budget/stripped.so target/tmp/budget/libplymouth_3dboot.so
    @size=$(stat -c %s target/tmp/budget/stripped.so); echo "stripped library: $size bytes"; \
        [ "$size" -lt 2097152 ] || { echo "over the 2 MiB size budget"; exit 1; }
    @out=$(target/tmp/budget/budget tests/fixtures/n64_logo/n64_logo_spin.dae 1920 1080 20); echo "1080p: $out"; \
        echo "$out" | awk -F'[= ]' '{ if ($2 >= 500 || $4 >= 200) { print "over the 1080p time budget"; exit 1 } }'

# Regenerate the committed C header after an intended API change.
capi-header:
    cargo cbuild --locked -p plymouth-3dboot-capi --target-dir target/capi
    cp target/capi/x86_64-unknown-linux-gnu/debug/include/plymouth-3dboot.h crates/plymouth-3dboot-capi/include/

# Licence, advisory, ban and source policy (deny.toml).
deny:
    cargo deny --locked check

# Re-run tests, regenerating golden images (review the diffs before committing).
golden-update:
    UPDATE_GOLDEN=1 cargo nextest run --locked --workspace --no-tests=pass

# Line coverage summary for the workspace.
cov:
    cargo llvm-cov nextest --locked --workspace --no-tests=pass

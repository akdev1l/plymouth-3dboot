#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the C API tests against the generated header and both the shared
# and the static library, and runs them under valgrind (`just test-c`).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lib="${1:?usage: test-c.sh BUILD_DIR}"
src="${root}/crates/plymouth-3dboot-capi/tests/c"
out="${root}/target/tmp/c-tests"
fixtures="${root}/tests/fixtures/n64_logo"
mkdir -p "${out}/lib"
# The build directory has no soname link (cargo cinstall creates it):
# stage the shared library under both names.
cp "${lib}/libplymouth_3dboot.so" "${out}/lib/"
major="$(sed -n 's/^version = "\([0-9]*\)\..*/\1/p' "${root}/Cargo.toml" | head -n1)"
ln -sf libplymouth_3dboot.so "${out}/lib/libplymouth_3dboot.so.${major}"

cflags=(-std=c11 -Wall -Wextra -Werror -pedantic -g -I "${lib}/include" -I "${src}")
# Native libraries the static Rust library needs, from cargo-c's pkg-config file.
static_libs="$(PKG_CONFIG_PATH="${lib}" pkg-config --static --libs-only-l plymouth-3dboot-uninstalled | sed 's/-lplymouth_3dboot//')"

valgrind=(valgrind --quiet --error-exitcode=1 --leak-check=full --errors-for-leak-kinds=definite,indirect)

run() {
    local name="$1"; shift
    for kind in shared static; do
        local bin="${out}/${name}-${kind}"
        if [[ "${kind}" == shared ]]; then
            cc "${cflags[@]}" "${src}/${name}.c" -o "${bin}" -L "${out}/lib" -lplymouth_3dboot -Wl,-rpath,"${out}/lib"
        else
            # shellcheck disable=SC2086
            cc "${cflags[@]}" "${src}/${name}.c" -o "${bin}" "${lib}/libplymouth_3dboot.a" ${static_libs}
        fi
        echo "== ${name} (${kind})"
        "${valgrind[@]}" "${bin}" "$@"
    done
}

run render "${fixtures}/n64_logo_spin.dae"
run errors "${fixtures}/n64_logo.obj"
run memory "${fixtures}/n64_logo.obj" "${fixtures}/n64_logo.mtl"
# The example builds with pkg-config flags alone and produces an image.
example="${out}/render_ppm"
# shellcheck disable=SC2046
cc -std=c11 -Wall -Wextra -Werror "${root}/crates/plymouth-3dboot-capi/examples/render_ppm.c" -o "${example}" \
    $(PKG_CONFIG_PATH="${lib}" pkg-config --cflags plymouth-3dboot-uninstalled) -L "${out}/lib" -lplymouth_3dboot -Wl,-rpath,"${out}/lib"
echo "== example render_ppm"
"${valgrind[@]}" "${example}" "${fixtures}/n64_logo_spin.dae" "${out}/frame.ppm" 1.0
[[ "$(head -c 2 "${out}/frame.ppm")" == "P6" && "$(stat -c %s "${out}/frame.ppm")" -eq $((15 + 320 * 240 * 3)) ]]

echo "C API tests passed"

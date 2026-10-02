#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Checks the C library built by `cargo cbuild` (run via `just capi`):
# artifacts exist, the soname carries the ABI major version, only `p3b_*`
# symbols are exported, and the generated header matches the committed one.
set -euo pipefail

dir="${1:?usage: check-capi.sh BUILD_DIR}"
for f in libplymouth_3dboot.so libplymouth_3dboot.a plymouth-3dboot.pc include/plymouth-3dboot.h; do
    test -s "${dir}/${f}" || { echo "missing ${dir}/${f}" >&2; exit 1; }
done

soname="$(readelf -d "${dir}/libplymouth_3dboot.so" | sed -n 's/.*Library soname: \[\(.*\)\]/\1/p')"
[[ "${soname}" == "libplymouth_3dboot.so.0" ]] || { echo "unexpected soname '${soname}'" >&2; exit 1; }

exported="$(nm -D --defined-only "${dir}/libplymouth_3dboot.so" | awk '$2 ~ /^[TDBR]$/ {print $3}')"
foreign="$(grep -v '^p3b_' <<<"${exported}" || true)"
[[ -z "${foreign}" ]] || { echo "unexpected exported symbols:" >&2; echo "${foreign}" >&2; exit 1; }
[[ -n "${exported}" ]] || { echo "no symbols exported" >&2; exit 1; }

committed="$(dirname "${BASH_SOURCE[0]}")/../crates/plymouth-3dboot-capi/include/plymouth-3dboot.h"
if ! diff -u "${committed}" "${dir}/include/plymouth-3dboot.h" >&2; then
    echo "the generated header differs from the committed one: review the ABI change and run 'just capi-header'" >&2
    exit 1
fi

echo "C library OK: ${soname}, $(wc -l <<<"${exported}") p3b_* symbols"

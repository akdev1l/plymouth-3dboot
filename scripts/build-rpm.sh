#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the RPMs from the working tree (`just rpm`, in the Fedora container:
# DEV_CONTAINER=fedora scripts/dev.sh just rpm).
#
# Creates the spec's two sources in target/rpmbuild/SOURCES: the working
# tree's files (tracked and untracked, not ignored, so uncommitted changes
# are included) and the vendored crates (Linux targets only; fetching them
# needs network access, the build itself does not). Binary and source RPMs
# are copied to dist/rpm/, replacing earlier ones.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if [[ ! -f /etc/fedora-release ]]; then
    echo "build-rpm.sh: run in the Fedora container: DEV_CONTAINER=fedora scripts/dev.sh just rpm" >&2
    exit 1
fi

spec=packaging/plymouth-3dboot.spec
version="$(sed -n 's/^Version: *//p' "${spec}")"
cargo_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
meson_version="$(sed -n "s/^  version: '\([^']*\)'.*/\1/p" plymouth/meson.build)"
if [[ "${version}" != "${cargo_version}" || "${version}" != "${meson_version}" ]]; then
    echo "build-rpm.sh: versions differ: spec ${version}, Cargo.toml ${cargo_version}, meson.build ${meson_version}" >&2
    exit 1
fi

name="plymouth-3dboot-${version}"
top="${repo_root}/target/rpmbuild"
rm -rf "${top}"
mkdir -p "${top}/SOURCES"

# Source0: existing files only (a deleted tracked file is still listed).
git ls-files -z --cached --others --exclude-standard --deduplicate |
    while IFS= read -r -d '' f; do [[ -e "${f}" ]] && printf '%s\0' "${f}"; done |
    tar --null -T - --transform "s,^,${name}/," -czf "${top}/SOURCES/${name}.tar.gz"

# Source1: crates for the Linux targets of Fedora's architectures (a
# wildcard would include targets Fedora's LLVM cannot build for); filtered-out
# crates become empty stubs so that the lock file still resolves.
# (--keep-dep-kinds=no-dev would also drop dev-dependencies, but
# cargo-vendor-filterer 0.5.18 fails to parse cargo tree's output with it.)
platforms=()
for target in x86_64 i686 aarch64 powerpc64le s390x riscv64gc; do
    platforms+=(--platform "${target}-unknown-linux-gnu")
done
cargo vendor-filterer "${platforms[@]}" \
    --format=tar.gz --prefix=vendor "${top}/SOURCES/${name}-vendor.tar.gz" >/dev/null

commit="$(git rev-parse --short HEAD)"
[[ -z "$(git status --porcelain)" ]] || commit="${commit}.dirty"
snapshot="$(date -u +%Y%m%d%H%M%S).git${commit}"

rpmbuild -ba --define "_topdir ${top}" --define "snapshot ${snapshot}" "${spec}"

rm -rf dist/rpm
mkdir -p dist/rpm
cp "${top}"/RPMS/*/*.rpm "${top}"/SRPMS/*.rpm dist/rpm/
ls -1 dist/rpm

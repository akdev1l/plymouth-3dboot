#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Run a command inside the development container.
#
#   scripts/dev.sh <command> [args...]    e.g. scripts/dev.sh just check
#   scripts/dev.sh                         interactive shell
#
# The image is tagged with a hash of the Containerfile and is (re)built
# automatically whenever the Containerfile changes, so the environment always
# matches the committed definition.
#
# Environment:
#   CONTAINER_ENGINE   container engine to use (default: podman)
#   DEV_ENV_PASS       space-separated extra variables to forward
#                      (UPDATE_GOLDEN, RUST_BACKTRACE, SDL_VIDEODRIVER and
#                      RUST_LOG are always forwarded when set)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
engine="${CONTAINER_ENGINE:-podman}"
image_name="plymouth-3dboot-dev"
containerfile="${repo_root}/Containerfile"

hash="$(sha256sum "${containerfile}" | cut -c1-12)"
image="${image_name}:${hash}"
emsdk_version="$(sed -n 's/^ARG EMSDK_VERSION=//p' "${containerfile}")"

if ! "${engine}" image exists "${image}" 2>/dev/null; then
    echo "dev.sh: building ${image} (Containerfile changed or image missing)" >&2
    "${engine}" build -t "${image}" -t "${image_name}:latest" \
        -f "${containerfile}" "${repo_root}" >&2
fi

run_args=(
    --rm
    --init
    --userns=keep-id
    --volume "${repo_root}:/work:Z"
    # Cargo registry/git cache shared across runs.
    --volume "plymouth-3dboot-cargo:/home/dev/.cargo"
    # Emscripten cache (seeded from the image on first use), per emsdk version.
    --volume "plymouth-3dboot-emcache-${emsdk_version}:/opt/emsdk/upstream/emscripten/cache"
    --workdir /work
)

for var in UPDATE_GOLDEN RUST_BACKTRACE RUST_LOG SDL_VIDEODRIVER ${DEV_ENV_PASS:-}; do
    if [[ -n "${!var:-}" ]]; then
        run_args+=(--env "${var}=${!var}")
    fi
done

if [[ -t 0 && -t 1 ]]; then
    run_args+=(--interactive --tty)
fi

if [[ $# -eq 0 ]]; then
    set -- bash
fi

exec "${engine}" run "${run_args[@]}" "${image}" "$@"

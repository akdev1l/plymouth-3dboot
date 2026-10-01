#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Run a command inside the development container (podman).
#
#   scripts/dev.sh <command> [args...]    e.g. scripts/dev.sh just check
#   scripts/dev.sh                         interactive shell
#
# The image is tagged with a hash of its inputs (Containerfile and
# rust-toolchain.toml), and is rebuilt automatically when either changes.
# The environment therefore always matches the committed definition.
# Superseded images and their cache volumes are pruned after a rebuild.
#
# Environment:
#   DEV_ENV_PASS   space-separated extra variables to forward (UPDATE_GOLDEN,
#                  RUST_BACKTRACE, RUST_LOG and SDL_VIDEODRIVER are always
#                  forwarded when set)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image_name="plymouth-3dboot-dev"
containerfile="${repo_root}/Containerfile"

hash="$(cat "${containerfile}" "${repo_root}/rust-toolchain.toml" | sha256sum | cut -c1-12)"
image="${image_name}:${hash}"
emcache_volume="${image_name}-emcache-${hash}"

if ! podman image exists "${image}"; then
    echo "dev.sh: building ${image} (inputs changed or image missing)" >&2
    # The Containerfile copies nothing from the repository, so build from an
    # empty context instead of uploading the working tree (and target/).
    context="$(mktemp -d)"
    trap 'rmdir "${context}"' EXIT
    podman build -t "${image}" -t "${image_name}:latest" -f "${containerfile}" "${context}" >&2

    # Prune superseded images and emscripten cache volumes (best effort:
    # anything still in use is kept).
    podman images --filter "reference=localhost/${image_name}" --format '{{.Repository}}:{{.Tag}}' |
        grep -v -e ":${hash}\$" -e ':latest$' |
        xargs -r podman rmi >/dev/null 2>&1 || true
    podman volume ls --format '{{.Name}}' |
        grep "^${image_name}-emcache-" | grep -v -e "-${hash}\$" |
        xargs -r podman volume rm >/dev/null 2>&1 || true
fi

run_args=(
    --rm
    --init
    # Map the host user onto the image's `dev` user (uid/gid 1000), so files
    # in the mounted repository and the cache volumes have consistent owners.
    --userns=keep-id:uid=1000,gid=1000
    # Shared SELinux label (`z`), so concurrent dev.sh containers can all
    # access the repository.
    --volume "${repo_root}:/work:z"
    # Cargo registry/git cache shared across runs.
    --volume "${image_name}-cargo:/home/dev/.cargo"
    # Emscripten cache, seeded from this exact image on first use.
    --volume "${emcache_volume}:/opt/emsdk/upstream/emscripten/cache"
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

exec podman run "${run_args[@]}" "${image}" "$@"

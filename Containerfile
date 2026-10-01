# SPDX-License-Identifier: GPL-3.0-or-later
#
# Reproducible development environment for plymouth-3dboot.
# Build:  podman build -t plymouth-3dboot-dev -f Containerfile .
# Use:    scripts/dev.sh <command>

FROM docker.io/library/rust:1.98.1-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7

ARG JUST_VERSION=1.58.0
ARG NEXTEST_VERSION=0.9.146
ARG CARGO_DENY_VERSION=0.20.2
ARG CARGO_LLVM_COV_VERSION=0.9.1
ARG CARGO_C_VERSION=0.10.25+cargo-0.99.0
ARG EMSDK_VERSION=6.0.10
# Commit of the emsdk ${EMSDK_VERSION} tag; verified after cloning.
ARG EMSDK_COMMIT=a2b92777574c2feda07994cd4f1079a3dfc151f8

# System packages, pinned twice: apt reads a dated snapshot.debian.org
# archive, and every package is installed at an exact version. A rebuild
# therefore installs the same versions, or fails loudly.
# - build-essential, cmake, pkg-config: C toolchain for native deps and C tests
# - libsdl3-dev: SDL3 for the native presenter and viewer
# - libplymouth-dev: Plymouth splash plugin headers and libraries
# - valgrind: memory checking of the C ABI tests
# - libssl-dev: required to build cargo-c
# - python3, git, xz-utils, bzip2: required by emsdk and general tooling
ARG DEBIAN_SNAPSHOT=20261001T000000Z
RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
        'Types: deb' \
        "URIs: http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}" \
        'Suites: trixie trixie-updates' \
        'Components: main' \
        'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        '' \
        'Types: deb' \
        "URIs: http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}" \
        'Suites: trixie-security' \
        'Components: main' \
        'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/debian-snapshot.sources \
    && apt-get -o Acquire::Check-Valid-Until=false update \
    && apt-get install -y --no-install-recommends \
        build-essential=12.12 \
        bzip2=1.0.8-6 \
        ca-certificates=20250419 \
        cmake=3.31.6-2 \
        git=1:2.47.3-0+deb13u1 \
        libplymouth-dev=24.004.60-5 \
        libsdl3-dev=3.2.10+ds-1 \
        libssl-dev=3.5.7-1~deb13u3 \
        pkg-config=1.8.1-4 \
        python3=3.13.5-1 \
        valgrind=1:3.24.0-3 \
        xz-utils=5.8.1-1+deb13u1 \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy llvm-tools-preview

RUN cargo install --locked just --version "${JUST_VERSION}" \
    && cargo install --locked cargo-nextest --version "${NEXTEST_VERSION}" \
    && cargo install --locked cargo-deny --version "${CARGO_DENY_VERSION}" \
    && cargo install --locked cargo-llvm-cov --version "${CARGO_LLVM_COV_VERSION}" \
    && cargo install --locked cargo-c --version "${CARGO_C_VERSION}" \
    && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git"

RUN rustup target add wasm32-unknown-emscripten

# Unprivileged user (uid 1000). scripts/dev.sh maps the host user onto it
# (`--userns=keep-id:uid=1000,gid=1000`), so files created in the mounted
# repository are owned by the host user whatever their host uid.
RUN useradd --create-home --uid 1000 --shell /bin/bash dev \
    && mkdir /opt/emsdk && chown dev:dev /opt/emsdk
USER dev

# Emscripten SDK, pinned. The emsdk tree is owned by `dev` so its cache can
# be written at runtime; SDL3 and the system libraries are prebuilt here so
# builds do not need network access.
RUN git clone --depth 1 --branch "${EMSDK_VERSION}" \
        https://github.com/emscripten-core/emsdk.git /opt/emsdk \
    && cd /opt/emsdk \
    && test "$(git rev-parse HEAD)" = "${EMSDK_COMMIT}" \
    && ./emsdk install "${EMSDK_VERSION}" \
    && ./emsdk activate "${EMSDK_VERSION}" \
    && nodes="$(find /opt/emsdk/node -path '*/bin/node')" \
    && test "$(printf '%s\n' "${nodes}" | wc -l)" -eq 1 \
    && ln -s "$(dirname "${nodes}")" /opt/emsdk/node/current \
    && . ./emsdk_env.sh \
    && embuilder build MINIMAL sdl3 \
    && rm -rf /opt/emsdk/downloads
ENV EMSDK=/opt/emsdk \
    EMSDK_NODE=/opt/emsdk/node/current/node \
    PATH=/opt/emsdk:/opt/emsdk/upstream/emscripten:/opt/emsdk/node/current:${PATH}
# Headless Chromium for the web viewer smoke test (`just browser-smoke`).
# A separate layer so it does not invalidate the toolchain layers above.
USER root
RUN apt-get -o Acquire::Check-Valid-Until=false update \
    && apt-get install -y --no-install-recommends \
        chromium-headless-shell=154.0.8037.57-1~deb13u1 \
    && rm -rf /var/lib/apt/lists/*
USER dev

# Tools stay in /usr/local/cargo/bin (on PATH); the registry cache is per-user
# and mounted as a volume by scripts/dev.sh.
ENV CARGO_HOME=/home/dev/.cargo \
    PATH=/usr/local/cargo/bin:${PATH}
WORKDIR /work

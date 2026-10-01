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

# System packages:
# - build-essential, cmake, pkg-config: C toolchain for native deps and C tests
# - libsdl3-dev: SDL3 for the native presenter and viewer
# - libplymouth-dev: Plymouth splash plugin headers and libraries
# - valgrind: memory checking of the C ABI tests
# - libssl-dev: required to build cargo-c
# - python3, git, xz-utils, bzip2: required by emsdk and general tooling
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        bzip2 \
        ca-certificates \
        cmake \
        git \
        libplymouth-dev \
        libsdl3-dev \
        libssl-dev \
        pkg-config \
        python3 \
        valgrind \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy llvm-tools-preview

RUN cargo install --locked just --version "${JUST_VERSION}" \
    && cargo install --locked cargo-nextest --version "${NEXTEST_VERSION}" \
    && cargo install --locked cargo-deny --version "${CARGO_DENY_VERSION}" \
    && cargo install --locked cargo-llvm-cov --version "${CARGO_LLVM_COV_VERSION}" \
    && cargo install --locked cargo-c --version "${CARGO_C_VERSION}" \
    && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git"

RUN rustup target add wasm32-unknown-emscripten

# Unprivileged user. Run with `--userns=keep-id` so files created in the
# mounted repository are owned by the host user.
RUN useradd --create-home --uid 1000 --shell /bin/bash dev \
    && mkdir /opt/emsdk && chown dev:dev /opt/emsdk
USER dev

# Emscripten SDK, pinned. The emsdk tree is owned by `dev` so its cache can
# be written at runtime; SDL3 and the system libraries are prebuilt here so
# builds do not need network access.
RUN git clone --depth 1 --branch "${EMSDK_VERSION}" \
        https://github.com/emscripten-core/emsdk.git /opt/emsdk \
    && cd /opt/emsdk \
    && ./emsdk install "${EMSDK_VERSION}" \
    && ./emsdk activate "${EMSDK_VERSION}" \
    && ln -s "$(dirname "$(find /opt/emsdk/node -path '*/bin/node' | head -n 1)")" /opt/emsdk/node/current \
    && . ./emsdk_env.sh \
    && embuilder build MINIMAL sdl3 \
    && rm -rf /opt/emsdk/downloads
ENV EMSDK=/opt/emsdk \
    EMSDK_NODE=/opt/emsdk/node/current/node \
    PATH=/opt/emsdk:/opt/emsdk/upstream/emscripten:/opt/emsdk/node/current:${PATH}
# Tools stay in /usr/local/cargo/bin (on PATH); the registry cache is per-user
# and mounted as a volume by scripts/dev.sh.
ENV CARGO_HOME=/home/dev/.cargo \
    PATH=/usr/local/cargo/bin:${PATH}
WORKDIR /work

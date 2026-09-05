# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only

FROM rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        clang \
        fonts-noto-core \
        fonts-noto-cjk \
        imagemagick \
        libegl1-mesa-dev \
        libgtk-3-dev \
        libwayland-dev \
        libx11-xcb-dev \
        libxcb-shape0-dev \
        libxcb-xfixes0-dev \
        libxkbcommon-dev \
        libxkbcommon-x11-dev \
        pkg-config \
        xclip \
        xdotool \
        xvfb \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
    && rustup target add aarch64-apple-darwin

RUN cargo install --locked --version 0.22.2 cargo-audit \
    && cargo install --locked --version 0.20.2 cargo-deny

WORKDIR /workspace

ENV CARGO_TARGET_DIR=/var/cache/notrum/target

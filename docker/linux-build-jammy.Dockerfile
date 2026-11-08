# Ambiente de build para Ubuntu 22.04: o binário linka contra a glibc 2.35 do notebook.
FROM ubuntu:22.04

ARG DEBIAN_FRONTEND=noninteractive
ARG RUST_VERSION=1.99.1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config \
      libx11-dev libxi-dev libxtst-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
      libgl1-mesa-dev \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy --default-toolchain "$RUST_VERSION" \
 && chmod -R a+rwX /opt/rustup /opt/cargo

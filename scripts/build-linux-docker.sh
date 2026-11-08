#!/usr/bin/env bash
# Compila e testa num container Ubuntu 22.04, sem instalar toolchain no sistema.
# Uso: bash scripts/build-linux-docker.sh [build-dir]   (padrão: ~/lan-kvm-build)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="${1:-$HOME/lan-kvm-build}"
IMAGE="lan-kvm-build:jammy"

mkdir -p "$BUILD/target" "$BUILD/cargo-registry" "$BUILD/bin"
docker build -q -t "$IMAGE" -f "$ROOT/docker/linux-build-jammy.Dockerfile" "$ROOT/docker" >/dev/null

docker run --rm \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e CARGO_TARGET_DIR=/build/target \
  -v "$ROOT:/src:ro" \
  -v "$BUILD/target:/build/target" \
  -v "$BUILD/cargo-registry:/opt/cargo/registry" \
  -w /src \
  "$IMAGE" \
  bash -c "cargo clippy --workspace --all-targets --release -- -D warnings && cargo test --workspace --release && cargo build --workspace --release"

cp "$BUILD/target/release/lan-kvm" "$BUILD/bin/lan-kvm.new"
mv -f "$BUILD/bin/lan-kvm.new" "$BUILD/bin/lan-kvm"
echo "Binário: $BUILD/bin/lan-kvm"

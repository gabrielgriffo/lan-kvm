#!/usr/bin/env bash
# Grava no Cargo.toml a versão da tag da release (v1.2.3 ou 1.2.3).
# Uso: bash scripts/set-version.sh <tag>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:-}"
VERSION="${VERSION#v}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "tag não é uma versão (ex.: v1.2.3): ${1:-}" >&2; exit 1; }

sed -i "0,/^version = \".*\"/s//version = \"$VERSION\"/" "$ROOT/Cargo.toml"
grep -q "^version = \"$VERSION\"$" "$ROOT/Cargo.toml" || { echo "versão não gravada no Cargo.toml" >&2; exit 1; }
echo "versão $VERSION"

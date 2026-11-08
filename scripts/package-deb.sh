#!/usr/bin/env bash
# Gera o pacote .deb do receptor com a versão do Cargo.toml.
# Uso: bash scripts/package-deb.sh <binário> [pasta-de-saída]   (padrão: target/deb)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:?informe o binário do lan-kvm}"
OUT="${2:-$ROOT/target/deb}"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || { echo "versão não encontrada no Cargo.toml" >&2; exit 1; }
[ -x "$BIN" ] || { echo "binário não encontrado: $BIN" >&2; exit 1; }

PKG="$(mktemp -d)"
trap 'rm -rf "$PKG"' EXIT
chmod 755 "$PKG"
FILES="$ROOT/packaging/linux"

install -Dm755 "$BIN" "$PKG/usr/bin/lan-kvm"
install -Dm755 "$FILES/receiver.sh" "$PKG/usr/libexec/lan-kvm/receiver"
install -Dm644 "$FILES/lan-kvm.desktop" "$PKG/etc/xdg/autostart/lan-kvm.desktop"
install -Dm644 "$FILES/60-lan-kvm-i2c.rules" "$PKG/usr/lib/udev/rules.d/60-lan-kvm-i2c.rules"
install -Dm644 /dev/stdin "$PKG/usr/lib/modules-load.d/lan-kvm-i2c.conf" <<< "i2c-dev"
install -Dm644 "$ROOT/branding/lan-kvm-symbol.svg" "$PKG/usr/share/icons/hicolor/scalable/apps/lan-kvm.svg"
install -Dm644 "$ROOT/README.md" "$PKG/usr/share/doc/lan-kvm/README.md"
install -Dm755 "$FILES/postinst" "$PKG/DEBIAN/postinst"
install -Dm755 "$FILES/postrm" "$PKG/DEBIAN/postrm"
echo /etc/xdg/autostart/lan-kvm.desktop > "$PKG/DEBIAN/conffiles"

cat > "$PKG/DEBIAN/control" <<CONTROL
Package: lan-kvm
Version: $VERSION
Architecture: amd64
Maintainer: Gabriel Griffo
Depends: libc6 (>= 2.35), libgcc-s1
Recommends: libnotify-bin
Section: utils
Priority: optional
Installed-Size: $(du -sk "$PKG/usr" | cut -f1)
Description: receptor do lan-kvm (um teclado e um mouse para o PC e o notebook)
 Recebe mouse, teclado e clipboard do PC com Windows na sessão X11 e troca a
 entrada do monitor por DDC/CI. Inicia junto com a sessão gráfica.
CONTROL

mkdir -p "$OUT"
DEB="$OUT/lan-kvm_${VERSION}_amd64.deb"
dpkg-deb --root-owner-group --build "$PKG" "$DEB" >/dev/null
echo "pacote: $DEB"

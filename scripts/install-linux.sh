#!/usr/bin/env bash
# Instala o receptor em ~/.local/bin, configura o autostart e reinicia o receptor.
# Uso: bash scripts/install-linux.sh [build-dir]    (padrão: ~/lan-kvm-build)
#      bash scripts/install-linux.sh --uninstall
# O pareamento (~/.config/lan-kvm) e o log (~/.local/state/lan-kvm) são preservados.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$HOME/.local/bin/lan-kvm"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/lan-kvm.desktop"

if [ "${1:-}" = "--uninstall" ]; then
  pkill -x lan-kvm || true
  rm -f "$DEST" "$AUTOSTART"
  echo "lan-kvm removido"
  exit 0
fi

BUILD="${1:-$HOME/lan-kvm-build}"
SRC="$BUILD/bin/lan-kvm"
[ -x "$SRC" ] || { echo "binário não encontrado: $SRC" >&2; exit 1; }

mkdir -p "$(dirname "$DEST")"
cp "$SRC" "$DEST.new"
chmod 755 "$DEST.new"
mv -f "$DEST.new" "$DEST"
bash "$ROOT/scripts/install-autostart-linux.sh" "$DEST"

if [ -z "${DISPLAY:-}" ]; then
  shell_pid="$(pgrep -u "$(id -u)" -x gnome-shell | head -n 1 || true)"
  if [ -n "$shell_pid" ] && [ -r "/proc/$shell_pid/environ" ]; then
    while IFS= read -r -d '' var; do
      case "$var" in
        DISPLAY=*|XAUTHORITY=*|DBUS_SESSION_BUS_ADDRESS=*) export "${var?}" ;;
      esac
    done < "/proc/$shell_pid/environ"
  fi
fi

pkill -x lan-kvm || true
for _ in $(seq 50); do pgrep -x lan-kvm >/dev/null || break; sleep 0.1; done
if [ -n "${DISPLAY:-}" ] && command -v gio >/dev/null; then
  sleep 0.5
  gio launch "$AUTOSTART"
  echo "receptor reiniciado"
else
  echo "sem sessão gráfica neste shell: o receptor inicia no próximo login"
fi
echo "lan-kvm instalado em $DEST"

#!/usr/bin/env bash
# Inicia o receptor ao entrar na sessão gráfica (autostart XDG do usuário, sem tocar no sistema).
# Uso: bash scripts/install-autostart-linux.sh [binário]
# Remover: rm ~/.config/autostart/lan-kvm.desktop
set -euo pipefail

BIN="${1:-$HOME/.local/bin/lan-kvm}"
LOG_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/lan-kvm"
LOG="$LOG_DIR/recv.log"
DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

case "$BIN$LOG" in
  *[\"\\\`\$\ ]*) echo "caminho com caracteres não suportados: $BIN" >&2; exit 1 ;;
esac
[ -x "$BIN" ] || { echo "binário não encontrado: $BIN" >&2; exit 1; }

mkdir -p "$DIR" "$LOG_DIR"
chmod 700 "$LOG_DIR"
touch "$LOG"
chmod 600 "$LOG"
cat > "$DIR/lan-kvm.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=lan-kvm
Comment=Receptor de teclado e mouse do lan-kvm
Exec=/bin/sh -c "umask 077; exec $BIN recv >> $LOG 2>&1"
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF
echo "Autostart instalado: $DIR/lan-kvm.desktop"

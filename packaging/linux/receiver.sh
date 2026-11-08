#!/bin/sh
dir="${XDG_STATE_HOME:-$HOME/.local/state}/lan-kvm"
mkdir -p "$dir"
chmod 700 "$dir"
umask 077
exec /usr/bin/lan-kvm recv >> "$dir/recv.log" 2>&1

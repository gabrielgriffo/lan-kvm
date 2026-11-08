#!/usr/bin/env bash
# Monta a tabela de downloads no texto da release a partir dos arquivos publicados.
# Uso: bash scripts/release-downloads.sh <tag>   (REPO=dono/repositorio fora do Actions, DRY_RUN=1 só imprime)
set -euo pipefail

TAG="${1:?uso: release-downloads.sh <tag>}"
REPO="${REPO:-${GITHUB_REPOSITORY:?defina REPO=dono/repositorio}}"
VERSION="${TAG#v}"
APP="${APP:-lan-kvm}"
BASE="https://github.com/$REPO/releases/download/$TAG"
START='<!-- downloads:start -->'
END='<!-- downloads:end -->'

ASSETS="$(gh release view "$TAG" -R "$REPO" --json assets -q '.assets[].name')"

link() {
  local label="$1" file="$2"
  if grep -qxF "$file" <<<"$ASSETS"; then
    printf '[%s](%s/%s)' "$label" "$BASE" "$file"
  fi
}

table() {
  echo "| Arquitetura | Windows (PC) | Linux X11 (notebook) |"
  echo "|---|---|---|"
  echo "| x86-64 | $(link EXE "$APP-$VERSION-windows-setup.exe") | $(link DEB "${APP}_${VERSION}_amd64.deb") |"
}

block="$(printf '%s\n\n%s\n\n%s' "$START" "$(table)" "$END")"
body="$(gh release view "$TAG" -R "$REPO" --json body -q .body | tr -d '\r')"

if grep -qF "$START" <<<"$body" && grep -qF "$END" <<<"$body"; then
  new_body="$(BLOCK="$block" awk -v s="$START" -v e="$END" '
    $0 == s { print ENVIRON["BLOCK"]; skip = 1; next }
    $0 == e { skip = 0; next }
    !skip { print }
  ' <<<"$body")"
elif [ -n "$body" ]; then
  new_body="$(printf '%s\n\n%s' "$block" "$body")"
else
  new_body="$block"
fi

if [ "${DRY_RUN:-0}" = 1 ]; then
  printf '%s\n' "$new_body"
  exit 0
fi

notes="$(mktemp)"
trap 'rm -f "$notes"' EXIT
printf '%s\n' "$new_body" > "$notes"
gh release edit "$TAG" -R "$REPO" --notes-file "$notes"

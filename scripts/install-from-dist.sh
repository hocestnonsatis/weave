#!/usr/bin/env bash
# Install weave from dist/ after verifying SHA256SUMS (fail closed).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${DIST_DIR:-$ROOT/dist}"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"

if [[ ! -f "$DIST/SHA256SUMS" || ! -x "$DIST/weave" ]]; then
  echo "error: run scripts/release-build.sh first" >&2
  exit 1
fi

( cd "$DIST" && sha256sum -c SHA256SUMS )

mkdir -p "$BIN_DIR"
install -m 0755 "$DIST/weave" "$BIN_DIR/weave"

echo "Installed $BIN_DIR/weave"
"$BIN_DIR/weave" --version
echo "Ensure $BIN_DIR is on PATH."
echo "Experimental exec features remain disabled until you opt in (docs/adoption.md)."

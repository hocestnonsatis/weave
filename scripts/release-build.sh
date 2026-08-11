#!/usr/bin/env bash
# Reproducible Weave 0.x release build (Linux x86_64).
#
# Produces:
#   dist/weave
#   dist/SHA256SUMS
#   dist/SHA256SUMS.sri
#   dist/BUILDINFO.json
#   dist/weave.sha256
#
# Never enables execution features. Does not network-fetch dependencies if
# vendored/offline (uses --locked against Cargo.lock).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET_TRIPLE="${TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"
DIST="${DIST_DIR:-$ROOT/dist}"
BIN_NAME="weave"
PACKAGE="weave-cli"

mkdir -p "$DIST"
rm -f "$DIST/$BIN_NAME" "$DIST/SHA256SUMS" "$DIST/SHA256SUMS.sri" \
  "$DIST/BUILDINFO.json" "$DIST/weave.sha256" "$DIST/${BIN_NAME}.sha256"

# Deterministic timestamp when available.
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  if command -v git >/dev/null 2>&1 && git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    SOURCE_DATE_EPOCH="$(git -C "$ROOT" log -1 --pretty=%ct 2>/dev/null || date +%s)"
  else
    SOURCE_DATE_EPOCH="$(date +%s)"
  fi
  export SOURCE_DATE_EPOCH
fi

export CARGO_INCREMENTAL=0
export TZ=UTC
export LANG=C
export LC_ALL=C

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"

GIT_COMMIT="unknown"
GIT_DIRTY="true"
if command -v git >/dev/null 2>&1 && git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  GIT_COMMIT="$(git -C "$ROOT" rev-parse HEAD)"
  if git -C "$ROOT" diff --quiet && git -C "$ROOT" diff --cached --quiet; then
    GIT_DIRTY="false"
  fi
fi

RUSTC_VERSION="$(rustc -V)"
CARGO_VERSION="$(cargo -V)"

echo "==> release build weave ${VERSION} (${TARGET_TRIPLE})"
echo "    SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
echo "    rustc: ${RUSTC_VERSION}"

# Prefer explicit target; fall back to host if target not installed.
if rustup target list --installed 2>/dev/null | grep -qx "$TARGET_TRIPLE" \
  || rustc -vV | grep -q "host: ${TARGET_TRIPLE}"; then
  cargo build --release --locked -p "$PACKAGE" --target "$TARGET_TRIPLE"
  SRC_BIN="$ROOT/target/${TARGET_TRIPLE}/release/${BIN_NAME}"
else
  echo "warn: target ${TARGET_TRIPLE} not installed; building for host"
  cargo build --release --locked -p "$PACKAGE"
  SRC_BIN="$ROOT/target/release/${BIN_NAME}"
fi

if [[ ! -x "$SRC_BIN" ]]; then
  SRC_BIN="$ROOT/target/release/${BIN_NAME}"
fi
if [[ ! -x "$SRC_BIN" ]]; then
  echo "error: release binary not found" >&2
  exit 1
fi

cp -f "$SRC_BIN" "$DIST/$BIN_NAME"
chmod 0755 "$DIST/$BIN_NAME"

# Refuse setuid/setgid bits if somehow present.
if [[ -u "$DIST/$BIN_NAME" || -g "$DIST/$BIN_NAME" ]]; then
  echo "error: release binary must not be setuid/setgid" >&2
  exit 1
fi

HASH="$(sha256sum "$DIST/$BIN_NAME" | awk '{print $1}')"
echo "${HASH}  ${BIN_NAME}" > "$DIST/SHA256SUMS"
echo "${HASH}" > "$DIST/weave.sha256"
cp "$DIST/SHA256SUMS" "$DIST/${BIN_NAME}.sha256sums"

# npm-style SRI for the binary artifact (measurement; install docs still require human verify).
SRI_B64="$(python3 - <<PY
import hashlib, base64, pathlib
data = pathlib.Path("$DIST/$BIN_NAME").read_bytes()
print("sha256-" + base64.b64encode(hashlib.sha256(data).digest()).decode())
PY
)"
echo "${SRI_B64}  ${BIN_NAME}" > "$DIST/SHA256SUMS.sri"

python3 - <<PY > "$DIST/BUILDINFO.json"
import json, os, time
print(json.dumps({
  "product": "weave",
  "version": "$VERSION",
  "target": "$TARGET_TRIPLE",
  "binary": "$BIN_NAME",
  "sha256": "$HASH",
  "sri": "$SRI_B64",
  "source_date_epoch": int("$SOURCE_DATE_EPOCH"),
  "rustc": "$RUSTC_VERSION",
  "cargo": "$CARGO_VERSION",
  "git_commit": "$GIT_COMMIT",
  "git_dirty": "$GIT_DIRTY" == "true",
  "profile": "release",
  "locked": True,
  "built_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
  "experimental_defaults": {
    "execution.enabled": False,
    "execution.profile": "offline",
    "env_cannot_enable_execution": True
  },
  "supported_platform": "linux-x86_64",
  "notes": "Experimental exec/native features are never silently enabled."
}, indent=2, sort_keys=True))
PY

echo "==> verifying checksum"
( cd "$DIST" && sha256sum -c SHA256SUMS )

echo "==> smoke --version"
"$DIST/$BIN_NAME" --version

echo "OK: artifacts in $DIST"
ls -la "$DIST"

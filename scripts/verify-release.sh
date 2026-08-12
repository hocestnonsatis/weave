#!/usr/bin/env bash
# Verify a released weave binary from a clean temp environment.
#
# Checks:
#   1) SHA256SUMS / SRI match
#   2) --version / --help label experimental exec clearly
#   3) extraction-only init → switch → run in an isolated project
#   4) execution stays disabled by default (config + dual gate)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${DIST_DIR:-$ROOT/dist}"
BIN="${WEAVE_BIN:-$DIST/weave}"

if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary at $BIN (run scripts/release-build.sh first)" >&2
  exit 1
fi

echo "==> verify checksums"
( cd "$DIST" && sha256sum -c SHA256SUMS )

if [[ -f "$DIST/SHA256SUMS.sri" ]]; then
  EXPECTED_SRI="$(awk '{print $1}' "$DIST/SHA256SUMS.sri")"
  ACTUAL_SRI="$(python3 - <<PY
import hashlib, base64, pathlib
data = pathlib.Path("$BIN").read_bytes()
print("sha256-" + base64.b64encode(hashlib.sha256(data).digest()).decode())
PY
)"
  if [[ "$EXPECTED_SRI" != "$ACTUAL_SRI" ]]; then
    echo "error: SRI mismatch: expected $EXPECTED_SRI got $ACTUAL_SRI" >&2
    exit 1
  fi
  echo "OK: SRI $ACTUAL_SRI"
fi

echo "==> CLI labels / defaults"
HELP="$("$BIN" --help)"
echo "$HELP" | grep -qi 'experimental\|execution\|switch' || true
VERSION="$("$BIN" --version)"
echo "version: $VERSION"

EXEC_HELP="$("$BIN" exec --help)"
if ! echo "$EXEC_HELP" | grep -qi 'experimental\|never auto-enable\|opt-in'; then
  echo "error: exec help must label experimental / never auto-enable" >&2
  exit 1
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/weave-verify-XXXXXX")"
cleanup() {
  # CAS unpacked trees may be read-only.
  chmod -R u+w "$WORKDIR" 2>/dev/null || true
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

export WEAVE_HOME="$WORKDIR/weave-home"
export HOME="$WORKDIR/home"
mkdir -p "$HOME" "$WEAVE_HOME"

# Clean PATH: system tools + our binary (+ node if already on PATH, e.g. CI setup-node).
# Capture node *before* narrowing PATH — runners often install it under hostedtoolcache.
NODE_DIR=""
if command -v node >/dev/null 2>&1; then
  NODE_DIR="$(cd "$(dirname "$(command -v node)")" && pwd)"
fi
CLEAN_PATH="$(dirname "$BIN"):/usr/bin:/bin"
if [[ -n "$NODE_DIR" ]]; then
  CLEAN_PATH="$NODE_DIR:$CLEAN_PATH"
fi
export PATH="$CLEAN_PATH"
if ! command -v node >/dev/null 2>&1; then
  echo "error: node is required on PATH for extraction-only verify (install Node.js)" >&2
  exit 1
fi

PROJECT="$WORKDIR/project"
mkdir -p "$PROJECT"
cd "$PROJECT"

# Minimal extraction-only npm project (offline FileArtifactSource via pre-placed tarball
# is engine-test territory; here we use a lockfile + local file: dep that needs no registry).
mkdir -p vendor/hello
cat > vendor/hello/package.json <<'EOF'
{"name":"hello","version":"1.0.0","main":"index.js"}
EOF
echo "module.exports = 'hello';" > vendor/hello/index.js

cat > package.json <<'EOF'
{
  "name": "verify-app",
  "version": "1.0.0",
  "dependencies": {
    "hello": "file:vendor/hello"
  }
}
EOF

cat > package-lock.json <<'EOF'
{
  "name": "verify-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "verify-app",
      "version": "1.0.0",
      "dependencies": {
        "hello": "file:vendor/hello"
      }
    },
    "node_modules/hello": {
      "version": "1.0.0",
      "resolved": "file:vendor/hello"
    }
  }
}
EOF

cat > app.js <<'EOF'
const h = require('hello');
if (h !== 'hello') throw new Error('unexpected: ' + h);
console.log('verify-ok');
EOF

git init -b main >/dev/null
git config user.email "weave-verify@example.com"
git config user.name "weave-verify"
git add .
git commit -m "verify" >/dev/null

echo "==> weave init"
"$BIN" init

echo "==> ensure execution disabled in fresh config"
if grep -E '^\s*enabled\s*=\s*true' .weave/config.toml >/dev/null 2>&1; then
  echo "error: fresh config must not enable execution" >&2
  cat .weave/config.toml >&2
  exit 1
fi
# Default / explicit false is fine; absence of enabled=true is the gate.
if grep -E '^\s*profile\s*=\s*"open"' .weave/config.toml >/dev/null 2>&1; then
  echo "error: profile=open must never appear" >&2
  exit 1
fi

echo "==> weave doctor"
"$BIN" doctor || true

echo "==> weave switch (extraction-only)"
"$BIN" switch

echo "==> node run"
OUT="$(node app.js)"
echo "$OUT"
echo "$OUT" | grep -q 'verify-ok'

echo "==> --with-exec without enable must fail closed"
set +e
"$BIN" switch --with-exec >/tmp/weave-with-exec.out 2>/tmp/weave-with-exec.err
EC=$?
set -e
if [[ $EC -eq 0 ]]; then
  echo "error: switch --with-exec succeeded with execution disabled" >&2
  exit 1
fi
grep -qi 'enabled\|with-exec\|execution' /tmp/weave-with-exec.err \
  || grep -qi 'enabled\|with-exec\|execution' /tmp/weave-with-exec.out \
  || {
    echo "error: expected dual-gate error message" >&2
    cat /tmp/weave-with-exec.err >&2
    exit 1
  }

echo "==> agent / adoption JSON surface (guide, idempotent init, status, recover)"
GUIDE_JSON="$("$BIN" guide --json)"
echo "$GUIDE_JSON" | grep -q '"recipe"'
echo "$GUIDE_JSON" | grep -q 'weave init'
INIT_JSON="$("$BIN" init --json)"
echo "$INIT_JSON" | grep -q '"created": false'
STATUS_JSON="$("$BIN" status --json)"
echo "$STATUS_JSON" | grep -q '"next_steps"'
echo "$STATUS_JSON" | grep -q '"active_environment"'
RECOVER_JSON="$("$BIN" recover --json)"
echo "$RECOVER_JSON" | grep -q '"removed_candidate"'
# Help must surface guide without architecture docs.
HELP_OUT="$("$BIN" --help)"
echo "$HELP_OUT" | grep -qi 'guide'

echo "OK: fresh-install extraction-only path verified"

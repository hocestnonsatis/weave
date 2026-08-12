# Weave 0.1.1 release notes

**Stabilization** of the extraction-first 0.x line after `v0.1.0`. Packages the
agent-facing adoption surface (guide / idempotent init / recover / JSON /
ownership) without changing the CAS or materialization architecture.

Source: https://github.com/hocestnonsatis/weave · License: MIT

## Install (verify checksums)

```bash
# After downloading weave + SHA256SUMS from the GitHub Release:
sha256sum -c SHA256SUMS
install -m 0755 weave ~/.local/bin/weave
weave --version
```

Or from a source tree:

```bash
bash scripts/release-build.sh
bash scripts/verify-release.sh
bash scripts/install-from-dist.sh
```

## Quick path (extraction-only)

```bash
weave guide --json
weave init --json
weave doctor --json
weave switch --json
weave status --json
```

After `git checkout`: `weave switch --json`.  
Interrupted switch leftovers: `weave recover --json`.

Agents: start at `weave guide --json` or `docs/agent-quickstart.md` — no CAS
internals required.

## Experimental (opt-in only — never silently enabled)

| Feature | How to enable |
|---------|----------------|
| Sandboxed lifecycle exec | `execution.enabled=true` **and** `weave switch --with-exec` |
| Allowlisted prebuild fetch | `execution.profile="prebuild-fetch"` + explicit hosts/SRI fetches |
| Policy packs / hash-artifact | Manual review; apply never flips `enabled` |

`WEAVE_EXEC=1` alone does **not** enable execution.

## Supported / unsupported

**Supported:** Linux x86_64 · Git · Node.js · npm `package-lock.json` (v1–3)

**Unsupported (fail closed):** Yarn/pnpm/Bun-only lockfiles · Windows exec sandbox ·
FUSE/overlayfs/daemon · inventing SRI/URLs/outputs · AI auto-detect/trust · MCP/IDE plugins

See `docs/supported-platforms.md`, `docs/security.md`, `CHANGELOG.md`.

## Verify this build

`dist/BUILDINFO.json` records version, git commit, `SOURCE_DATE_EPOCH`, sha256, and SRI.
GitHub Actions attaches build provenance attestations on tagged releases.

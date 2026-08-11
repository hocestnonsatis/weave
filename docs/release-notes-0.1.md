# Weave 0.1.0 release notes

**Extraction-first 0.x.** Plain `weave switch` materializes `node_modules` from
npm `package-lock.json` without running lifecycle scripts and without opening
network access for install scripts.

Source: https://github.com/hocestnonsatis/weave · License: MIT

## Install (verify checksums)

```bash
# After downloading weave + SHA256SUMS from the GitHub Release:
# https://github.com/hocestnonsatis/weave/releases
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

## Quick path

```bash
weave init
weave doctor
weave switch
node <your-entrypoint>
```

No `[execution]` configuration is required for ordinary JS dependencies.

## Experimental (opt-in only — never silently enabled)

| Feature | How to enable |
|---------|----------------|
| Sandboxed lifecycle exec | `execution.enabled=true` **and** `weave switch --with-exec` |
| Allowlisted prebuild fetch | `execution.profile="prebuild-fetch"` + explicit hosts/SRI fetches |
| Policy packs / hash-artifact | Manual review; apply never flips `enabled` |

`WEAVE_EXEC=1` alone does **not** enable execution.

## Supported platform

- Linux x86_64
- Git + Node.js + npm `package-lock.json` (v2/v3)
- Bubblewrap (`bwrap`) required only for experimental exec

See `docs/supported-platforms.md` and `docs/security.md`.

## Verify this build

`dist/BUILDINFO.json` records version, git commit (when available),
`SOURCE_DATE_EPOCH`, sha256, and SRI. GitHub Actions attaches build provenance
attestations on tagged releases.

# Real-world lockfile corpus

Pinned snapshots of upstream `package-lock.json` files for Phase 3 analysis.

## Reproducibility

Each project directory contains:

- `package-lock.json` — exact bytes captured
- `package.json` — companion manifest when available
- `PROVENANCE.json` — source repo, git ref, raw URL, SHA-256 of the lockfile

Root `MANIFEST.json` indexes all entries.

To refresh (network required):

```bash
# Re-run the capture script used in Phase 3 (see agent transcript / tools),
# or re-download using each PROVENANCE.json raw_url and verify lockfile_sha256.
```

## Categories

Folders are organizational. Prefer metrics from Weave graph analysis over folder names
(some “small” projects still have large transitive graphs).

## What this corpus is NOT

- Not a vendored `node_modules`
- Not a license redistrib of upstream source trees
- Not sufficient alone for offline Weave *materialization* (registry tarballs not bundled)
- **Not Weave production dependencies** — Dependabot must not “fix” these lockfiles
  for CVEs; that would break provenance. See [`docs/dependabot.md`](../../docs/dependabot.md).

Offline **analysis** (graph stats, artifact overlap, lifecycle classification) needs only these lockfiles.

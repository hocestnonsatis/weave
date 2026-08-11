# Release process (0.x)

## Versioning

- Workspace version lives in root `Cargo.toml` → `[workspace.package].version`
- Tag releases as `vMAJOR.MINOR.PATCH` (example: `v0.1.0`)
- Update `CHANGELOG.md` under `[Unreleased]` then move entries into the version section
- Write/refresh `docs/release-notes-X.Y.md` for the GitHub Release body

## Local release candidate

```bash
bash scripts/ci-gates.sh
bash scripts/release-build.sh
bash scripts/verify-release.sh
# optional: bash scripts/install-from-dist.sh
```

Artifacts land in `dist/`:

| File | Purpose |
|------|---------|
| `weave` | Stripped release binary |
| `SHA256SUMS` | Classic checksums |
| `SHA256SUMS.sri` | `sha256-…` SRI for the binary |
| `BUILDINFO.json` | Provenance metadata (toolchains, epoch, git) |

## CI

- `.github/workflows/ci.yml` — fmt, clippy, tests, release build, verify, provenance attest on push
- `.github/workflows/release.yml` — tag-triggered draft GitHub Release with artifacts + attestation

## Checklist

See [`docs/RELEASE_CHECKLIST.md`](./RELEASE_CHECKLIST.md).

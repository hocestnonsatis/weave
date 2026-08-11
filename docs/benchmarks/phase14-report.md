# Phase 14 report: 0.x Release Engineering

Date: 2026-08-12  
No new runtime architecture. Focus: reproducible release, CI gates, checksums,
provenance, docs, and verification of the extraction-first path.

## Delivered

| Item | Location |
|------|----------|
| CI gates script | `scripts/ci-gates.sh` |
| Reproducible release build | `scripts/release-build.sh` → `dist/` |
| Checksums + SRI | `dist/SHA256SUMS`, `dist/SHA256SUMS.sri` |
| Build metadata | `dist/BUILDINFO.json` |
| Fresh-install verify | `scripts/verify-release.sh` |
| Checksummed install | `scripts/install-from-dist.sh` |
| CI workflow | `.github/workflows/ci.yml` (gates + artifact + attest) |
| Tag release workflow | `.github/workflows/release.yml` (draft release + attest) |
| Changelog / notes | `CHANGELOG.md`, `docs/release-notes-0.1.md` |
| Platform / security | `docs/supported-platforms.md`, `docs/security.md` |
| Checklist | `docs/RELEASE_CHECKLIST.md` |
| Experimental CLI labels | `weave` / `weave exec` / `--with-exec` help |
| CLI `file:` deps | `DefaultArtifactSource` (path snapshot + HTTPS registry) |

## Release candidate

Produced by `scripts/release-build.sh` + verified by `scripts/verify-release.sh`.
Version: **0.1.0** (workspace). Tag when publishing: **`v0.1.0`**.

## Blockers for safe public release

**None identified** that prevent a safe extraction-first 0.x publish.
Remaining gaps (native SRI human review, Linux+bwrap for experimental exec,
npm-lockfile-only) are documented boundaries, not silent security holes.

## Concise 0.x checklist

1. `bash scripts/ci-gates.sh`
2. `bash scripts/release-build.sh && bash scripts/verify-release.sh`
3. Review `CHANGELOG.md` + `docs/release-notes-0.1.md`
4. Tag `v0.1.0` → Release workflow drafts GitHub Release with attestations
5. Publish after human review of draft + checksums

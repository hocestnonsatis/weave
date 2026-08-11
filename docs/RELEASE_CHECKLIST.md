# Weave 0.1.0 release checklist

Use this before publishing a public 0.x tag.

## Gates

- [ ] `bash scripts/ci-gates.sh` passes locally
- [ ] CI green on the release commit
- [ ] `bash scripts/release-build.sh` produces `dist/weave` + `SHA256SUMS` + `SHA256SUMS.sri` + `BUILDINFO.json`
- [ ] `bash scripts/verify-release.sh` passes (fresh install + extraction-only + dual-gate refuse)
- [ ] Binary is not setuid/setgid; mode `0755`

## Docs / labels

- [ ] `CHANGELOG.md` has the version section
- [ ] `docs/release-notes-0.1.md` matches shipped behavior
- [ ] Experimental exec/native features labeled in CLI help and release notes
- [ ] `docs/supported-platforms.md` + `docs/security.md` linked from README

## Provenance

- [ ] `BUILDINFO.json` records version, sha256/SRI, rustc, SOURCE_DATE_EPOCH
- [ ] GitHub Actions provenance attestation attached (tag release workflow)
- [ ] Release assets include binary + checksums + BUILDINFO

## Product boundaries

- [ ] Fresh `.weave/config.toml` has execution disabled / no `profile=open`
- [ ] `WEAVE_EXEC=1` alone does not enable execution (covered by unit tests)
- [ ] Extraction-only path documented as the supported default

## Publish

- [ ] Tag `v0.1.0`
- [ ] Draft GitHub Release reviewed then published
- [ ] Announce: extraction-first; experimental features opt-in only

## Blocker rule

Only block the release for issues that prevent a **safe** public install or
silently weaken the security model. Missing native SRI automation is **not** a
blocker (documented human-review boundary).

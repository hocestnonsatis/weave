# Changelog

All notable changes to Weave are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for the **0.x** series: breaking changes may occur in minor versions; patch
releases are for fixes and docs.

## [Unreleased]

## [0.1.0] — 2026-08-11

First public **0.x** release candidate scope (extraction-first).

### Added
- CAS-backed `weave init` / `switch` / `materialize` / `status` / `env` / `gc` / `doctor`
- Adoption diagnostics (`ExtractionReady` / `PartialNeedsPolicy` / `Blocked`)
- Opt-in sandboxed execution under dual gate (`execution.enabled` + `--with-exec`) — **experimental**
- Allowlisted HTTPS prebuild fetch (`profile=prebuild-fetch`) — **experimental**
- `weave exec plan|suggest|hash-artifact|apply-pack|run` — discovery ≠ approval
- Reproducible release scripts + checksums/SRI (`scripts/release-build.sh`)
- CI gates + release provenance attestation workflow

### Security
- Plain `weave switch` never runs lifecycle scripts and never opens install-script networking
- Environment variables cannot enable execution
- `profile=open` rejected; HTTP prebuild URLs rejected
- Fail closed on integrity / host / peer / platform mismatches

### Intentionally unsupported
- Windows exec sandbox, yarn/pnpm lockfiles as first-class inputs, FUSE/overlayfs/daemon,
  open lifecycle networking, inventing SRI/URLs/outputs

[Unreleased]: https://github.com/hocestnonsatis/weave/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hocestnonsatis/weave/releases/tag/v0.1.0

# Changelog

All notable changes to Weave are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for the **0.x** series: breaking changes may occur in minor versions; patch
releases are for fixes and docs.

## [Unreleased]

## [0.1.1] — 2026-08-12

Stabilization release packaging post-`v0.1.0` adoption and agent-workflow work.
**Not a new architecture or feature-phase expansion** (see ADR-0020).

### Added
- `weave guide [--json]` — minimal adopt/switch/recover recipe for agents/humans
- `weave recover [--json]` — clear leftover `.weave/candidate` / dangling active
- `weave init --json` (idempotent) with `created` + `next_steps`
- Agent environment ownership: `--owner` on switch/create; `env remove` / `env prune --owner`
- Machine-readable `--json` on switch / materialize / env / gc / status (environments + `next_steps`)
- Docs: `docs/agent-quickstart.md`; Phase 16–19 benchmark reports; ADR-0018/0019/0020
- Release verify covers guide/init/status/recover JSON after extraction-only path

### Changed
- Clearer refuse path when Yarn/pnpm/Bun lockfiles are present without `package-lock.json`
- Adoption docs point agents at `guide --json` first
- Post-0.1 development rule: no autonomous feature phases without approved design

### Security
- Unchanged dual gate: plain `switch` never runs scripts / never opens install-script network
- Env vars still cannot enable execution; recover never enables exec or mutates live `node_modules` by default

### Intentionally unsupported (unchanged)
- Windows exec sandbox, yarn/pnpm/Bun as first-class inputs, FUSE/overlayfs/daemon,
  open lifecycle networking, inventing SRI/URLs/outputs, AI auto-detect/trust, MCP/IDE plugins

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

[Unreleased]: https://github.com/hocestnonsatis/weave/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/hocestnonsatis/weave/releases/tag/v0.1.1
[0.1.0]: https://github.com/hocestnonsatis/weave/releases/tag/v0.1.0

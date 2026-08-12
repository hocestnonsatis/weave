# Weave

Git-aware development environment engine for Node.js projects.

Weave separates **source state**, **dependency state**, and **workspace state**. `node_modules` is treated as a materialized view of a content-addressed dependency environment — not as the source of truth.

> Architecture contract: [`WEAVE.md`](./WEAVE.md)

## Status

**0.1.0** (draft GitHub Release). Extraction-first public 0.x scope.
Plain `weave switch` stays execution- and network-free; experimental exec
requires config opt-in + `--with-exec` and is never silently enabled.

- Repository: https://github.com/hocestnonsatis/weave
- License: [MIT](./LICENSE)
- Release notes: [`docs/release-notes-0.1.md`](./docs/release-notes-0.1.md) · Changelog: [`CHANGELOG.md`](./CHANGELOG.md)

Compatibility: [`docs/compatibility.md`](./docs/compatibility.md) · Platforms: [`docs/supported-platforms.md`](./docs/supported-platforms.md) · Security: [`docs/security.md`](./docs/security.md) · Dependabot: [`docs/dependabot.md`](./docs/dependabot.md) · Release: [`docs/release.md`](./docs/release.md)


## Requirements

See [`docs/supported-platforms.md`](./docs/supported-platforms.md). Short form:

- Linux (x86_64)
- Rust stable (1.75+) to build
- Git
- Node.js projects with npm `package-lock.json`

## Build / release

```bash
bash scripts/ci-gates.sh          # fmt + clippy + tests
bash scripts/release-build.sh     # dist/weave + SHA256SUMS + SRI + BUILDINFO
bash scripts/verify-release.sh    # fresh-install extraction-only check
bash scripts/install-from-dist.sh # install after checksum verify
```

Dev build:

```bash
cargo build -p weave-cli
cargo run -p weave-cli -- --help
```

## Quick start

From a Git repository that contains `package.json` and `package-lock.json`:

```bash
weave init
weave doctor
weave switch
weave status
```

Extraction-only projects need no `[execution]` block. For native/lifecycle
packages, see [`docs/adoption.md`](./docs/adoption.md).

## Commands (MVP surface)

| Command | Status |
|---------|--------|
| `weave init` | Implemented |
| `weave status` | Implemented |
| `weave env list` | Implemented |
| `weave env create` | Implemented |
| `weave switch [--with-exec]` | Implemented (exec dual-gated) |
| `weave materialize` | Implemented |
| `weave gc` | Implemented (reachability + `--dry-run`) |
| `weave doctor` | Implemented (adoption verdict) |
| `weave exec plan|suggest|run|hash-artifact|apply-pack` | Implemented (discovery ≠ approval) |

## Workspace layout

```text
crates/
  weave-cli       CLI
  weave-core      Domain models
  weave-git       Git adapter (CLI-backed)
  weave-lockfile  npm lockfile detection/parsing
  weave-store     Content-addressed store
  weave-fs        Materialization primitives
  weave-engine    Orchestration
  weave-bench     Offline cold/warm/switch benchmarks
```

## Benchmarks

```bash
cargo run -p weave-bench --release -- run --suite tiny
cargo run -p weave-bench --release -- analyze-corpus
cargo run -p weave-bench --release -- phase3 --out-dir benchmarks/out/phase3
cargo run -p weave-bench --release -- report --out-dir benchmarks/out
```

See [`benchmarks/README.md`](./benchmarks/README.md),
[`benchmarks/corpus/README.md`](./benchmarks/corpus/README.md),
[`docs/benchmarks/phase3-report.md`](./docs/benchmarks/phase3-report.md), and
[`docs/architecture/Q1-Q5-phase2-evidence.md`](./docs/architecture/Q1-Q5-phase2-evidence.md).

## Design constraints (short)

- Do not modify `package.json` / `package-lock.json`
- Prefer reuse of immutable store objects before fetching
- Never leave a partially activated environment after failure
- Fail clearly on unsupported lockfiles / native packages rather than guessing

See `WEAVE.md` for the full charter.

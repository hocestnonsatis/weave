# Weave benchmarks

Phase 2–3 methodology. **Do not claim Weave is faster than npm/pnpm until
numbers are measured on your machine with hardware context and comparable
conditions.**

## Suites

### Synthetic / offline (deterministic)

| Command | Purpose |
|---------|---------|
| `run --suite tiny\|small\|medium\|large\|monorepo\|native\|all` | Offline Weave timings |
| `report` | Phase 2 offline aggregate |

### Real-world corpus (Phase 3)

| Command | Purpose |
|---------|---------|
| `analyze-corpus` | Graph metrics on pinned lockfiles (no network) |
| `phase3` | Full Phase 3 pipeline + architectural gate |

Corpus provenance: [`corpus/README.md`](./corpus/README.md).

## Measurement classes (do not mix)

1. **Offline / reproducible** — synthetic tarballs or lockfile-only analysis.
2. **Network-dependent** — registry fetches / `npm ci` / `pnpm install` (optional flags; label clearly).

## Phase 3 highlights (see `docs/benchmarks/phase3-report.md`)

- Real lockfiles up to ~3459 packages analyzed.
- NestJS 10.3→10.4 artifact overlap ≈ 74% of A; axios 1.6→1.7 ≈ 100%.
- Synthetic shared 95%→0% A↔B stays ~4–5 ms for 40-pkg trees (this host).
- Materialize pressure: 250 pkgs / 5 files → 20 ms, 1500 hardlinks (offline).
- **No evidence** for FUSE/overlayfs/daemon yet (ADR-0013).

## Running

```bash
cargo run -p weave-bench --release -- analyze-corpus
cargo run -p weave-bench --release -- phase3
cargo run -p weave-bench --release -- run --suite tiny --with-npm --with-pnpm
```

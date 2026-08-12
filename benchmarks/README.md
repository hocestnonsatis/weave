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

### AI-agent parallel validation (Phase 16)

| Command | Purpose |
|---------|---------|
| `phase16` | Offline npm/pnpm/Weave parallel env comparison |
| `phase16 --network` | Separate network class using corpus lockfiles |

Report: [`docs/benchmarks/phase16-ai-agent-report.md`](../docs/benchmarks/phase16-ai-agent-report.md).

### AI-agent scale validation (Phase 17)

| Command | Purpose |
|---------|---------|
| `phase17` | Offline scale ladder (80–280 pkgs, parallel 2/4/8/16, high/low overlap) |
| `phase17 --network` | Separate network class (rimraf / typescript / axios) |
| `phase17 --quick` | Skip largest offline ladder steps |
| `phase17 --rerender-json benchmarks/out/phase17/phase17-report.json` | Rebuild markdown from prior JSON |

Report: [`docs/benchmarks/phase17-ai-scale-report.md`](../docs/benchmarks/phase17-ai-scale-report.md).

### Agent-native workflow (Phase 18)

| Command | Purpose |
|---------|---------|
| `phase18` | Owned concurrent agent roots + JSON lifecycle vs Phase-17-shaped workload |

Report: [`docs/benchmarks/phase18-agent-workflow-report.md`](../docs/benchmarks/phase18-agent-workflow-report.md).  
ADR: [`docs/decisions/ADR-0018-agent-native-workflow.md`](../docs/decisions/ADR-0018-agent-native-workflow.md).

### Zero-friction adoption (Phase 19)

| Command | Purpose |
|---------|---------|
| `phase19` | Adoption friction measurements + agent help/JSON simulation |

Report: [`docs/benchmarks/phase19-adoption-report.md`](../docs/benchmarks/phase19-adoption-report.md).  
ADR: [`docs/decisions/ADR-0019-zero-friction-adoption.md`](../docs/decisions/ADR-0019-zero-friction-adoption.md).

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

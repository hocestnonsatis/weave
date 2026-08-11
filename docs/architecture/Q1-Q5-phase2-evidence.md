# Q1–Q5 evidence review (Phase 2)

Date: 2026-08-11  
Basis: ADR-0011/0012, `docs/lifecycle.md`, crash/concurrency tests, and
`benchmarks/out/phase2-report.md` (regenerate with `weave-bench report`).

WEAVE.md §44 requires these questions stay **open**. This note records evidence
and **provisional** leanings only — not permanent architecture locks.

## Measured context (this host)

Offline `all-offline` suite (linux/x86_64, release):

| Suite | cold ms | warm ms | A→B ms | notes |
|-------|--------:|--------:|-------:|-------|
| tiny | 4 | 4 | 3 | hardlinks only |
| small (~25 pkgs) | 5 | 4 | 4 | hardlinks |
| medium (~80 pkgs) | 9 | 6 | 7 | hardlinks |
| monorepo | 4 | 4 | — | workspace links |
| native | 4 | 4 | 4 | prefer_copy for addon |

Warm and prepared A↔B switches stay in the same low-ms band as cold for these
synthetic trees. No evidence yet that hardlink materialization is a bottleneck
requiring overlayfs/FUSE.

## Q1 — Filesystem view (hardlink / reflink / overlayfs / VFS)

**Evidence:** Hardlink + copy path works; native/install-script packages copy
(`prefer_copy`). Medium suite A↔B ~7 ms with cache hits. No correctness failure
forcing overlayfs/FUSE.

**Provisional leaning:** Keep hardlink+copy. Do **not** introduce FUSE/overlayfs
until a larger real lockfile shows inode/latency pressure hardlinks cannot meet.

## Q2 — Lifecycle script execution

**Evidence:** Scripts are detected and doctor-warned; never executed
(ADR-0012). Native fixture materializes with copies. Auto-running scripts would
break reproducibility and security goals in WEAVE.md.

**Provisional leaning:** Stay on detect+copy+document. Prefer a future
**controlled/explicit** execution layer (or deliberate `npm rebuild`) over
silent auto-run. Still open.

## Q3 — Git source isolation as a core feature

**Evidence:** Current product value is dependency-environment switching with
Git as source-of-truth for the working tree, not isolated source checkouts.
No Phase 2 benchmark measured multi-worktree source isolation.

**Provisional leaning:** Defer. Dependency CAS + switch already addresses the
central pain; source virtualization waits for demonstrated need.

## Q4 — Replace package managers vs environment layer

**Evidence:** Weave acquires/materializes from lockfiles with pluggable
sources; optional npm/pnpm compare rows exist but are not required for the
offline path. Directory `file:` still NotImplemented.

**Provisional leaning:** Remain an **environment layer** around lockfile truth
for now. Full installer replacement is premature without registry protocol
coverage and lifecycle policy (Q2).

## Q5 — Store metadata (SQLite vs filesystem)

**Evidence:** Filesystem CAS + JSON environment records + registry JSON pins
support reachability GC and concurrent puts. No metadata-query workload yet
that needs SQL.

**Provisional leaning:** Keep filesystem metadata. Revisit SQLite if
cross-project queries, pinning UX, or GC planning become hot paths.

## What would reopen an aggressive redesign

1. Real monorepo lockfile where warm A↔B is dominated by inode creation cost.
2. Native addon correctness requiring per-platform rebuild orchestration (Q2).
3. Multi-project GC/registry scale where JSON scan is too slow (Q5).

Until then: continue hardening the current architecture.

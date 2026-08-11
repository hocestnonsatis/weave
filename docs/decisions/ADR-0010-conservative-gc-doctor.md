# ADR-0010: Conservative GC and doctor diagnostics

## Status

Accepted (Milestone 9)

## Date

2026-08-11

## Context

WEAVE.md requires failure hardening and a GC that never deletes reachable
artifacts. Full reachability GC needs pinned roots and careful concurrency.

## Decision

- `weave gc` removes temps **and** unreachable complete artifacts (ADR-0011).
  Roots: project environments + registered projects + `$WEAVE_HOME/pins.json`.
- Reachable tarballs / unpacked packages referenced by known environments are kept.
- `weave gc --dry-run` counts unreachable objects without deleting them.
- `weave doctor` checks Git, lockfile parse, `.weave` config, store presence,
  environment artifact integrity, and leftover candidate/backup dirs.
- Concurrent identical `ContentStore::put` calls are covered by tests.

## Consequences

- Safe to run GC anytime without losing prepared environments.
- Disk reclamation of unused complete artifacts waits for a later reachability GC.

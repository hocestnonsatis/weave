# ADR-0008: Transactional node_modules activation via rename

## Status

Accepted (Milestone 7)

## Date

2026-08-11

## Context

WEAVE.md §25 requires candidate → validate → atomic activate, leaving the prior
active environment intact on failure.

## Decision

- Materialize into `.weave/candidate/` using lockfile path keys (`node_modules/…`).
- Validate extractable package directories exist.
- Activate by:
  1. moving existing `node_modules` → `.weave/backup-node_modules` (if present),
  2. renaming `.weave/candidate/node_modules` → `node_modules`,
  3. deleting the backup on success; restoring it if step 2 fails.
- Update `.weave/metadata/active` only after a successful rename.
- `weave switch` does not run `git switch`; a target label must already match the
  current lockfile-derived environment id.

## Consequences

- Same-filesystem rename provides atomic directory swap on Linux.
- Cross-device projects may need a copy fallback later.
- Hardlink/reflink optimization remains open (Q1).

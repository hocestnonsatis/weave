# ADR-0011: Reachability-based garbage collection

## Status

Accepted (Phase 2)

## Date

2026-08-11

## Context

WEAVE.md §27 requires `weave gc` to use reachability. Milestone 9 only cleaned
temps. The store is global (`WEAVE_HOME`) while environments are per-project.

## Decision

- GC roots are the union of:
  - artifact ids referenced by all environments in the current project
  - environments in other projects registered under `$WEAVE_HOME/registry/projects/`
    for the same store path
  - explicit pins in `$WEAVE_HOME/pins.json` (`{"artifacts":["<sha256-hex>",…]}`)
- Projects register on `weave init` and whenever an environment is saved.
- After temp cleanup, complete objects and unpacked cache entries not in the
  root set are deleted.
- `weave gc --dry-run` counts reachability deletions without performing them
  (temps are still removed).

## Consequences

- Shared-store safety depends on registration; unregistered projects can lose
  artifacts if another project GCs the same store. `weave init` / env creation
  re-registers.
- Pins protect long-lived artifacts without an environment record.

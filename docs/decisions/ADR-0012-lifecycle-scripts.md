# ADR-0012: Lifecycle scripts are detected, not executed

## Status

Accepted (Phase 2)

## Date

2026-08-11

## Context

WEAVE.md warns against automatic lifecycle execution on branch switch. Packages
with install scripts / native addons still need correct materialization.

## Decision

- Parse and surface `hasInstallScript` / native heuristics.
- Materialize those packages with **copy** (not hardlink) into candidates.
- Do **not** execute lifecycle scripts inside Weave.
- Document unsupported cases in `docs/lifecycle.md`.
- `weave doctor` warns when the active graph contains install-script packages.

## Consequences

- Environments are reproducible filesystem trees of tarball contents.
- Native rebuilds remain a deliberate user/CI step.
- Q1 stays open for a future controlled execution layer.

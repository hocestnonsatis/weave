# ADR-0007: Environment identity from graph + platform + materializer

## Status

Accepted (Milestone 6 partial)

## Date

2026-08-11

## Context

WEAVE.md §13 defines environment identity as a hash of dependency graph,
platform, runtime, and materialization format — not branch name.

## Decision

- `EnvironmentId = SHA-256(graph_identity || os || arch || materialization_version)`.
- Persist records as `.weave/environments/<id>.json`.
- Branch association is optional metadata (`label`), not the id.
- Active pointer is `.weave/metadata/active` (atomic replace).
- Artifact maps may start empty and be filled when acquisition/materialize runs.

## Consequences

- Multiple branches can share one environment id.
- Runtime/Node ABI is not yet in the identity (deferred until native install needs it).
- Full transactional activation of `node_modules` remains Milestone 7.

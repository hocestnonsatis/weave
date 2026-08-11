# ADR-0006: Copy-extract materialization (Milestone 5)

## Status

Accepted (temporary)

## Date

2026-08-11

## Context

WEAVE.md leaves Q1 open (hardlink/reflink/overlay/VFS). Milestone 5 needs a
correct `node_modules` tree from stored tarballs before optimizing.

## Decision

- Build a [`MaterializationPlan`] from the dependency graph + artifact ids.
- Extract npm `.tgz` contents into `dest/<package-key>/`, stripping `package/`.
- Reject path traversal and absolute/escaping symlinks.
- Skip link/workspace nodes (no blind global package symlinks).
- Use plain file copies from extraction; no hardlink/reflink yet.

## Consequences

- Correctness over deduplication for the first materializer.
- Later milestones may replace extraction with linking without changing the plan shape.

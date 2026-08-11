# ADR-0003: Lockfile-derived dependency graph identity

## Status

Accepted (Milestone 2)

## Date

2026-08-11

## Context

WEAVE.md requires a deterministic dependency representation from npm
`package-lock.json`, and environment identity must not be branch name alone.

## Decision

- Parse lockfileVersion 1 (nested `dependencies`) and 2–3 (`packages` map).
- Model nodes by lockfile path key (`PackageKey`), not by name@version alone.
- Preserve peer / optional / install-script / os / cpu metadata on nodes.
- Build edges using Node-style `node_modules` resolution from each parent key.
- Graph identity is SHA-256 over sorted nodes (key, name, version, integrity,
  source) and edges.

## Consequences

- Nested duplicate versions remain distinct nodes.
- Workspace/link packages are first-class (`PackageSource::{Workspace,Link,Path}`).
- Native detection is heuristic until install/materialization needs a stronger model.

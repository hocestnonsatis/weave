# ADR-0015: npm workspace link wiring

## Status

Accepted (Phase 4)

## Context

npm workspaces record packages both as workspace path keys (`packages/a`) and
as `node_modules/@scope/name` entries with `"link": true` and a relative
`resolved` path. Phase 3 skipped these during extraction, so monorepo packages
could not resolve each other after materialization.

## Decision

Minimum npm-compatible wiring:

- Plan workspace/`link:` nodes as `link_only` with a `link_target`.
- After registry packages are placed, create a **relative symlink** from
  `node_modules/<name>` to the workspace directory (targets valid after
  activation into `{project}/node_modules`).
- Absolute link targets and `..` traversal are rejected.
- Do not build a universal monorepo abstraction (no Nx/Turbo/pnpm workspace
  emulation).

## Consequences

- `packages/a` can `require('@acme/b')` when `b` is linked under `node_modules`.
- Symlinks point at mutable workspace sources (intentional; matches npm).
- Distinct from `file:` path deps (immutable snapshots, ADR-0014).

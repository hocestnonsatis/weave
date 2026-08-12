# ADR-0018: Agent-native workflow via existing env model

## Status

Accepted (Phase 18)

## Date

2026-08-12

## Context

Phase 17 showed Weave wins for warm, high-overlap, parallel environments sharing
CAS — not cold one-shot installs. Agents need a minimal, machine-readable
workflow without a parallel API, MCP, daemon, or auto-detection of AI clients.

Investigation: `status`, `switch`, environment identity (ADR-0007), and
reachability `gc` already cover create/activate/share. Gaps were JSON coverage,
explicit ownership for cleanup, and safe remove/prune of non-active records.

## Decision

- Prefer extending existing commands (`switch`, `env`, `status`, `gc`,
  `materialize`) with `--json` and optional `--owner`.
- Owner/session is caller-supplied metadata only; Weave never auto-detects or
  trusts AI agents.
- Environment identity remains graph+platform+materializer (unchanged).
- `env remove` / `env prune --owner` operate on metadata only; refuse active;
  never mutate another environment’s `node_modules`.
- Artifact reclamation stays `weave gc` (reachability).
- One agent = one project root; shared CAS via `WEAVE_HOME`.

## Consequences

- No MCP/IDE/daemon/FUSE in this phase.
- Agents must pass `--owner` deliberately to use prune.
- Cold registry installs remain outside Weave’s win domain (Phase 17).

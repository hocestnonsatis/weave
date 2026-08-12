# ADR-0019: Zero-friction adoption without automation

## Status

Accepted (Phase 19)

## Date

2026-08-12

## Context

Phase 18 showed the env model is enough for concurrent agents. Remaining pain
was adoption friction: non-idempotent init, sparse JSON, unclear unsupported
projects, leftover candidate recovery, and docs that assume architecture knowledge.

## Decision

- Keep Weave explicit (not automatic): no silent npm replacement, no auto-exec,
  no AI auto-detect.
- Make `weave init` idempotent with `--json` and `next_steps`.
- Ship `weave guide --json` + `docs/agent-quickstart.md` so agents can operate
  from CLI help + JSON alone.
- Ship `weave recover` for leftover candidate / dangling active (safe defaults).
- Fail closed with actionable errors on Yarn/pnpm/Bun-only trees.
- Enrich `status.next_steps` for follow-through after branch changes.

## Consequences

- Happy path is 3–4 commands for extraction-ready npm projects.
- Native/lifecycle policy and package-manager choice remain manual.
- Cold network installs remain outside Weave’s win domain.

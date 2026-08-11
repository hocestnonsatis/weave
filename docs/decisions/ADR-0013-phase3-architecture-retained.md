# ADR-0013: Phase 3 retains hardlink+copy architecture

## Status

Accepted (Phase 3 decision gate)

## Date

2026-08-11

## Context

Phase 3 collected real lockfile corpus metrics, synthetic divergence timings,
materialization pressure, lifecycle classification, and a correctness audit.
See `docs/benchmarks/phase3-report.md`.

## Decision

- **Retain** content-addressed store + hardlink/copy materialization + transactional
  activation.
- **Do not** introduce FUSE, overlayfs, or a daemon based on Phase 3 evidence.
- **Do not** implement arbitrary lifecycle-script execution yet (ADR-0012 stands).
- Track correctness gaps (bin links, exports, `file:` dirs) as explicit work items.
- Next evidence step for scale: network-gated materialize of large corpus lockfiles
  (nestjs ~3.4k packages) using a warm npm cache — clearly labeled non-deterministic
  relative to offline suites.

## Consequences

- Product roadmap stays on correctness/coverage (bins, peers, scripts policy)
  rather than filesystem virtualization.
- Q1–Q5 remain formally open but provisional leanings are documented in the
  Phase 3 report with confidence levels.

# ADR-0020: Post-0.1 evidence-first development

## Status

Accepted

## Date

2026-08-12

## Context

Phases through 19 established Weave’s CAS/materialization core, agent-facing
workflow surface, and adoption path for extraction-ready npm projects. Continuing
to open “next feature phases” on theoretical grounds would reintroduce
architecture expansion without evidence.

## Decision

After `v0.1.0` / Phase 19, classify every request before coding:

1. Correctness bug  
2. Security issue  
3. Data-loss/recovery issue  
4. Reproducible performance regression  
5. Real-world adoption blocker  
6. New feature  

Categories 1–5 may be implemented when supported by evidence.

Category 6 requires a short design/evidence report and **explicit approval**
before implementation. Prefer deleting complexity over adding it. Do not expand
architecture for theoretical compatibility alone.

Cursor rule: `.cursor/rules/post-0.1-development.mdc` (always apply). This
supersedes autonomous next-milestone continuation for post-0.1 work.

## Consequences

- Agents stop inventing Phase 20+ feature work without approval.
- Bug/security/recovery/regression/adoption fixes remain unblocked when evidenced.
- Feature ideas land as design reports first, not code.

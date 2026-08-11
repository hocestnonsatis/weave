# Lifecycle scripts

## Current policy (default)

Weave **does not execute** npm lifecycle scripts during `weave switch` /
`materialize` (ADR-0012).

Detection remains first-class (`hasInstallScript`, `prefer_copy`, doctor warnings).

## Controlled execution (Phase 7 MVP)

ADR-0018 offline Bubblewrap vertical slice is available:

```bash
weave exec plan
weave exec run <package> --input /path/to/package-copy
```

Requirements: `[execution] enabled = true` in `.weave/config.toml`, package in
`allow_packages`, declared outputs listed, `bwrap` installed. Environment
variables alone never enable execution. Default `weave switch` still does not
run scripts.

Adoption workflow (what to do when packages need scripts): see
[`docs/adoption.md`](./adoption.md).

See:

- [`docs/decisions/ADR-0018-sandboxed-lifecycle-execution.md`](./decisions/ADR-0018-sandboxed-lifecycle-execution.md)
- [`docs/lifecycle-classification.md`](./lifecycle-classification.md)
- [`docs/native.md`](./native.md)

## Why scripts are not auto-run

WEAVE.md: do not automatically run arbitrary package lifecycle scripts during a
simple branch switch unless the environment requires them and behavior is
explicit. Untrusted lifecycle execution is a security and reproducibility hazard.

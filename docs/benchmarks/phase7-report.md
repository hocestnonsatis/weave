# Phase 7 report: Sandboxed Execution MVP

Date: 2026-08-11  
Follows ADR-0018. Architecture unchanged (no FUSE/overlayfs/daemon/resolver).

## Vertical slice implemented

1. **`[execution]` config** — disabled by default; `enabled` only from
   version-controlled `.weave/config.toml` (env vars cannot enable).
2. **`weave exec plan`** — dry-run planner with allowlist filtering.
3. **`weave exec run`** — Bubblewrap offline adapter; fail closed if `bwrap`
   missing; refuses live `node_modules` inputs.
4. **CAS seal** — only `execution.declared_outputs` paths packed into CAS.
5. **Cache identity** — OS/CPU/Node ABI + profile + scripts + outputs + input
   tree digest (`weave-exec-v1`).
6. **Fixture** — `crates/weave-engine/fixtures/exec-offline-gen` (writes
   `generated/hello.txt` only).
7. **Gated test** — `WEAVE_EXEC_TESTS=1 cargo test -p weave-engine --test exec_bwrap -- --ignored`

## Security / failure tests

| Case | Coverage |
|------|----------|
| execution disabled | `require_execution_enabled` |
| env cannot enable | `env_var_cannot_enable_execution` |
| sandbox unavailable | `WEAVE_BWRAP_PATH` → fail closed |
| undeclared / empty seal | `seal_declared_outputs` empty list |
| path traversal | `validate_declared_output` |
| failed script | install exits 1 → no seal |
| missing declared output | seal error |
| live tree refusal | input under `node_modules` rejected |
| cache identity mismatch | ABI/outputs change key |

## What this proves

Opt-in offline sandboxed install can generate a declared file, seal it into the
CAS with a platform/ABI-qualified identity, without touching the active
`node_modules` tree or requiring network.

## Next smallest implementation step

Wire sealed outputs into materialization/activation for allowlisted packages
(candidate-only), still never auto-run on plain `weave switch` unless
`execution.enabled` and an explicit `--with-exec` flag are both present.

**Done in Phase 8** — see `docs/benchmarks/phase8-report.md`.

# Phase 6 report: Sandboxed Lifecycle & Native Execution Design

Date: 2026-08-11  
Architecture: **unchanged** (CAS + hardlink/copy + transactional activation)  
Execution: **not implemented** (design + dry-run plan only)

## ADR

[`docs/decisions/ADR-0018-sandboxed-lifecycle-execution.md`](../decisions/ADR-0018-sandboxed-lifecycle-execution.md)

## Chosen design

- **Controlled Weave execution layer** (answers WEAVE.md Q1), not silent npm
  delegation and not automatic-on-switch.
- **Default deny** — ADR-0012 remains default for `switch` / `materialize`.
- **Opt-in** via version-controlled `.weave/config.toml` `[execution]` plus
  explicit CLI (future); env alone cannot enable.
- **Linux-first sandbox:** Bubblewrap primary; weak Landlock/seccomp only with
  explicit acknowledgment; fail closed if no sandbox.
- **Offline by default;** optional allowlisted prebuild fetch profile.
- **Outputs sealed into CAS** with platform + Node ABI–qualified cache keys;
  activation still transactional; live tree never mutated by the runner.
- **No daemon / FUSE / overlayfs / new resolver.**

## Rejected alternatives

| Alternative | Why rejected |
|-------------|--------------|
| Auto-run scripts on every switch | Violates WEAVE.md; trust/repro hazard |
| Unsandboxed `npm rebuild` on live `node_modules` | No CAS seal; mutates “immutable” env |
| Docker/Podman-per-script as v1 | Daemon-ish ops; heavier than needed |
| Firecracker/VM per package | Latency/ops cost; overkill for v1 |
| FUSE/overlayfs execution mounts | Architecture freeze (ADR-0013) |
| Long-lived Weave daemon | Explicitly out of scope |
| Open network profile in v1 | Exfil risk; allowlist-only later |

## Minimal safe experiment (done)

Non-executing `plan_execution` classifier + doctor `exec-plan` info finding.
Unit-tested; never spawns package scripts.

## Smallest next implementation step

1. Add disabled-by-default `[execution]` config schema.
2. Ship `weave exec plan` CLI (JSON/human) over `plan_execution`.
3. Spike one allowlisted fixture under `bwrap` offline → seal outputs to CAS →
   smoke behind `WEAVE_EXEC_TESTS=1`.

## Gates

`cargo test --workspace`, `cargo fmt --check`, `cargo clippy -D warnings` — pass.

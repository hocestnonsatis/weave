# Phase 18: Agent-Native Workflow

Host: `linux` / `x86_64` · Weave `0.1.0` · workload `p18-agent-hi`

## 1. Minimal agent workflow

One agent = one project root (worktree/checkout). Agents share `WEAVE_HOME` for CAS.

```bash
export WEAVE_HOME=/shared/weave-home
cd /work/agent-$ID && weave init
weave switch --owner agent-$ID --json    # activate; no scripts/network for install
weave status --json                      # id, owner, active, matches_lockfile
weave env list --owner agent-$ID --json
# teardown metadata (never mutates another env):
weave env prune --owner agent-$ID --json
weave gc --json                          # reclaim unreachable store artifacts
```

Identity remains graph+platform+materializer (ADR-0007). Branch name is not identity. Owner is optional metadata supplied by the caller — Weave never detects AI agents.

## 2. Why each new capability is necessary

| Capability | Why |
|---|---|
| `--json` on switch / env / gc / materialize | Agents need stable machine-readable outcomes; human text is insufficient. |
| `--owner` on switch / env create | Lifecycle + cleanup must distinguish agent sessions without auto-detection. |
| `env remove` | Safe delete of a non-active record; refuses active; no cross-env mutation. |
| `env prune --owner` | Abandoned agent metadata cleanup; requires explicit owner (fail closed). |
| status `environments[]` | Lifecycle visibility (active, owner, matches_lockfile) in one snapshot. |
| Existing `switch` / CAS / transactional activate | Unchanged — still the create/activate path; no parallel API. |
| Existing `gc` | Artifact reachability GC; env prune does not replace it. |

## 3. Benchmark evidence

Phase 18 agent workflow (offline, Phase-17-shaped ~80 pkgs high-overlap): warm parallel_8 agent roots with explicit --owner remain in the same material win domain as Phase 17 (shared WEAVE_HOME CAS). New capabilities are lifecycle/JSON/ownership — not a faster materializer.

| scenario | N | wall_ms | per_env_ms | total_unique | nm_apparent | owners_ok | note |
|---|---:|---:|---:|---:|---:|---:|---|
| cold_seed_switch | 1 | 9 | 9 | 36656 | 14822 | 1 | owner=seed; fetched=80 reused=0 |
| parallel_4_warm_owned | 4 | 12 | 3 | 37176 | 59288 | 4 | unique/apparent_nm=0.250; status+owner stamped |
| parallel_8_warm_owned | 8 | 12 | 1 | 38216 | 118576 | 8 | unique/apparent_nm=0.125; status+owner stamped |
| env_prune_dry_run_active_owner | 1 | 2 | 2 | 38337 | 0 | 1 | removed=0 skipped_active=true |

Compare with Phase 17 offline `p17-small-hi` parallel_4/8 warm rows: same shape (shared CAS, flat unique disk as N grows). Phase 18 adds ownership/JSON lifecycle cost that is negligible vs materialization.

## 4. Deliberately outside Weave

- MCP server / IDE plugin / daemon / FUSE / overlayfs
- Auto-detection or trust of AI agents
- Hidden execution, network, or mutation of another environment
- Agent orchestration, scheduling, or prompt protocols
- Replacing npm/pnpm for cold one-shot CI installs (Phase 17)
- Changing CAS / materialization architecture

## Caveats

- Offline only — not comparable to network cold installs.
- One agent = one project root; shared CAS via WEAVE_HOME.
- Owner is always caller-supplied; Weave never auto-detects agents.
- env prune removes metadata only; weave gc reclaims store artifacts.

## Reproduce

```bash
cargo run -p weave-bench --release -- phase18
cargo test -p weave-engine --test agent_workflow
```

Work dir: `/tmp/weave-phase18-1k2ZTa`

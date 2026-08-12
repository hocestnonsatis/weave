# Phase 19: Zero-Friction Adoption

Host: `linux` / `x86_64` · Weave `0.1.0`

## Question

> Can a coding agent use Weave correctly without needing to understand its internal architecture?

## Verdict

YES — suitable as an agent dependency substrate for extraction-ready npm lockfile projects when the agent follows `weave guide --json` and status.next_steps without learning CAS internals. Not automatic; not a silent npm replacement.

Phase 19 adoption: 4 supported cases agent-operable via CLI help/JSON; happy path remains 3–4 commands (guide→init→doctor→switch). Init is idempotent; recover clears interrupted state; Yarn/pnpm-only trees fail closed with actionable errors.

## 1. Remaining adoption friction

- Network cold first switch still slower than npm/pnpm (Phase 17) — expected.
- Native/lifecycle projects still need human-reviewed policy before --with-exec.
- Yarn/pnpm/Bun lockfiles unsupported — intentional fail-closed.
- Agents must still choose WEAVE_HOME when sharing CAS across worktrees.

## 2. Changes actually worth keeping

- Idempotent `weave init --json`
- `weave guide --json` + docs/agent-quickstart.md
- `weave recover --json` for leftover candidate / dangling active
- status.next_steps for agent follow-through
- Clear UnsupportedLockfile when pnpm/yarn/bun present without package-lock.json
- Actionable recovery hints pointing at guide/status --json

## 3. Deliberately left manual

- npm remains the lockfile/resolver owner
- execution.enabled / --with-exec
- git checkout (Weave never runs git switch)
- AI agent detection/trust
- MCP / IDE / daemon / FUSE

## 4. Agent dependency substrate?

YES — suitable as an agent dependency substrate for extraction-ready npm lockfile projects when the agent follows `weave guide --json` and status.next_steps without learning CAS internals. Not automatic; not a silent npm replacement.

## Measurements

| case | supported | cmds | cold_init_ms | warm_init_ms | cold_switch_ms | recover_ms | doctor_err | agent_ok | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| extraction-fixture | true | 4 | 2 | 2 | 4 | 2 | 0 | true | init idempotent; recover removed leftover candidate; next_steps=weave status --json; weave doctor --json |
| pnpm-only-unsupported | false | 1 | - | - | - | - | 1 | true | init error: unsupported project lockfile at /tmp/weave-phase19-gs8zhI/pnpm-only/pnpm-lock.yaml: pnpm lockfile present without package-lock.json; Weave currently supports npm package-lock.json only
Weave does not replace npm and will not convert lockfiles automatically.
Next: keep your existing package manager, or add package-lock.json via `npm i --package-lock-only` if you intentionally want Weave. |
| corpus-rimraf-init-doctor | true | 3 | 2 | - | - | - | 0 | true | pkgs=Some(351); next=weave switch --json; weave status --json; network switch skipped (offline measurement class) |
| corpus-typescript-init-doctor | true | 3 | 3 | - | - | - | 0 | true | pkgs=Some(389); next=weave switch --json; weave status --json; network switch skipped (offline measurement class) |
| agent-help-json-sim | true | 1 | - | - | - | - | 0 | true | help_mentions_guide=true; guide --json parsed |

## Reproduce

```bash
cargo run -p weave-bench --release -- phase19
cargo run -p weave-cli -- guide --json
```

Work dir: `/tmp/weave-phase19-gs8zhI`

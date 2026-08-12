# Weave agent quickstart

Use this page (or `weave guide --json`) — you do **not** need Weave internals.

## Adopt an existing npm repo

```bash
weave guide --json
weave init --json          # idempotent
weave doctor --json        # stop if Error / unsupported lockfile
weave switch --json        # materialize node_modules (no scripts, no script network)
weave status --json        # follow next_steps
```

Required: Git + `package.json` + npm `package-lock.json` (v1–3).

## After `git checkout`

```bash
git checkout <branch>      # Weave never runs git for you
weave switch --json
weave status --json
```

## Recovery / cleanup

```bash
weave recover --json       # leftover .weave/candidate / dangling active
weave gc --json            # reclaim unreachable CAS artifacts
```

## Rules (do not violate)

- Do **not** treat Weave as a silent npm replacement.
- Do **not** edit `package.json` / `package-lock.json` via Weave.
- Do **not** enable `execution.enabled` or `--with-exec` without human-reviewed policy.
- Do **not** invent SRI/URLs/outputs.
- Pass `--owner` only for explicit agent session cleanup (`env prune`); Weave never auto-detects agents.
- Prefer `--json` on every command you parse.

## Unsupported (fail closed)

- Yarn / pnpm / Bun lockfiles alone
- Missing Git or `package.json`
- Native/lifecycle completeness without reviewed policy (see `docs/adoption.md`)

# Adopting Weave in an existing Node project

Shortest **safe** path. Weave never invents SRI, URLs, outputs, or permissions,
and never grants lifecycle scripts general network access.

**Agents:** start with [`docs/agent-quickstart.md`](./agent-quickstart.md) or
`weave guide --json` — architecture knowledge is not required.

## 1. Prerequisites

- Git repository
- `package.json` + npm `package-lock.json` (lockfileVersion 1/2/3)
- Linux x86_64 for the current exec sandbox target

Yarn/pnpm/Bun-only trees are detected and refused clearly. Weave will not
convert lockfiles or replace those package managers.

## 2. Extraction-only projects (most apps)

```bash
weave init --json       # idempotent
weave doctor --json     # ExtractionReady / no execution required
weave switch --json     # materialize node_modules from the lockfile
weave status --json     # follow next_steps
```

No `[execution]` configuration is required. Plain `weave switch` stays
execution- and network-free.

Interrupted switch leftovers: `weave recover --json`.

## 3. Projects with native / lifecycle packages

1. Start with extraction: `weave init && weave switch && weave doctor && weave exec plan`
2. Read the adoption verdict (`ExtractionReady` / `PartialNeedsPolicy` / `Blocked`).
3. If you already downloaded and independently verified a binary:

   ```bash
   weave exec hash-artifact ./addon.node \
     --package demo-native \
     --output prebuilds/linux-x64/addon.node \
     --url https://cdn.example.com/addon.node
   ```

   Hashing measures bytes only — it does not invent trust or enable execution.

4. Optional reviewed policy packs:

   ```bash
   weave exec apply-pack policy-packs/example-demo.toml          # dry-run
   weave exec apply-pack policy-packs/example-demo.toml --write  # merge only
   ```

   Apply never sets `execution.enabled` or `profile = "open"`.

5. After review: `weave exec suggest`, edit config (`profile=prebuild-fetch` if
   fetching), set `execution.enabled = true`, then `weave switch --with-exec`.

6. NeedsIntegrity / Opaque gaps still require human-verified
   `[[execution.prebuild.fetches]]` — Weave will not invent SRI or open script networking.

## 4. What Weave refuses (on purpose)

| Situation | Behavior |
|-----------|----------|
| Missing required peers | Switch fails; doctor Error |
| Unsafe install (`curl \| sh`, …) | Never allowlisted / suggested |
| HTTP prebuild URL | BlockedUnsafe |
| Missing SRI | NeedsIntegrity — hash-artifact / manual policy |
| Symlink passed to hash-artifact | Refused |
| `WEAVE_EXEC=1` alone | Does not enable execution |
| Future config version | Fail closed until Weave is upgraded |

## 5. Docs

- [`docs/native.md`](./native.md)
- [`docs/lifecycle.md`](./lifecycle.md)
- [`docs/compatibility.md`](./compatibility.md)
- [`policy-packs/README.md`](../policy-packs/README.md)
- Phase 13 report: [`docs/benchmarks/phase13-report.md`](./benchmarks/phase13-report.md)

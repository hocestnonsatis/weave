# Dependabot & dependency-alert policy

Release hygiene for Weave: keep meaningful coverage for what Weave **ships and
builds**, without treating pinned benchmark/corpus lockfiles as production
dependencies.

## Quantified baseline (2026-08-12, before triage)

| Source | Open alerts | Ecosystem |
|--------|-------------|-----------|
| `benchmarks/corpus/**/package-lock.json` | **1873** | npm |
| `crates/weave-lockfile/fixtures/**` | **1** | npm |
| Root `Cargo.lock` / workspace crates | **0** | cargo |
| **Total open** | **1874** | |

Severity mix on that set: critical 173 · high 905 · medium 606 · low 190.
Scope mix: development 1567 · runtime 307 — still all fixture/corpus paths.

There is **no** npm package that Weave itself executes or ships. The binary is
Rust (`Cargo.lock`). Corpus lockfiles exist so Weave can analyze / materialize
**pinned real-world graphs** without mutating upstream provenance.

## Categories and treatment

### 1. Production / build dependencies (Weave ships or builds)

| Surface | Paths | Dependabot treatment |
|---------|-------|----------------------|
| Rust workspace | `/Cargo.toml`, `/Cargo.lock`, `crates/*/Cargo.toml` | **Version updates** via `.github/dependabot.yml` (`package-ecosystem: cargo`). Keep Dependabot **alerts** and **security updates** enabled. |
| GitHub Actions | `.github/workflows/*` | **Version updates** via `github-actions` ecosystem. |

These are the only dependency classes that affect the released `weave` binary
or the release/CI pipeline.

### 2. Test / benchmark harness dependencies

| Surface | Paths | Treatment |
|---------|-------|-----------|
| `weave-bench` crate | `crates/weave-bench/Cargo.toml` | Covered by the **cargo** workspace update (same `Cargo.lock`). Not npm. |
| Benchmark scripts / harness | `benchmarks/` (non-corpus) | No separate npm product surface. |

### 3. Pinned real-world corpus fixtures

| Surface | Paths | Treatment |
|---------|-------|-----------|
| Lockfile corpus | `benchmarks/corpus/**/package-lock.json` (+ companion `package.json`, `PROVENANCE.json`) | **Do not** Dependabot-update. Bytes are provenance for Phase 3+ analysis (`benchmarks/corpus/README.md`). Alerts are **`not_used`**: not Weave runtime code. |

Changing these to “fix” CVEs would destroy reproducibility and is out of
scope for silence-the-alerts.

### 4. Vendored / generated / example / parser fixtures

| Surface | Paths | Treatment |
|---------|-------|-----------|
| Lockfile parser fixtures | `crates/weave-lockfile/fixtures/**` | **Do not** update for CVEs. Inputs to unit/integration tests; shape matters more than freshness. |
| Engine policy / adoption fixtures | `crates/weave-engine/fixtures/**` | Mostly hand-authored `package.json` (often no lockfile). Not product deps. Exclude from npm version updates. |

## What we configure in-repo

`.github/dependabot.yml`:

- Enables **cargo** + **github-actions** weekly updates.
- Does **not** register an `npm` ecosystem. Adding `directory: "/"` for npm
  would open PRs against corpus/fixtures (observed: Dependabot already opened
  grouped npm bumps under `benchmarks/corpus/small/*`).

`scripts/dismiss-fixture-dependabot-alerts.sh`:

- Bulk-dismisses open alerts whose `manifest_path` is under the fixture/corpus
  prefixes above, reason `not_used`.
- Prints and **keeps** any non-fixture alert (e.g. future cargo findings).

## What stays enabled (do not weaken globally)

- Dependabot **alerts** (vulnerability scanning) remain on.
- Dependabot **security updates** remain available for the cargo/actions surface.
- Code scanning / CodeQL workflows are unrelated and unchanged.

`exclude-paths` in `dependabot.yml` only affects **version-update PRs**, not
alert generation. Alert noise for fixtures is handled by dismiss + auto-triage,
not by disabling scanning.

## Human follow-up: auto-triage rules (UI only)

GitHub has **no public API** to create Dependabot auto-triage rules. After
pushing this policy, a maintainer should add repository rules:

**Settings → Advanced Security → Dependabot rules → New rule**

Recommended rules (dismiss indefinitely, reason equivalent to unused code):

1. **Corpus lockfiles** — target manifest paths under `benchmarks/corpus/`
   (list each `…/package-lock.json`, or use path filters if the UI allows a
   prefix / multi-select of the 20 corpus manifests).
2. **Lockfile fixtures** — target
   `crates/weave-lockfile/fixtures/**/package-lock.json`.
3. Optional: engine fixture `package.json` paths under
   `crates/weave-engine/fixtures/`.

Without these rules, newly published GHSAs will reopen alerts on the same
pinned trees; re-run:

```bash
DRY_RUN=1 bash scripts/dismiss-fixture-dependabot-alerts.sh
bash scripts/dismiss-fixture-dependabot-alerts.sh
```

## Closing noise PRs

Dependabot PRs that only bump packages inside corpus/fixture lockfiles should
be closed without merge (they fight provenance). Prefer the dismiss script +
auto-triage so Dependabot stops opening them.

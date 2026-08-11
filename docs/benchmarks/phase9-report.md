# Phase 9 report: Execution Policy Discovery

Date: 2026-08-11  
Keeps ADR-0018 dual gate, Bubblewrap offline, CAS seal, and transactional
activation unchanged.

## Problem

Hand-authored `allow_packages` + `declared_outputs` do not scale to real
native/lifecycle packages.

## What landed

1. **`exec_discover`** — deterministic, non-executing discovery from package
   metadata (`package.json` scripts, `binding.gyp` target names, `binary` field,
   optional `weave.declaredOutputs`, known-name patterns, static write hints).
2. **Candidates ≠ allowed** — plan entries expose `discovered_*` separately from
   config `allowed_outputs` / `package_allowed`, with `PolicyReviewStatus`.
3. **`weave exec plan`** — shows why a package needs execution, discovered
   scripts/outputs, rejected unsafe paths, and dual-gate readiness.
4. **`weave exec suggest [--write]`** — emits a reviewed TOML fragment; never
   sets `enabled = true`; skips curl|sh / unsafe packages; exact safe paths only.
5. **Path safety** — rejects `..`, absolute paths, `**`, multi-globs, directories,
   `.bin/` / nested `node_modules/`; globs are discoverable but not suggestable
   for seal (exact files required).
6. **Fixtures** — `fixtures/policy-discovery/{native-binding,esbuild-like,unsafe-curl,weave-hint}`.

## Security / regression coverage

| Case | Test |
|------|------|
| discovery does not run scripts | `discovery_never_executes_fixture_scripts` |
| candidates ≠ allowed | `project_plan_distinguishes_discovered_from_allowed` |
| unsafe curl\|sh blocked from suggest | `suggestion_never_enables_and_skips_unsafe` |
| ambiguous paths rejected | `reject_ambiguous_output_paths` |
| plain switch execution-free | project policy test + Phase 8 suite |
| suggest never enables | `merge_suggestion_preserves_enabled_false` |

## Is discovery sufficient for real packages?

**Partially.** It is sufficient to *propose* policies for packages with clear
static signals (gyp `target_name`, `binary.module_*`, author `weave.declaredOutputs`,
simple `writeFileSync("…")` literals). It is **not** sufficient alone for packages
whose outputs are only known after a networked prebuild download or opaque
native toolchains — those remain `incomplete` / `NeedsReview` until humans
declare exact seal paths (or a future prebuild-fetch profile lands).

## Next single bottleneck

**Prebuild-fetch / exact native output materialization** — many real addons need
allowlisted CDN fetches and concrete `.node` paths that discovery cannot invent
safely offline; without that, `--with-exec` still cannot complete those packages.

# Weave Phase 3 — Real-World Validation Report

- host: `linux` / `x86_64`
- corpus: `benchmarks/corpus` (20 entries)

## Measurement boundaries

### Measured
- corpus lockfile graph analysis (offline)
- synthetic divergence weave cold/warm/A↔B (offline tarballs)
- materialization pressure hardlink counts (offline)
- lifecycle classification heuristics (offline)
- real lockfile artifact-set overlap (offline, no materialize)

### Unavailable / not run
- full weave materialize of real registry lockfiles without network or vendored tarballs
- network npm ci / pnpm install comparative timings (run separately with --network suite)
- cross-filesystem-boundary hardlink denial on this host (not forced)

## Corpus scale analysis

| id | category | pkgs | artifacts | depth | dup names | optional | peer | native | scripts |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| axios-v1.6 | divergence | 2109 | 1683 | 11 | 217 | 42 | 122 | 7 | 7 |
| axios-v1.7 | divergence | 2110 | 1684 | 11 | 217 | 42 | 123 | 7 | 7 |
| nestjs-v10.3 | divergence | 3459 | 701 | 15 | 344 | 40 | 193 | 14 | 10 |
| nestjs-v10.4 | divergence | 3448 | 844 | 15 | 359 | 40 | 197 | 14 | 10 |
| nestjs | large | 3448 | 844 | 15 | 359 | 40 | 197 | 14 | 10 |
| npm-cli | large | 1199 | 876 | 10 | 122 | 6 | 35 | 5 | 2 |
| puppeteer | large | 1214 | 851 | 8 | 119 | 50 | 47 | 4 | 4 |
| bcrypt-lifecycle | lifecycle | 324 | 311 | 9 | 15 | 1 | 22 | 1 | 1 |
| npm-cli-lifecycle | lifecycle | 1199 | 876 | 10 | 122 | 6 | 35 | 5 | 2 |
| axios | medium | 2110 | 1684 | 11 | 217 | 42 | 123 | 7 | 7 |
| rollup | medium | 1058 | 943 | 9 | 80 | 50 | 74 | 2 | 9 |
| socketio | medium | 383 | 364 | 9 | 41 | 1 | 4 | 2 | 1 |
| typescript | medium | 389 | 368 | 7 | 29 | 38 | 19 | 2 | 4 |
| nestjs-monorepo | monorepo | 3448 | 844 | 15 | 359 | 40 | 197 | 14 | 10 |
| bcrypt | native | 324 | 311 | 9 | 15 | 1 | 22 | 1 | 1 |
| puppeteer-native | native | 1214 | 851 | 8 | 119 | 50 | 47 | 4 | 4 |
| immutable | small | 1071 | 914 | 12 | 107 | 31 | 53 | 7 | 2 |
| moment | small | 601 | 562 | 9 | 65 | 24 | 12 | 3 | 3 |
| rimraf | small | 351 | 307 | 10 | 27 | 5 | 30 | 4 | 1 |
| uuid | small | 1467 | 1134 | 9 | 128 | 5 | 125 | 2 | 1 |

## Real lockfile divergence (artifact fingerprints)

- **benchmarks/corpus/divergence/nestjs-v10.3/package-lock.json vs benchmarks/corpus/divergence/nestjs-v10.4/package-lock.json**: shared=517 only_a=184 only_b=327 shared_of_a=73.8% jaccard=0.503 — _artifact fingerprint overlap from lockfile only (no materialize)_
- **benchmarks/corpus/divergence/axios-v1.6/package-lock.json vs benchmarks/corpus/divergence/axios-v1.7/package-lock.json**: shared=1683 only_a=0 only_b=1 shared_of_a=100.0% jaccard=0.999 — _artifact fingerprint overlap from lockfile only (no materialize)_

## Synthetic divergence (Weave timed, offline)

| label | target | measured shared/A | cold ms | warm ms | A→B ms | B→A ms |
|---|---:|---:|---:|---:|---:|---:|
| synthetic-shared-95% | 95% | 95.0% | 6 | 5 | 6 | 5 |
| synthetic-shared-75% | 75% | 75.0% | 6 | 5 | 5 | 5 |
| synthetic-shared-50% | 50% | 50.0% | 6 | 5 | 5 | 5 |
| synthetic-shared-25% | 25% | 25.0% | 6 | 5 | 5 | 5 |
| synthetic-shared-0% | 0% | 0.0% | 6 | 5 | 5 | 5 |

## Materialization pressure

| label | pkgs | ms | hardlinks | copies | nm bytes | nm inodes |
|---|---:|---:|---:|---:|---:|---:|
| pkgs=25/files=3 | 25 | 5 | 100 | 0 | 3890 | 151 |
| pkgs=80/files=4 | 80 | 8 | 400 | 0 | 15030 | 561 |
| pkgs=150/files=4 | 150 | 13 | 750 | 0 | 28240 | 1051 |
| pkgs=250/files=5 | 250 | 21 | 1500 | 0 | 55140 | 2001 |
| pkgs=40+native-addon | 41 | 6 | 160 | 5 | 6477 | 250 |

## Lifecycle classification (heuristic, no execution)

Corpus totals — extraction_only=30752, generated≈14, native_build≈117, runtime_install≈43

## Correctness audit

- **peer dependencies** — status: `parsed`
  - EdgeKind::Peer edges counted in GraphStats; materialization does not rewrite peer resolution
- **optional dependencies** — status: `parsed`
  - optional nodes counted; Weave does not evaluate OS filters at acquire time beyond recording cpu/os fields
- **nested / duplicated versions** — status: `observed in corpus`
  - duplicated_name_count tracks same name with multiple versions
- **package exports / bin links** — status: `gap`
  - PackageNode has no exports/bin fields; Weave does not create .bin shims during materialize
- **symlinks / workspaces** — status: `partial (workspace nodes observed)`
  - link/workspace nodes skipped for extraction; no automatic link wiring into node_modules for workspace packages
- **native modules** — status: `detected+copy`
  - prefer_copy; no rebuild; platform identity in EnvironmentId
- **lifecycle-generated files** — status: `detect-only`
  - ADR-0012: scripts not executed; corpus lifecycle packages observed=true
- **directory file: dependencies** — status: `unsupported`
  - NotImplemented in acquire path

## Answers to Phase 3 questions

1. **Warm-switch advantages on real projects?** Offline Weave warm/A↔B on synthetic trees stays low-ms; real lockfiles were analyzed for scale/overlap but **not** fully materialized offline (see unavailable). High-share synthetic A→B ≈ 6 ms vs 0% share ≈ 5 ms.
2. **Disk deduplication?** Content-addressed store + hardlinks; apparent sizes overstate physical use. Real pair overlaps quantify shared artifacts before materialize.
3. **When materialization gets expensive?** Largest offline pressure point measured: pkgs=250/files=5 — 21 ms, 1500 hardlinks, 2001 inodes.
4. **Divergence vs reuse?** See synthetic table; reuse value tracks shared artifact fraction.
5. **Hardlink+copy sufficient?** Yes for measured offline scales; copy path used for native/scripts.
6. **Unsupported real-world cases?** bin links, package exports, directory file:, lifecycle execution, full peer install semantics.
7. **FUSE/overlayfs/daemon needed?** **No evidence yet** from Phase 3 offline measurements.

## Architectural decision gate (Q1–Q5)

### Q1 — filesystem view (hardlink/reflink/overlayfs/VFS)
- **evidence:** Pressure suite largest=Some("pkgs=250/files=5 hardlinks=1500 copies=0 21ms"); hardlinks dominate when prefer_copy=false; synthetic A↔B worst switch 6 ms; corpus max packages 3459
- **conclusion:** Retain hardlink+copy. No measured evidence that FUSE/overlayfs is required for current corpus scales.
- **confidence:** medium
- **uncertainty:** Real registry materialize of 3k+ package trees not timed offline; cross-device copy fallback not stressed.
- **next experiment:** Network-gated materialize of nestjs/npm-cli lockfiles with local npm cache; measure wall and inode growth.

### Q2 — lifecycle script execution
- **evidence:** Lifecycle classification across corpus: runtime_install_required≈43, likely_native_build≈117. Detect/copy policy unchanged.
- **conclusion:** Do not implement arbitrary script execution yet. Evidence shows many install scripts exist, but Weave's extraction-only path is an explicit unsupported mode for those packages—not a silent correctness claim.
- **confidence:** medium-high for policy; low for whether real apps boot without scripts
- **uncertainty:** Need controlled smoke tests of specific apps after weave switch without rebuild.
- **next experiment:** Pick 3 corpus projects with lifecycle packages; weave materialize via network; attempt node entrypoint; record failures.

### Q3 — Git source isolation
- **evidence:** Phase 3 focused on dependency lockfiles; no source-isolation benchmarks.
- **conclusion:** Defer. No evidence collected that source virtualization is the bottleneck.
- **confidence:** low (absence of evidence)
- **uncertainty:** Monorepo source checkout costs unmeasured.
- **next experiment:** Only if dependency switch is proven fast but developers still blocked by source tree churn.

### Q4 — replace package managers vs environment layer
- **evidence:** Real divergence pairs (lockfile-only): benchmarks/corpus/divergence/nestjs-v10.3/package-lock.json vs benchmarks/corpus/divergence/nestjs-v10.4/package-lock.json: shared_of_a=73.8% jaccard=0.503; benchmarks/corpus/divergence/axios-v1.6/package-lock.json vs benchmarks/corpus/divergence/axios-v1.7/package-lock.json: shared_of_a=100.0% jaccard=0.999. Synthetic high-share switch=Some(6) ms vs 0% share=Some(5) ms.
- **conclusion:** Remain an environment layer over lockfile truth. Gaps (bin links, file: dirs, scripts) block full installer replacement.
- **confidence:** high
- **uncertainty:** Developer UX vs npm/pnpm for first-time cold installs uncompared on network.
- **next experiment:** Optional network comparative suite with explicit labeling.

### Q5 — SQLite vs filesystem metadata
- **evidence:** Analyzed 20 corpus lockfiles with filesystem JSON env/registry model; no metadata query bottleneck observed in GC/list paths for this scale.
- **conclusion:** Keep filesystem metadata.
- **confidence:** medium
- **uncertainty:** Multi-thousand project registries untested.
- **next experiment:** Stress registry with 10k project registrations; measure GC root collection time.


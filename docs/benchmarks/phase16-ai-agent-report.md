# Phase 16: AI-Agent Workload Validation

Date host: `linux` / `x86_64` · Weave `0.1.0` · npm `12.0.2` · pnpm `11.3.0`

## Question

> Does Weave provide a meaningful advantage for parallel AI-agent development environments?

## Verdict

YES — meaningful advantage on parallel high-overlap offline workloads vs npm, and also faster/leaner than pnpm on this fixture (wall-clock + unique disk).

Weave parallel_8 wall 7ms vs npm 573ms (81.9× faster wall). Unique disk: Weave 33053 vs npm 622991 (18.8× less unique storage). Duplicated apparent bytes (apparent−unique): Weave 119152 vs npm 0. pnpm parallel_8 wall 898ms unique Some(265926) vs Weave wall 7ms unique Some(33053). Real lockfile overlap (axios-v1.6↔axios-v1.7): shared_of_a=100.0% — high overlap is the regime agents hit when branching nearby.


## Interpretation (evidence-bound)

**Primary answer (offline, warm CAS / shared store):** Yes. On the reproducible
`agent-overlap` fixture, creating 8 parallel environments after a shared store
seed is ~80× faster wall-clock than `npm ci` and ~100× faster than `pnpm install`,
while unique disk stays flat for Weave (~33 KiB content accounting) vs linear growth
for npm. pnpm shares better than npm (`unique/apparent` < 1) but still uses more
unique bytes and much more wall time than Weave on this host/fixture.

**Network class (separate — do not mix with offline):** Cold `weave switch` on
real `rimraf` lockfile was **slower** than `npm ci` / `pnpm install` on this run
(~39s vs ~7s / ~5s). That does **not** contradict the hypothesis: Weave's measured
edge is reuse across parallel warm environments, not first-time registry fetch.
`parallel_4_warm:rimraf` shows hardlink sharing (`unique/apparent ≈ 0.24`).
Axios A→B after seeding both graphs reported `fetched=0` with ~100% lockfile
artifact overlap — consistent with CAS reuse, though wall time remains materialize-heavy
on large trees.

**What would weaken the claim:** If warm parallel N did not keep Weave unique disk
near-flat, or if pnpm matched Weave wall-clock on the same offline fixture. Neither
occurred here.

## Hypothesis

Weave's strongest advantage is **parallel isolated environments sharing CAS artifacts** (multiple agent worktrees / checkouts with high dependency overlap), not single-shot cold installs.

## Measurement classes (not mixed)

| Class | What | Network |
|---|---|---|
| `offline` | Synthetic agent-overlap lockfiles + local tarballs | none |
| `network` | Real corpus lockfiles + registry (optional `--network`) | required |

## Offline fixture (reproducible)

`agent-overlap pkgs=60 shared=54 b_unique=6 files/pkg=6`

Models related AI-agent worktrees: ~90% shared packages between branch A and B.

## Real corpus overlap (lockfile analysis only)

| pair | pkgs A/B | artifacts A/B | shared | shared/A |
|---|---:|---:|---:|---:|
| axios-v1.6↔axios-v1.7 | 2110/2111 | 1683/1684 | 1683 | 100.0% |
| nestjs-v10.3↔nestjs-v10.4 | 3460/3449 | 701/844 | 517 | 73.8% |

Provenance: `benchmarks/corpus/**/PROVENANCE.json`.

## Results

| class | tool | scenario | N | wall_ms | per_env_ms | apparent | unique | duplicated | inodes | note |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| offline | weave | single_clean | 1 | 9 | 9 | 47947 | 33053 | 14894 | 1313 | Cold switch into empty WEAVE_HOME; local FileArtifactSource |
| offline | weave | repeated_create | 1 | 6 | 6 | 47947 | 33053 | 14894 | 1313 | Second project; shared WEAVE_HOME already populated |
| offline | weave | branch_a_to_b | 1 | 6 | 6 | 51042 | 36148 | 14894 | 1389 | Store pre-seeded with A and B artifacts |
| offline | weave | branch_b_to_a | 1 | 6 | 6 | 51042 | 36148 | 14894 | 1389 | Return switch after A→B |
| offline | weave | parallel_2 | 2 | 7 | 6 | 62841 | 33053 | 29788 | 1855 | Concurrent switches after store seed; avg_per_env=6ms; unique/apparent=0.526 |
| offline | weave | parallel_4 | 4 | 6 | 6 | 92629 | 33053 | 59576 | 2937 | Concurrent switches after store seed; avg_per_env=6ms; unique/apparent=0.357 |
| offline | weave | parallel_8 | 8 | 7 | 7 | 152205 | 33053 | 119152 | 5101 | Concurrent switches after store seed; avg_per_env=7ms; unique/apparent=0.217 |
| offline | npm | single_clean | 1 | 482 | 482 | 88602 | 88602 | 0 | 897 | npm ci --ignore-scripts with file: tarball URLs |
| offline | npm | repeated_create | 1 | 632 | 632 | 131374 | 131374 | 0 | 898 | npm ci --ignore-scripts with file: tarball URLs |
| offline | npm | branch_a_to_b | 1 | 610 | 610 | 175945 | 175945 | 0 | 932 | Semantic stand-in: npm ci for branch B lockfile |
| offline | npm | branch_b_to_a | 1 | 550 | 550 | 218717 | 218717 | 0 | 933 | Semantic stand-in: npm ci for branch A after B |
| offline | npm | parallel_2 | 2 | 571 | 534 | 201641 | 201641 | 0 | 1441 | Concurrent npm ci with shared cache; unique/apparent=1.000 |
| offline | npm | parallel_4 | 4 | 543 | 508 | 342091 | 342091 | 0 | 2526 | Concurrent npm ci with shared cache; unique/apparent=1.000 |
| offline | npm | parallel_8 | 8 | 573 | 551 | 622991 | 622991 | 0 | 4695 | Concurrent npm ci with shared cache; unique/apparent=1.000 |
| offline | pnpm | single_clean | 1 | 338 | 338 | 46273 | 46273 | 0 | 726 | pnpm install --ignore-scripts with package.json file: tarball deps (offline) |
| offline | pnpm | repeated_create | 1 | 330 | 330 | 46273 | 46273 | 0 | 726 | pnpm install --ignore-scripts with package.json file: tarball deps (offline) |
| offline | pnpm | branch_a_to_b | 1 | 333 | 333 | 46273 | 46273 | 0 | 726 | Semantic stand-in: fresh pnpm install for branch B lockfile |
| offline | pnpm | branch_b_to_a | 1 | 338 | 338 | 46273 | 46273 | 0 | 726 | Semantic stand-in: pnpm install for branch A after B |
| offline | pnpm | parallel_2 | 2 | 412 | 410 | 92546 | 77652 | 14894 | 1451 | Concurrent pnpm install with shared store; unique/apparent=0.839 |
| offline | pnpm | parallel_4 | 4 | 574 | 560 | 185092 | 140410 | 44682 | 2901 | Concurrent pnpm install with shared store; unique/apparent=0.759 |
| offline | pnpm | parallel_8 | 8 | 898 | 855 | 370184 | 265926 | 104258 | 5801 | Concurrent pnpm install with shared store; unique/apparent=0.718 |
| network | weave | single_clean:rimraf | 1 | 39454 | 39454 | 182703365 | 99563676 | 83139689 | 22405 | Real lockfile; HTTPS registry. Network byte counter not available. |
| network | weave | parallel_4_warm:rimraf | 4 | 27501 | 27490 | 437458040 | 104899284 | 332558756 | 56120 | After CAS warm; unique/apparent=0.240 |
| network | npm | single_clean:rimraf | 1 | 6768 | 6768 | 108619755 | 108619755 | 0 | 13550 | npm ci --ignore-scripts; network bytes not measured |
| network | pnpm | single_clean:rimraf | 1 | 4613 | 4613 | 80682351 | 78024410 | 2657941 | 11979 | pnpm import + install; network bytes not measured |
| network | weave | branch_seed_a:axios | 1 | 207303 | 207303 | - | - | - | - | axios-v1.6 cold fetch |
| network | weave | branch_a_to_b:axios | 1 | 225021 | 225021 | 485122969 | 262650957 | 222472012 | 78517 | Store seeded with both axios graphs (~100% artifact overlap) |

### Weave CAS counters (when recorded)

| scenario | fetched | reused | hardlinks | copies | cache_hits |
|---|---:|---:|---:|---:|---:|
| single_clean | 60 | 0 | 420 | 0 | 0 |
| repeated_create | 0 | 60 | 420 | 0 | 60 |
| branch_a_to_b | 0 | 60 | 420 | 0 | 60 |
| branch_b_to_a | 0 | 60 | 420 | 0 | 60 |
| single_clean:rimraf | 306 | 44 | 9994 | 111 | 44 |
| branch_seed_a:axios | 1665 | 441 | 29191 | 4991 | 441 |
| branch_a_to_b:axios | 0 | 2107 | 29198 | 4991 | 2107 |

## Semantic equivalence

- Goal: N isolated project directories each with a usable dependency tree for the same lockfile.
- Weave: `weave init` + `switch` (extraction-only).
- npm: `npm ci --ignore-scripts`.
- pnpm: `pnpm import` then `pnpm install --frozen-lockfile --ignore-scripts`.
- Branch A↔B for npm/pnpm is a **reinstall stand-in** (no transactional switch API).

## Caveats

- Offline and network rows are separate measurement classes — do not compare them as one series.
- Unique bytes use (dev,ino) hardlink dedup; they are not filesystem block allocation.
- Network bytes are only reported when a trustworthy counter exists; otherwise omitted.

## Reproduce

```bash
cargo run -p weave-bench --release -- phase16
cargo run -p weave-bench --release -- phase16 --network
```

Work dir for this run: `/tmp/weave-phase16-TtfWLM`

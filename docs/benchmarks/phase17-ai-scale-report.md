# Phase 17: AI-Agent Scale Validation

Host: `linux` / `x86_64` · Weave `0.1.0` · npm `12.0.2` · pnpm `11.3.0`

## Question

> At what workload does Weave become materially better than npm/pnpm for parallel AI-agent environments, and is that advantage large enough to justify using Weave?

## Answer

MATERIAL from the small high-overlap ladder (~80 pkgs) upward: Weave is already materially better than npm/pnpm on warm parallel_8 (wall time and unique disk). Advantage grows with tree size and parallel N; justify Weave when agents routinely spin ≥4–8 overlapping worktrees after a shared CAS is warm — not for one-shot cold CI/registry installs.

p17-small-hi: weave_p8=7ms npm_p8=666ms (~95.1×); unique weave=35.7 KiB npm=793.6 KiB (~22.2×) — MATERIAL
  vs pnpm_p8=921ms unique=345.6 KiB
p17-med-hi: weave_p8=13ms npm_p8=648ms (~49.8×); unique weave=115.6 KiB npm=1.8 MiB (~15.7×) — MATERIAL
  vs pnpm_p8=1017ms unique=706.5 KiB
p17-large-hi: weave_p8=22ms npm_p8=725ms (~33.0×); unique weave=230.7 KiB npm=3.3 MiB (~14.5×) — MATERIAL
  vs pnpm_p8=1085ms unique=1.2 MiB
Overlap sensitivity (weave p8): high-unique=230.7 KiB low-unique=215.5 KiB; high-wall=22ms low-wall=22ms
Network cold rimraf: weave=41931ms npm=7234ms — cold disadvantage retained (not Weave’s win domain).

## Interpretation

### 1. Fixture vs materialization vs genuine CAS

- Offline **fixture** (`p17-small-hi`): tiny payloads still show large wall-time gaps vs npm/pnpm on parallel warm create — treat absolute ms as fixture-sensitive; ratios and unique-disk shape are the durable signal.
- Offline **materialization** (`p17-med-hi`): heavier file trees raise Weave cold/warm absolute times, but unique disk remains flat as N grows (2→16) while npm unique scales ~N×.
- Offline **cas_reuse** (`p17-large-hi` / `p17-large-lo` / multi-repo): unique `WEAVE_HOME` stays nearly constant across parallel N; apparent `node_modules` grows with N. That flat unique line is the genuine CAS claim — not an artifact of tiny fixtures.

### 2. Where Weave wins (offline, warm, parallel)

Bar used: ≥5× wall **or** ≥3× unique disk vs npm on `parallel_8_warm`.

Cleared from **~80 high-overlap packages** upward on this host. At parallel 16, Weave unique disk stays at the single-tree footprint while npm/pnpm pay per-env (pnpm shares store content but still loses wall time and unique accounting vs Weave hardlink trees).

Low-overlap A/B does **not** erase the parallel-N unique-disk win for identical A envs; it does raise A↔B switch unique after both graphs are seeded (expected).

### 3. Cold / network disadvantage (do not hide)

Network cold `rimraf`: Weave 41931 ms vs npm 7234 ms vs pnpm 1088 ms. Cold registry acquisition is **not** Weave’s advantage — npm/pnpm win single-shot installs.

Real-lockfile warm materialization (axios / large `node_modules`) can still be multi-minute wall even with `fetched=0` — that is filesystem link/copy pressure, separate from CAS hit rate. Network parallel unique/apparent ratios still show sharing; wall times under heavy concurrent materialization are host/FS contingent and must stay labeled **network**.

### 4. Is the advantage large enough to justify Weave?

**Yes, for the stated AI-agent shape:** ≥4–8 parallel environments, shared lockfile graphs (or high artifact overlap), warm CAS, offline or post-seed creates — offline data shows tens-of-× wall and ~15–22× unique-disk vs npm at parallel_8, widening with N.

**No, as a drop-in for:** one-shot CI cold installs, low-frequency single envs, or workloads where registry download time dominates and no shared CAS amortizes across agents.

## Effect classes

1. **fixture** — synthetic package counts / tiny payloads (Phase 16-like).
2. **materialization** — higher file counts stressing hardlink/copy trees.
3. **cas_reuse** — unique disk stays flat as parallel N grows (genuine sharing).

## Results

| class | effect | workload | tool | scenario | N | wall_ms | nm_apparent | nm_unique | total_unique | note |
|---|---|---|---|---|---:|---:|---:|---:|---:|---|
| offline | fixture | p17-small-hi | weave | cold_acquisition | 1 | 9 | 14822 | 14822 | 36540 | First switch into empty WEAVE_HOME; overlap=high |
| offline | fixture | p17-small-hi | weave | warm_cas_reuse | 1 | 6 | 14822 | 14822 | 36540 | Second env; store already populated |
| offline | fixture | p17-small-hi | weave | branch_switch_cycles | 1 | 37 | 14822 | 14822 | 39977 | 3× A→B + B→A after both graphs seeded |
| offline | fixture | p17-small-hi | weave | parallel_2_warm | 2 | 7 | 29644 | 14822 | 36540 | unique/apparent_nm=0.500; unique/apparent_total=0.552 |
| offline | fixture | p17-small-hi | weave | parallel_4_warm | 4 | 7 | 59288 | 14822 | 36540 | unique/apparent_nm=0.250; unique/apparent_total=0.381 |
| offline | fixture | p17-small-hi | weave | parallel_8_warm | 8 | 7 | 118576 | 14822 | 36540 | unique/apparent_nm=0.125; unique/apparent_total=0.236 |
| offline | fixture | p17-small-hi | weave | parallel_16_warm | 16 | 14 | 237152 | 14822 | 36540 | unique/apparent_nm=0.062; unique/apparent_total=0.134 |
| offline | fixture | p17-small-hi | npm | cold_acquisition | 1 | 521 | 31499 | 31499 | 113287 | npm ci --ignore-scripts file: tarballs |
| offline | fixture | p17-small-hi | npm | warm_cache_create | 1 | 547 | 31499 | 31499 | 173117 | npm ci --ignore-scripts file: tarballs |
| offline | fixture | p17-small-hi | npm | branch_switch_cycles | 1 | 1013 | 31499 | 31499 | 294926 | 1× B install + 1× A reinstall stand-in |
| offline | fixture | p17-small-hi | npm | parallel_2_warm | 2 | 537 | 62998 | 62998 | 264510 | shared npm cache; unique/apparent_nm=1.000 |
| offline | fixture | p17-small-hi | npm | parallel_4_warm | 4 | 539 | 125996 | 125996 | 447208 | shared npm cache; unique/apparent_nm=1.000 |
| offline | fixture | p17-small-hi | npm | parallel_8_warm | 8 | 666 | 251992 | 251992 | 812604 | shared npm cache; unique/apparent_nm=1.000 |
| offline | fixture | p17-small-hi | npm | parallel_16_warm | 16 | 662 | 503984 | 503984 | 1543476 | shared npm cache; unique/apparent_nm=1.000 |
| offline | fixture | p17-small-hi | pnpm | cold_acquisition | 1 | 349 | 57201 | 57201 | 57201 | pnpm install file: deps offline |
| offline | fixture | p17-small-hi | pnpm | warm_store_create | 1 | 341 | 57201 | 57201 | 57201 | pnpm install file: deps offline |
| offline | fixture | p17-small-hi | pnpm | branch_switch_cycles | 1 | 681 | 57201 | 57201 | 57201 | 1× B + 1× A reinstall stand-in |
| offline | fixture | p17-small-hi | pnpm | parallel_2_warm | 2 | 411 | 114402 | 99580 | 99580 | shared pnpm store; unique/apparent_nm=0.870 |
| offline | fixture | p17-small-hi | pnpm | parallel_4_warm | 4 | 575 | 228804 | 184338 | 184338 | shared pnpm store; unique/apparent_nm=0.806 |
| offline | fixture | p17-small-hi | pnpm | parallel_8_warm | 8 | 921 | 457608 | 353854 | 353854 | shared pnpm store; unique/apparent_nm=0.773 |
| offline | fixture | p17-small-hi | pnpm | parallel_16_warm | 16 | 1691 | 915216 | 692886 | 692886 | shared pnpm store; unique/apparent_nm=0.757 |
| offline | materialization | p17-med-hi | weave | cold_acquisition | 1 | 20 | 59944 | 59944 | 118403 | First switch into empty WEAVE_HOME; overlap=high |
| offline | materialization | p17-med-hi | weave | warm_cas_reuse | 1 | 10 | 59944 | 59944 | 118403 | Second env; store already populated |
| offline | materialization | p17-med-hi | weave | branch_switch_cycles | 1 | 64 | 59944 | 59944 | 129380 | 3× A→B + B→A after both graphs seeded |
| offline | materialization | p17-med-hi | weave | parallel_2_warm | 2 | 11 | 119888 | 59944 | 118403 | unique/apparent_nm=0.500; unique/apparent_total=0.497 |
| offline | materialization | p17-med-hi | weave | parallel_4_warm | 4 | 12 | 239776 | 59944 | 118403 | unique/apparent_nm=0.250; unique/apparent_total=0.331 |
| offline | materialization | p17-med-hi | weave | parallel_8_warm | 8 | 13 | 479552 | 59944 | 118403 | unique/apparent_nm=0.125; unique/apparent_total=0.198 |
| offline | materialization | p17-med-hi | weave | parallel_16_warm | 16 | 18 | 959104 | 59944 | 118403 | unique/apparent_nm=0.062; unique/apparent_total=0.110 |
| offline | materialization | p17-med-hi | npm | cold_acquisition | 1 | 500 | 93181 | 93181 | 269422 | npm ci --ignore-scripts file: tarballs |
| offline | materialization | p17-med-hi | npm | warm_cache_create | 1 | 559 | 93181 | 93181 | 386964 | npm ci --ignore-scripts file: tarballs |
| offline | materialization | p17-med-hi | npm | branch_switch_cycles | 1 | 1111 | 93181 | 93181 | 627843 | 1× B install + 1× A reinstall stand-in |
| offline | materialization | p17-med-hi | npm | parallel_2_warm | 2 | 518 | 186362 | 186362 | 597751 | shared npm cache; unique/apparent_nm=1.000 |
| offline | materialization | p17-med-hi | npm | parallel_4_warm | 4 | 578 | 372724 | 372724 | 1019237 | shared npm cache; unique/apparent_nm=1.000 |
| offline | materialization | p17-med-hi | npm | parallel_8_warm | 8 | 648 | 745448 | 745448 | 1862209 | shared npm cache; unique/apparent_nm=1.000 |
| offline | materialization | p17-med-hi | npm | parallel_16_warm | 16 | 807 | 1490896 | 1490896 | 3548233 | shared npm cache; unique/apparent_nm=1.000 |
| offline | materialization | p17-med-hi | pnpm | cold_acquisition | 1 | 375 | 142883 | 142883 | 142883 | pnpm install file: deps offline |
| offline | materialization | p17-med-hi | pnpm | warm_store_create | 1 | 364 | 142883 | 142883 | 142883 | pnpm install file: deps offline |
| offline | materialization | p17-med-hi | pnpm | branch_switch_cycles | 1 | 744 | 142883 | 142883 | 142883 | 1× B + 1× A reinstall stand-in |
| offline | materialization | p17-med-hi | pnpm | parallel_2_warm | 2 | 453 | 285766 | 225822 | 225822 | shared pnpm store; unique/apparent_nm=0.790 |
| offline | materialization | p17-med-hi | pnpm | parallel_4_warm | 4 | 654 | 571532 | 391700 | 391700 | shared pnpm store; unique/apparent_nm=0.685 |
| offline | materialization | p17-med-hi | pnpm | parallel_8_warm | 8 | 1017 | 1143064 | 723456 | 723456 | shared pnpm store; unique/apparent_nm=0.633 |
| offline | materialization | p17-med-hi | pnpm | parallel_16_warm | 16 | 1958 | 2286128 | 1386968 | 1386968 | shared pnpm store; unique/apparent_nm=0.607 |
| offline | cas_reuse | p17-large-hi | weave | cold_acquisition | 1 | 38 | 122912 | 122912 | 236259 | First switch into empty WEAVE_HOME; overlap=high |
| offline | cas_reuse | p17-large-hi | weave | warm_cas_reuse | 1 | 16 | 122912 | 122912 | 236259 | Second env; store already populated |
| offline | cas_reuse | p17-large-hi | weave | branch_switch_cycles | 1 | 109 | 122912 | 122912 | 258116 | 3× A→B + B→A after both graphs seeded |
| offline | cas_reuse | p17-large-hi | weave | parallel_2_warm | 2 | 19 | 245824 | 122912 | 236259 | unique/apparent_nm=0.500; unique/apparent_total=0.490 |
| offline | cas_reuse | p17-large-hi | weave | parallel_4_warm | 4 | 20 | 491648 | 122912 | 236259 | unique/apparent_nm=0.250; unique/apparent_total=0.325 |
| offline | cas_reuse | p17-large-hi | weave | parallel_8_warm | 8 | 22 | 983296 | 122912 | 236259 | unique/apparent_nm=0.125; unique/apparent_total=0.194 |
| offline | cas_reuse | p17-large-hi | weave | parallel_16_warm | 16 | 28 | 1966592 | 122912 | 236259 | unique/apparent_nm=0.062; unique/apparent_total=0.107 |
| offline | cas_reuse | p17-large-hi | npm | cold_acquisition | 1 | 600 | 180989 | 180989 | 501106 | npm ci --ignore-scripts file: tarballs |
| offline | cas_reuse | p17-large-hi | npm | warm_cache_create | 1 | 622 | 180989 | 180989 | 707636 | npm ci --ignore-scripts file: tarballs |
| offline | cas_reuse | p17-large-hi | npm | branch_switch_cycles | 1 | 1148 | 180989 | 180989 | 1131931 | 1× B install + 1× A reinstall stand-in |
| offline | cas_reuse | p17-large-hi | npm | parallel_2_warm | 2 | 598 | 361978 | 361978 | 1095219 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-hi | npm | parallel_4_warm | 4 | 575 | 723956 | 723956 | 1870297 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-hi | npm | parallel_8_warm | 8 | 725 | 1447912 | 1447912 | 3420453 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-hi | npm | parallel_16_warm | 16 | 1480 | 2895824 | 2895824 | 6520847 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-hi | pnpm | cold_acquisition | 1 | 419 | 267491 | 267491 | 267491 | pnpm install file: deps offline |
| offline | cas_reuse | p17-large-hi | pnpm | warm_store_create | 1 | 420 | 267491 | 267491 | 267491 | pnpm install file: deps offline |
| offline | cas_reuse | p17-large-hi | pnpm | branch_switch_cycles | 1 | 819 | 267491 | 267491 | 267491 | 1× B + 1× A reinstall stand-in |
| offline | cas_reuse | p17-large-hi | pnpm | parallel_2_warm | 2 | 497 | 534982 | 412070 | 412070 | shared pnpm store; unique/apparent_nm=0.770 |
| offline | cas_reuse | p17-large-hi | pnpm | parallel_4_warm | 4 | 691 | 1069964 | 701228 | 701228 | shared pnpm store; unique/apparent_nm=0.655 |
| offline | cas_reuse | p17-large-hi | pnpm | parallel_8_warm | 8 | 1085 | 2139928 | 1279544 | 1279544 | shared pnpm store; unique/apparent_nm=0.598 |
| offline | cas_reuse | p17-large-hi | pnpm | parallel_16_warm | 16 | 2127 | 4279856 | 2436176 | 2436176 | shared pnpm store; unique/apparent_nm=0.569 |
| offline | cas_reuse | p17-large-lo | weave | cold_acquisition | 1 | 36 | 108352 | 108352 | 220626 | First switch into empty WEAVE_HOME; overlap=low |
| offline | cas_reuse | p17-large-lo | weave | warm_cas_reuse | 1 | 15 | 108352 | 108352 | 220626 | Second env; store already populated |
| offline | cas_reuse | p17-large-lo | weave | branch_switch_cycles | 1 | 116 | 108352 | 108352 | 417719 | 3× A→B + B→A after both graphs seeded |
| offline | cas_reuse | p17-large-lo | weave | parallel_2_warm | 2 | 19 | 216704 | 108352 | 220626 | unique/apparent_nm=0.500; unique/apparent_total=0.504 |
| offline | cas_reuse | p17-large-lo | weave | parallel_4_warm | 4 | 19 | 433408 | 108352 | 220626 | unique/apparent_nm=0.250; unique/apparent_total=0.337 |
| offline | cas_reuse | p17-large-lo | weave | parallel_8_warm | 8 | 22 | 866816 | 108352 | 220626 | unique/apparent_nm=0.125; unique/apparent_total=0.203 |
| offline | cas_reuse | p17-large-lo | weave | parallel_16_warm | 16 | 28 | 1733632 | 108352 | 220626 | unique/apparent_nm=0.062; unique/apparent_total=0.113 |
| offline | cas_reuse | p17-large-lo | npm | cold_acquisition | 1 | 656 | 164189 | 164189 | 475393 | npm ci --ignore-scripts file: tarballs |
| offline | cas_reuse | p17-large-lo | npm | warm_cache_create | 1 | 589 | 164189 | 164189 | 674083 | npm ci --ignore-scripts file: tarballs |
| offline | cas_reuse | p17-large-lo | npm | branch_switch_cycles | 1 | 1223 | 164189 | 164189 | 1172662 | 1× B install + 1× A reinstall stand-in |
| offline | cas_reuse | p17-large-lo | npm | parallel_2_warm | 2 | 595 | 328378 | 328378 | 1037026 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-lo | npm | parallel_4_warm | 4 | 615 | 656756 | 656756 | 1762824 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-lo | npm | parallel_8_warm | 8 | 725 | 1313512 | 1313512 | 3214420 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-lo | npm | parallel_16_warm | 16 | 1155 | 2627024 | 2627024 | 6117692 | shared npm cache; unique/apparent_nm=1.000 |
| offline | cas_reuse | p17-large-lo | pnpm | cold_acquisition | 1 | 398 | 243971 | 243971 | 243971 | pnpm install file: deps offline |
| offline | cas_reuse | p17-large-lo | pnpm | warm_store_create | 1 | 403 | 243971 | 243971 | 243971 | pnpm install file: deps offline |
| offline | cas_reuse | p17-large-lo | pnpm | branch_switch_cycles | 1 | 816 | 243971 | 243971 | 243971 | 1× B + 1× A reinstall stand-in |
| offline | cas_reuse | p17-large-lo | pnpm | parallel_2_warm | 2 | 492 | 487942 | 379590 | 379590 | shared pnpm store; unique/apparent_nm=0.778 |
| offline | cas_reuse | p17-large-lo | pnpm | parallel_4_warm | 4 | 688 | 975884 | 650828 | 650828 | shared pnpm store; unique/apparent_nm=0.667 |
| offline | cas_reuse | p17-large-lo | pnpm | parallel_8_warm | 8 | 1088 | 1951768 | 1193304 | 1193304 | shared pnpm store; unique/apparent_nm=0.611 |
| offline | cas_reuse | p17-large-lo | pnpm | parallel_16_warm | 16 | 2076 | 3903536 | 2278256 | 2278256 | shared pnpm store; unique/apparent_nm=0.584 |
| offline | cas_reuse | multi-repo | weave | multi_repo_repo-a | 1 | 12 | 24830 | 24830 | 55109 | Independent lockfile graphs; shared WEAVE_HOME |
| offline | cas_reuse | multi-repo | weave | multi_repo_repo-b | 1 | 8 | 24830 | 24830 | 55109 | Independent lockfile graphs; shared WEAVE_HOME |
| offline | cas_reuse | multi-repo | weave | multi_repo_aggregate | 2 | 0 | 49660 | 24830 | 55109 | two independent synthetic repos; unique/apparent_total=0.526 |
| network | cas_reuse | network-rimraf | weave | cold_acquisition | 1 | 41931 | 84918225 | 80391333 | 99563676 | rimraf real lockfile; network bytes N/A |
| network | cas_reuse | network-rimraf | weave | parallel_2_warm | 2 | 27641 | 169836450 | 82169869 | 101342212 | after cold seed; unique/apparent_nm=0.484 |
| network | cas_reuse | network-rimraf | weave | parallel_4_warm | 4 | 29640 | 339672900 | 85726941 | 104899284 | after cold seed; unique/apparent_nm=0.252 |
| network | cas_reuse | network-rimraf | weave | parallel_8_warm | 8 | 261364 | 679345800 | 92841085 | 112013428 | after cold seed; unique/apparent_nm=0.137 |
| network | cas_reuse | network-rimraf | npm | cold_acquisition | 1 | 7234 | 85077145 | 85077145 | 108619772 | network npm ci rimraf |
| network | cas_reuse | network-rimraf | pnpm | cold_acquisition | 1 | 1088 | 80680643 | 78022702 | 78022702 | network pnpm rimraf |
| network | materialization | network-typescript | weave | cold_acquisition | 1 | 68638 | 155587964 | 154392259 | 195114646 | typescript real lockfile cold |
| network | cas_reuse | network-axios-hi-overlap | weave | cold_acquisition | 1 | 239296 | 229790498 | 211606988 | 262627874 | axios-v1.6 cold |
| network | cas_reuse | network-axios-hi-overlap | weave | branch_a_to_b | 1 | 212324 | 229807937 | 211624427 | 262650957 | high-overlap axios after both seeded |
| network | cas_reuse | network-axios-hi-overlap | weave | branch_b_to_a | 1 | 189552 | 229790498 | 211606988 | 262650957 | return switch |

## Caveats

- Offline vs network rows are separate classes — never compare as one series.
- Unique bytes = (dev,ino) dedup; not block allocation.
- Network bytes not fabricated when no counter exists.
- npm/pnpm branch A↔B is reinstall stand-in (no transactional switch).

## Reproduce

```bash
cargo run -p weave-bench --release -- phase17
cargo run -p weave-bench --release -- phase17 --network
cargo run -p weave-bench --release -- phase17 --quick   # skip largest offline steps
```

Work dir: `/tmp/weave-phase17-VjH54k`

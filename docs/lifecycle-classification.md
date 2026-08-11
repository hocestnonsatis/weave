# Lifecycle classification report (Phase 4)

Weave still does **not** execute lifecycle scripts (ADR-0012). This report
classifies corpus packages so gaps are explicit.

## Classes

| Class | Meaning | Weave behavior |
|-------|---------|----------------|
| extraction-only | Tarball contents are sufficient | Materialize + activate |
| generated-files required | Install script writes files needed at runtime | Unsupported without execution |
| native-build required | Needs compile/`node-gyp` | Unsupported without toolchain+exec |
| runtime-install required | Postinstall mutates install for runtime | Unsupported without execution |
| unsupported/unsafe | Arbitrary/privileged scripts | Remains unsupported |

## Corpus evidence (from Phase 3 classify; not re-executed)

Approximate totals across the pinned corpus:

- extraction-only: ~30 752 package nodes (vast majority)
- generated-file hints: ~14
- native-build hints: ~117
- runtime-install hints: ~43

Classification is heuristic (install-script flags + native markers). It does
**not** prove a package fails without scripts — only that scripts are present.

## Phase 4 application smoke (offline)

Offline smoke tests (`crates/weave-engine/tests/smoke.rs`) succeed for:

- bin links
- package exports / subpath requires
- nested dependency resolution
- workspace links
- `file:` immutable snapshots

**without** running any lifecycle scripts.

## Phase 6 note

ADR-0018 accepts the controlled execution design. Default behavior is unchanged
(no script execution). The Phase 6 experiment is a **non-executing** plan
classifier (`weave_engine::plan_execution`) surfaced in `weave doctor` as
`exec-plan`.



## Reproducing classification

```bash
cargo run -p weave-bench -- analyze-corpus
cargo run -p weave-bench -- phase3 --out benchmarks/out/phase3
```

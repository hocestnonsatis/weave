# Phase 4 report: Production Compatibility

Date: 2026-08-11  
Architecture retained: content-addressed store + hardlink/copy + transactional activation  
**No FUSE / overlayfs / daemon introduced.**

## Summary

Weave can correctly materialize ordinary npm application trees for the
filesystem semantics closed in this phase: bin links, exports topology,
`file:` snapshots, and npm workspace links — verified by offline Node smoke
tests and a network-gated registry materialize of the `small/rimraf` corpus.

## Questions

### Can Weave correctly materialize ordinary npm applications?

**Yes, for extraction-only apps** covered by the smoke suite and the rimraf
registry path. Lifecycle-generated state remains unsupported (by design).

Evidence:

- Offline: `crates/weave-engine/tests/smoke.rs` (bins, exports, nested, file, workspaces)
- Network (measured): `small/rimraf` → packages=351, bins=48, `require('glob')` ok
  (`WEAVE_NETWORK_TESTS=1 cargo test -p weave-engine --test network rimraf -- --ignored`)

### Which npm filesystem semantics are fully supported?

| Area | Status |
|------|--------|
| Registry package layout | supported |
| Nested `node_modules` versions | supported |
| Linux `.bin` relative symlinks | supported (ADR-0016) |
| `exports` / `imports` filesystem topology | supported (Node resolves) |
| `file:` directory deps | supported as immutable snapshots (ADR-0014) |
| npm workspace `link: true` | supported (ADR-0015) |
| Path traversal / malicious tar | supported (adversarial extract tests) |
| Cross-filesystem hardlink fallback | supported (`same_filesystem` + copy) |

See `docs/compatibility.md`.

### Which remain partial?

- Peer / optional: parsed; no install-time peer rewrite or optional OS skip
- Native addons: copy + platform identity; no rebuild
- Platform cpu/os fields: recorded, not used to skip acquire
- Windows bin shims: unsupported

### What breaks without lifecycle execution?

Packages that need install-time generated files, native compilation, or
runtime install mutation. Classification: `docs/lifecycle-classification.md`.
Offline smokes and rimraf registry smoke did **not** require script execution.

### Are bin links correct?

**Yes on Linux.** Relative symlinks under nearest `node_modules/.bin`; scoped,
multi-bin, nested, and conflict cases covered by fixtures + smoke. Documented
in ADR-0016.

### Are workspace links correct?

**Yes for npm `link: true` wiring.** Relative symlinks to workspace paths after
activation; smoke verifies `@acme/a` → `@acme/b` resolution.

### Are file dependencies reproducible?

**Yes under the snapshot model.** Acquire packs the directory into CAS; later
mutations of the vendor tree do not change the activated package (smoke proof).

### Does hardlink/copy remain correct?

**Yes.** Cross-device detection + copy fallback tested; prefer_copy for
native/install-script packages unchanged. No evidence requiring FUSE/overlayfs.

### Reasons to revisit FUSE/overlayfs/daemon?

**None from Phase 4 correctness work.** Remaining scale questions are
performance/ops (large NestJS network materialize optional via
`WEAVE_NETWORK_LARGE=1`), not semantic blockers.

## Materialization version

Bumped to **`3`** (bin links + workspace wiring participate in environment
identity).

## Remaining gaps

1. Windows `.bin` shims
2. Peer auto-install / optional platform filtering
3. Lifecycle execution (only if a real app proves necessity — new ADR first)
4. NestJS-scale network smoke not run in this session (optional large flag)

## Commands

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
WEAVE_NETWORK_TESTS=1 cargo test -p weave-engine --test network rimraf -- --ignored
```

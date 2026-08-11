# Phase 5 report: Ecosystem Hardening

Date: 2026-08-11  
Architecture: CAS + hardlink/copy + transactional activation (**retained**)  
**No FUSE / overlayfs / daemon / new package resolver.**

## Newly supported semantics

1. **Peer dependencies (presence semantics)** — Required peers must be
   Node-resolvable in the lockfile; `weave switch` fails otherwise. Optional
   peers (`peerDependenciesMeta.optional`) may be absent. ADR-0017.
2. **Optional OS/CPU filtering** — Optional packages that fail npm `os`/`cpu`
   matching against the host are skipped at acquire and materialize
   (e.g. `fsevents` on Linux).
3. **Required platform reject** — Non-optional packages incompatible with the
   host error out instead of silently installing wrong binaries.
4. **Platform identity** — Host os/arch (npm token mapping) drives filtering;
   already participates in environment ids; `materialization_version` → **4**.
5. **Native rebuild clarity** — `docs/native.md` + doctor `native-rebuild`
   when `.node` binaries are missing; still no automatic rebuild.

## Remaining unsupported / partial

- Peer **semver range** solving (presence only; ranges recorded)
- Windows `.bin` shims
- Lifecycle / native **execution** (classify-only)
- Peer auto-install (intentionally never — Weave is not a resolver)

## Real-world evidence

| Evidence | Result |
|----------|--------|
| Offline smoke: peers + fsevents skip | `smoke_peers_and_optional_platform_filter` |
| Offline smoke: missing required peer rejected | `smoke_rejects_missing_required_peer` |
| Fixtures: peer-missing, peer-optional-missing, optional-platform | parse + audit + platform_fit tests |
| Prior Phase 4 network `small/rimraf` | still valid for registry path |
| Lifecycle | No new evidence requiring script execution |

## Does the architecture still hold?

**Yes.** Peer/optional/platform correctness are graph + filter decisions on top
of CAS materialization. No filesystem virtualization required.

## Single most important next engineering problem

**Controlled native/lifecycle execution for packages that ship without
prebuilt binaries** — specifically a sandboxed, opt-in rebuild/install model
with explicit provenance, so Weave can support real native apps without
becoming an unbounded script runner. Design an ADR before implementing.

## Gates

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

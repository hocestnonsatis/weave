# Weave compatibility matrix

Status vocabulary:

| Status | Meaning |
|--------|---------|
| supported | Covered by tests/fixtures with explicit evidence |
| partially supported | Works for common cases; known gaps documented |
| experimental | Implemented but lightly validated |
| unsupported | Explicitly out of scope or failing by design |
| not yet tested | No claim |

| Feature | Status | Evidence | Notes |
|---------|--------|----------|-------|
| package dependencies | supported | smoke tests; lockfile fixtures | Registry + CAS materialize |
| nested dependencies | supported | smoke nested-lib; `nested-versions` | Path-key layout preserved |
| peer dependencies | supported | ADR-0017; `peer-deps` / `peer-missing` / `peer-optional-missing`; smoke | Presence audit + fail on missing required; no auto-install; no semver solver |
| optional dependencies | supported | ADR-0017; `optional-platform`; smoke fsevents skip | OS/CPU filter skips incompatible optional pkgs |
| bin links | supported | ADR-0016; smoke bin | Linux relative symlinks; Windows shims unsupported |
| exports / imports | supported | smoke exports-pkg | Filesystem topology; Node owns resolution |
| file: dependencies | supported | ADR-0014; smoke file snapshot | Immutable directory snapshot at acquire |
| npm workspaces | supported | ADR-0015; smoke workspace links | Relative symlinks for `link: true` |
| native addons | partially supported | `docs/native.md`; doctor `native-rebuild` | Copy + platform identity; no rebuild |
| lifecycle scripts | unsupported (default) | ADR-0012; ADR-0018 | Plain `switch` never runs scripts |
| exec plan (dry-run) | supported | `weave exec plan` | Classification + allowlist filter |
| sandboxed exec (offline) | experimental | Phase 7–8; `WEAVE_EXEC_TESTS` | bwrap + CAS seal; `--with-exec` + config |
| sealed output activation | experimental | Phase 8; `exec_integrate` | Candidate-only apply; dual gate |
| exec policy discovery | experimental | Phase 9; `exec_discover` / `exec_policy` | Candidates ≠ allowed; `exec suggest` |
| allowlisted prebuild fetch | experimental | Phase 10; `prebuild_fetch` | HTTPS + host allowlist + SRI; offline default |
| native prebuild resolution | experimental | Phase 11; `prebuild_resolve` | Static detect + plan diagnostics; suggest never auto-approves |
| adoption diagnostics | experimental | Phase 12; `adoption` | ExtractionReady / Partial / Blocked + next actions |
| hash-artifact / policy packs | experimental | Phase 13 | Offline SRI helper + reviewed packs; never auto-enable |
| platform-specific packages | supported | `HostPlatform` + `platform_fit`; env identity | Optional skip / required reject; npm os/cpu tokens |
| path traversal hardening | supported | extract adversarial tests | Tar `..`, absolute/escaping symlinks rejected |
| cross-filesystem hardlinks | supported | link cross-fs test | Detect + copy fallback |

Lifecycle remains classify-only unless a concrete project proves execution is required.

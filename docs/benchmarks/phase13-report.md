# Phase 13 report: Productization & Release Hardening

Date: 2026-08-11  
Security model unchanged: dual gate, no open lifecycle networking, no invented SRI,
no FUSE/overlayfs/daemon/new resolver.

## What landed

1. **`weave exec hash-artifact`** — offline SRI for a regular file the human already
   verified; refuses symlinks; emits reviewable TOML; never enables execution.
2. **Policy packs** (`policy-packs/`, `apply-pack`) — versioned reviewed drafts;
   merge never flips `enabled` or `profile=open`.
3. **Config compatibility** — `load_compat`: migrate missing/older v1 fields with
   warnings; reject future versions fail-closed.
4. **CLI UX** — clearer help, init next steps, richer recovery hints for first-time
   failures (peers, dual gate, corrupt store, symlinks).
5. **Release tests** (`tests/release.rs`) — fresh-clone reproducibility, leftover
   candidate diagnosis, corrupt `profile=open` rejected, `--with-exec` without
   enable leaves tree untouched, live `node_modules` exec input refused,
   hash↔pack integrity match.

## Release readiness

### Production-ready (0.x usable)

- Git + npm lockfile discovery, CAS acquire, materialize, transactional activate
- Extraction-only `init → switch → run` without execution config
- Doctor / adoption verdicts and actionable next steps
- Offline dual-gated sandboxed execution for allowlisted packages with declared outputs
- Allowlisted HTTPS prebuild fetch under `prebuild-fetch` (explicit hosts + SRI)
- GC, status, env create/list
- Fail-closed integrity, peer, platform, and unsafe-script handling

### Experimental

- Native prebuild static resolution + suggest drafts (Phase 11)
- Policy packs / hash-artifact workflow (Phase 13)
- Bubblewrap exec path (Linux; requires `bwrap`)
- Network-gated corpus smoke (`WEAVE_NETWORK_TESTS`)

### Intentionally unsupported

- Open lifecycle networking / `profile=open`
- Invented SRI, URLs, outputs, or permissions
- FUSE, overlayfs, daemon, alternate package resolvers
- Automatic enablement via environment variables
- Windows exec sandbox
- Non-npm lockfiles (yarn/pnpm) as first-class inputs
- Silent rebuild of native addons

### Remaining release blockers (for a cautious 0.x)

1. **Native completeness still needs human SRI** for most real packages — documented
   boundary, not a silent fix.
2. **Linux + bwrap required** for opt-in exec; document clearly in release notes.
3. **npm lockfile only** — call out in compatibility matrix.
4. **No signed release/CI attestation story yet** (process, not architecture).

### Is the architecture ready for a 0.x public release?

**Yes, as an explicit 0.x with documented boundaries.** The CAS + dual-gate +
fail-closed policy model is stable enough to ship for extraction-first workflows
and opt-in execution. Do not market native/lifecycle completeness as automatic.

## Footgun review (security)

| Area | Status |
|------|--------|
| Execution enablement | Config only; env ignored |
| Live node_modules exec | Refused |
| Symlink hash-artifact | Refused |
| HTTP / open profile | Rejected |
| Policy pack / suggest write | Never enables |
| Activation failure | Prior env preserved |
| Path traversal outputs | Rejected at seal |
| Redirect hosts (prebuild) | Re-checked per hop |

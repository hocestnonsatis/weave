# Native packages in Weave

## Policy

Weave **does not rebuild** native addons (`node-gyp`, N-API, etc.).

Supported behavior:

1. Detect likely-native packages (lockfile heuristics + `binding.gyp` / markers).
2. Prefer **copy** materialization (`prefer_copy`) so shared CAS/unpacked cache
   is not mutated.
3. Include host `os` / `arch` in environment identity.
4. Materialize prebuilt `.node` binaries **when present in the tarball**.
5. Surface rebuild needs via `weave doctor` (`native-rebuild` check).

## When a rebuild is required

If a native package:

- declares install scripts, and
- has no usable `.node` binary after materialization,

the environment is **not** production-complete for that package. Rebuild outside
Weave (for example `npm rebuild <pkg>` in a disposable tree) and treat the
result as lifecycle-generated state — not something Weave claims to produce.

## Optional platform natives

Optional packages with `os` / `cpu` constraints that reject the host are
**skipped** during acquire/materialize (Phase 5). Darwin-only packages such as
`fsevents` do not appear under `node_modules` on Linux.

## Lifecycle boundary

Native rebuilds are a form of lifecycle execution. Default Weave behavior remains
**classify-only** (ADR-0012).

ADR-0018 designs an opt-in sandboxed rebuild/install path. Phase 7–8 implement
offline Bubblewrap + CAS seal + candidate activation under the dual gate
(`execution.enabled` + `--with-exec`). Phase 9 discovers candidate scripts/outputs
from metadata without executing. Phase 10 adds **allowlisted HTTPS prebuild fetch**
(`execution.profile = "prebuild-fetch"`) with explicit hosts/URLs/SRI — never on
plain `switch`, never with an open network profile.

Phase 11 adds **native prebuild resolution**: static detection of real-package
download patterns (node-pre-gyp, prebuild-install, HTTPS literals, author SRI),
`weave exec plan` diagnostics for unresolved artifacts, and reviewable
`prebuild.fetches` drafts when URL+output+integrity are known. Scripts still
receive no network; discovered URLs are never auto-approved.

Dry-run: `weave exec plan` and doctor `exec-plan` never run scripts or contact
the network.


# Phase 10 report: Allowlisted Prebuild Fetch

Date: 2026-08-11  
Keeps ADR-0018 dual gate, Bubblewrap offline default, CAS seal, and
transactional activation unchanged.

## What landed

1. **`execution.profile = "prebuild-fetch"`** — offline remains default; `open` rejected.
2. **`[execution.prebuild]`** — exact `allow_hosts` + explicit `fetches[]`
   (`package`, HTTPS `url`, required `integrity`, `output`, optional OS/CPU/ABI).
3. **HTTPS allowlisted fetch** — redirects re-validated per hop; denied hosts fail closed.
4. **Integrity verification** before CAS put; provenance records URL, SRI, package,
   OS/CPU/Node ABI, artifact id, cache key.
5. **Candidate-only writes** — never live `node_modules`; only declared outputs sealed.
6. **Prebuild + exec caches** — verified blobs reused; no unnecessary refetch.
7. **`weave exec plan`** — surfaces `needs_network`, host allow status, ABI/platform
   match, and offline dry-run reasons **without contacting the network**.

## Fixtures / tests

| Case | Coverage |
|------|----------|
| allowed fetch | `allowed_fetch_verifies_integrity`, `cache_hit_skips_second_fetch` |
| denied host | `denies_unallowlisted_host`, config validate |
| redirect → denied | `redirect_to_denied_host_fails` |
| integrity mismatch | `integrity_mismatch_fails` |
| ABI mismatch | `abi_mismatch_refuses_fetch` |
| cache hit | `cache_hit_skips_second_fetch` |
| offline mode | `offline_profile_refuses_network`, `dry_run_plan_needs_network_flag` |
| HTTP rejected | `denies_http_scheme`, `rejects_http_url` |

## What this proves

Opt-in native prebuilds can be fetched from **explicitly allowlisted HTTPS hosts**,
integrity-checked, CAS-sealed, and applied onto the candidate under the dual gate —
without opening general network access, without arbitrary URL discovery, and without
changing plain `weave switch`.

## Next single bottleneck

**Lifecycle scripts that themselves perform networked installs** remain out of scope;
real packages that embed download logic inside `postinstall` still need either an
explicit Weave `prebuild.fetches` entry or an offline-capable rebuild path — Weave
still will not grant scripts an open network namespace.

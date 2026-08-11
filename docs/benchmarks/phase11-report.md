# Phase 11 report: Native Prebuild Resolution

Date: 2026-08-11  
Keeps ADR-0018 dual gate, allowlisted HTTPS fetch (Phase 10), CAS seal, and
transactional activation unchanged. No open networking for lifecycle scripts.

## What landed

1. **Static native prebuild detection** (`prebuild_resolve`) — node-pre-gyp
   `binary`, prebuild-install layouts, HTTPS literals in install entrypoints,
   author `weave.prebuildFetches`, and known heuristics (esbuild/sharp).
2. **Statuses** — Configured / Suggestable / NeedsIntegrity / UnresolvedTokens /
   Opaque / BlockedUnsafe — with explicit reasons for manual policy.
3. **`weave exec plan`** — surfaces required native artifacts and why they cannot
   currently be resolved (`native_prebuilds`, `native_policy_gap_count`).
4. **Reviewable suggestions** — `weave exec suggest` emits TOML drafts only when
   HTTPS URL + output + SRI are statically established; never auto-approves;
   never flips `enabled` or `profile = open`.
5. **Security invariants preserved** — HTTPS + allow_hosts + SRI + ABI/OS/CPU;
   scripts still receive no general network access.

## Fixtures / tests

| Case | Coverage |
|------|----------|
| node-pre-gyp NeedsIntegrity | `resolves_node_pre_gyp_needs_integrity` |
| author SRI Suggestable | `weave_author_prebuild_is_suggestable` |
| never suggest without SRI | `never_auto_approves_without_integrity` |
| prebuild-install Opaque | `opaque_prebuild_install_explains_gap` |
| esbuild HTTPS archive | `https_literal_without_output_is_opaque_or_needs_integrity` |
| sharp Opaque | `sharp_like_is_opaque_manual_policy` |
| HTTP blocked | `http_literal_blocked` |
| configured marked | `configured_fetch_marked_configured` |
| plan diagnostics | `plan_surfaces_native_prebuild_gaps_from_on_disk_metadata` |
| suggest merge safety | `prebuild_suggestion_merge_never_enables_or_opens_profile` |

## What this proves

Weave can **explain** real-package native download patterns and emit
**reviewable** explicit `prebuild.fetches` drafts when metadata is complete —
without executing install scripts and without granting them network.

## Remaining single bottleneck

**Most real native packages do not publish static SRI (or a concrete sealed
output path) in package metadata.** Detection often stops at NeedsIntegrity or
Opaque (dynamic URLs like sharp / prebuild-install registries). Humans must
still verify artifacts and write `execution.prebuild.fetches` by hand for those
packages; Weave will not invent integrity or open script networking to finish
the job.

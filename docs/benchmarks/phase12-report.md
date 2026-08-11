# Phase 12 report: Real-World Adoption

Date: 2026-08-11  
Keeps ADR-0018 dual gate, CAS materialization, allowlisted prebuild fetch, and
static native resolution unchanged. No open script networking; no invented SRI.

## What landed

1. **Adoption assessment** (`adoption.rs`) — ExtractionReady / PartialNeedsPolicy /
   Blocked with per-package gaps and ordered next actions (what + why).
2. **`weave doctor`** — actionable messages; embeds adoption verdict + next steps;
   clearer peer/native/lifecycle guidance.
3. **`weave exec plan` / `suggest`** — print adoption block; JSON includes
   `adoption`; native gaps explain manual SRI path without guessing.
4. **E2E offline fixtures** (`fixtures/adoption/` + `tests/adoption.rs`):
   - extraction-only: init → switch → node run, no execution config
   - native-incomplete: switch succeeds; PartialNeedsPolicy + clear gaps
   - unsafe-lifecycle: never suggested
   - missing required peer: switch fails; Blocked
5. **Adoption guide** — `docs/adoption.md` (shortest safe path).

## Workflows that work end-to-end

| Workflow | Result |
|----------|--------|
| Extraction-only JS app | `init` → `switch` → `node` without `[execution]` |
| Peers + optional platform | Materialize; skip darwin-only optionals (existing smoke) |
| Workspace + file deps | Snapshot + links (existing smoke) |
| Offline generated outputs | Dual gate + declared outputs (Phase 8 integrate tests) |
| Native incomplete | Switch extracts; doctor/plan explain incompleteness |

## Most important remaining adoption blocker

**Real native packages rarely ship static SRI + sealed output paths**, so Weave
stops at NeedsIntegrity / Opaque and requires humans to verify artifacts and
write `execution.prebuild.fetches` by hand (Phase 11 boundary).

## Is that worth solving inside Weave?

**Not by inventing SRI or opening script networking** — that would break the
security model. Worthwhile follow-ups (outside this phase) are operational, not
heuristic: curated allowlist packs maintained out-of-band, or an explicit
offline “hash this file I already downloaded” helper that still requires human
review before enablement — never silent URL discovery.

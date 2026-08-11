# ADR-0017: Peer satisfaction and optional platform filtering

## Status

Accepted (Phase 5)

## Context

Phase 4 left peer and optional dependencies as parse-only. Real npm apps need:

1. Required peers present in the lockfile install graph (Node-resolvable).
2. Optional peers allowed to be absent (`peerDependenciesMeta.optional`).
3. Optional packages with `os`/`cpu` constraints skipped on mismatch.
4. Required packages with mismatched platform rejected.

Weave is an environment materializer, not a package resolver — it must not
invent peer installs.

## Decision

- **Peers:** Audit the lockfile graph with Node-style resolution. Fail
  `weave switch` / prepare when required peers are missing. Optional missing
  peers are allowed. Documented in doctor.
- **Optional platform filter:** Skip acquire + materialize for
  `optional: true` packages that fail npm `os`/`cpu` matching against the host.
  Map Rust platform tokens to npm (`macos`→`darwin`, `x86_64`→`x64`, …).
- **Required platform reject:** Error if a non-optional package rejects the host.
- **Native:** Prefer copy; doctor warns when `.node` binaries are absent;
  no automatic rebuild (`docs/native.md`).
- Bump `materialization_version` to **4**.

## Consequences

- Environments are host-platform accurate for optional native trees.
- Incomplete lockfiles with missing required peers fail loudly.
- Still no peer version-range semver solver beyond presence in the lockfile.

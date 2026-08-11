# ADR-0005: Artifact acquisition behind ArtifactSource

## Status

Accepted (Milestone 4)

## Date

2026-08-11

## Context

WEAVE.md §18 requires fetching tarballs through an abstraction so registries,
mirrors, offline stores, and tests are interchangeable. Integrity from the
lockfile must be verified before objects become trusted store content.

## Decision

- `ArtifactSource::fetch` returns raw bytes.
- `HttpArtifactSource` (ureq) handles registry URLs.
- `FileArtifactSource` supports path overrides and local file blobs.
- `acquire_one` verifies npm SRI (`sha1`/`sha256`/`sha512`) then `ContentStore::put`.
- Workspace/link nodes are skipped (no registry artifact).

## Consequences

- Core domain does not hard-code npmjs.org.
- Network tests are optional; unit tests use file sources.
- Directory `file:` packages are explicitly `NotImplemented` until materialization needs them.

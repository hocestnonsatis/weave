# ADR-0009: Unpacked CAS cache with hardlink materialization

## Status

Accepted (Milestone 8)

## Date

2026-08-11

## Context

Milestone 5–7 extract tarballs directly into each candidate tree (copy). Warm
switches re-extract identical packages. WEAVE.md Q1 remains open; hardlinks are
a reversible step before reflink/overlay/VFS.

## Decision

- Keep immutable tarballs in `store/objects`.
- Extract once into `store/unpacked/sha256/<id>/` with a `.ready` marker.
- Mark unpacked trees read-only (best-effort) after extract.
- Materialize into candidates by hardlinking files when:
  - source and dest share a filesystem device, and
  - the package does not declare install scripts / likely-native needs.
- Otherwise copy file bytes.
- Bump `materialization_version` to `2` (participates in EnvironmentId).

## Consequences

- Warm materialization avoids re-extraction and can share inodes across envs.
- Install-script packages never hardlink, protecting the shared cache.
- Cross-device checkouts fall back to copy automatically.
- Reflink/overlay remain future options under Q1.

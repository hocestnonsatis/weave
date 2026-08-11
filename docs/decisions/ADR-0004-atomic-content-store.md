# ADR-0004: Atomic content-addressed store puts

## Status

Accepted (Milestone 3)

## Date

2026-08-11

## Context

WEAVE.md requires `put` to be atomic: a crashed process must not create a
valid-looking corrupt object. Artifact identity is the SHA-256 of content.

## Decision

- Layout: `<store>/sha256/<ab>/<cdef…>` (2-hex shard).
- `put` writes to a sibling temp file, `fsync`s, then `rename`s into place.
- Temp names are never treated as objects (`contains` / `get` only see final paths).
- Existing verified objects are reused (idempotent put).
- `verify` re-hashes on-disk bytes and errors on mismatch.

## Consequences

- Concurrent puts of the same digest are safe via rename races + verify.
- Incomplete temp files may accumulate; GC can clean `.*.tmp-*` later.
- Metadata DB choice (Q5) remains open; object bytes need no DB for M3.

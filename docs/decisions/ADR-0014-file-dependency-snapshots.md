# ADR-0014: `file:` dependencies as immutable snapshots

## Status

Accepted (Phase 4)

## Context

npm `file:` / path dependencies may point at local directories. Treating those
directories as live mounts would make Weave environments non-reproducible:
mutating the source tree would silently change an already-activated
`node_modules` tree.

Alternatives considered:

1. **Live symlink** (npm's default for many local dirs)
2. **Copy tree** on each materialize
3. **Immutable snapshot** into the content-addressed store at acquire time
4. Explicitly unsupported

## Decision

First supported behavior for `PackageSource::Path` directory dependencies is
**immutable snapshot**:

- At acquire time, Weave packs the directory into an npm-style tarball
  (deterministic walk; skips `node_modules`, `.git`, `.weave`; skips symlinks).
- Bytes are stored in the CAS like registry tarballs.
- Materialization extracts/links from the snapshot — not from the live tree.

Workspace packages (`link: true` with a workspace path, `PackageSource::Link`)
remain **symlinks** into the project (ADR-0015), which is a different npm
construct.

## Consequences

- Reproducible environments after acquire.
- Source mutations after acquire do not affect activated trees until re-acquire.
- Not identical to npm's live `file:` directory linking — documented as Weave's
  reproducibility-first model.
- Large local trees pay pack cost once per content change.

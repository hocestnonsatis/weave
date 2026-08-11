# ADR-0016: Linux bin-link semantics

## Status

Accepted (Phase 4)

## Context

npm packages declare executables via `bin` in package metadata / lockfile.
Phase 3 documented that Weave did not create `node_modules/.bin` entries.

On Unix, npm creates relative symlinks. On Windows, npm writes cmd/ps1 shims.
A plain symlink is **not** sufficient on Windows, but it **is** the correct
Linux/macOS behavior.

## Decision

On Linux (and other Unix):

1. Parse `bin` from the lockfile (string or map) into `PackageNode.bin`.
2. Fall back to reading `package.json` `bin` after extraction when missing.
3. Create relative symlinks at the nearest `node_modules/.bin/<name>`.
4. Ensure the target script is executable without mutating shared hardlink
   cache inodes (break-link-then-chmod when needed).
5. Conflicting names: last writer wins (packages sorted by key).
6. Nested packages install bins into their nearest `node_modules/.bin`.

Windows shims are **unsupported** in this phase.

## Consequences

- Real Node tooling can invoke `node_modules/.bin/<name>` after switch.
- Materialization version bumped to `3` (environment identity changes).

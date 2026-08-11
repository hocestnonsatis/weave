# ADR-0001: Git CLI adapter for repository discovery

## Status

Accepted

## Date

2026-08-11

## Context

Weave needs repository root, branch, HEAD, and working-tree dirtiness. WEAVE.md §28 allows either the Git CLI or a library, and asks that the adapter hide the choice.

## Decision

Version 1 uses the system `git` executable via `std::process::Command`, wrapped in `weave-git::GitCli` / `GitRepository`.

## Consequences

- No `git2` / `gix` dependency yet; builds stay simple and match local Git behavior.
- Requires `git` on `PATH`.
- A future swap to a library can stay behind the same types without changing `weave-engine` or the CLI.

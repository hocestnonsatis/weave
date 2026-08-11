# Bin-link fixtures

Covers:

- single binary (`demo-cli`)
- multiple binaries (`multi-bin`)
- scoped package binary (`@scope/tool`)
- conflicting binary names (`conflict-a` / `conflict-b` → `shared-name`)
- nested dependency binaries (`parent/node_modules/nested-cli`)

## Linux behavior

Weave creates relative symlinks under `node_modules/.bin/` (npm Unix
semantics). Nested bins land in the nearest `node_modules/.bin`. Conflicting
names are last-writer-wins by sorted package key.

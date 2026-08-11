# WEAVE

## Project Charter & Technical Architecture

**Status:** Initial implementation specification  
**Project:** Weave  
**Primary language:** Rust  
**Initial platform:** Linux  
**Initial ecosystem:** Node.js / npm  
**Document role:** Source of truth for the implementation agent

---

# 1. Executive Summary

Weave is a Git-aware development environment engine designed to separate:

1. source state,
2. dependency state,
3. workspace state,
4. runtime/toolchain state.

The core thesis is:

> A Git branch should not require a duplicated repository, a duplicated `node_modules`, or a fresh dependency installation merely because its dependency graph differs from another branch.

Weave treats `node_modules` as a **materialized view**, not as the source of truth.

Dependency artifacts are stored immutably in a content-addressed store. A project environment is represented by a dependency graph plus metadata describing how that graph should be materialized. The filesystem view is generated from that environment.

Weave is not intended to replace npm as a package registry or package-resolution ecosystem in the first implementation.

Its initial responsibility is:

> Resolve, store, reuse, and materialize dependency state efficiently and transactionally.

---

# 2. The Problem

Modern JavaScript development combines several kinds of state that are frequently coupled too tightly:

```text
Git branch
    |
    +-- source files
    |
    +-- package.json
    |
    +-- lockfile
    |
    +-- node_modules
    |
    +-- build artifacts
    |
    +-- native addons
    |
    +-- local tooling
```

Switching between branches can therefore cause:

- expensive dependency installation,
- large amounts of duplicated disk usage,
- invalid or stale `node_modules`,
- conflicts between concurrent workspaces,
- slow CI/local iteration,
- unnecessary Git worktrees,
- fragile symlink arrangements,
- inconsistent native addon state.

Existing package managers improve parts of this problem, but the broader issue remains:

**dependency state is still commonly treated as a directory belonging to one checkout.**

Weave changes the abstraction.

---

# 3. Core Thesis

Weave defines the following relationship:

```text
Git state
   +
lockfile
   +
environment metadata
   =
development environment
```

`node_modules` is only the materialized filesystem representation of that environment.

Conceptually:

```text
              ENVIRONMENT
                   |
          +--------+--------+
          |                 |
      dependency         metadata
         graph              |
          |                 |
          +--------+--------+
                   |
             materializer
                   |
                   v
              node_modules
```

The same dependency graph can therefore be materialized into multiple workspaces without duplicating the underlying package contents.

---

# 4. Explicit Non-Goals

The first implementation MUST NOT attempt to:

- replace npmjs.org,
- create a new JavaScript package registry,
- invent a new package.json format,
- invent a new lockfile format,
- replace Node.js,
- replace Git,
- implement a new JavaScript runtime,
- support every package manager simultaneously,
- solve every language ecosystem,
- immediately eliminate Git worktrees,
- build a distributed package store,
- implement a full virtual filesystem before the basic architecture is proven.

These may become future projects.

The first objective is a small, correct, measurable system.

---

# 5. Design Principles

## 5.1 Dependency data is immutable

Once an artifact is stored, its content must never be modified in place.

If a different artifact has different content, it has a different identity.

---

## 5.2 Content addressing

The fundamental storage identity should be based on content.

Conceptually:

```text
content
   |
   v
SHA-256
   |
   v
object ID
```

The exact hashing/storage abstraction must remain replaceable.

---

## 5.3 Environment state is transactional

An active environment must always be internally consistent.

Never leave the user with a partially materialized environment after an error.

Desired sequence:

```text
resolve
   |
prepare
   |
materialize
   |
validate
   |
atomic activate
```

If any stage fails:

```text
old environment remains active
```

---

## 5.4 Reuse before fetching

When an artifact already exists locally, Weave should reuse it.

The expected optimization path is:

```text
local immutable object
        |
        +--> reuse
        |
        +--> materialize
```

not:

```text
download
install
copy
delete
```

---

## 5.5 Source state and dependency state are separate

A branch may point to an environment already known to Weave.

The branch name itself is not the environment identity.

A lockfile-derived graph should be the principal dependency identity.

---

## 5.6 Correctness before cleverness

Do not introduce FUSE, overlayfs, custom filesystems, daemon architecture, or kernel-specific tricks until a conventional implementation has measurable limitations.

A boring correct prototype is preferable to an impressive broken prototype.

---

# 6. Initial Platform

The first implementation targets:

- Linux
- x86_64
- Rust stable
- Node.js projects
- npm-compatible package-lock.json

The codebase should use platform abstraction where practical, but Windows and macOS behavior should not block Linux development.

---

# 7. Why Rust

Rust is selected because Weave will eventually need to handle:

- filesystem operations,
- concurrent downloads,
- hashing,
- atomic state transitions,
- Git integration,
- process execution,
- filesystem metadata,
- native artifacts,
- potentially low-level filesystem mechanisms.

The implementation should avoid unnecessary unsafe code.

Unsafe code requires a documented justification.

---

# 8. High-Level Architecture

```text
+------------------------------------------------------+
|                    weave CLI                         |
+------------------------------------------------------+
| command layer                                        |
+------------------------------------------------------+
| project | environment | git | dependency | gc       |
+------------------------------------------------------+
| core domain                                           |
|                                                      |
| Environment                                         |
| DependencyGraph                                     |
| Artifact                                           |
| Workspace                                           |
| MaterializationPlan                                |
+------------------------------------------------------+
| infrastructure                                       |
|                                                      |
| Git adapter                                         |
| npm lockfile adapter                                |
| Content store                                       |
| Filesystem backend                                  |
| Process runner                                      |
| State database                                      |
+------------------------------------------------------+
| Linux filesystem / Git / Node.js                     |
+------------------------------------------------------+
```

The domain layer must not directly depend on CLI concerns.

---

# 9. Proposed Rust Workspace

Use a Cargo workspace.

Initial structure:

```text
weave/
├── Cargo.toml
├── README.md
├── WEAVE.md
├── crates/
│   ├── weave-cli/
│   ├── weave-core/
│   ├── weave-git/
│   ├── weave-lockfile/
│   ├── weave-store/
│   ├── weave-fs/
│   └── weave-engine/
├── tests/
├── benchmarks/
└── docs/
```

Responsibilities:

### `weave-cli`

CLI parsing, user-facing output, exit codes.

### `weave-core`

Domain models and core traits.

### `weave-git`

Git integration.

### `weave-lockfile`

npm package-lock parsing and dependency graph extraction.

### `weave-store`

Content-addressed artifact store.

### `weave-fs`

Filesystem materialization primitives.

### `weave-engine`

Orchestration of resolution, preparation, materialization, validation and activation.

Do not create dozens of crates prematurely.

---

# 10. CLI

The initial CLI should be intentionally small.

Required commands:

```text
weave init
weave status
weave env list
weave env create
weave switch
weave materialize
weave gc
weave doctor
```

The exact argument syntax can evolve, but command semantics should remain clear.

Example:

```bash
weave init
weave env create
weave switch main
weave status
```

Future commands may include:

```text
weave store
weave diff
weave workspace
weave branch
weave cache
```

Do not implement future commands just to make the CLI look complete.

---

# 11. Project Initialization

`weave init` should:

1. locate the Git repository,
2. verify that package.json exists,
3. detect package-lock.json,
4. create the `.weave/` metadata directory,
5. initialize the local state database if required,
6. initialize or validate the global content store configuration.

Do not modify package.json.

Do not modify package-lock.json.

Do not silently rewrite npm metadata.

---

# 12. `.weave/`

Project-local metadata should live in:

```text
.weave/
```

Suggested structure:

```text
.weave/
├── config.toml
├── state.db
├── environments/
└── metadata/
```

`.weave/` should generally be ignored by Git.

The exact persistence technology is intentionally not fixed.

SQLite is acceptable if it simplifies atomic metadata operations.

Do not introduce a database solely because it sounds sophisticated.

---

# 13. Environment Identity

An environment should not be identified only by branch name.

A conceptual environment identity is:

```text
EnvironmentID =
    hash(
        dependency graph identity
        +
        relevant platform identity
        +
        relevant runtime identity
        +
        materialization format/version
    )
```

This prevents unrelated environments from accidentally sharing incompatible state.

Branch association is metadata:

```text
branch -> environment
```

not the reverse.

Multiple branches may point to the same environment.

---

# 14. Dependency Graph

The dependency graph is the central domain object.

Conceptually:

```text
root project
    |
    +-- react
    |    |
    |    +-- loose-envify
    |
    +-- vite
         |
         +-- esbuild
```

Each package node should eventually carry enough information to distinguish:

- package identity,
- version,
- resolved source,
- integrity,
- dependency relationships,
- optional dependencies,
- peer dependency information,
- platform constraints,
- architecture constraints,
- install-script/native-addon requirements.

The graph model must not assume every package is pure JavaScript.

---

# 15. npm Lockfile Support

Version 1 supports npm `package-lock.json`.

The implementation should support the modern lockfile formats encountered in contemporary npm projects.

Do not attempt to implement npm's entire resolver from scratch.

The lockfile is the initial source of dependency resolution truth.

If a project has no supported lockfile:

```text
weave init
```

should explain the limitation rather than silently inventing dependency state.

Future versions may integrate npm itself as a resolver.

---

# 16. Content-Addressed Store

The global store may look conceptually like:

```text
~/.weave/
└── store/
    └── objects/
        └── sha256/
            ├── ab/
            │   └── cdef...
            └── 91/
                └── 72ab...
```

The physical layout is implementation detail.

The logical contract is:

```text
ArtifactID -> immutable artifact
```

The store must provide operations conceptually equivalent to:

```text
contains(id)
put(id, bytes)
open(id)
remove(id)
verify(id)
```

`put` must be atomic.

A crashed process must not create a valid-looking corrupt object.

---

# 17. Package Artifact Identity

Do not use only:

```text
react@19.1.0
```

as the immutable artifact identity.

Two artifacts with the same package/version should not be assumed identical unless their verified content matches.

Prefer:

```text
ArtifactID = content hash
```

with package metadata stored alongside it.

---

# 18. Acquisition

The first acquisition implementation may use npm-compatible registry metadata and tarballs.

However, keep acquisition behind a trait.

Conceptually:

```rust
trait ArtifactSource {
    fn fetch(&self, request: &ArtifactRequest) -> Result<Artifact>;
}
```

This permits future:

- npm registry,
- local cache,
- mirror,
- offline store,
- custom registry.

Do not hard-code the registry into the core domain.

---

# 19. Materialization

Materialization converts immutable stored artifacts into a project-visible filesystem tree.

Conceptually:

```text
Store
  |
  v
MaterializationPlan
  |
  v
Filesystem
```

The materializer must determine the cheapest safe mechanism available.

Possible mechanisms:

1. hardlink,
2. reflink,
3. symlink,
4. copy.

The chosen mechanism must account for:

- file mutability,
- package lifecycle scripts,
- executables,
- symlink semantics,
- native binaries,
- filesystem boundaries.

Do not assume symlinks are universally equivalent to copies.

---

# 20. Critical `node_modules` Rule

Weave MUST NOT blindly construct:

```text
node_modules/<package>
```

as one symlink to a global package directory without understanding Node module resolution and package-local layout.

Nested dependencies and peer dependencies make this incorrect in general.

The materializer must derive the actual required filesystem tree from the dependency graph.

Correctness takes priority over maximal deduplication.

---

# 21. Peer Dependencies

Peer dependencies are a first-class concern.

The implementation must not flatten dependency trees simply because two package names match.

The materialized layout must preserve the semantics expected by Node.js resolution.

Peer dependency compatibility should be represented explicitly in the graph.

---

# 22. Optional Dependencies

Optional dependencies may depend on:

- OS,
- architecture,
- libc,
- CPU features,
- runtime conditions.

Weave must model the relevant environment identity.

For example:

```text
linux/x86_64
linux/arm64
darwin/arm64
```

must not accidentally share an incompatible native artifact.

---

# 23. Install Scripts

Packages may execute:

```text
preinstall
install
postinstall
prepare
```

scripts.

This is one of the hardest parts of the system.

Version 1 must NOT pretend that package extraction alone always produces a runnable environment.

The architecture must distinguish:

```text
package artifact
```

from:

```text
installed package state
```

A package that produces generated/native files during installation may require environment-specific materialization.

This state must be modeled rather than hidden.

---

# 24. Native Addons

Native modules are a design constraint from day one.

Examples include:

```text
*.node
node-gyp
prebuild
node-pre-gyp
```

A native artifact may depend on:

- OS,
- architecture,
- libc,
- Node ABI,
- compiler/runtime assumptions.

The artifact/environment identity must therefore be extensible.

Do not make native addons a Phase 99 problem.

---

# 25. Transactional Activation

Never modify the active environment in place if doing so can leave a broken state.

Preferred conceptual model:

```text
active/
candidate/
```

Build the candidate environment completely.

Validate it.

Then perform an atomic transition.

If activation fails:

```text
candidate -> delete/retain for diagnostics
active    -> unchanged
```

---

# 26. Concurrent Processes

Weave should assume multiple processes can operate simultaneously.

Examples:

```text
Terminal A: weave switch main
Terminal B: weave switch feature
Terminal C: weave gc
```

The store and metadata database must therefore support concurrency.

Avoid global process locks unless necessary.

Prefer:

- atomic filesystem operations,
- transactional metadata,
- per-object locking where needed,
- deterministic conflict handling.

---

# 27. Garbage Collection

The store will eventually contain artifacts no longer referenced by environments.

`weave gc` should use reachability.

Conceptually:

```text
roots
  |
  +-- active environments
  +-- known environments
  +-- explicitly pinned artifacts
       |
       v
reachable artifacts
       |
       v
unreachable -> candidate for deletion
```

GC must never delete an artifact still reachable by a valid environment.

The first GC implementation may be conservative.

Correctness is more important than maximum reclamation.

---

# 28. Git Integration

Git should be treated as an external source of source-state truth.

Weave needs to understand at minimum:

- repository root,
- current HEAD,
- current branch,
- working tree state,
- relevant lockfile state.

Avoid implementing Git internals.

Use the Git CLI initially unless a library is clearly superior.

The Git adapter should hide that implementation decision.

---

# 29. Dirty Working Trees

Weave must not assume:

```text
working tree == HEAD
```

A user may edit:

```text
package.json
package-lock.json
```

without committing.

`weave status` should clearly distinguish:

```text
Git state
Dependency state
Materialized environment state
```

Do not silently discard local changes.

---

# 30. Branch Switching

A switch should conceptually perform:

```text
1. inspect target Git state
2. identify target lockfile
3. identify dependency environment
4. prepare missing artifacts
5. prepare materialization plan
6. materialize candidate
7. validate
8. activate
```

Do not automatically run arbitrary package lifecycle scripts during a simple branch switch unless the environment requires them and the behavior is explicit.

---

# 31. Workspaces and Git Worktrees

Weave should initially support ordinary Git checkouts.

Do not make Git worktree elimination a prerequisite for the MVP.

The long-term research direction is:

```text
traditional:
    repository copy/worktree
        +
    node_modules

Weave:
    shared repository objects
        +
    isolated source view
        +
    shared dependency store
        +
    environment-specific materialization
```

Only after the dependency/environment layer is proven should Weave attempt deeper workspace virtualization.

---

# 32. Security Model

Dependency installation executes untrusted code in many Node.js projects.

Weave must never imply that content-addressing makes package execution safe.

The system must distinguish:

```text
trusted storage
```

from:

```text
untrusted lifecycle execution
```

Future work may include sandboxing.

Version 1 should at minimum:

- avoid executing scripts unexpectedly,
- make script execution explicit,
- avoid privilege escalation,
- preserve file ownership/security semantics where applicable,
- never follow unsafe paths outside the intended materialization root.

Path traversal must be tested aggressively.

---

# 33. Failure Handling

Every operation must have explicit failure semantics.

Examples:

```text
network failure
corrupt artifact
hash mismatch
permission denied
disk full
broken symlink
unsupported lockfile
unsupported native package
concurrent modification
```

The CLI should report:

1. what failed,
2. what state was preserved,
3. whether retrying is safe,
4. what diagnostic command can be run.

Never turn a low-level error into a misleading success.

---

# 34. Observability

The implementation must have structured internal diagnostics.

At minimum:

```text
debug
info
warn
error
```

User-facing output should remain readable.

A machine-readable mode should be considered early, for example:

```bash
weave status --json
```

Do not make pretty terminal output the only interface.

---

# 35. Testing Strategy

Testing is a core part of the architecture.

Required layers:

## Unit tests

For:

- hashing,
- artifact IDs,
- dependency graph,
- path handling,
- environment identity,
- state transitions.

## Integration tests

For:

- real Git repositories,
- real package-lock files,
- store operations,
- materialization.

## End-to-end tests

Create temporary Node projects and verify:

```text
source
+
package-lock.json
+
weave
=
working Node environment
```

## Crash/failure tests

Simulate:

- interrupted materialization,
- interrupted download,
- disk errors where practical,
- concurrent operations.

---

# 36. Golden Fixtures

Create fixtures representing difficult Node dependency structures:

1. flat dependency tree,
2. nested versions,
3. peer dependencies,
4. optional dependencies,
5. native addon,
6. lifecycle scripts,
7. monorepo,
8. workspace packages,
9. symlinked package,
10. package with unusual filesystem contents.

Every major materialization change should run against these fixtures.

---

# 37. Benchmark Program

Do not claim Weave is faster until measured.

Benchmarks should compare:

```text
npm install
npm ci
pnpm install
Weave cold
Weave warm
Weave branch switch
```

Measure:

- wall-clock time,
- network bytes,
- disk consumption,
- inode count,
- number of files created,
- number of files copied,
- branch-switch latency.

Test at multiple scales.

Example project classes:

```text
small
medium
large
monorepo
native-heavy
```

---

# 38. The Central Benchmark

The most important benchmark is not:

> "How fast can Weave install?"

It is:

> "How fast can a developer switch between two dependency states that have already been observed?"

Example:

```text
Environment A:
    900 packages

Environment B:
    920 packages
    890 shared with A
```

After both environments have been prepared, switching should primarily be a filesystem/state activation operation.

This is the killer path.

---

# 39. Disk Efficiency Benchmark

Measure:

```text
traditional separate node_modules:
    size(A) + size(B)

Weave:
    unique artifacts(A ∪ B)
    +
    environment metadata
    +
    materialization overhead
```

Do not count logical file size alone.

Measure actual filesystem blocks where meaningful.

---

# 40. MVP Definition

The MVP is complete only when it can:

1. initialize a real Node.js Git project,
2. read its supported package-lock.json,
3. construct a dependency representation,
4. acquire package artifacts,
5. store them content-addressably,
6. materialize a correct `node_modules`,
7. create a second dependency environment,
8. switch between them,
9. reuse previously stored artifacts,
10. survive interrupted operations without corrupting the active environment,
11. provide measurable disk/time improvements in at least some realistic workloads.

A fake demo is not an MVP.

---

# 41. MVP Deliberate Limitations

The MVP may explicitly reject:

- unsupported lockfile versions,
- unsupported native packages,
- unusual lifecycle behavior,
- unsupported workspace configurations,
- unsupported filesystem types.

It must fail clearly.

Do not silently fall back to behavior that violates Weave's consistency guarantees.

---

# 42. Development Milestones

## Milestone 0: Repository skeleton

Create:

```text
Cargo workspace
CLI
core traits
basic documentation
CI
```

No complex behavior.

---

## Milestone 1: Git + project discovery

Implement:

```text
weave init
weave status
```

Detect:

- Git root,
- branch,
- package.json,
- package-lock.json,
- dirty state.

---

## Milestone 2: Lockfile model

Parse supported npm lockfiles.

Produce a deterministic dependency graph.

Add extensive fixtures.

---

## Milestone 3: Content store

Implement:

```text
put
get
contains
verify
```

with atomic writes.

Add corruption tests.

---

## Milestone 4: Artifact acquisition

Fetch package tarballs through an abstraction.

Verify integrity.

Store immutable artifacts.

---

## Milestone 5: Basic materializer

Materialize a correct dependency tree into a temporary environment.

Do not optimize prematurely.

---

## Milestone 6: Environment manager

Implement:

```text
environment identity
environment metadata
environment creation
environment lookup
environment activation
```

---

## Milestone 7: Branch switching

Implement:

```text
weave switch <target>
```

with transactional activation.

---

## Milestone 8: Reuse and deduplication

Measure and optimize:

- artifact reuse,
- filesystem links,
- reflinks,
- metadata overhead.

---

## Milestone 9: Failure hardening

Crash tests, concurrent operations, disk failures, corruption.

---

## Milestone 10: Benchmark suite

Publish reproducible benchmark methodology and results.

**Done:** `weave-bench` crate + `benchmarks/README.md`. Offline `small` suite
covers cold / warm / A↔B switch; optional `--with-npm`. JSON output under
`benchmarks/out/` (gitignored).

---

# 43. Future Architecture

After the MVP is stable, investigate:

## 43.1 Workspace virtualization

Can source trees also be shared using:

- overlayfs,
- copy-on-write filesystems,
- reflinks,
- Git object storage?

---

## 43.2 Background daemon

A daemon might maintain:

```text
artifact cache
dependency metadata
filesystem state
```

But do not introduce a daemon before profiling proves it useful.

---

## 43.3 FUSE / virtual filesystem

Potential future architecture:

```text
immutable store
       |
       v
virtual filesystem
       |
       v
node_modules
```

This may drastically reduce materialization overhead.

It is explicitly NOT part of the initial MVP.

---

## 43.4 Cross-language environments

The dependency model could eventually generalize:

```text
Node
Python
Rust
Go
Java
```

Do not prematurely abstract everything around this possibility.

---

# 44. Architectural Questions That Must Remain Open

The agent must not silently make permanent decisions about these questions:

### Q1
Should the final filesystem view use hardlinks, reflinks, overlayfs, or a virtual filesystem?

### Q2
Should Weave eventually execute npm lifecycle scripts itself, delegate them to npm, or introduce a controlled execution layer?

### Q3
Should Git source isolation become a core Weave feature?

### Q4
Should Weave eventually replace package-manager installation entirely or remain an environment layer around existing package managers?

### Q5
Should the store use SQLite metadata, filesystem metadata, or both?

These questions require benchmarks and experiments.

---

# 45. Agent Coding Rules

The implementation agent MUST follow these rules.

## Rule 1: Read this document before coding

This document is the architectural contract.

---

## Rule 2: Do not over-engineer

If a simple implementation can validate a hypothesis, implement it first.

---

## Rule 3: Every major architectural decision must be documented

Use:

```text
docs/decisions/
```

for Architecture Decision Records.

Format:

```text
ADR-0001-title.md
ADR-0002-title.md
```

---

## Rule 4: No speculative abstraction explosion

Do not create traits for every struct.

Create abstractions around real substitution boundaries.

---

## Rule 5: Never hide correctness problems

If an npm package structure cannot currently be materialized safely, return an explicit error.

Do not silently produce a potentially broken `node_modules`.

---

## Rule 6: Never mutate immutable store objects

If an artifact changes, create a new artifact.

---

## Rule 7: Never destroy user state to make a command succeed

Git working-tree changes, environment state, and store state must be treated conservatively.

---

## Rule 8: Add tests with every meaningful feature

A feature without a regression test is incomplete unless there is a documented reason.

---

## Rule 9: Benchmark before optimizing

Do not claim an optimization is useful without measurement.

---

## Rule 10: Keep the CLI usable

Developer tooling is interactive infrastructure.

Errors must be actionable.

---

# 46. Definition of Done for Code

A task is not done merely because it compiles.

For meaningful implementation work, the agent should verify:

```text
cargo fmt
cargo check
cargo test
cargo clippy
```

and relevant integration tests.

If benchmarks exist for the affected subsystem, run them.

---

# 47. Repository Hygiene

Do not commit:

- downloaded package caches,
- generated `node_modules`,
- temporary environments,
- benchmark garbage,
- credentials,
- registry tokens.

Use test-specific temporary directories.

---

# 48. Recommended Initial Dependencies

Choose dependencies conservatively.

Potential categories:

- CLI parsing,
- serialization,
- hashing,
- error handling,
- Git process integration,
- filesystem utilities,
- temporary test directories.

Do not add large frameworks without a concrete requirement.

The agent should verify current crate versions and licenses before adding dependencies.

---

# 49. First Implementation Task

The first implementation task is NOT dependency installation.

Start with:

```text
Cargo workspace
        |
        +-- weave-cli
        +-- weave-core
        +-- weave-git
        +-- weave-lockfile
        +-- weave-store
        +-- weave-fs
        +-- weave-engine
```

Then implement:

```bash
weave init
weave status
```

against a real Git repository.

The first milestone should produce a small but real vertical slice:

```text
CLI
 ↓
Git discovery
 ↓
Project discovery
 ↓
state representation
 ↓
human-readable status
```

Only after that should dependency acquisition begin.

---

# 50. Final Product Vision

Weave should ultimately make this workflow feel normal:

```bash
git switch feature/payment
weave switch
```

and then:

```text
Environment already prepared.

No install required.
No duplicated dependency tree.
No manual cleanup.

Environment activated.
```

The developer should not need to think about:

```text
"Which node_modules belongs to this branch?"
```

because that question should disappear.

The long-term vision is not simply:

> "a faster npm install."

It is:

> **Development environments should be derived state.**

Git describes source state.

Lockfiles describe dependency state.

Weave combines those states into an environment and materializes only what the developer actually needs.

---

# 51. Guiding Sentence

When architectural choices become ambiguous, return to this:

> **Weave exists to make development environments cheap, isolated, reproducible, and shareable without requiring physical duplication of the state that does not need to be duplicated.**

If a proposed feature does not contribute to that goal, question whether it belongs in Weave.

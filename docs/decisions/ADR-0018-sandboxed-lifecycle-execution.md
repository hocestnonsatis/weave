# ADR-0018: Opt-in sandboxed lifecycle & native execution

## Status

Accepted (Phase 6 design) — Phases 7–10 implemented (offline Bubblewrap, sealed
output activation, policy discovery, allowlisted prebuild fetch). Default
`switch` still does not execute or use the network.

## Date

2026-08-11

## Context

Phases 2–5 established that Weave materializes reproducible filesystem trees from
content-addressed tarballs. Lifecycle scripts and native rebuilds remain
**classify-only** (ADR-0012). Phase 5 identified the single most important next
problem: packages that ship without usable prebuilt artifacts cannot become
runtime-complete without some form of execution.

WEAVE.md forbids automatic lifecycle execution on ordinary branch switch and
leaves **Q1** open (execute vs delegate vs controlled layer). Phase 3–5 evidence
shows most packages are extraction-only, but native-build and install-script
packages exist in real corpora. Doctor already warns when `.node` binaries are
missing after materialize.

This ADR defines a **controlled execution layer** that:

- remains opt-in and explicit,
- is sandboxed on Linux first,
- stores results in the CAS like other artifacts,
- does **not** introduce a daemon, FUSE, overlayfs, or a new package resolver.

## Decision summary

| Question | Decision |
|----------|----------|
| Why execute? | Only when extraction cannot produce a usable package tree |
| Who decides? | Explicit user/CI opt-in per project or invocation |
| How? | Controlled Weave execution layer (not silent `npm install`) |
| Where? | Ephemeral sandbox workspace; outputs sealed into CAS |
| Default on `weave switch`? | **Still no execution** (ADR-0012 holds until opt-in) |

---

## 1. Why execution is necessary

Extraction-only materialization is insufficient when a package’s tarball does
not contain the runtime files the application needs on this host. Typical cases:

1. **Native addons** that compile or download ABI-specific `.node` binaries.
2. **Generated-file packages** whose `install`/`postinstall` writes code, assets,
   or metadata required at `require()` time.
3. **Toolchain-bound prebuilds** that select a binary for Node ABI + OS + CPU
   and fail closed if the matching artifact is absent.

Without Weave-mediated execution, users must run untracked `npm rebuild` /
manual scripts against the activated tree, which:

- mutates an environment Weave believes is immutable,
- has no CAS identity for generated outputs,
- has no sandbox or audit trail.

Execution is therefore necessary for **completeness**, not for performance.

## 2. Package types that require execution

Aligned with `docs/lifecycle-classification.md`:

| Class | Requires Weave execution? | Notes |
|-------|---------------------------|-------|
| extraction-only | No | Default path |
| generated-files required | Yes (limited script allowlist) | Only declared npm lifecycle names |
| native-build required | Yes (rebuild profile) | `node-gyp` / N-API / prebuild fetch |
| runtime-install required | Yes, with stricter review | May need network; prefer deny |
| unsupported/unsafe | **Never** | Shell `curl \| sh`, privilege escalation, etc. |

Heuristics (non-authoritative): `hasInstallScript`, `likely_native`, presence of
`binding.gyp`, absence of `.node` after materialize, known package names.

**Hard rule:** scripts outside the npm lifecycle vocabulary
(`preinstall`, `install`, `postinstall`, `prepare` — and only those Weave
explicitly enables) are unsupported.

## 3. Trust & security model

### Trust boundary

- **Trusted:** Weave binary, project config, lockfile graph identity, CAS bytes
  already verified by integrity.
- **Untrusted:** package lifecycle scripts, downloadable prebuild URLs, anything
  the script process writes before seal.

### Principles

1. **Default deny** — no execution unless opt-in is present and valid.
2. **Least privilege** — sandbox drops capabilities; no root; no ambient Docker
   socket; no access to SSH keys or unrelated home dirs.
3. **Explicit network policy** — default **offline**; optional allowlist for
   known prebuild CDNs when the profile permits.
4. **No secrets by default** — environment scrubbed; only whitelisted vars
   (`PATH` subset, `HOME` = sandbox home, Node ABI vars Weave sets).
5. **Auditability** — every run records package key, script name, platform
   identity, sandbox profile, exit status, output artifact ids.
6. **Fail closed** — sandbox setup failure or policy violation aborts; never
   falls back to unsandboxed host execution.

### Threats in scope

- Malicious `postinstall` reading `~/.ssh` or cloud credentials
- Exfiltrating via unrestricted network
- Escaping to mutate the live `node_modules` or global CAS mid-run
- Privilege escalation via setuid helpers

### Threats deferred

- Kernel 0-days escaping namespaces/seccomp
- Supply-chain compromise of Weave itself
- Covert channels via timing

## 4. Filesystem / network / environment isolation

### Filesystem

Execution runs in an **ephemeral workspace** under
`$WEAVE_HOME/exec/<run-id>/` (not the project’s active `node_modules`).

Layout (conceptual):

```text
exec/<run-id>/
  work/           # package copy (prefer_copy semantics)
  out/            # only paths Weave collects after success
  home/           # fake HOME
  log/            # stdout/stderr
```

Mount / bind policy (Linux):

- **Read-only:** Node binary, system libs required to run Node/`node-gyp`,
  content-addressed **inputs** (tarball / unpacked cache snapshot).
- **Read-write:** `work/`, `out/`, `home/`, `log/`, temp.
- **Invisible / denied:** project `.git`, user home (except sandbox home),
  other projects, Docker socket, Weave object store root (writes go through
  Weave post-seal API only).

After success, Weave **seals** declared output paths into CAS and discards the
workspace. After failure, workspace may be retained when `--keep-failed` is set
for debugging; never activated into the project.

### Network

| Profile | Network |
|---------|---------|
| `offline` (default) | None (`network` namespace empty / `EBADF` on sockets) |
| `prebuild-fetch` | Allowlist only (e.g. `registry.npmjs.org`, GitHub release hosts configured in policy) |
| `open` | **Rejected** in v1 |

### Environment

Whitelist only:

- `PATH` (sandbox-controlled)
- `HOME` → sandbox home
- `TMPDIR` → sandbox temp
- `npm_config_*` Weave sets for offline/rebuild
- `npm_config_node_gyp`, `npm_config_cache` → sandbox paths
- Node identity: `npm_config_runtime`, `npm_config_target`, etc. as needed

Everything else unset (including `AWS_*`, `SSH_*`, `DOCKER_*`, `CURL_*`
credential vars).

## 5. Allowed inputs and outputs

### Inputs (read-only)

- Package artifact bytes (CAS id) already acquired for the lockfile node
- Declared lifecycle script name(s) from package metadata / lockfile
- Host platform identity (OS, CPU, Node ABI)
- Sandbox profile id and policy hash
- Optional: toolchain fingerprints (`node -p process.versions`, compiler ids)

### Outputs (sealed)

- Files under the package root that differ from the pre-execution snapshot
  **or** an explicit allowlist (`*.node`, `build/Release/**`, package-defined
  `files` additions)
- Structured run record JSON (metadata only)

### Forbidden outputs

- Paths outside the package work root
- Modifications to lockfile / `package.json` of the project
- Writes into the shared unpacked cache in place (cache remains immutable;
  new CAS objects are created instead)

## 6. Node ABI / platform / CPU identity

Execution results are **not** interchangeable across:

| Dimension | Source |
|-----------|--------|
| OS | `HostPlatform.npm_os()` (`linux`, `darwin`, …) |
| CPU | `HostPlatform.npm_cpu()` (`x64`, `arm64`, …) |
| Node ABI | `process.versions.modules` (NODE_MODULE_VERSION) |
| Node major/minor | `process.versions.node` (recorded; ABI is primary for `.node`) |
| libc flavor (Linux) | `glibc` vs `musl` when detectable |
| Sandbox profile | profile id + policy hash |
| Script set | ordered lifecycle names actually run |
| Input artifact id | CAS id of the package tarball / snapshot |

**Derived artifact identity** (conceptual):

```text
weave-exec-v1 || input_artifact_id || platform_tuple || node_abi ||
libc || script_digest || profile_hash || output_tree_hash
```

Environment identity (ADR-0007) must incorporate a digest of sealed execution
results when opt-in execution is enabled for that environment — otherwise two
hosts could share a graph id but differ in `.node` contents.

`materialization_version` will bump when execution-backed materialization lands
(not in this ADR’s implementation phase).

## 7. Reproducibility and caching

### Goals

- Same inputs + same platform tuple → **cache hit**; skip re-execution.
- Cache entries live in CAS (content-addressed) with a side index:
  `$WEAVE_HOME/exec-cache/index/<key>.json` → output artifact id(s).

### Non-goals (v1)

- Bit-identical object files across compiler patch versions (best-effort)
- Cross-host reuse when libc/toolchain fingerprints differ

### Cache key

Hash of the identity tuple in §6 (excluding `output_tree_hash`, which is the
value). On hit, Weave materializes sealed outputs like ordinary artifacts.

### Cache invalidation

- Input CAS id change
- Platform / ABI / profile change
- Policy hash change
- Explicit `weave exec purge` / GC of unreachable exec artifacts (via existing
  reachability GC roots once registered)

## 8. Failure and rollback behavior

1. Execution never mutates the **active** `node_modules` directly.
2. Candidate trees that depend on failed execution **do not activate**.
3. Transactional activation (ADR-0008) remains the only path to live trees.
4. On script non-zero exit: discard seal; leave active env unchanged; surface
   log path and package key.
5. On sandbox violation (seccomp/landlock deny): treat as failure; do not retry
   unsandboxed.
6. Partial outputs are never published to CAS.
7. Crash mid-seal: CAS put remains atomic (ADR-0004); incomplete objects are GC
   candidates.

## 9. Explicit user opt-in semantics

### Defaults

- `weave switch` / `weave materialize` → **no execution** (ADR-0012 unchanged).

### Opt-in surfaces (v1 design)

1. **Project config** (`.weave/config.toml`):

   ```toml
   [execution]
   enabled = false
   profile = "offline"          # offline | prebuild-fetch
   allow_packages = []          # empty = none; or ["bcrypt", "sqlite3"]
   allow_scripts = ["install"]  # subset of lifecycle names
   ```

2. **CLI flag** (must be combined with config enablement or an explicit
   `--i-know-what-im-doing` style confirmation for one-shot CI):

   ```text
   weave exec plan          # dry-run (safe; Phase 6 experiment)
   weave exec run           # future — blocked until implementation ADR slice
   weave switch --with-exec # Phase 8 — only if execution.enabled + flag
   ```

3. **CI contract:** opt-in must be in version-controlled config; ambient
   `WEAVE_EXEC=1` alone is insufficient (too easy to leak into developer
   machines). Env may *additionally* require enablement but cannot replace
   config.

### Denial cases

- `enabled = false` → plan may still be shown; run refused.
- Package not in `allow_packages` (when list non-empty) → skipped/refused.
- Script not in `allow_scripts` → refused.
- Classified `unsupported/unsafe` → refused even if listed.

## 10. How generated artifacts enter the CAS

Post-success pipeline:

1. Diff or collect allowlisted paths under `work/<package>/`.
2. Pack collected files into a **secondary artifact** (npm-style tarball or
   Weave “overlay blob” format — v1 prefers npm-style tarball for reuse of
   extract/link paths).
3. `ContentStore::put` → `ArtifactId` (integrity optional but recorded).
4. Record mapping:
   `exec-cache key → { input_id, output_id, platform, abi, profile, scripts }`.
5. Materialization plan gains an optional `execution_output_id` per package;
   link/copy that tree **instead of** (or layered over) the pristine extract
   for that package key.

Generated artifacts are first-class CAS citizens: pinned by environment
records, GC-reachable, and never confused with registry downloads
(`ArtifactRequest` source kind `ExecutionOutput { … }` — future enum variant).

## 11. How execution differs from ordinary artifact acquisition

| Aspect | Acquire (registry/file) | Execution |
|--------|-------------------------|-----------|
| Trigger | Lockfile resolved URL / path | Opt-in + incomplete package |
| Trust | Integrity from lockfile | Untrusted code; sealed outputs re-hashed |
| Network | Fetch tarball | Default none |
| Identity | Bytes of upstream tarball | Bytes of **outputs** + platform tuple |
| Cache | CAS by content hash | CAS + platform-qualified index |
| Failure | Package missing | Environment incomplete; no activate |
| Resolver | None (URL given) | None (still no dependency solving) |

Execution **does not** fetch missing dependencies, rewrite the lockfile, or
satisfy peers. Those remain Phase 5 semantics.

## 12. Linux-first sandbox options and tradeoffs

| Option | Isolation strength | Ops cost | Notes |
|--------|-------------------|----------|-------|
| **A. bwrap (Bubblewrap)** | Strong (user NS, mount, network NS) | Low if installed | Widely used (Flatpak); good default candidate |
| **B. Landlock + seccomp + rlimit only** | Medium | Very low | No user NS needed; weaker FS isolation |
| **C. systemd-nspawn / unshare scripts** | Strong | Medium | Heavier; host policy dependent |
| **D. Firecracker / VM** | Strongest | High | Overkill for package scripts; slow cold start |
| **E. Unsandboxed subprocess** | None | None | **Rejected** for Weave-mediated runs |
| **F. Docker/Podman per script** | Strong | High + daemon-ish | Conflicts with “no daemon” product stance; optional later |

### Chosen direction (v1)

**Primary: Bubblewrap (`bwrap`) when available**, with a **degraded Landlock +
seccomp profile** only when:

- explicitly allowed by config `execution.allow_weak_sandbox = true`, and
- user acknowledges reduced isolation.

If neither sandbox can be established → **refuse execution** (fail closed).

### Explicitly rejected for this phase

- FUSE / overlayfs for execution mounts (architecture freeze)
- Long-lived Weave daemon to host sandboxes
- Delegating silently to `npm install` / `npm rebuild` on the live tree
  (no CAS seal, no Weave policy)

Delegation as an **escape hatch** (`execution.backend = "external-npm"`) may be
documented later as unsupported/experimental; it is not the controlled layer.

## Consequences

### Positive

- Clear path to native/completeness without abandoning CAS + hardlink/copy.
- Security defaults match WEAVE.md (no automatic scripts on switch).
- Execution outputs become reproducible, GC-able artifacts.

### Negative / costs

- Toolchain dependency (`bwrap`, compilers, Python for node-gyp) on builder hosts.
- Cache keyed by ABI → more CAS growth.
- Classification heuristics will have false positives/negatives until refined.

### Compatibility with prior ADRs

- ADR-0012 remains the **default** behavior.
- ADR-0008 activation stays the only publish path to live `node_modules`.
- ADR-0004/0009 CAS + unpacked cache stay immutable inputs; outputs are new objects.
- ADR-0013 architecture retention stands: no FUSE/overlayfs/daemon.

## Minimal safe experiment (Phase 6)

**Implement now (non-executing):**

- An **execution plan** builder that classifies packages and emits what *would*
  run under ADR-0018 policy, without spawning scripts.
- Surface via library API + unit tests (and doctor info finding).

**Do not implement yet:**

- `bwrap` invocation
- CAS seal of outputs
- `weave switch --with-exec`

**Next implementation step (Phase 7 candidate):**

1. Add `execution` config schema (disabled by default).
2. Implement `weave exec plan` CLI printing the dry-run plan.
3. Spike `bwrap` offline `install` on a single allowlisted fixture package
   (`prefer_copy` work dir), seal outputs to CAS, wire one smoke test behind
   `WEAVE_EXEC_TESTS=1`.

## Open questions (non-blocking)

1. Overlay format vs full replacement tarball for outputs.
2. Whether `prepare` scripts for git/file deps are ever in allowlist.
3. musl detection robustness on exotic distros.

## References

- ADR-0012 lifecycle detect-only
- `docs/lifecycle.md`, `docs/lifecycle-classification.md`, `docs/native.md`
- WEAVE.md § lifecycle / Q1
- Phase 5 report: next problem statement

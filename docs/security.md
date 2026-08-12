# Security notes for Weave 0.x

## Threat model (short)

Weave treats package install scripts as untrusted by default. The production path
is **extract tarballs → materialize → activate**. Execution and prebuild network
are opt-in, narrowly scoped, and fail closed.

## Guarantees

1. Plain `weave switch` does not execute lifecycle scripts.
2. Plain `weave switch` does not grant install scripts general network access.
3. `execution.enabled` is version-controlled config only — env vars cannot enable it.
4. Dual gate: config enablement **and** `--with-exec` are both required.
5. Prebuild fetches require HTTPS, exact `allow_hosts`, and SRI; redirects re-checked.
6. Sealed outputs apply onto the candidate only; activation is transactional.
7. Live `node_modules` is never used as an execution work tree.
8. `weave exec hash-artifact` refuses symlinks; hashing is measurement, not trust.
9. Policy pack / suggest `--write` never sets `enabled=true` or `profile=open`.

## Installing release binaries

Always verify `SHA256SUMS` (and preferably GitHub attestation) before running a
downloaded `weave` binary. Prefer `scripts/install-from-dist.sh`, which checks
checksums and installs mode `0755` without setuid.

## Dependabot / dependency alerts

Weave’s shipped surface is Rust (`Cargo.lock`). Thousands of GitHub Dependabot
alerts on this repo historically came from **pinned npm corpus and test
fixtures**, not from Weave runtime deps. Policy, dismiss tooling, and required
auto-triage follow-ups: [`docs/dependabot.md`](./dependabot.md).

## Reporting issues

Report security bugs privately to the repository maintainers. Do not file public
issues that include exploit PoCs against third-party systems.

# Phase 8 report: Execution Output Integration

Date: 2026-08-11  
Follows ADR-0018 and Phase 7 design. Architecture unchanged (no FUSE/overlayfs/daemon/resolver/Docker).

## What landed

1. **Dual gate** — `weave switch --with-exec` (and `materialize --with-exec`) require both
   `[execution] enabled = true` and the CLI flag. Plain `weave switch` never executes,
   even when config is enabled. Env vars still cannot enable execution.
2. **Candidate-only integration** — after materialize into `.weave/candidate`, allowlisted
   packages get sealed outputs applied there; activation still transactional.
3. **CAS/cache path** — identity (`weave-exec-v1`: OS/CPU/Node ABI + profile + scripts +
   declared outputs + input digest excluding declared paths) → cache index → apply
   sealed artifact. Hits skip Bubblewrap entirely.
4. **Miss path** — offline bwrap against a copy of the candidate package (never live
   `node_modules`), seal declared outputs, apply onto candidate, then activate.
5. **Fail closed** — exec/sandbox/seal failures abort before `activate_candidate`; active
   `node_modules` stays untouched.
6. **Undeclared rejection** — applying a seal that contains any non-declared path fails;
   those files never enter the candidate/activated tree.
7. **Network** — still `--unshare-net` only; no open-network profile.

## Tests

| Scenario | Coverage |
|----------|----------|
| execution disabled + `--with-exec` | `with_exec_rejected_when_config_disabled` |
| config on, flag absent | `plain_switch_stays_execution_free_even_when_config_enabled` |
| first execution + activation | `first_execution_and_declared_output_activation` (`WEAVE_EXEC_TESTS=1`) |
| CAS/cache hit (no re-exec) | `cache_hit_applies_declared_output_without_bwrap` (+ gated second switch) |
| declared output activation | same gated test; live `node_modules/.../generated/hello.txt` |
| failed execution rollback | `failed_execution_leaves_active_untouched`; gated `failed_script_leaves_active_untouched` |
| undeclared output rejection | `undeclared_output_never_enters_package_dir` + unit apply tests |
| platform/ABI mismatch | `platform_abi_cache_mismatch_does_not_hit` + `platform_abi_mismatch_fails_verify` |

Gated: `WEAVE_EXEC_TESTS=1 cargo test -p weave-engine --test exec_integrate -- --ignored`

## What this proves

Opt-in sandboxed install outputs can be reused from CAS and transactionally activated into
`node_modules` without ever executing against the live tree, without running on plain
`switch`, and without admitting undeclared files.

## Next smallest problem

Allowlists and per-package `declared_outputs` still require hand-authored config; real
native packages need a practical way to declare outputs (and eventually a prebuild-fetch
profile) without opening arbitrary lifecycle execution.

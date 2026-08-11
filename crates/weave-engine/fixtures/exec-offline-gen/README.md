# Offline sandboxed execution fixture (`exec-gen`)

Tightly controlled Phase 7 allowlist target:

- Install script writes only `generated/hello.txt`
- No network, no native toolchain
- Intended to run under Bubblewrap via `weave exec run`

Declared output in project config:

```toml
[execution.declared_outputs]
exec-gen = ["generated/hello.txt"]
```

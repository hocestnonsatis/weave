# Supported platforms & requirements (0.x)

## Supported (production path)

| Component | Requirement |
|-----------|-------------|
| OS / CPU | Linux x86_64 (`x86_64-unknown-linux-gnu`) |
| Filesystem | POSIX; hardlink preferred, copy fallback across devices |
| Rust (build) | 1.75+ (CI uses stable) |
| Git | Required for project discovery |
| Node.js | Any current LTS sufficient to *run* the app after switch |
| Package lock | npm `package-lock.json` lockfileVersion **2 or 3** |
| Network (default switch) | Only to fetch registry tarballs into CAS when not cached; **no** lifecycle script network |

## Experimental

| Feature | Extra requirements |
|---------|-------------------|
| `weave switch --with-exec` | `execution.enabled=true` in `.weave/config.toml` + Bubblewrap (`bwrap`) |
| `execution.profile=prebuild-fetch` | Explicit `allow_hosts` + `fetches[]` with HTTPS + SRI |

## Unsupported (by design in 0.x)

- Windows exec sandbox / bin shims
- yarn.lock / pnpm-lock.yaml as first-class inputs
- FUSE, overlayfs, background daemon
- Open lifecycle networking (`profile=open`)
- Inventing SRI, URLs, outputs, or permissions
- Enabling execution via environment variables

## Security baseline

See [`docs/security.md`](./security.md).

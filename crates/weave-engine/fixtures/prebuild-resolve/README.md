# Prebuild resolution fixtures (Phase 11)

Static metadata mimicking real native download packages.
Discovery/resolution never executes these scripts and never contacts the network.

| Fixture | Pattern | Expected status |
|---------|---------|-----------------|
| `node-pre-gyp-like` | package.json `binary` (bcrypt-style) | NeedsIntegrity / UnresolvedTokens |
| `prebuild-install-like` | `prebuild-install` + `prebuilds/` | Opaque (no concrete URL) |
| `esbuild-download` | HTTPS literal in `install.js` | Opaque (archive → no seal path) |
| `author-sri` | `weave.prebuildFetches` + SRI | Suggestable (review only) |
| `sharp-like` | known sharp install heuristic | Opaque (dynamic vendor URL) |

Discovered URLs are never auto-approved. Suggestable drafts still require human
review, `profile = "prebuild-fetch"`, and the dual gate before any fetch.

# Adoption fixtures (Phase 12)

Representative mini-projects for clone/init → prepare → switch → run / fail-clearly.

| Fixture | Mode | Expectation |
|---------|------|-------------|
| `extraction-only` | JS deps only | ExtractionReady; no `[execution]` needed |
| `native-incomplete` | install script, no `.node` in tarball | PartialNeedsPolicy; clear doctor/plan gaps |
| `unsafe-lifecycle` | curl-like install | Blocked / never allowlisted |

Scripts in these fixtures are never executed by Weave discovery or tests.

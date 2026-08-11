# Weave Phase 2 benchmark report

- host: `linux` / `x86_64`
- suite: `all-offline`

| scenario | wall_ms | disk_bytes | files | inodes |
|---|---:|---:|---:|---:|
| tiny::weave-cold | 4 | 690 | 12 | 25 |
| tiny::weave-warm | 4 | 162 | 4 | 7 |
| tiny::weave-switch-a-to-b | 3 | 240 | 6 | 10 |
| tiny::weave-switch-b-to-a | 3 | 162 | 4 | 7 |
| small::weave-cold | 5 | 13696 | 250 | 401 |
| small::weave-warm | 4 | 3680 | 100 | 151 |
| small::weave-switch-a-to-b | 4 | 3680 | 100 | 151 |
| small::weave-switch-b-to-a | 5 | 3680 | 100 | 151 |
| medium::weave-cold | 9 | 50144 | 960 | 1423 |
| medium::weave-warm | 6 | 14270 | 400 | 561 |
| medium::weave-switch-a-to-b | 7 | 14270 | 400 | 561 |
| medium::weave-switch-b-to-a | 6 | 14270 | 400 | 561 |
| monorepo::weave-cold | 4 | 5510 | 96 | 171 |
| monorepo::weave-warm | 4 | 1358 | 36 | 61 |
| native::weave-cold | 4 | 4673 | 76 | 137 |
| native::weave-warm | 4 | 1207 | 29 | 50 |
| native::weave-switch-a-to-b | 4 | 939 | 24 | 41 |
| native::weave-switch-b-to-a | 4 | 1207 | 29 | 50 |

## Summary

warm re-switch 4 ms; A→B switch 4 ms

## Notes

- Offline Weave scenarios use synthetic `FileArtifactSource` fixtures.
- Apparent disk size does not account for hardlink sharing across trees.

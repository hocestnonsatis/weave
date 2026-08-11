# Policy packs
#
# Version-controlled, reviewed fragments for `execution.prebuild` (+ optional
# allow_packages / declared_outputs). They are **not** auto-applied and never
# enable execution.
#
# ```bash
# weave exec apply-pack policy-packs/example-demo.toml          # dry-run
# weave exec apply-pack policy-packs/example-demo.toml --write  # merge only
# # then human enables dual gate after review
# ```
#
# Pack schema: `version = 1`, required `id`, optional description, prebuild
# block with HTTPS + SRI + exact hosts. See `docs/adoption.md`.

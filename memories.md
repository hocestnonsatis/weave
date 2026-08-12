# Weave session memory

## Proje Durumu
- Kaynak sözleşme: `WEAVE.md`
- Tamamlanan: M0–M10, Phase 2–18; **Phase 19 zero-friction adoption**
- Repo: `https://github.com/hocestnonsatis/weave` (main) — do not rename
- Tag: `v0.1.0` — **draft** (not published)
- Phase 18: agent workflow (`--json`, `--owner`, env remove/prune); ADR-0018
- Phase 19: idempotent init, `guide`, `recover`, status.next_steps, clear pnpm/yarn refuse; ADR-0019; `docs/agent-quickstart.md`; `weave-bench phase19`
- Verdict: YES as agent dependency substrate for extraction-ready npm lockfile projects via guide/JSON — not automatic, not silent npm replacement
- **Post-0.1 mode active** (ADR-0020): no autonomous new feature phases
- Güncelleme: 2026-08-12

## Mimari Kararlar
- Dual gate unchanged; experimental exec never silently enabled
- Phase 16–17: offline vs network never mixed
- Phase 18: one agent = one project root; owner caller-supplied
- Phase 19: prefer friction fixes over automation; init idempotent; recover ≠ gc
- **Post-0.1 (ADR-0020):** classify before coding; cats 1–5 with evidence OK; cat 6 (new feature) = design report + explicit approval only; prefer delete complexity; no autonomous feature phases
- Cursor: `.cursor/rules/post-0.1-development.mdc` supersedes auto next-milestone for features

## Tercihler
- Extraction-first public messaging
- Agents start at `weave guide --json` / docs/agent-quickstart.md
- Fail closed on dual-gate / unsupported lockfiles / HTTP
- Do not auto-publish; human reviews draft first
- Do not invent Phase 20+ features without approved design/evidence

## Bilinen Kısıtlar / Blockers
- Draft awaiting human publish
- Native SRI human-supplied; Linux+bwrap for exec; npm lockfile only
- Yarn/pnpm/Bun unsupported (intentional)
- Network cold weave slower than npm/pnpm (expected)

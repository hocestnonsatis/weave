# Weave session memory

## Proje Durumu
- Kaynak sözleşme: `WEAVE.md`
- Tamamlanan: M0–M10, Phase 2–13, **Phase 14 0.x Release Engineering**
- RC: `0.1.0` via `scripts/release-build.sh` → `dist/`
- Güncelleme: 2026-08-12

## Mimari Kararlar
- Dual gate unchanged; experimental exec never silently enabled
- Release: `--locked` + `SOURCE_DATE_EPOCH` + SHA256/SRI + BUILDINFO
- CI attest-build-provenance on push/tag release workflows

## Tercihler
- Extraction-first public messaging
- Verify checksums before install
- Fail closed on dual-gate / open profile / HTTP

## Bilinen Kısıtlar / Blockers
- Safe 0.x public release: **no blockers** (documented boundaries remain)
- Native SRI human-supplied; Linux+bwrap for exec; npm lockfile only

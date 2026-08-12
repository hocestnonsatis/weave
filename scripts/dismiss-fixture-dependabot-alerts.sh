#!/usr/bin/env bash
# Dismiss Dependabot alerts that target pinned corpus / test fixtures only.
# Never dismisses Cargo or GitHub Actions alerts.
set -euo pipefail

OWNER="${OWNER:-hocestnonsatis}"
REPO="${REPO:-weave}"
COMMENT="${COMMENT:-Pinned corpus/fixture lockfile — not a Weave runtime dependency. See docs/dependabot.md.}"
DRY_RUN="${DRY_RUN:-0}"
# Parallel PATCH workers (keep modest to avoid secondary rate limits).
JOBS="${JOBS:-6}"

is_fixture_manifest() {
  local path="$1"
  case "$path" in
    benchmarks/corpus/*) return 0 ;;
    crates/weave-lockfile/fixtures/*) return 0 ;;
    crates/weave-engine/fixtures/*) return 0 ;;
    benchmarks/fixtures/*) return 0 ;;
    *) return 1 ;;
  esac
}

echo "Listing open Dependabot alerts for ${OWNER}/${REPO}…"
mapfile -t ALERT_JSON < <(
  gh api --paginate \
    -H "Accept: application/vnd.github+json" \
    "repos/${OWNER}/${REPO}/dependabot/alerts?state=open&per_page=100" \
    --jq '.[] | [.number, .dependency.manifest_path, .dependency.package.ecosystem] | @tsv'
)

total=${#ALERT_JSON[@]}
dismiss=()
keep=0
for row in "${ALERT_JSON[@]}"; do
  [[ -z "$row" ]] && continue
  number="${row%%$'\t'*}"
  rest="${row#*$'\t'}"
  path="${rest%%$'\t'*}"
  eco="${rest##*$'\t'}"
  if is_fixture_manifest "$path"; then
    dismiss+=("$number")
  else
    keep=$((keep + 1))
    echo "KEEP  #${number}  ${eco}  ${path}"
  fi
done

echo "Open alerts: ${total}"
echo "Fixture/corpus dismiss candidates: ${#dismiss[@]}"
echo "Kept (production/build surface): ${keep}"

if [[ ${#dismiss[@]} -eq 0 ]]; then
  echo "Nothing to dismiss."
  exit 0
fi

if [[ "$DRY_RUN" == "1" ]]; then
  echo "DRY_RUN=1 — would dismiss ${#dismiss[@]} alerts."
  printf '%s\n' "${dismiss[@]}" | head -20
  exit 0
fi

dismiss_one() {
  local number="$1"
  local tries=0
  while true; do
    tries=$((tries + 1))
    if gh api --method PATCH \
      -H "Accept: application/vnd.github+json" \
      "repos/${OWNER}/${REPO}/dependabot/alerts/${number}" \
      -f state=dismissed \
      -f dismissed_reason=not_used \
      -f dismissed_comment="${COMMENT}" \
      >/dev/null; then
      echo "DISMISSED #${number}"
      return 0
    fi
    if [[ $tries -ge 5 ]]; then
      echo "FAILED #${number}" >&2
      return 1
    fi
    sleep $((tries * 2))
  done
}

export -f dismiss_one
export OWNER REPO COMMENT

printf '%s\n' "${dismiss[@]}" | xargs -P "${JOBS}" -n 1 bash -c 'dismiss_one "$1"' _

echo "Done. Re-count with:"
echo "  gh api graphql -f query='query { repository(owner:\"${OWNER}\", name:\"${REPO}\") { vulnerabilityAlerts(states:OPEN) { totalCount } } }'"

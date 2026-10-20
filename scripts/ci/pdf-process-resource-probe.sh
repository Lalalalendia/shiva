#!/usr/bin/env bash
# CI-only OS subprocess resource evidence. Not a permission boundary for
# arbitrary documents and not a production Shiva worker configuration.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != "true" || "${RUNNER_OS:-}" != "Linux" ]]; then
  echo "SHIVA_PDF_QA_REQUIRES_GITHUB_LINUX" >&2
  exit 2
fi
if [[ $# -ne 1 || ! -x "$1" ]]; then
  echo "SHIVA_PDF_QA_BINARY_UNAVAILABLE" >&2
  exit 2
fi
for command in prlimit timeout awk mktemp; do
  command -v "$command" >/dev/null || { echo "SHIVA_PDF_QA_MISSING_TOOL" >&2; exit 2; }
done
if [[ ! -x /usr/bin/time ]]; then
  echo "SHIVA_PDF_QA_TIME_BINARY_MISSING" >&2
  exit 2
fi
binary="$1"
# Bounds apply to the separate child process; cargo/build/runner are outside.
# Fixed QA acceptance numbers, NOT production tuning recommendations:
# 3-GiB virtual address space, 20 CPU seconds, 30 wall seconds and 1-GiB RSS.
limit_as=$((3 * 1024 * 1024 * 1024))
limit_rss_kb=$((1024 * 1024))
run_case() {
  local scenario="$1"
  local metrics
  metrics="$(mktemp)"
  if ! /usr/bin/time -f '%M' -o "$metrics"       timeout --signal=TERM --kill-after=3s 30s       prlimit --as="$limit_as" --cpu=20 -- "$binary" "$scenario"; then
    rm -f "$metrics"
    echo "SHIVA_PDF_QA_CHILD_OR_LIMIT_FAILED" >&2
    return 1
  fi
  local rss_kb
  rss_kb="$(tail -n 1 "$metrics")"
  rm -f "$metrics"
  if [[ ! "$rss_kb" =~ ^[0-9]+$ ]]; then
    echo "SHIVA_PDF_QA_INVALID_RSS_EVIDENCE" >&2
    return 1
  fi
  if (( rss_kb > limit_rss_kb )); then
    echo "SHIVA_PDF_QA_RSS_LIMIT_EXCEEDED" >&2
    return 1
  fi
  printf 'SHIVA_PDF_QA_%s_MAX_RSS_KB=%s\n' "$scenario" "$rss_kb"
}
run_case smoke
run_case cumulative
echo "SHIVA_PDF_QA_OS_RESOURCE_BOUNDARY=pass"

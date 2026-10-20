#!/usr/bin/env bash
# Synthetic PDF process-resource CI. Never a production parser or sandbox.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != "true" || "${RUNNER_OS:-}" != "Linux" ]]; then
  echo "SHIVA_PDF_QA_REQUIRES_GITHUB_LINUX" >&2
  exit 2
fi
if [[ $# -ne 1 || ! -x "$1" ]]; then
  echo "SHIVA_PDF_QA_BINARY_UNAVAILABLE" >&2
  exit 2
fi
for tool_name in prlimit timeout mktemp; do
  command -v "$tool_name" >/dev/null || { echo "SHIVA_PDF_QA_MISSING_TOOL" >&2; exit 2; }
done
if [[ ! -x /usr/bin/time ]]; then
  echo "SHIVA_PDF_QA_TIME_BINARY_MISSING" >&2
  exit 2
fi
binary="$1"
ulimit -c 0
# Linux kernel address-space and CPU ceilings for the QA child only.
# RSS is a post-exit observation bound, not a hard physical-memory RLIMIT.
limit_as=$((3 * 1024 * 1024 * 1024))
limit_rss_kb=$((1024 * 1024))
run_case() {
  local scenario="$1"
  local metrics
  local stdout_text
  metrics="$(mktemp)"
  # Never expose an unreviewed child stdout to the CI log.
  if ! stdout_text="$(/usr/bin/time -f '%M' -o "$metrics" timeout --signal=TERM --kill-after=3s 30s prlimit --as="$limit_as" --cpu=20 -- "$binary" "$scenario")"; then
    rm -f "$metrics"
    echo "SHIVA_PDF_QA_CHILD_OR_LIMIT_FAILED" >&2
    return 1
  fi
  if [[ "$stdout_text" != "SHIVA_PDF_QA_CHILD_OK" ]]; then
    rm -f "$metrics"
    echo "SHIVA_PDF_QA_UNEXPECTED_CHILD_STDOUT" >&2
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
  echo "SHIVA_PDF_QA_${scenario}_MAX_RSS_KB=$rss_kb"
}
run_case smoke
run_case cumulative
echo "SHIVA_PDF_QA_OS_RESOURCE_BOUNDARY=pass"

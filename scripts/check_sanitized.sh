#!/usr/bin/env bash
# Fail if a tracked file matches a pattern from .github/sanitize-patterns.txt.
set -euo pipefail
export LC_ALL=C
cd "$(git rev-parse --show-toplevel)"
patterns=.github/sanitize-patterns.txt
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

# Translate the pattern file's leading and trailing word boundaries to POSIX ERE.
sed -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' \
    -e 's/^\\b/(^|[^[:alnum:]_])/' \
    -e 's/\\b$/([^[:alnum:]_]|$)/' "$patterns" > "$tmp"
if [[ ! -s "$tmp" ]]; then
  echo "sanitization guard: no scan patterns" >&2
  exit 2
fi

scan() {
  local rc=0
  git grep -z -nEI -f "$tmp" -- . ":(exclude)$patterns" ":(exclude)scripts/check_sanitized.sh" || rc=$?
  return "$rc"
}

redact() {
  local file line discarded
  while IFS= read -r -d '' file; do
    IFS= read -r -d '' line || return 2
    IFS= read -r discarded || return 2
    [[ "$line" =~ ^[0-9]+$ ]] || return 2
    printf '%q:%s\n' "$file" "$line"
  done
}

if scan | redact; then
  statuses=("${PIPESTATUS[@]}")
else
  statuses=("${PIPESTATUS[@]}")
fi
if [[ "${statuses[1]}" -ne 0 || "${statuses[0]}" -ge 2 ]]; then
  echo "sanitization guard: scan failed" >&2
  exit 2
fi
if [[ "${statuses[0]}" -eq 0 ]]; then
  echo "sanitization guard: forbidden pattern found" >&2
  exit 1
fi
echo "sanitization guard: clean"

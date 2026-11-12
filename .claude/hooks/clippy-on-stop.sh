#!/usr/bin/env bash
# Stop hook: format and lint when Rust files have uncommitted changes.
set -u

input=$(cat)

# A stop that follows a blocked one passes, so a failing lint cannot loop.
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && exit 0

cd "$(jq -r '.cwd // "."' <<<"$input")" || exit 0
git status --porcelain -- '*.rs' | grep -q . || exit 0

cargo fmt --all >/dev/null 2>&1

# Same invocation as the CI lint job.
if ! out=$(cargo clippy --all-targets -- -D warnings 2>&1); then
  echo "cargo clippy failed; fix these before finishing:" >&2
  tail -n 60 <<<"$out" >&2
  exit 2
fi

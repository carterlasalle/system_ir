#!/bin/bash
# Atlas-aware ground-truth verifier.
# A key passes if ANY of:
#  V1. verbatim in repo source (git grep -F), after stripping backticks
#  V2. "METHOD /path" route form -> the /path part is verbatim in source
#  V3. "Type.method" symbol form -> both "Type" and "method" (and "Type." prefix
#      of a call site like `srv.serve` or `router.GET(`) appear in source
#  V4. path-like key (file or dir) exists under the repo
# Flow-chain keys: the FIRST step is checked; chains are explanatory.
# Alternative notation "A / B": first alternative is checked.
# Usage: verify_gt.sh <ground-truth-dir> <corpus-dir>
GT="$(cd "$1" && pwd)"; CORPUS="$(cd "$2" && pwd)"
total=0; fail=0; : > /tmp/gt_misses.txt

key_ok() { # $1=key $2=repodir
  local key="$1" repo="$2" path t
  key="${key//\`/}"
  # alternatives: "A / B" -> first (only when slash is surrounded by spaces)
  key="$(printf '%s' "$key" | sed 's| * / *| / |' | cut -d'/' -f1 | sed 's| *$||')"
  # flow chains: first segment
  key="$(printf '%s' "$key" | sed 's| -> .*||')"
  # V4 path-like (file OR dir)
  if printf '%s' "$key" | grep -qE '(\.(py|ts|tsx|js|mjs|cjs|rs|go|java|kt|json|md|yaml|yml|toml|cabal|mod|sum|proto|sh|xml|properties|gradle|sql|css|svelte|html|prisma|env)|/)$'; then
    [ -e "$repo/$key" ] && return 0
  fi
  # V1 verbatim
  git -C "$repo" grep -qF -- "$key" 2>/dev/null && return 0
  # V2 route form "METHOD /path" or bare "/path"
  if printf '%s' "$key" | grep -qE '^(GET|POST|PUT|DELETE|PATCH|OPTIONS|HEAD|ANY|route) /|^/'; then
    local p="${key#* }"
    [ "$p" = "$key" ] && p="$key"
    [ -z "$p" ] && p="$key"
    git -C "$repo" grep -qF -- "$p" 2>/dev/null && return 0
  fi
  # V3 Type.method / Type::method: verify both parts and a dot call site
  local dot="$(printf '%s' "$key" | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)[.:]\([A-Za-z_][A-Za-z0-9_]*\)$/\1.\2/p')"
  if [ -n "$dot" ]; then
    local t="${dot%%.*}" m="${dot##*.}"
    git -C "$repo" grep -qF -- "$t" 2>/dev/null && git -C "$repo" grep -qF -- "$m" 2>/dev/null && return 0
  fi
  # V5 CLI flag "--flag" / "-f": the flag name (dashes stripped) appears in
  # source (covers clap-derive style `long = "paging"` definitions where the
  # literal "--paging" never appears)
  if printf '%s' "$key" | grep -qE '^--?[a-zA-Z][a-zA-Z0-9-]*$'; then
    local flag="${key#-}"; flag="${flag#-}"
    git -C "$repo" grep -qF -- "$flag" 2>/dev/null && return 0
  fi
  return 1
}

for doc in "$GT"/*.md; do
  repo="$(basename "$doc" .md)"
  [ -d "$CORPUS/$repo" ] || { echo "NO REPO: $repo"; continue; }
  while IFS= read -r line; do
    total=$((total+1))
    key="${line#- }"
    key="$(printf '%s' "$key" | sed 's| *— .*||; s| *- .*||')"
    if ! key_ok "$key" "$CORPUS/$repo"; then
      echo "MISS[$repo]: $key" >> /tmp/gt_misses.txt
      fail=$((fail+1))
    fi
  done < <(grep '^- ' "$doc")
done
echo "=== total keys checked: $total; misses: $fail ==="
cat /tmp/gt_misses.txt

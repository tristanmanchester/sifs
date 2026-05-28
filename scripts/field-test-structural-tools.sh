#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: $0 /path/to/repo [/path/to/repo ...]" >&2
  exit 2
fi

if [[ -n "${SIFS_BIN:-}" ]]; then
  sifs_bin="$SIFS_BIN"
elif [[ -x "./target/release/sifs" ]]; then
  sifs_bin="./target/release/sifs"
elif [[ -x "./target/debug/sifs" ]]; then
  sifs_bin="./target/debug/sifs"
else
  sifs_bin="sifs"
fi

for repo in "$@"; do
  if [[ ! -d "$repo" ]]; then
    echo "## $repo"
    echo "missing directory"
    continue
  fi

  echo "## $repo"
  "$sifs_bin" status --source "$repo" --offline --no-cache --json \
    | jq -r '"status files=\(.index_stats.indexed_files) chunks=\(.index_stats.total_chunks) languages=\((.index_stats.languages // {}) | keys | join(",")) warnings=\((.warnings // []) | length)"'

  first_file="$("$sifs_bin" list-files --source "$repo" --offline --no-cache --limit 1 --json \
    | jq -r '.files[0] // empty')"
  if [[ -z "$first_file" ]]; then
    echo "no indexed files"
    continue
  fi

  echo "first_file=$first_file"
  "$sifs_bin" outline "$first_file" --source "$repo" --offline --no-cache --symbols-limit 8 --no-chunks --json \
    | jq -r '"outline symbols=\(.total_symbols) kinds=\([.outline.symbols[].kind] | unique | join(","))"'

  first_symbol="$("$sifs_bin" outline "$first_file" --source "$repo" --offline --no-cache --symbols-limit 1 --no-chunks --json \
    | jq -r '.outline.symbols[0].name // empty')"
  if [[ -n "$first_symbol" ]]; then
    echo "first_symbol=$first_symbol"
    "$sifs_bin" symbol "$first_symbol" --source "$repo" --offline --no-cache --limit 3 --json \
      | jq -r '"symbol total=\(.total) shown=\(.symbols | length) kinds=\([.symbols[].kind] | unique | join(","))"'
  fi

  "$sifs_bin" pack "$first_file" --source "$repo" --mode bm25 --offline --no-cache --limit 1 --include-symbol-definitions --json \
    | jq -r '"pack items=\(.items | length) symbol_terms=\(.symbol_definition_terms | join(","))"'
done

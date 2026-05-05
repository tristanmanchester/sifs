#!/usr/bin/env sh
set -eu

SIFS_BIN="${SIFS_BIN:-sifs}"

command -v "$SIFS_BIN" >/dev/null
"$SIFS_BIN" --version
"$SIFS_BIN" agent-context --json >/dev/null

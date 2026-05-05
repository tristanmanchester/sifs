#!/usr/bin/env sh
set -eu

command -v sifs >/dev/null
sifs --version
sifs agent-context --json >/dev/null

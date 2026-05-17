#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "$#" -gt 0 ]]; then
  exec cargo run -p meshmend -- "$@"
fi

if [[ -f rose/raw.stl ]]; then
  exec cargo run -p meshmend -- rose/raw.stl
fi

exec cargo run -p meshmend

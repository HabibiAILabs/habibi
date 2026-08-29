#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
destination="$root/.benchmarks/gaia"
if [[ -f "$destination/data/gaia/validation/metadata.jsonl" ]]; then
  echo "GAIA validation data already present at $destination"
  git -C "$destination" rev-parse HEAD
  exit 0
fi
mkdir -p "$root/.benchmarks"
git clone --depth 1 https://github.com/aymeric-roucher/GAIA.git "$destination"
echo "Downloaded GAIA validation data and reference scorer."
git -C "$destination" rev-parse HEAD

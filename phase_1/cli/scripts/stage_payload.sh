#!/usr/bin/env bash
# Compatibility entry point. Prebuilt artifacts and a release CLI are required.
# Includes native Dev Shells by default; set INCLUDE_DEVSHELL=0 for build-only use.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/stage_payload.py" "$@"

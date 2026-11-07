#!/usr/bin/env bash
# Native Linux package (x64 or ARM64). See README.md for prerequisites.
# ASCII_PAINTER_DIST_DIR selects an output root inside this workspace.
set -euo pipefail
cd -- "$(dirname -- "$0")"
rust_env="${CARGO_HOME:-$HOME/.cargo}/env"
if [[ -f "$rust_env" ]]; then source "$rust_env"; fi
if ! command -v python3 >/dev/null 2>&1; then
    echo 'Install Python 3.12 or newer (Ubuntu: sudo apt install python3).' >&2
    exit 1
fi
exec python3 tools/release_package.py build --platform linux \
    --destination-root "${ASCII_PAINTER_DIST_DIR:-dist}" "$@"

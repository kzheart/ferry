#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python_bin=${PYTHON:-python3.12}
if [ -n "${TAURI_TARGET_TRIPLE:-}" ]; then
  exec "$python_bin" "$repo_dir/scripts/build-sidecar.py" --clean --target "$TAURI_TARGET_TRIPLE"
fi
exec "$python_bin" "$repo_dir/scripts/build-sidecar.py" --clean

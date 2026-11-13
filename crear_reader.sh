#!/usr/bin/env bash
# Compila el lector cosmic-reader en modo release.
set -euo pipefail
cd "$(dirname "$0")"

target_dir() {
    cargo metadata --format-version 1 --no-deps \
        | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
}

cargo build --release -p cosmic-reader
echo "Listo: $(target_dir)/release/cosmic-reader"

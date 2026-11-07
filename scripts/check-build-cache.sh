#!/usr/bin/env bash
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
build_dir=${CARGO_TARGET_DIR:-"$project_root/../.rust-builds/document-signing"}
if [[ "$build_dir" != /* ]]; then
    build_dir="$project_root/$build_dir"
fi
budget_bytes=10737418240
usage_bytes=0
if [[ -d "$build_dir" ]]; then
    # Resolve a local symlink before measuring; du otherwise measures the link itself.
    build_dir=$(cd -- "$build_dir" && pwd -P)
    read -r usage_kib _ < <(du -sk "$build_dir")
    usage_bytes=$((usage_kib * 1024))
fi
printf 'document-signing build cache: %s bytes; budget: %s bytes (%s)\n' "$usage_bytes" "$budget_bytes" "$build_dir"
if (( usage_bytes > budget_bytes )); then
    printf 'OVER BUDGET: When no builds are active, run cargo clean from %s.\n' "$project_root" >&2
    exit 1
fi

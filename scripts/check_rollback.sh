#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
mode="native"
artifact=""
output=""
allow_dirty=0
self_test=0
forward=()

usage() {
    printf '%s\n' \
        "Usage: $0 [--native|--browser|--full] [--artifact DIR] [--output FILE]" \
        "          [--runtime-browser chrome|firefox] [--campaign all|matrix|soak]" \
        "          [--allow-dirty] [--self-test]" \
        "          [rollback_validation.py options]"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --native)
            mode="native"
            shift
            ;;
        --browser)
            mode="browser"
            shift
            ;;
        --full)
            mode="full"
            shift
            ;;
        --artifact)
            [ "$#" -ge 2 ] || {
                echo "--artifact needs a directory" >&2
                exit 2
            }
            artifact="$2"
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || {
                echo "--output needs a file" >&2
                exit 2
            }
            output="$2"
            shift 2
            ;;
        --runtime-browser)
            [ "$#" -ge 2 ] || {
                echo "--runtime-browser needs chrome or firefox" >&2
                exit 2
            }
            forward+=(--browser "$2")
            shift 2
            ;;
        --allow-dirty)
            allow_dirty=1
            shift
            ;;
        --self-test)
            self_test=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            forward+=("$1")
            shift
            ;;
    esac
done

if [ "$self_test" -eq 1 ]; then
    python3 -B "$project_root/scripts/rollback_ci.py" self-test
    exec python3 -B "$project_root/scripts/rollback_validation.py" --self-test
fi

if [ -z "$output" ]; then
    evidence_root="$(mktemp -d "${TMPDIR:-/tmp}/galactic-cup-omp2-rollback.XXXXXX")"
    output="$evidence_root/omp2_rollback.json"
fi

if [ "$mode" = "browser" ] || [ "$mode" = "full" ]; then
    if [ -z "$artifact" ]; then
        artifact_root="$(mktemp -d "${TMPDIR:-/tmp}/galactic-cup-omp2-web.XXXXXX")"
        artifact="$artifact_root/artifact"
        "$project_root/scripts/web_build.sh" "$artifact"
    fi
fi

command=(
    python3
    -B
    "$project_root/scripts/rollback_validation.py"
    --mode
    "$mode"
    --output
    "$output"
)
if [ -n "$artifact" ]; then
    command+=(--artifact "$artifact")
fi
if [ "$allow_dirty" -eq 1 ]; then
    command+=(--allow-dirty)
fi
command+=("${forward[@]}")

"${command[@]}"
printf 'OMP-2 rollback evidence: %s\n' "$output"

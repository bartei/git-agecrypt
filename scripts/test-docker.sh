#!/usr/bin/env bash
# Run the full test suite + coverage inside a sandboxed Docker container.
#
# Output ends up in ./coverage/ on the host:
#   coverage/html/index.html   browseable line-by-line coverage
#   coverage/lcov.info         machine-readable coverage report
#   coverage/summary.txt       per-file coverage table
#   coverage/test-output.txt   raw test runner output
#
# Usage: scripts/test-docker.sh [extra docker-build args...]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

IMAGE="${IMAGE:-git-agecrypt-test:latest}"
COVERAGE_DIR="${COVERAGE_DIR:-$REPO_ROOT/coverage}"

echo ">>> building image $IMAGE"
docker build "$@" -t "$IMAGE" -f Dockerfile.test .

mkdir -p "$COVERAGE_DIR"
# Wipe any previous artifacts so a stale report doesn't masquerade as the
# current run's output.
rm -rf "$COVERAGE_DIR"/*

echo ">>> running tests + coverage in container"
# --rm    : ephemeral container; nothing persists outside the volume mount
# --user  : writes are owned by the invoking user, not by root
# Using -v with absolute paths keeps the bind-mount portable.
docker run --rm \
    --user "$(id -u):$(id -g)" \
    -v "$COVERAGE_DIR:/out" \
    "$IMAGE"

echo ""
echo ">>> coverage artifacts:"
echo "    summary:  $COVERAGE_DIR/summary.txt"
echo "    html:     $COVERAGE_DIR/html/index.html"
echo "    lcov:     $COVERAGE_DIR/lcov.info"

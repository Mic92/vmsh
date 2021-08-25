#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"

TMPDIR="$(mktemp -d)"
trap 'rm -rf -- "$TMPDIR"' EXIT

export PTS_USER_PATH_OVERRIDE=$TMPDIR/phoronix-test-suite/
export PTS_DOWNLOAD_CACHE_OVERRIDE=$TMPDIR/phoronix-test-suite/download-cache/
mkdir -p "$PTS_DOWNLOAD_CACHE_OVERRIDE"

cd "$SCRIPT_DIR/.."
nix build .#phoronix-test-suite --out-link .git/nix-results/phoronix-test-suite
set +o pipefail
yes | .git/nix-results/phoronix-test-suite/bin/phoronix-test-suite make-download-cache pts/disk
set -o pipefail
rm "$PTS_USER_PATH_OVERRIDE/core.pt2so"
tar -C "$TMPDIR" --owner=0 --group=0 -czf target/phoronix.tar.gz phoronix-test-suite
echo "build target/phoronix.tar.gz"

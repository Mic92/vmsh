#!/usr/bin/env nix-shell
#! nix-shell -i bash -p bash -p coreutils -p gh
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ASSET="$SCRIPT_DIR/../target/phoronix.tar.gz"
printf -v date '%(%Y-%m-%d)T\n' -1
UPLOAD_NAME="$SCRIPT_DIR/../target/phoronix-${date}.tar.gz"

if [[ ! -f "$ASSET" ]]; then
    echo "$ASSET does not exists. Run build-phoronix-cache.sh first"
    exit 1
fi
ln -f "$ASSET" "$UPLOAD_NAME"
gh release upload assets --clobber "$UPLOAD_NAME"

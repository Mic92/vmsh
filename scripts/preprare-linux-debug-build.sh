#!/usr/bin/env bash
set -euo pipefail

cd ../../linux

make x86_64_defconfig
make kvm_guest.config
scripts/config --set-val DEBUG_INFO y
scripts/config --set-val DEBUG y
scripts/config --set-val GDB_SCRIPTS y
scripts/config --set-val DEBUG_DRIVER y

# does not link
#scripts/config --set-val CONFIG_EFIVAR_FS n

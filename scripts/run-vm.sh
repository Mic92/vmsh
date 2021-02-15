#!/usr/bin/env bash
set -euo pipefail

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"

qemu-system-x86_64 -s -S  \
  -kernel ${DIR}/../../linux/arch/x86/boot/bzImage \
  -hda ${DIR}/../../linux/nixos.qcow2 \
  -append "root=/dev/sda console=ttyS0" \
  -m 512M \
  -nographic -enable-kvm
  #-append "root=/dev/sda console=ttyS0 nokalsr" \

# -mem-prealloc \
# -mem-path /dev/hugepages/qemu \

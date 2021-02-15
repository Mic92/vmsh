#!/usr/bin/env bash
set -euo pipefail

qemu-system-x86_64 -s -S  \
  -kernel ../../linux/arch/x86/boot/bzImage \
  -hda ../../linux/nixos.qcow2 \
  -append "root=/dev/sda console=ttyS0" \
  -m 512M \
  -nographic -enable-kvm
  #-append "root=/dev/sda console=ttyS0 nokalsr" \

# -mem-prealloc \
# -mem-path /dev/hugepages/qemu \

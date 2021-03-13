#!/bin/sh
#mkdir /mnt
#mount -t 9p -o trans=virtio home /mnt

linux_dir=../linux

qemu-system-x86_64 \
  -kernel $linux_dir/arch/x86/boot/bzImage \
  -hda $linux_dir/nixos_nested.qcow2 \
  -append "root=/dev/sda console=ttyS0 nokaslr" \
  -m 256M \
  -nographic -enable-kvm

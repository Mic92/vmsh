# Local Variables:
# mode: makefile
# End:
# vim: set ft=make :

linux_dir := invocation_directory() + "/../linux"

kernel_fhs := `nix-build --no-out-link nix/kernel-fhs.nix` + "/bin/linux-kernel-build"

clone-linux:
  [[ -d {{linux_dir}} ]] || \
    git clone https://github.com/torvalds/linux {{linux_dir}}

configure-linux: clone-linux
  #!/usr/bin/env bash
  set -euxo pipefail
  if [[ ! -f {{linux_dir}}/.config ]]; then
    cd {{linux_dir}}
    {{kernel_fhs}} "make x86_64_defconfig"
    {{kernel_fhs}} "make kvm_guest.config"
    {{kernel_fhs}} "yes \n | scripts/config --set-val DEBUG_INFO y"
    {{kernel_fhs}} "yes \n | scripts/config --set-val DEBUG y"
    {{kernel_fhs}} "yes \n | scripts/config --set-val GDB_SCRIPTS y"
    {{kernel_fhs}} "yes \n | scripts/config --set-val DEBUG_DRIVER y"
  fi

build-linux-shell:
  nix-shell {{invocation_directory()}}/nix/fhs-shell.nix

build-linux: configure-linux
  {{kernel_fhs}} "yes \n | make -C {{linux_dir}} -j$(nproc)"

nixos-image:
  [[ {{linux_dir}}/nixos.qcow2 -nt nix/nixos-image.nix ]] || \
  [[ {{linux_dir}}/nixos.qcow2 -nt nix/sources.json ]] || \
  install -m600 "$(nix-build --no-out-link nix/nixos-image.nix)/nixos.qcow2" {{linux_dir}}/nixos.qcow2

qemu: build-linux nixos-image
  qemu-system-x86_64 \
    -kernel {{linux_dir}}/arch/x86/boot/bzImage \
    -hda {{linux_dir}}/nixos.qcow2 \
    -append "root=/dev/sda console=ttyS0" \
    -m 512M \
    -nographic -enable-kvm

inspect-qemu:
  cargo run -- inspect "$(pidof qemu-system-x86_64)"

coredump-qemu:
  cargo run -- coredump "$(pidof qemu-system-x86_64)"

trace-qemu:
  perf trace "$(pidof qemu-system-x86_64)"

capsh:
  @ if [ -n "${IN_CAPSH:-}" ]; then \
    echo "you are already in a capsh session"; exit 1; \
  else \
    true; \
  fi
  sudo modprobe kheaders || true
  sudo chown -R $(id -u) /sys/kernel/debug/
  trap "sudo chown -R 0 /sys/kernel/debug" EXIT && \
  sudo -E IN_CAPSH=1 \
      capsh \
      --caps="cap_sys_ptrace,cap_sys_admin,cap_sys_resource+epi cap_setpcap,cap_setuid,cap_setgid+ep" \
      --keep=1 \
      --groups=$(id -G | sed -e 's/ /,/g') \
      --gid=$(id -g) \
      --uid=$(id -u) \
      --addamb=cap_sys_resource \
      --addamb=cap_sys_admin \
      --addamb=cap_sys_ptrace \
      -- -c 'direnv exec "$0" "$1"' . "$SHELL"

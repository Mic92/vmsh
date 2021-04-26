# Local Variables:
# mode: makefile
# End:
# vim: set ft=make :

linux_dir := invocation_directory() + "/../linux"
linux_rev := "v5.11"

kernel_fhs := `nix-build --no-out-link nix/kernel-fhs.nix` + "/bin/linux-kernel-build"

qemu_pid := `pgrep -u $USER qemu-system | awk '{print $1}'`
qemu_ssh_port := "2222"

lint:
  flake8 tests
  black --check tests
  mypy tests
  cargo clippy
  cargo fmt -- --check

fmt:
  isort tests
  black tests
  cargo fmt

test:
  cargo test
  pytest -n $(nproc --ignore=2) -s tests

clone-linux:
  [[ -d {{linux_dir}} ]] || \
    git clone https://github.com/torvalds/linux {{linux_dir}}
  git -C {{linux_dir}} checkout {{linux_rev}}

configure-linux: clone-linux
  #!/usr/bin/env bash
  set -euxo pipefail
  if [[ ! -f {{linux_dir}}/.config ]]; then
    cd {{linux_dir}}
    {{kernel_fhs}} "make x86_64_defconfig kvm_guest.config"
    {{kernel_fhs}} "scripts/config --set-val DEBUG_INFO_DWARF5 y"
    {{kernel_fhs}} "scripts/config --set-val DEBUG y"
    {{kernel_fhs}} "scripts/config --set-val GDB_SCRIPTS y"
    {{kernel_fhs}} "scripts/config --set-val DEBUG_DRIVER y"
    {{kernel_fhs}} "scripts/config --set-val KVM y"
    {{kernel_fhs}} "scripts/config --set-val KVM_INTEL y"
    {{kernel_fhs}} "scripts/config --set-val BPF_SYSCALL y"
    {{kernel_fhs}} "scripts/config --set-val IKHEADERS y"
    {{kernel_fhs}} "scripts/config --set-val VIRTIO_MMIO m"
    {{kernel_fhs}} "scripts/config --set-val VIRTIO_MMIO_CMDLINE_DEVICES y"
    {{kernel_fhs}} "scripts/config --set-val PTDUMP_CORE y"
    {{kernel_fhs}} "scripts/config --set-val PTDUMP_DEBUGFS y"
  fi

sign-drone:
  DRONE_SERVER=https://drone.thalheim.io \
  DRONE_TOKEN=$(cat $HOME/.secret/drone-token) \
    nix-shell -p drone-cli --run 'drone sign Mic92/vmsh --save'

build-linux-shell:
  nix-shell {{invocation_directory()}}/nix/kernel-fhs-shell.nix

build-linux: configure-linux
  {{kernel_fhs}} "yes \n | make -C {{linux_dir}} -j$(nproc)"

nixos-image:
  [[ {{linux_dir}}/nixos.qcow2 -nt nix/nixos-image.nix ]] || \
  [[ {{linux_dir}}/nixos.qcow2 -nt nix/sources.json ]] || \
  install -m600 "$(nix-build --no-out-link nix/nixos-image.nix)/nixos.qcow2" {{linux_dir}}/nixos.qcow2

notos-image:
  nix-build nix/not-os-image.nix -A json

# built image for qemu_nested.sh
nested-nixos-image: nixos-image
  ln -f "{{linux_dir}}/nixos.qcow2" {{linux_dir}}/nixos-nested.qcow2

# run qemu with kernel build by `build-linux` and filesystem image build by `nixos-image`
qemu EXTRA_CMDLINE="nokalsr": build-linux nixos-image
  qemu-system-x86_64 \
    -kernel {{linux_dir}}/arch/x86/boot/bzImage \
    -hda {{linux_dir}}/nixos.qcow2 \
    -append "root=/dev/sda console=ttyS0 {{EXTRA_CMDLINE}}" \
    -net nic,netdev=user.0,model=virtio \
    -netdev user,id=user.0,hostfwd=tcp::{{qemu_ssh_port}}-:22 \
    -m 512M \
    -cpu host \
    -virtfs local,path={{invocation_directory()}}/..,security_model=none,mount_tag=home \
    -virtfs local,path={{linux_dir}},security_model=none,mount_tag=linux \
    -nographic -enable-kvm \
    -s

# SSH into vm started by `just qemu`
ssh-qemu $COMMAND="":
  ssh -i {{invocation_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      root@localhost \
      -p {{qemu_ssh_port}} "$COMMAND"

nested-qemu: nested-nixos-image
  just ssh-qemu qemu-nested

# Build debug kernel module for VM using kernel build by `just build-linux`
build-debug-kernel-mod:
  # don't invoke linux kernel build every time because it is a bit slow...
  if [[ ! -d {{linux_dir}} ]]; then just build-linux; fi
  cd {{invocation_directory()}}/tests/debug-kernel-mod && make KERNELDIR={{linux_dir}}

# Load debug kernel module into VM started by `just qemu` using ssh
load-debug-kernel-mod: build-debug-kernel-mod
  just ssh-qemu "rmmod debug-kernel-mod; insmod /mnt/vmsh/tests/debug-kernel-mod/debug-kernel-mod.ko && dmesg"

inspect-qemu:
  cargo run -- inspect "{{qemu_pid}}"

coredump-qemu:
  cargo run -- coredump "{{qemu_pid}}"

trace-qemu:
  perf trace -p "{{qemu_pid}}"

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

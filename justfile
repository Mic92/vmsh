# Local Variables:
# mode: makefile
# End:
# vim: set ft=make :

linux_dir := invocation_directory() + "/../linux"
linux_repo := "https://github.com/Mic92/linux"

kernel_fhs := `nix build --json '.#kernel-fhs' | jq -r '.[] | .outputs | .out'` + "/bin/linux-kernel-build"

virtio_blk_img := invocation_directory() + "/../linux/nixos.ext4"

qemu_pid := `pgrep -u $(id -u) qemu-system | awk '{print $1}'`
qemu_ssh_port := "2222"

# Interactively select a task from just file
default:
  @just --choose

# Linux python and rust code
lint:
  flake8 tests
  black --check tests
  mypy tests
  cargo clippy
  cargo fmt -- --check

# Format python and rust code
fmt:
  isort tests
  black tests
  cargo fmt

# Continously run clippy checks whenever the source changes
watch:
  cargo watch -x clippy

# Run unit and integration tests
test:
  cargo test
  pytest -n $(nproc --ignore=2) -s tests

# Fuzz - or rather stress test the blkdev (run `just qemu` and `just attach-qemu-img` before)
stress-test DEV="/dev/vda":
  just ssh-qemu "head -c 10 {{DEV}}"
  just ssh-qemu "tail -c 10 {{DEV}}"
  just ssh-qemu "tail -c 8192 {{DEV}}"
  just ssh-qemu "tail -c 8193 {{DEV}}"
  just ssh-qemu "tail -c 12288 {{DEV}}"
  just ssh-qemu "tail -c 12289 {{DEV}}"
  just ssh-qemu "tail -c 1000000 {{DEV}}"
  just ssh-qemu "mkdir -p /mnt2"
  just ssh-qemu "mount {{DEV}} /mnt2"
  just ssh-qemu "ls /mnt2"
  just ssh-qemu "umount /mnt2"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  just ssh-qemu "hdparm -Tt {{DEV}}"
  [ $(sha256sum {{virtio_blk_img}} | cut -c'1-64') == $(just ssh-qemu "sha256sum {{DEV}}" | cut -c'1-64') ] || echo "ok"
  echo "stress test ok"

# Git clone linux kernel
clone-linux:
  [[ -d {{linux_dir}} ]] || \
    git clone {{linux_repo}} {{linux_dir}}
  set -x; commit="$(nix eval --raw .#linux_ioregionfd.src.rev)"; \
  if [[ $(git -C {{linux_dir}} rev-parse HEAD) != "$commit" ]]; then \
     git -C {{linux_dir}} fetch {{linux_repo}} HEAD:Mic92; \
     git -C {{linux_dir}} checkout "$commit"; \
     rm -f {{linux_dir}}/.config; \
  fi

# Configure linux kernel build
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
    {{kernel_fhs}} "scripts/config --set-val IKCONFIG_PROC y"
    {{kernel_fhs}} "scripts/config --set-val VIRTIO_MMIO y"
    {{kernel_fhs}} "scripts/config --set-val DRM n"
    {{kernel_fhs}} "scripts/config --set-val PTDUMP_CORE y"
    {{kernel_fhs}} "scripts/config --set-val PTDUMP_DEBUGFS y"
  fi

# Sign drone ci configuration
sign-drone:
  DRONE_SERVER=https://drone.thalheim.io \
  DRONE_TOKEN=$(cat $HOME/.secret/drone-token) \
    nix-shell -p drone-cli --run 'drone sign Mic92/vmsh --save'

# Linux kernel development shell
build-linux-shell:
  nix develop '.#kernel-fhs-shell'

# Build linux kernel
build-linux: configure-linux
  {{kernel_fhs}} "yes \n | make -C {{linux_dir}} -j$(nproc)"

# Build kernel-less disk image for NixOS
nixos-image:
  [[ {{linux_dir}}/nixos.ext4 -nt nix/nixos-image.nix ]] || \
  [[ {{linux_dir}}/nixos.ext4 -nt flake.lock ]] || \
  (nix build --builders '' .#nixos-image --out-link nixos-image && \
  install -m600 "nixos-image/nixos.img" {{linux_dir}}/nixos.ext4)

# Build kernel/disk image for not os
notos-image:
  nix build '.#not-os-image.json'
  jq < result

# built image for qemu_nested.sh
nested-nixos-image: nixos-image
  [[ {{linux_dir}}/nixos.ext4 -nt {{linux_dir}}/nixos-nested.ext4 ]] || \
  cp --reflink=auto "{{linux_dir}}/nixos.ext4" {{linux_dir}}/nixos-nested.ext4

vmsh-image: nixos-image
  [[ {{linux_dir}}/nixos.ext4 -nt {{linux_dir}}/vmsh-image.ext4 ]] || \
  cp --reflink=auto "{{linux_dir}}/nixos.ext4" {{linux_dir}}/vmsh-image.ext4

# run qemu with kernel build by `build-linux` and filesystem image build by `nixos-image`
qemu EXTRA_CMDLINE="nokalsr": build-linux nixos-image
  qemu-system-x86_64 \
    -kernel {{linux_dir}}/arch/x86/boot/bzImage \
    -drive format=raw,file={{linux_dir}}/nixos.ext4 \
    -append "root=/dev/sda console=hvc0 {{EXTRA_CMDLINE}}" \
    -net nic,netdev=user.0,model=virtio \
    -m 512M \
    -netdev user,id=user.0,hostfwd=tcp:127.0.0.1:{{qemu_ssh_port}}-:22 \
    -cpu host \
    -virtfs local,path={{invocation_directory()}}/..,security_model=none,mount_tag=home \
    -virtfs local,path={{linux_dir}},security_model=none,mount_tag=linux \
    -nographic -serial null -enable-kvm \
    -device virtio-serial \
    -chardev stdio,mux=on,id=char0 \
    -mon chardev=char0,mode=readline \
    -device virtconsole,chardev=char0,id=vmsh,nr=0

# run qemu with filesystem/kernel from notos (same as in tests)
qemu-notos:
  #!/usr/bin/env python3
  import sys, os, subprocess
  sys.path.insert(0, os.path.join("{{invocation_directory()}}", "tests"))
  from nix import notos_image
  from qemu import qemu_command
  cmd = qemu_command(notos_image(), "qmp.sock", ssh_port={{qemu_ssh_port}})
  print(" ".join(cmd))
  subprocess.run(cmd)

# Attach gdb to vmsh
gdb:
  sudo gdb --pid $(pidof vmsh) -ex 'thread apply all bt' -ex 'info threads'

# Attach strace to vmsh
strace:
  sudo strace -p $(pidof vmsh) -f

# SSH into vm started by `just qemu`
ssh-qemu $COMMAND="":
  ssh -i {{invocation_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      root@localhost \
      -p {{qemu_ssh_port}} "$COMMAND"

# Start qemu in qemu based on nixos image
nested-qemu: nested-nixos-image
  just ssh-qemu qemu-nested

# Copy programs from the host store to the guest nix store
qemu-copy STORE_PATH:
  mkdir -p target/mnt
  sudo mount {{virtio_blk_img}} {{invocation_directory()}}/target/mnt
  sudo nix copy {{STORE_PATH}} --to {{invocation_directory()}}/target/mnt
  sudo umount {{invocation_directory()}}/target/mnt

# Build debug kernel module for VM using kernel build by `just build-linux`
build-debug-kernel-mod:
  # don't invoke linux kernel build every time because it is a bit slow...
  if [[ ! -d {{linux_dir}} ]]; then just build-linux; fi
  cd {{invocation_directory()}}/tests/debug-kernel-mod && make KERNELDIR={{linux_dir}}

# Load debug kernel module into VM started by `just qemu` using ssh
load-debug-kernel-mod: build-debug-kernel-mod
  just qemu_ssh_port={{qemu_ssh_port}} ssh-qemu "rmmod debug-kernel-mod; insmod /mnt/vmsh/tests/debug-kernel-mod/debug-kernel-mod.ko && dmesg"

attach-qemu-img: nixos-image
  cargo run -- \
  -l info,vmsh::device::virtio::block::inorder_handler=warn,vm_memory::mmap=warn,vm_memory::remote_mem=warn,vmsh::device::threads=debug attach \
  "{{qemu_pid}}" -f {{virtio_blk_img}} \
  -- --ssh-args " -i {{invocation_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost"

# Attach block device to first qemu vm found by pidof and owned by our own user
attach-qemu: vmsh-image
  cargo run -- attach -f "{{linux_dir}}/nixos.ext4" "{{qemu_pid}}" --ssh-args " -i {{invocation_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost" -- /nix/var/nix/profiles/system/sw/bin/ls -la

# Inspect first qemu vm found by pidof and owned by our own user
inspect-qemu:
  cargo run -- inspect "{{qemu_pid}}"

# Generate a core dump of the first qemu vm found by pidof and owned by our own user
coredump-qemu:
  cargo run -- coredump "{{qemu_pid}}"

# Generate a core dump of the first qemu vm found by pidof and owned by our own user
trace-qemu:
  perf trace -p "{{qemu_pid}}"

# Start subshell with capabilities/permissions suitable for vmsh
capsh:
  @ if [ -n "${IN_CAPSH:-}" ]; then \
    echo "you are already in a capsh session"; exit 1; \
  else \
    true; \
  fi
  sudo modprobe kheaders || true
  sudo -E IN_CAPSH=1 \
      capsh \
      --caps="cap_sys_ptrace,cap_dac_override,cap_sys_admin,cap_sys_resource+epi cap_setpcap,cap_setuid,cap_setgid+ep" \
      --keep=1 \
      --groups=$(id -G | sed -e 's/ /,/g') \
      --gid=$(id -g) \
      --uid=$(id -u) \
      --addamb=cap_sys_resource \
      --addamb=cap_sys_admin \
      --addamb=cap_sys_ptrace \
      --addamb=cap_dac_override \
      -- -c 'export USER=$(id -un); direnv exec "$0" "$1"' . "$SHELL"

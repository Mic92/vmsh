# Local Variables:
# mode: makefile
# End:
# vim: set ft=make :

rev := `nix eval --raw .#lib.nixpkgsRev`
linux_dir := justfile_directory() + "/../linux"
linux_repo := "https://github.com/Mic92/linux"
nix_results := justfile_directory() + "/.git/nix-results/" + rev
kernel_shell := "$(nix build --out-link " + nix_results + "/kernel-fhs --json " + justfile_directory() + "#kernel-deps | jq -r '.[] | .outputs | .out')/bin/linux-kernel-build"
hypervisor_socket := justfile_directory() + "/.git/cloud-hypervisor-socket"

virtio_blk_img := justfile_directory() + "/../linux/nixos.ext4"

qemu_pid := `pgrep -u $(id -u) qemu-system | awk '{print $1}'`
qemu_ssh_port := "2222"
qemu_ssh_remote := "root@localhost"

# Interactively select a task from just file
default:
  @just --choose

# build vmsh
build:
   cargo build

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
  cargo watch -x build

passwordless_sudo:
  sudo echo "Some compenents require passwordless sudo."

# Run unit and integration tests
test: passwordless_sudo
  nix flake check
  cargo test
  pytest -n $(nproc --ignore=2) -s tests

# stress test the host, guest-qemu-blk and vmsh-blk device
xfstests: passwordless_sudo
  @ if [ -n "${IN_CAPSH:-}" ]; then \
    true; \
  else \
    echo "Please use \`just capsh\` to elevate your privileges."; exit 1; \
  fi
  nix shell .#xfstests --command python3 tests/xfstests.py

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

reliability-attach:
  #!/usr/bin/env python3
  import sys, os
  sys.path.insert(0, os.path.join("{{justfile_directory()}}", "tests"))
  from test_attach import test_attach
  from conftest import Helpers
  helpers = Helpers()

  test_attach(helpers, attach_repetitions=100, vcpus=2, mmio="wrap_syscall") 
  # rep 36, 1, 4, 3, 0, 13
  
  #test_attach(helpers, attach_repetitions=100, vcpus=2, mmio="ioregionfd") 
  # rep 4, 15, 3, 45, 34, 2

# Git clone linux kernel
clone-linux:
  #!/usr/bin/env bash
  set -euo pipefail
  if [[ ! -d {{linux_dir}} ]]; then
    git clone {{linux_repo}} {{linux_dir}}
  fi

  set -x
  commit="$(nix eval --raw .#linux_ioregionfd.src.rev)"
  if [[ $(git -C {{linux_dir}} rev-parse HEAD) != "$commit" ]]; then
     git -C {{linux_dir}} fetch {{linux_repo}} $commit
     git -C {{linux_dir}} checkout "$commit"
     rm -f {{linux_dir}}/.config
  fi

# Configure linux kernel build
configure-linux: #clone-linux
  #!/usr/bin/env bash
  set -xeuo pipefail
  if [[ ! -f {{linux_dir}}/.config ]]; then
    cd {{linux_dir}}
    {{kernel_shell}} "make defconfig kvm_guest.config"
    {{kernel_shell}} "scripts/config \
       --disable DRM \
       --disable USB \
       --disable WIRELESS \
       --disable WLAN \
       --disable SOUND \
       --disable SND \
       --disable HID \
       --disable INPUT \
       --disable NFS_FS \
       --disable ETHERNET \
       --disable NETFILTER \
       --enable DEBUG \
       --enable GDB_SCRIPTS \
       --enable DEBUG_DRIVER \
       --enable KVM \
       --enable KVM_INTEL \
       --enable KVM_AMD \
       --enable KVM_IOREGION \
       --enable BPF_SYSCALL \
       --enable CONFIG_MODVERSIONS \
       --enable IKHEADERS \
       --enable IKCONFIG_PROC \
       --enable VIRTIO_MMIO \
       --enable VIRTIO_MMIO_CMDLINE_DEVICES \
       --enable PTDUMP_CORE \
       --enable PTDUMP_DEBUGFS \
       --enable OVERLAY_FS \
       --enable SQUASHFS \
       --enable SQUASHFS_XZ \
       --enable SQUASHFS_FILE_DIRECT \
       --enable PVH \
       --disable SQUASHFS_FILE_CACHE \
       --enable SQUASHFS_DECOMP_MULTI \
       --disable SQUASHFS_DECOMP_SINGLE \
       --disable SQUASHFS_DECOMP_MULTI_PERCPU"
  fi

# Sign drone ci configuration
sign-drone:
  DRONE_SERVER=https://drone.thalheim.io \
  DRONE_TOKEN=$(cat $HOME/.secret/drone-token) \
    nix-shell -p drone-cli --run 'drone sign Mic92/vmsh --save'

# Linux kernel development shell
build-linux-shell:
  {{kernel_shell}} bash

# Clean build directory of linux
clean-linux: configure-linux
  cd {{linux_dir}} && {{kernel_shell}} "make -C {{linux_dir}} mrproper"

# Build linux kernel
build-linux: configure-linux
  #!/usr/bin/env bash
  set -xeu
  cd {{linux_dir}}
  #{{kernel_shell}} "make -C {{linux_dir}} oldconfig"
  yes "" | {{kernel_shell}} "make -C {{linux_dir}} -j$(nproc)"

# Build a disk image
image NAME="nixos" PATH="/nixos.img":
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ nix/{{NAME}}-image.nix -nt {{linux_dir}}/{{NAME}}.ext4 ]] \
     || [[ flake.lock -nt {{linux_dir}}/{{NAME}}.ext4 ]]; then
     nix build --out-link {{nix_results}}/{{NAME}}-image/ --builders '' .#{{NAME}}-image
     install -m600 "{{nix_results}}/{{NAME}}-image{{PATH}}" {{linux_dir}}/{{NAME}}.ext4
  fi

# Build kernel-less disk image for NixOS
nixos-image: image

# Build disk image with busybox
busybox-image:
  just image busybox ""

# Build disk image with passwd from shadow
passwd-image:
  just image passwd ""

# Build kernel/disk image for not os
notos-image: build-linux
  nix build --out-link {{nix_results}}/notos-image '.#not-os-image'
  jq < {{nix_results}}/notos-image

# built image for qemu_nested.sh
nested-nixos-image: nixos-image
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ ! -e {{linux_dir}}/nixos-nested.ext4 ]] || [[ {{linux_dir}}/nixos.ext4 -nt {{linux_dir}}/nixos-nested.ext4 ]]; then
    cp -a --reflink=auto "{{linux_dir}}/nixos.ext4" {{linux_dir}}/nixos-nested.ext4
  fi

mkramdisk SRC="/dev/zero" NAME="ramdisk" SIZEG="2":
  #!/usr/bin/env bash
  set +x
  mkdir -p /tmp/{{NAME}}
  if [[ ! -e /tmp/{{NAME}}/raw ]]; then 
    sudo mount -t tmpfs -o size={{SIZEG}}G vmshramdisk /tmp/{{NAME}}
    sudo touch /tmp/{{NAME}}/raw
    sudo dd if={{SRC}} of=/tmp/{{NAME}}/raw bs=1024 count={{SIZEG}}M
    sudo chown $USER /tmp/{{NAME}}/raw
  fi

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
    -virtfs local,path={{justfile_directory()}}/..,security_model=none,mount_tag=home \
    -virtfs local,path={{linux_dir}},security_model=none,mount_tag=linux \
    -nographic -serial null -enable-kvm \
    -device virtio-serial \
    -chardev stdio,mux=on,id=char0,signal=off \
    -mon chardev=char0,mode=readline \
    -device virtconsole,chardev=char0,id=vmsh,nr=0

alias cl := cloud-hypervisor
cloud-hypervisor: build-linux nixos-image
  cloud-hypervisor \
      --memory size=500M,mergeable=on,shared=on \
      --cpus boot=1 --rng --watchdog --console tty \
      --kernel {{linux_dir}}/vmlinux \
      --cmdline "console=hvc0 root=/dev/vda" \
      --seccomp false \
      --disk path={{linux_dir}}/nixos.ext4 \
      --api-socket {{hypervisor_socket}}

# run qemu with kernel build by `build-linux` and filesystem image build by `busybox-image`
kvmtool: build-linux busybox-image
  lkvm run --debug --name nixos -m 512 -c 1 --disk {{linux_dir}}/busybox.ext4 --rng -k {{linux_dir}}/arch/x86/boot/bzImage \
    -p "root=/dev/vda init=/bin/sh"

stop-cloud-hypervisor:
  curl --unix-socket {{hypervisor_socket}} -X PUT http://localhost/api/v1/vm.power-button

# checkout and build upstream firecracker kernel
firecracker-kernel:
   curl https://raw.githubusercontent.com/firecracker-microvm/firecracker/main/resources/microvm-kernel-x86_64.config > {{linux_dir}}/.config
   git -C {{linux_dir}} fetch https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux-stable.git v4.14.245
   git -C {{linux_dir}} checkout v4.14.245
   just build-linux

# run firecracker hypervisor
firecracker: build-linux nixos-image
  firectl -m512 -c1 --kernel={{linux_dir}}/vmlinux \
    --kernel-opts "console=ttyS0" \
    --root-drive={{linux_dir}}/nixos.ext4

# run crosvm hypervisor
crosvm: build-linux nixos-image
  crosvm run -m500 -c1 --rwdisk {{linux_dir}}/nixos.ext4 \
    --disable-sandbox --serial type=stdout,console=true,stdin=true \
    -p "console=ttyS0 root=/dev/vda" \
    {{linux_dir}}/vmlinux

# download alpine iso
# this version is intentionally out-of-date so we can test our alpine-sec-scanner
alpine-initrd:
  mkdir -p {{linux_dir}}
  [[ -f {{linux_dir}}/alpine-initramfs ]] || curl -o {{linux_dir}}/alpine-initramfs https://dl-cdn.alpinelinux.org/alpine/v3.10/releases/x86_64/netboot-3.10.0/initramfs-virt
  [[ -f {{linux_dir}}/alpine.iso ]] || curl -o {{linux_dir}}/alpine.iso https://dl-cdn.alpinelinux.org/alpine/v3.10/releases/x86_64/alpine-virt-3.10.0-x86_64.iso

alpine-sec-scanner-image:
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ -f {{linux_dir}}/alpine-sec-scanner-image.ext4 ]]; then
    exit 0
  fi
  nix build --out-link {{nix_results}}/alpine-sec-scanner-image --builders '' .#alpine-sec-scanner-image
  install -m755 -D {{nix_results}}/alpine-sec-scanner-image {{linux_dir}}/alpine-sec-scanner-image.ext4

# run alpine linux in qemu
qemu-alpine: alpine-initrd
  qemu-system-x86_64 \
    -enable-kvm \
    -name test-os \
    -m 512 \
    -kernel {{linux_dir}}/arch/x86/boot/bzImage \
    -initrd {{linux_dir}}/alpine-initramfs \
    -drive id=drive1,file={{linux_dir}}/alpine.iso,format=raw,if=none \
    -device virtio-blk-pci,drive=drive1,bootindex=1 \
    -net nic,netdev=user.0,model=virtio \
    -netdev user,id=user.0 \
    -append "console=hvc0 ip=dhcp" \
    -no-reboot \
    -nographic \
    -device virtio-rng-pci \
    -serial null \
    -device virtio-serial \
    -chardev stdio,mux=on,id=char0,signal=off \
    -mon chardev=char0,mode=readline \
    -device virtconsole,chardev=char0,id=vmsh,nr=0

# scan alpine vm for security vulnerabilities
scan-alpine-linux: alpine-sec-scanner-image
  cargo run -- attach -f "{{linux_dir}}/alpine-sec-scanner-image.ext4" $(pgrep -n -u $(id -u) qemu) -- /bin/alpine-sec-scanner /var/lib/vmsh

qemu-ramdisk EXTRA_CMDLINE="nokalsr": build-linux nixos-image
  just mkramdisk {{linux_dir}}/nixos.ext4 nixos.ext4 4
  qemu-system-x86_64 \
    -kernel {{linux_dir}}/arch/x86/boot/bzImage \
    -drive format=raw,file=/tmp/nixos.ext4/raw \
    -append "root=/dev/sda console=hvc0 {{EXTRA_CMDLINE}}" \
    -net nic,netdev=user.0,model=virtio \
    -m 512M \
    -netdev user,id=user.0,hostfwd=tcp:127.0.0.1:{{qemu_ssh_port}}-:22 \
    -cpu host \
    -virtfs local,path={{justfile_directory()}}/..,security_model=none,mount_tag=home \
    -virtfs local,path={{linux_dir}},security_model=none,mount_tag=linux \
    -nographic -serial null -enable-kvm \
    -device virtio-serial \
    -chardev stdio,mux=on,id=char0,signal=off \
    -mon chardev=char0,mode=readline \
    -device virtconsole,chardev=char0,id=vmsh,nr=0

# run qemu with filesystem/kernel from notos (same as in tests)
qemu-notos image="not-os-image": build-linux
  #!/usr/bin/env python3
  import sys, os, subprocess
  sys.path.insert(0, os.path.join("{{justfile_directory()}}", "tests"))
  from nix import notos_image, notos_image_custom_kernel
  from qemu import qemu_command
  #image = notos_image()
  print("run {{image}}")
  image = notos_image(".#{{image}}")
  #image = notos_image_custom_kernel(".#{{image}}")
  cmd = qemu_command(image, "qmp.sock", ssh_port={{qemu_ssh_port}})
  print(" ".join(cmd))
  subprocess.run(cmd)

# run qemu with measurement image
qemu-measurement-image:
  just qemu-notos measurement-image

# Attach gdb to vmsh
gdb:
  sudo gdb --pid $(pidof vmsh) -ex 'thread apply all bt' -ex 'info threads'

# Attach strace to vmsh
strace:
  sudo strace -p $(pidof vmsh) -f

# SSH into vm started by `just qemu`
ssh-qemu $COMMAND="":
  ssh -i {{justfile_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      {{qemu_ssh_remote}} \
      -p {{qemu_ssh_port}} -- "$COMMAND"

qemu-wait-for-ssh: 
  #!/usr/bin/env bash
  printf "%s" "waiting for qemu to come online"
  i=0
  COMMAND="ssh -i {{justfile_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      -o ConnectTimeout=2 \
      {{qemu_ssh_remote}} \
      -p {{qemu_ssh_port}} 'ls'"
  while ! $COMMAND &> /dev/null
  do
    if [ $i -gt 800 ]; then
      echo "server didn't come online ERROR"
      exit 1
    fi
    printf "%c" "."
    sleep 1
    i=$(($i+1))
  done
  echo

# SCP to/from vm started by `just qemu`. Use {{qemu_ssh_remote}} as remote name.
scp-qemu $SRC="" $DST="":
  scp -i {{invocation_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      -P {{qemu_ssh_port}} \
      {{SRC}} {{DST}}

# Start qemu in qemu based on nixos image
nested-qemu: nested-nixos-image
  just ssh-qemu qemu-nested

# Start cloud-hypervisor in qemu based on nixos image
nested-cloud-hypervisor: nested-nixos-image
  just ssh-qemu cloud-hypervisor-nested

# Copy programs from the host store to the guest nix store
qemu-copy STORE_PATH:
  mkdir -p target/mnt
  sudo mount {{virtio_blk_img}} {{justfile_directory()}}/target/mnt
  sudo nix copy {{STORE_PATH}} --to {{justfile_directory()}}/target/mnt
  sudo umount {{justfile_directory()}}/target/mnt

# Build debug kernel module for VM using kernel build by `just build-linux`
build-debug-kernel-mod:
  # don't invoke linux kernel build every time because it is a bit slow...
  if [[ ! -d {{linux_dir}} ]]; then just build-linux; fi
  cd {{justfile_directory()}}/tests/debug-kernel-mod && make KERNELDIR={{linux_dir}}

# Load debug kernel module into VM started by `just qemu` using ssh
load-debug-kernel-mod: build-debug-kernel-mod
  just qemu_ssh_port={{qemu_ssh_port}} ssh-qemu "rmmod debug-kernel-mod; insmod /mnt/vmsh/tests/debug-kernel-mod/debug-kernel-mod.ko && dmesg"

attach-qemu-img: nixos-image
  cargo run -- \
  -l info,vmsh::device::virtio::block::inorder_handler=warn,vm_memory::mmap=warn,vm_memory::remote_mem=warn,vmsh::device::threads=debug attach \
  "{{qemu_pid}}" -f {{virtio_blk_img}}

# Use this to get a pts for use with `just attach-qemu-sh` or `vmsh attach --pts`
pts:
  #!/usr/bin/env python3
  import time
  import os
  pts = os.readlink(f"/proc/self/fd/0")
  print(f"--pts {pts}")
  while True:
    time.sleep(1)

attach-qemu-sh pts: busybox-image
  cargo run -- attach -f "{{linux_dir}}/busybox.ext4" --pts {{pts}} "{{qemu_pid}}" -- /bin/sh

# Attach hypervisor matched by name
attach TARGET="qemu": busybox-image
  cargo run -- attach -f "{{linux_dir}}/busybox.ext4" $(pgrep -n -u $(id -u) {{TARGET}}) -- /bin/ls -la

# Attach block device to first qemu vm found by pidof and owned by our own user
attach-qemu: attach

# Attach to cloud-hypervisor (not working yet)
attach-cloud-hypervisor:
  just attach cloud-hyperviso

attach-crosvm: busybox-image
  just attach crosvm

attach-firecracker: busybox-image
  just attach firecracker

attach-kvmtool: busybox-image
  just attach lkvm

measure-block: passwordless_sudo
  rm tests/measurements/stats.json || true
  rm tests/measurements/fio-stats.json || true
  python3 tests/measure_block.py

measure-console: passwordless_sudo
  rm tests/measurements/console-stats.json || true
  python3 tests/measure_console.py

# mom says we already have a benchmark at home
benchmark-qemu-at-home DISK="/dev/vda": 
  just ssh-qemu "yes | wc -l & hdparm -t {{DISK}} && pkill yes"

benchmark-qemu DISK="/dev/vda":
  just ssh-qemu 'sysbench cpu --cpu-max-prime=10000 run & hdparm -t {{DISK}}; wait'

perf COMMAND="top" PGREP="":
  sudo perf kvm --host {{COMMAND}} -p $(pgrep {{PGREP}})

perf-record PGREP="":
  sudo perf kvm --host record -g -p $(pgrep {{PGREP}})

perf-report ARGS="":
  sudo perf kvm --host report {{ARGS}}

profile PGREP="" SEC="15" OUTFILE="some.profile":
  sudo profile --stack-storage-size 65536 -df -p $(pgrep {{PGREP}}) {{SEC}} > {{OUTFILE}}

# no worky
perf-kvm-guest:
  rm -f {{linux_dir}}/kallsyms
  rm -f {{linux_dir}}/modules
  just ssh-qemu "cat /proc/kallsyms" > {{linux_dir}}/kallsyms
  just ssh-qemu "cat /proc/modules" > {{linux_dir}}/modules
  perf kvm --guest --guestvmlinux={{linux_dir}}/vmlinux --guestkallsyms {{linux_dir}}/kallsyms  --guestmodules {{linux_dir}}/modules top -a

shortread DISK="/dev/vda":
  just ssh-qemu "hdparm -t {{DISK}}"

longread DISK="/dev/vda" BS="64M":
  just ssh-qemu "dd if={{DISK}} of=/dev/null bs={{BS}} count=1000000"

# ptrace detached
benchmark-detached DISK="/dev/sda" TEST="longread" SAMPLES="7":
  #!/usr/bin/env bash
  just qemu-ramdisk &
  just qemu-wait-for-ssh
  sleep 5
  COLLECT="$COLLECT
  # ptrace detached {{TEST}} {{DISK}}
  "
  for i in {1..{{SAMPLES}}};
  do
    COLLECT="$COLLECT $(just {{TEST}} {{DISK}} 2>&1 | tee /dev/tty)"
  done
  echo "======= RESULTS ========"
  echo "$COLLECT" | grep -E "#|copied|Timing" | tee -a out.benchmark
  set -x
  pkill qemu
  wait

# vmsh attached
benchmark-attached DISK="/dev/sda" TEST="longread" SAMPLES="7":
  #!/usr/bin/env bash
  just qemu-ramdisk &
  just qemu-wait-for-ssh
  sleep 5
  # setsid: apparently wrap_syscall must run with different gid than qemu
  setsid just attach-qemu-ramdisk &
  sleep 15
  COLLECT="$COLLECT
  # vmsh attached {{TEST}} {{DISK}}
  "
  for i in {1..{{SAMPLES}}};
  do
    COLLECT="$COLLECT $(just {{TEST}} {{DISK}} 2>&1 | tee /dev/tty)"
  done
  echo "======= RESULTS ========"
  echo "$COLLECT" | grep -E "#|copied|Timing" | tee -a out.benchmark
  set -x
  ps aux | grep qemu-system
  pkill -SIGKILL qemu-system
  pkill -SIGKILL vmsh
  wait

benchmark SAMPLES="15": mkramdisk
  just benchmark-attached /dev/vda shortread {{SAMPLES}}
  just benchmark-attached /dev/sda shortread {{SAMPLES}}
  just benchmark-detached /dev/sda shortread {{SAMPLES}}
  just benchmark-attached /dev/vda longread {{SAMPLES}}
  just benchmark-attached /dev/sda longread {{SAMPLES}}
  just benchmark-detached /dev/sda longread {{SAMPLES}}

# inspect test/measurements from the console
gnuplot:
  #!/bin/sh
  cd tests/measurements
  gnuplot -e "set terminal dumb; set key autotitle columnhead;" -

attach-qemu-ramdisk: mkramdisk
  cargo run --release -- \
    -l warn attach -f "/tmp/ramdisk/raw" "{{qemu_pid}}" \
    --ssh-args " -i {{invocation_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost" \
    -- /bin/ls -la

BLOCK_DEV_LOG_FILTER := "info,vmsh::devices::virtio::block::threads=trace,vmsh::kvm::hypervisor=info"

attach-nested-qemu: busybox-image build
  just ssh-qemu 'exec /mnt/vmsh/target/debug/vmsh -l {{BLOCK_DEV_LOG_FILTER}} attach -f "/linux/busybox.ext4" $(pgrep qemu) -- /bin/ls -la'

attach-nested-cloud-hypervisor: busybox-image build
  just ssh-qemu 'exec /mnt/vmsh/target/debug/vmsh -l {{BLOCK_DEV_LOG_FILTER}} attach -f "/linux/busybox.ext4" $(pgrep cloud-hyperviso) -- /bin/ls -la'

# Inspect first qemu vm found by pidof and owned by our own user
inspect-qemu:
  cargo run -- inspect "{{qemu_pid}}"

# Set password in qemu VM, expects input in the form of username:password
change-password user_password TARGET="qemu": passwd-image
  cargo run -- attach -f "{{linux_dir}}/passwd.ext4" $(pgrep -n -u $(id -u) {{TARGET}}) -- /bin/sh -c "echo '{{user_password}}' | /bin/chpasswd -R /var/lib/vmsh"

inspect-cl:
  cargo run -- inspect $(pgrep cloud-hyperviso)

# Generate a core dump of the first qemu vm found by pidof and owned by our own user
coredump-qemu:
  cargo run -- coredump "{{qemu_pid}}"

# Generate a core dump of the first qemu vm found by pidof and owned by our own user
trace-qemu:
  perf trace -p "{{qemu_pid}}"

clean-coredumps:
  rm -f core.*

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
      -- -c 'export USER=$(id -un); touch .envrc; direnv exec "$0" "$1"' . "$SHELL"

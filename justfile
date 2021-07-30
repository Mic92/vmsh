# Local Variables:
# mode: makefile
# End:
# vim: set ft=make :

rev := `nix eval --raw .#lib.nixpkgsRev`
linux_dir := justfile_directory() + "/../linux"
linux_repo := "https://github.com/Mic92/linux"
nix_results := justfile_directory() + "/.git/nix-results/" + rev
kernel_fhs := "$(nix build --out-link " + nix_results + "/kernel-fhs --json '.#kernel-fhs' | jq -r '.[] | .outputs | .out')/bin/linux-kernel-build"

virtio_blk_img := justfile_directory() + "/../linux/nixos.ext4"

qemu_pid := `pgrep -u $(id -u) qemu-system | awk '{print $1}'`
qemu_ssh_port := "2222"
qemu_ssh_remote := "root@localhost"

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
  cargo watch -x build

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
configure-linux: #clone-linux
  #!/usr/bin/env bash
  set -xeuo pipefail
  if [[ ! -f {{linux_dir}}/.config ]]; then
    {{kernel_fhs}} "make -C {{linux_dir}} defconfig kvm_guest.config"
    {{kernel_fhs}} "cd {{linux_dir}} && scripts/config \
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
       --enable DEBUG_INFO_DWARF5 \
       --enable DEBUG \
       --enable GDB_SCRIPTS \
       --enable DEBUG_DRIVER \
       --enable KVM \
       --enable KVM_INTEL \
       --enable KVM_AMD \
       --enable KVM_IOREGION \
       --enable BPF_SYSCALL \
       --enable IKHEADERS \
       --enable IKCONFIG_PROC \
       --enable VIRTIO_MMIO \
       --enable PTDUMP_CORE \
       --enable PTDUMP_DEBUGFS \
       --enable OVERLAY_FS \
       --enable SQUASHFS \
       --enable SQUASHFS_XZ \
       --enable SQUASHFS_FILE_DIRECT \
       --disable SQUASHFS_FILE_CACHE \
       --enable SQUASHFS_DECOMP_MULTI \
       --disable SQUASHFS_DECOMP_SINGLE \
       --disable SQUASHFS_DECOMP_MULTI_PERCPU \
    "
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
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ nix/nixos-image.nix -nt {{linux_dir}}/nixos.ext4 ]] || [[ flake.lock -nt {{linux_dir}}/nixos.ext4 ]]; then
     nix build --out-link {{nix_results}}/nixos-image --builders '' .#nixos-image --out-link nixos-image
     install -m600 "nixos-image/nixos.img" {{linux_dir}}/nixos.ext4
  fi

# Build kernel/disk image for not os
notos-image:
  nix build --out-link {{nix_results}}/notos-image '.#not-os-image.json'
  jq < {{nix_results}}/notos-image

# built image for qemu_nested.sh
nested-nixos-image: nixos-image
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ ! -e {{linux_dir}}/nixos-nested.ext4 ]] || [[ {{linux_dir}}/nixos.ext4 -nt {{linux_dir}}/nixos-nested.ext4 ]]; then
    cp -a --reflink=auto "{{linux_dir}}/nixos.ext4" {{linux_dir}}/nixos-nested.ext4
  fi

vmsh-image: nixos-image
  #!/usr/bin/env bash
  set -eux -o pipefail
  if [[ ! -e {{linux_dir}}/vmsh-image.ext4 ]] || [[ {{linux_dir}}/nixos.ext4 -nt {{linux_dir}}/vmsh-image.ext4 ]]; then
      cp -a --reflink=auto "{{linux_dir}}/nixos.ext4" "{{linux_dir}}/vmsh-image.ext4"
      touch -r "{{linux_dir}}/nixos.ext4" "{{linux_dir}}/vmsh-image.ext4"
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
qemu-notos:
  #!/usr/bin/env python3
  import sys, os, subprocess
  sys.path.insert(0, os.path.join("{{justfile_directory()}}", "tests"))
  from nix import notos_image, notos_image_custom_kernel
  from qemu import qemu_command
  #image = notos_image()
  image = notos_image_custom_kernel()
  cmd = qemu_command(image, "qmp.sock", ssh_port={{qemu_ssh_port}})
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
  ssh -i {{justfile_directory()}}/nix/ssh_key \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      {{qemu_ssh_remote}} \
      -p {{qemu_ssh_port}} "$COMMAND"

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
  "{{qemu_pid}}" -f {{virtio_blk_img}} \
  -- --ssh-args " -i {{justfile_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost"

# Attach block device to first qemu vm found by pidof and owned by our own user
attach-qemu: vmsh-image
  cargo run -- attach -f "{{linux_dir}}/vmsh-image.ext4" "{{qemu_pid}}" --ssh-args " -i {{justfile_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost" -- /nix/var/nix/profiles/system/sw/bin/ls -la

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

attach-qemu-ramdisk: mkramdisk
  cargo run --release -- -l warn attach -f "/tmp/ramdisk/raw" "{{qemu_pid}}" --ssh-args " -i {{invocation_directory()}}/nix/ssh_key -p {{qemu_ssh_port}} root@localhost" -- /nix/var/nix/profiles/system/sw/bin/ls -la

attach-nested-qemu: vmsh-image
  cargo build
  just ssh-qemu '/mnt/vmsh/target/debug/vmsh -l info,vmsh::devices::virtio::block::threads=trace,vmsh::kvm::hypervisor=info attach -f "/linux/vmsh-image.ext4" $(pgrep qemu) --ssh-args " -i /mnt/vmsh/nix/ssh_key -p 3333 root@localhost" -- /nix/var/nix/profiles/system/sw/bin/ls -la'

# Inspect first qemu vm found by pidof and owned by our own user
inspect-qemu:
  cargo run -- inspect "{{qemu_pid}}"

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

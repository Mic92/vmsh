{ pkgs, ... }:
{
  environment.systemPackages = [
    pkgs.linuxPackages.bcc
    pkgs.gdb # for debugging the nested guest
    (pkgs.writeShellScriptBin "qemu-nested" ''
      exec ${pkgs.qemu_kvm}/bin/qemu-system-x86_64 \
        -kernel /linux/arch/x86/boot/bzImage \
        -drive format=raw,file=/linux/nixos-nested.ext4 \
        -append "root=/dev/sda console=ttyS0 nokaslr" \
        -m 256M \
        -net nic,netdev=user.0,model=virtio \
        -netdev user,id=user.0,hostfwd=tcp:127.0.0.1:3333-:22 \
        -nographic -enable-kvm \
        "$@"
    '')
  ];
}

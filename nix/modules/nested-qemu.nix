{ pkgs, ... }:
{
  environment.systemPackages = [
    pkgs.linuxPackages.bcc
    (pkgs.writeShellScriptBin "qemu-nested" ''
      exec ${pkgs.qemu_kvm}/bin/qemu-system-x86_64 \
        -kernel /linux/arch/x86/boot/bzImage \
        -drive format=raw,file=/linux/nixos-nested.ext4 \
        -append "root=/dev/sda console=ttyS0 nokaslr" \
        -m 256M \
        -nographic -enable-kvm \
        "$@"
    '')
  ];
}

{ pkgs }:

let
  keys = map (key: "${builtins.getEnv "HOME"}/.ssh/${key}")
    ["id_rsa.pub" "id_ecdsa.pub" "id_ed25519.pub"];
in import (pkgs.path + "/nixos/lib/make-disk-image.nix") {
  inherit pkgs;
  inherit (pkgs) lib;
  config = (import (pkgs.path + "/nixos/lib/eval-config.nix") {
    inherit (pkgs) system;
    modules = [({ lib, ... }: {
      imports = [
        (pkgs.path + "/nixos/modules/profiles/qemu-guest.nix")
      ];
      boot.loader.grub.enable = false;
      boot.initrd.enable = false;
      boot.isContainer = true;
      boot.loader.initScript.enable = true;
      # login with empty password
      users.extraUsers.root.initialHashedPassword = "";
      services.openssh.enable = true;

      users.users.root.openssh.authorizedKeys.keyFiles = lib.filter builtins.pathExists keys;
      networking.firewall.enable = false;

      fileSystems."/mnt" = {
        device = "home";
        fsType = "9p";
        # skip mount in nested qemu
        options = [ "trans=virtio" "nofail" ];
      };

      fileSystems."/linux" = {
        device = "linux";
        fsType = "9p";
        # skip mount in nested qemu
        options = [ "trans=virtio" "nofail" ];
      };

      users.users.root.openssh.authorizedKeys.keys = [
        (builtins.readFile ./ssh_key.pub)
      ];

      services.getty.helpLine = ''
        Log in as "root" with an empty password.
        If you are connect via serial console:
        Type Ctrl-a c to switch to the qemu console
        and `quit` to stop the VM.
      '';
      documentation.doc.enable = false;
      environment.systemPackages = [ 
        pkgs.linuxPackages.bcc
        pkgs.busybox
        pkgs.file
        (pkgs.writeShellScriptBin "qemu-nested" ''
          exec ${pkgs.qemu_kvm}/bin/qemu-system-x86_64 \
            -kernel /linux/arch/x86/boot/bzImage \
            -hda /linux/nixos-nested.qcow2 \
            -append "root=/dev/sda console=ttyS0 nokaslr" \
            -m 256M \
            -nographic -enable-kvm \
            "$@"
        '')
      ];
    })];
  }).config;
  partitionTableType = "none";
  diskSize = 8192;
  format = "qcow2";
}

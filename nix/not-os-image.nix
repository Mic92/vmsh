{ pkgs ? (import (import ../nix/sources.nix).nixpkgs { })
, not-os ? (import ../nix/sources.nix).not-os
}:
let
  config = (import not-os {
    nixpkgs = pkgs.path;
    inherit (pkgs) system;
    extraModules = [
      (not-os  + "/qemu.nix")
      ({ pkgs, ... }: {
        environment.etc = {
          "service/backdoor/run".source = pkgs.writeScript "backdoor_run" ''
            #!/bin/sh
            export USER=root
            export HOME=/root
            cd $HOME

            source /etc/profile

            echo 'nixos' > /proc/sys/kernel/hostname

            exec < /dev/ttyS0 > /dev/ttyS0 2>&1
            echo "If you are connect via serial console:"
            echo "Type Ctrl-a c to switch to the qemu console"
            echo "and 'quit' to stop the VM."
            exec ${pkgs.utillinux}/bin/setsid ${pkgs.bash}/bin/bash -l
          '';
        };
        environment.etc.profile.text = ''
          export PS1="\e[0;32m[\u@\h \w]\$ \e[0m"
        '';
        boot.initrd.availableKernelModules = [ "virtio_console" ];
      })
    ];
  }).config;
in
{
  inherit (config.system.build) runvm kernel squashfs initialRamdisk kernelParams;
  json = pkgs.writeText "not-os.json" (builtins.toJSON {
    inherit (config.system.build) kernel squashfs initialRamdisk;
    inherit (config.boot) kernelParams;
  });
}

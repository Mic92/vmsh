{ pkgs ? (import (import ../nix/sources.nix).nixpkgs { })
, not-os ? (import ../nix/sources.nix).not-os
}:
let
  inherit (pkgs) stdenv lib;
  inherit (pkgs.pkgsMusl.hostPlatform) system parsed;
  useMusl = false;

  config = (import not-os {
    nixpkgs = pkgs.path;
    system = if useMusl then null else pkgs.system;
    configuration = { pkgs, ... }: {
      imports = [
        (not-os  + "/qemu.nix")
      ];
      environment.systemPackages = [
        pkgs.utillinux
        pkgs.gnugrep
        pkgs.kmod
      ];
      nixpkgs.localSystem = lib.mkIf useMusl {
        inherit system parsed;
      };

      networking.timeServers = [];

      not-os.nix = true;
      not-os.simpleStaticIp = true;
      not-os.preMount = ''
        echo 'nixos' > /proc/sys/kernel/hostname
        ip addr add 127.0.0.1/8 dev lo
        ip addr add ::1/128 dev lo
        ip link dev lo up
        ip addr add 10.0.2.15/24 dev eth0
      '';

      environment.etc = {
        "hosts".text = ''
          127.0.0.1 localhost
          ::1 localhost
          127.0.0.1 nixos
          ::1 nixos
        '';
        "ssh/authorized_keys.d/root" = {
          source = ./ssh_key.pub;
          mode = "444";
        };
        "service/shell/run".source = pkgs.writeScript "shell" ''
          #!/bin/sh
          export USER=root
          export HOME=/root
          cd $HOME

          source /etc/profile

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
    };
  }).config;
in
{
  inherit (config.system.build) runvm kernel squashfs initialRamdisk kernelParams;
  json = pkgs.writeText "not-os.json" (builtins.toJSON {
    inherit (config.system.build) kernel squashfs initialRamdisk;
    inherit (config.boot) kernelParams;
  });
}

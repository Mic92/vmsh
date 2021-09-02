{ pkgs, not-os, notos-config }:
let
  inherit (pkgs) stdenv lib;
  inherit (pkgs.pkgsMusl.hostPlatform) system parsed;

  useMusl = false;

  config = (import not-os {
    nixpkgs = pkgs.path;
    system = if useMusl then null else pkgs.system;
    configuration = { ... }: {
      imports = [
        notos-config
        (not-os  + "/qemu.nix")
      ];

      nixpkgs.localSystem = lib.mkIf useMusl {
        inherit system parsed;
      };
    };
  }).config;
  #inherit (config.system.build) kernel;
  kernel = pkgs.linux.override {
    structuredExtraConfig = with lib.kernel; {
      XFS_ONLINE_SCRUB = yes;
    };
    ignoreConfigErrors = true;
  };
  kerneldir = "${kernel.dev}/lib/modules/${kernel.modDirVersion}/build";
in
{
  inherit (config.system.build) runvm kernel squashfs initialRamdisk kernelParams;
  inherit kerneldir;
  json = pkgs.writeText "not-os.json" (builtins.toJSON {
    inherit kerneldir;
    inherit (config.system.build) kernel squashfs initialRamdisk;
    inherit (config.boot) kernelParams;
  });
}

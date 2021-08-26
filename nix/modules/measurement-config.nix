{ lib, pkgs, ... }:
let
  phoronix = pkgs.callPackage ../phoronix.nix {};
  myxfstests = pkgs.callPackage ../xfstests.nix { };
in {
  imports = [ ./not-os-config.nix ];
  environment.systemPackages = [
    pkgs.hdparm
    pkgs.sysbench
    pkgs.fio
    phoronix
    myxfstests
  ];
}

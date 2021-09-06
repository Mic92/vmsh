{ lib, pkgs, ... }:
let
  phoronix = pkgs.callPackage ../phoronix.nix {};
  myxfstests = pkgs.callPackage ../xfstests.nix { };
in {
  imports = [
    ./not-os-config.nix
  ];
  environment.systemPackages = [
    pkgs.hdparm
    pkgs.sysbench
    pkgs.fio
    phoronix
    myxfstests
    pkgs.su
  ];
  not-os.simpleStaticIp = false;
  # no default gateway to isolate phoronix from internet
  environment.etc."service/network/run".source = pkgs.writeScript "network" ''
    #!${pkgs.stdenv.shell}
    ip link set eth0 up
    # HACK: fake oneshot service
    exec tail -f /dev/null
  '';
}

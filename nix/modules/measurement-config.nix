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
  system.activationScripts.qemu-network = ''
    ip addr add 10.0.2.15 dev eth0
    ip link set eth0 up
    ip route add 10.0.2.0/24 dev eth0
  '';
  environment.etc.passwd.text = ''
    daemon:1:daemon:/usr/sbin:/noshell
    fsgqa:x:1001:1002:Fsgqa:/:/bin/sh
    fsgqa2:x:1001:1002:Fsgqa:/:/bin/sh
    123456-fsgqa:x:1003:1003:Fsgqa:/:/bin/sh
  '';
  environment.etc.group.text = ''
    daemon:x:1:
    fsgqa:x:1001:
    fsgqa2:x:1002:
    123456-fsgqa:x:1003:
  '';
}

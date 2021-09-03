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
    pkgs.su
  ];
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

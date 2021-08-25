{ lib, pkgs, ... }:
let
  phoronix = pkgs.callPackage ../phoronix.nix {};
in {
  imports = [ ./not-os-config.nix ];
  environment.systemPackages = [
    pkgs.hdparm
    pkgs.sysbench
    pkgs.fio
    phoronix

  ];

  system.activationScripts.phoronix-unpack-cache =  ''
    mkdir -p /var/lib/phoronix-test-suite
    cp -r "${phoronix.phoronix-cache}"/* /var/lib/phoronix-test-suite/
  '';

  environment.etc.profile.text = ''
    export PTS_DOWNLOAD_CACHE_OVERRIDE=/var/lib/phoronix-test-suite/download-cache/
    export PTS_USER_PATH_OVERRIDE=/var/lib/phoronix-test-suite/
  '';
}

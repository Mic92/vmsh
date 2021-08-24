{ pkgs ? import <nixpkgs> {} }:

(pkgs.buildFHSUserEnv {
  name = "phoronix-env";
  targetPkgs = pkgs: with pkgs; [
    php
    bash
    coreutils
    binutils
    automake
    autoconf
    m4
    popt
    libaio
    perl
    gcc7
    pcre
    glibc
    glibc.static
    bc
    openmpi
    python3
  ];
  runScript = "bash";
  multiPkgs = null;
  extraOutputsToInstall = [ "dev" ];
  profile = ''
    export hardeningDisable=all
  '';
}).env

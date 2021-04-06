{ pkgs ? (import (import ./sources.nix).nixpkgs { })
, runScript ? ''bash -c''
}:

(pkgs.buildFHSUserEnv {
  name = "linux-kernel-build";
  targetPkgs = pkgs: (with pkgs;  [
    getopt
    flex
    bison
    binutils
    gcc
    gnumake
    bc
    perl
    hostname
    cpio
  ] ++ map lib.getDev [
    elfutils
    ncurses
    openssl
    zlib
  ]);
  inherit runScript;
})

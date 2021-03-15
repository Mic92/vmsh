{ pkgs ? (import (import ./sources.nix).nixpkgs { })
, runScript ? ''bash -c''
}:

(pkgs.buildFHSUserEnv {
  name = "linux-kernel-build";
  targetPkgs = pkgs: (with pkgs;  [
    getopt
    flex
    bison
    elfutils
    binutils
    ncurses.dev
    openssl.dev
    zlib.dev
    gcc
    gnumake
    bc
    perl
    hostname
    cpio
  ]);
  inherit runScript;
})

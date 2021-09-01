{ buildFHSUserEnv
, lib
, getopt
, elfutils
, ncurses
, openssl
, zlib
, flex
, bison
, binutils
, gcc
, gnumake
, bc
, perl
, hostname
, cpio
, runScript ? ''bash -c''
}:
buildFHSUserEnv {
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
}

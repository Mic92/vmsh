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
, pkg-config
, runScript ? ''bash -c''
}:
buildFHSUserEnv {
  name = "linux-kernel-build";
  targetPkgs = _pkgs: ([
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
    pkg-config
  ] ++ map lib.getDev [
    elfutils
    ncurses
    openssl
    zlib
  ]);
  profile = ''
    export hardeningDisable=all
  '';

  inherit runScript;
}

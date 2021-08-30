{ mkShell
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
, buildFHSUserEnv
}:
(buildFHSUserEnv {
#mkShell {
  name = "linux-kernel-build";
  targetPkgs = pkgs: [
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
  ];
  runScript = "bash -c";
})

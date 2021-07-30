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
}:
mkShell {
  name = "linux-kernel-build";
  buildInputs = [
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
}

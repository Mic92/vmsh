{ getopt
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
, stdenv
}:
stdenv.mkDerivation {
  name = "env";
  nativeBuildInputs = [
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
  ];
  buildInputs = [
    elfutils
    ncurses
    openssl
    zlib
  ];
}

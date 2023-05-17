{ stdenv, fetchurl, xfsprogs, attr, acl, fetchpatch, autoreconfHook, gnumake, gettext, ncurses }:
let
  dmapi = stdenv.mkDerivation rec {
    pname = "dmapi";
    version = "2.2.12";
    src = fetchurl {
      url = "https://sources.archlinux.org/other/packages/dmapi/dmapi-${version}.tar.gz";
      sha256 = "sha256-sY409HN09q33wWSZPCbfNphqAJuGqgBO+URBAmU66mk=";
    };
    patches = [(fetchpatch {
      url = "https://raw.githubusercontent.com/archlinux/svntogit-packages/c18887a3d1fa150de9745b36b32ff699a9b27fa5/trunk/dmapi-headers.patch";
      sha256 = "sha256-7YFpGOfPdUFdla37ut/B4TauSNFFxHiqz7aEARydRw8=";
    })];
    nativeBuildInputs = [ autoreconfHook ];
    buildInputs = [ xfsprogs ];
    MAKE = "${gnumake}/bin/make";
  };
in
stdenv.mkDerivation rec {
  # based on https://github.com/archlinux/svntogit-community/blob/49b20d922a79113724e507c45fcb18fce80eaa2d/trunk/PKGBUILD#L15
  pname = "xfsdump";
  version = "3.1.12";
  MAKE = "${gnumake}/bin/make";
  src = fetchurl {
    url = "https://kernel.org/pub/linux/utils/fs/xfs/xfsdump/xfsdump-${version}.tar.xz";
    sha256 = "sha256-85xMGzBrLdfsl5wOlNYP5pCD0uz5rwUcrF7zvtdyx0o=";
  };
  postPatch = ''
    patchShebangs ./
  '';
  MSGFMT = "${gettext}/bin/msgfmt";
  MSGMERGE = "${gettext}/bin/msgmerge";
  XGETTEXT = "${gettext}/bin/xgettext";
  buildInputs = [ xfsprogs attr acl dmapi gettext ncurses ];
}

{ rustPlatform
, pkgs
, kernel ? pkgs.linuxPackages.kernel
, bcc ? pkgs.linuxPackages.bcc
, pkgSrc ? ./.
, }:

rustPlatform.buildRustPackage {
  name = "vmsh";
  src = pkgSrc;
  buildInputs = [ bcc ];
  KERNELDIR = "${kernel.dev}/lib/modules/${kernel.modDirVersion}/build";
  cargoSha256 = "sha256-CEuXsOPZ23g8ZSRVqarAqOd+stKeSAV/mH7HWPg3Y3c=";
}

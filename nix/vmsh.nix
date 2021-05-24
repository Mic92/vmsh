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
  cargoSha256 = "sha256-QsY9V8+qMOMGE/nXg8KMjv7vZfF/RYjRIo7klkkXTYk=";
}

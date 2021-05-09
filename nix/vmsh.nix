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
  cargoSha256 = "sha256-atEarIQ21uEoVdKfldm91NGJO53PUhVSJszW9RFx49Q=";
}

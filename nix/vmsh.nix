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
  cargoSha256 = "sha256-Equ4yTDRPEW0M0+63WfxTVn4WY49jCan7EljwaAECBU=";
}

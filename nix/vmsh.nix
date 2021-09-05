{ rustPlatform
, pkgs
, bcc ? pkgs.linuxPackages.bcc
, pkgSrc ? ./.
, }:

rustPlatform.buildRustPackage {
  name = "vmsh";
  src = pkgSrc;
  buildInputs = [ bcc ];
  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "bcc-0.0.32-alpha.0" = "sha256-BEegNjqf+6xi/xchyl0Oaf5/pHlC4+/iaoZPsS0Va2g=";
      "virtio-blk-0.1.0" = "sha256-3eXSPy3+5uI0FBpSVwRKWJmWxgrpwfl4rYpPLn0bf/4=";
      "vm-device-0.1.0" = "sha256-kHiEfk3/+ped39Dm4Lzo62E7IWiVDd+PnSsPr1YDj94=";
      "vm-memory-0.5.0" = "sha256-vnSWs9n5nRDO0R8eKII/7P5c731cVN02iQh3L7AdoQA=";
      "container-pid-0.1.0" = "sha256-iTnrt4HdKLnrgSn981on8+6ejYu+QfglOw/OBxUrJvE=";
    };
  };
}

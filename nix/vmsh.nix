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
      "virtio-blk-0.1.0" = "sha256-3eXSPy3+5uI0FBpSVwRKWJmWxgrpwfl4rYpPLn0bf/4=";
      "vm-device-0.1.0" = "sha256-kHiEfk3/+ped39Dm4Lzo62E7IWiVDd+PnSsPr1YDj94=";
      "vm-memory-0.8.0" = "sha256-3XJ6OhY+WeoxokbaQr8F2YaZ2uPilqZj8A+hkRa0EXU=";
    };
  };
}

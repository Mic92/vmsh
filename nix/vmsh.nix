{ rustPlatform
, pkgs
, bcc ? pkgs.linuxPackages.bcc
, pkgSrc ? ./.
}:

rustPlatform.buildRustPackage {
  name = "vmsh";
  src = pkgSrc;
  buildInputs = [ bcc ];
  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "virtio-blk-0.1.0" = "sha256-93QvL6gqflf/bKtWRXfiO+fgHejGkwqvRgQfXhe/T4I=";
      "vm-device-0.1.0" = "sha256-Zh7gFIbIIE+GYEqXvag3ej9ywRBp1UlWFa0K+Ows8ak=";
      "vm-memory-0.11.0" = "sha256-TvsHvooWndcdkNiN9+ibNzv9DCYC4ZTloOfK08EBvm0=";
    };
  };
}

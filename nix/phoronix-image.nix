{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix { };
  phoronix = pkgs.callPackage ./phoronix.nix { };
in
buildDiskImage {
  diskSize = "1024M";
  extraCommands = ''
    pushd root
    mkdir -p phoronix-test-suite
    cp -r "${phoronix.phoronix-cache}"/* phoronix-test-suite
    popd
  '';
}

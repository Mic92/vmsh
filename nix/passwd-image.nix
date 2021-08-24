{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix {};
in
buildDiskImage {
  diskSize = "10M";
  extraCommands = ''
    pushd root
    install -D -m755 ${pkgs.pkgsStatic.shadow}/bin/passwd bin/passwd
    popd
  '';
}

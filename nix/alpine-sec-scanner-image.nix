{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix {};
  alpine-sec-scanner = pkgs.callPackage ./alpine-sec-scanner.nix {};
  inherit (pkgs.pkgsStatic) busybox;
in
buildDiskImage {
  packages = [ alpine-sec-scanner ];
  diskSize = "15M";
  extraCommands = ''
    pushd root
    ln -s ${alpine-sec-scanner}/bin bin
    popd
  '';
}

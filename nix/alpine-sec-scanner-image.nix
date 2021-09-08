{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix {};
  alpine-sec-scanner = pkgs.callPackage ./alpine-sec-scanner.nix {};
  inherit (pkgs.pkgsStatic) busybox;
in
buildDiskImage {
  packages = [ alpine-sec-scanner pkgs.cacert ];
  diskSize = "15M";
  extraCommands = ''
    pushd root
    ln -s ${alpine-sec-scanner}/bin bin
    mkdir -p etc/ssl/certs
    ln -s ${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt etc/ssl/certs/ca-bundle.crt
    popd
  '';
}

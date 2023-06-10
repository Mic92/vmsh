{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix { };
in
buildDiskImage {
  diskSize = "100M";
  packages = [ pkgs.bash pkgs.shadow ];
  extraCommands = ''
    pushd root
    mkdir bin
    ln -s ${pkgs.bash}/bin/bash bin/sh
    ln -s ${pkgs.shadow}/bin/chpasswd bin/chpasswd
    popd
  '';
}

{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix { };
  inherit (pkgs.pkgsStatic) busybox;
in
buildDiskImage {
  packages = [ busybox ];
  extraFiles = {
    "etc/profile" = ''
      export PATH=/bin
    '';
  };
  diskSize = "10M";
  extraCommands = ''
    pushd root
    ln -s ${busybox}/bin bin
    mkdir -p proc dev tmp sys
    popd
  '';
}

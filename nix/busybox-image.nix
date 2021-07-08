{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix {};
  inherit (pkgs.pkgsStatic) busybox;
in
buildDiskImage {
  packages = [ busybox pkgs.bcc pkgs.just ];
  extraFiles = {
    "etc/profile" = ''
      export PATH=/bin
    '';
  };
  diskSize = "10M";
  extraCommands = ''
    pushd root
    ln -s ${busybox}/bin bin
    popd
  '';
}

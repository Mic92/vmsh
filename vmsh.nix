{ naersk, pkgs, bcc ? pkgs.linuxPackages.bcc }:

naersk.buildPackage {
  nativeBuildInputs = [ pkgs.linuxPackages.bcc ];
  root = ./.;
}

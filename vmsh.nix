{ naersk, pkgs, bcc ? pkgs.linuxPackages.bcc }:

naersk.buildPackage {
  buildInputs = [ pkgs.linuxPackages.bcc ];
  root = ./.;
}

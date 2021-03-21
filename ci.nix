{ pkgs ? (import (import ./nix/sources.nix).nixpkgs { }) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk { };

  vmsh = pkgs.callPackage ./vmsh.nix {
    inherit naersk;
  };
in
pkgs.mkShell {
  nativeBuildInputs = [
    pkgs.rustc
    pkgs.cargo
    pkgs.qemu_kvm
    pkgs.tmux # needed for integration test
    (pkgs.python3.withPackages(ps: [
      ps.pytest
      ps.pyelftools
    ]))
  ] ++ vmsh.nativeBuildInputs;
  buildInputs = vmsh.buildInputs;
}

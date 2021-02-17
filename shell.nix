{ pkgs ? (import (import ./nix/sources.nix).nixpkgs {}) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk {};
  niv = pkgs.callPackage sources.niv {};

  vmsh = pkgs.callPackage ./vmsh.nix {
    inherit naersk;
  };
in
pkgs.mkShell {
  nativeBuildInputs = [
    niv.niv
    pkgs.rust-analyzer
    pkgs.rustfmt
    pkgs.just
    pkgs.qemu_kvm
    pkgs.cargo-watch
  ] ++ vmsh.nativeBuildInputs;
}

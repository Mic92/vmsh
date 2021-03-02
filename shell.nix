{ pkgs ? (import (import ./nix/sources.nix).nixpkgs { }) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk { };
  niv = pkgs.callPackage sources.niv { };

  vmsh = pkgs.callPackage ./vmsh.nix {
    inherit naersk;
  };
in
pkgs.mkShell {
  RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
  nativeBuildInputs = [
    niv.niv
    pkgs.rust-analyzer
    pkgs.rustfmt
    pkgs.just
    pkgs.qemu_kvm
    pkgs.clippy
    pkgs.rustfmt
    pkgs.rustc
    pkgs.cargo-watch
    pkgs.cargo-deny
    pkgs.pre-commit
    pkgs.python3.pkgs.pytest
    pkgs.git # needed for pre-commit install
  ] ++ vmsh.nativeBuildInputs;
  shellHook = ''
    pre-commit install
  '';
}

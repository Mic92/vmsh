{ pkgs ? (import (import ./nix/sources.nix).nixpkgs { }) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk { };

  vmsh = pkgs.callPackage ./vmsh.nix {
    inherit naersk;
  };

  pythonEnv = (pkgs.python3.withPackages (ps: [
    ps.pytest
    ps.pytest-xdist
    ps.pyelftools
    ps.intervaltree

    # linting
    ps.black
    ps.flake8
    ps.isort
    ps.mypy
  ]));
in
pkgs.mkShell {
  RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
  nativeBuildInputs = [
    pkgs.niv
    pkgs.rls
    pkgs.rust-analyzer
    pkgs.rustfmt
    pkgs.just
    pkgs.clippy
    pkgs.rustfmt
    pkgs.rustc
    pkgs.cargo-watch
    pkgs.cargo-deny
    pkgs.pre-commit
    pkgs.git # needed for pre-commit install
    pythonEnv

    pkgs.qemu_kvm
    pkgs.gdb
    pkgs.tmux # needed for integration test
  ] ++ vmsh.nativeBuildInputs;
  buildInputs = vmsh.buildInputs;
  shellHook = ''
    pre-commit install
  '';
}

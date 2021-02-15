{ pkgs ? (import (import ./nix/sources.nix).nixpkgs {}) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk {};
in pkgs.callPackage ./vmsh.nix {
  inherit naersk;
}

{ pkgs ? (import (import ./sources.nix).nixpkgs {}) }:

(pkgs.callPackage ./kernel-fhs.nix {
  runScript = "bash";
}).env

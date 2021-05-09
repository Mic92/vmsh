{
  description = "Spawn debug shells in virtual machines";

  inputs = {
    not-os.url = "github:cleverca22/not-os";
    not-os.flake = false;
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix, not-os }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        inherit (fenix.packages.${system}.latest) cargo rustc;

        toolchain = with fenix.packages.${system};
          combine [
            rustc
            cargo
            targets."aarch64-unknown-linux-gnu".latest.rust-std
            targets."x86_64-unknown-linux-gnu".latest.rust-std
          ];

        vmsh = pkgs.callPackage ./nix/vmsh.nix {
          pkgSrc = self;
        };

        kernel-fhs = pkgs.callPackage ./nix/kernel-fhs.nix {};

        rustPlatform = (pkgs.makeRustPlatform {
          inherit cargo rustc;
        });

        ciDeps = [
          rustc
          cargo
          pkgs.qemu_kvm
          pkgs.tmux # needed for integration test
          (pkgs.python3.withPackages (ps: [
            ps.pytest
            ps.pytest-xdist
            ps.pyelftools
            ps.intervaltree

            # linting
            ps.black
            ps.flake8
            ps.isort
            ps.mypy
          ]))
        ] ++ vmsh.nativeBuildInputs;
      in
      rec {
        # default target for `nix build`
        defaultPackage = vmsh;
        packages = {
          inherit vmsh;

          # used in .drone.yml
          ci-shell = pkgs.mkShell {
            inherit (vmsh) buildInputs;
            nativeBuildInputs = ciDeps;
          };

          # see justfile/build-linux-shell
          kernel-fhs-shell = (kernel-fhs.override { runScript = "bash"; }).env;

          kernel-fhs = kernel-fhs;

          # see justfile/not-os
          not-os-image = pkgs.callPackage ./nix/not-os-image.nix {
            inherit not-os;
          };

          # see justfile/nixos-image
          nixos-image = pkgs.callPackage ./nix/nixos-image.nix {};
        };
        # used by `nix develop`
        devShell = pkgs.mkShell {
          inherit (vmsh) buildInputs;
          RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
          nativeBuildInputs = ciDeps ++ [
            pkgs.jq
            pkgs.rls
            pkgs.rust-analyzer
            pkgs.rustfmt
            pkgs.just
            pkgs.clippy
            pkgs.rustfmt
            pkgs.cargo-watch
            pkgs.cargo-deny
            pkgs.pre-commit
            pkgs.git # needed for pre-commit install

            pkgs.gdb
          ];

          shellHook = ''
            pre-commit install
            export KERNELDIR=$(pwd)/../linux;
          '';
          # interesting when supporting aarch64
          #CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER =
          #  "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-gcc";
        };
      });
}

{
  description = "Spawn debug shells in virtual machines";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # TODO: switch back when https://github.com/cleverca22/not-os/pull/19 is merged
    #not-os.url = "github:cleverca22/not-os";
    not-os.url = "github:Mic92/not-os/overridable-kernel";
    not-os.flake = false;
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix, not-os }:
    (flake-utils.lib.eachSystem ["x86_64-linux"] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};
        rustToolchain = with fenixPkgs; combine [
          latest.cargo
          latest.rustc
          latest.rust-src
          latest.rust-std
          latest.clippy-preview
          latest.rustfmt-preview
          targets.x86_64-unknown-linux-musl.latest.rust-std
          # fenix.packages.x86_64-linux.targets.aarch64-unknown-linux-gnu.latest.rust-std
        ];

        rustPlatform = (pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        });

        vmsh = pkgs.callPackage ./nix/vmsh.nix {
          pkgSrc = self;
          inherit rustPlatform;
        };

        kernel-deps = pkgs.callPackage ./nix/kernel-deps.nix {};
        kernel-deps-shell = (pkgs.callPackage ./nix/kernel-deps.nix {
          runScript = "bash";
        }).env;

        measureDeps = [
          pkgs.numactl
          pkgs.fio
          (pkgs.python3.withPackages (ps: [
            ps.matplotlib
            ps.pandas
          ]))
        ];

        ciDeps = [
          rustToolchain
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
        ] ++ vmsh.nativeBuildInputs ++ measureDeps;

        not-os-image = (pkgs.callPackage ./nix/not-os-image.nix {
          inherit not-os;
          notos-config = ./nix/modules/not-os-config.nix;
        }).json;
        measurement-image = (pkgs.callPackage ./nix/not-os-image.nix {
          inherit not-os;
          notos-config = import ./nix/modules/measurement-config.nix {
            inherit (self.nixosModules) linux-ioregionfd;
          };
        }).json;
        linux_ioregionfd = pkgs.callPackage ./nix/linux-ioregionfd.nix { };
      in {
        # default target for `nix build`
        defaultPackage = vmsh;
        packages = rec {
          inherit vmsh linux_ioregionfd;

          # see justfile/build-linux-shell
          inherit kernel-deps;

          # used in tests/xfstests.py
          xfstests = pkgs.callPackage ./nix/xfstests.nix { };

          # see justfile/not-os
          inherit not-os-image measurement-image;

          # see justfile/nixos-image
          nixos-image = pkgs.callPackage ./nix/nixos-image.nix {};
          busybox-image = pkgs.callPackage ./nix/busybox-image.nix {};
          passwd-image = pkgs.callPackage ./nix/passwd-image.nix {};
          phoronix-image = pkgs.callPackage ./nix/phoronix-image.nix {};

          phoronix-test-suite = pkgs.callPackage ./nix/phoronix.nix {};
        };
        devShells  = rec {
          # used in .drone.yml
          ci-shell = pkgs.mkShell {
            inherit (vmsh) buildInputs;
            nativeBuildInputs = ciDeps;
          };
          # see justfile/build-linux-shell
          inherit kernel-deps-shell;
        };
        # not supported by nix flakes yet, but useful
        packageSets = rec {
          linuxPackages_ioregionfd = pkgs.recurseIntoAttrs (pkgs.linuxPackagesFor linux_ioregionfd);
        };
        checks = self.packages.${system};
        # used by `nix develop`
        devShell = pkgs.mkShell {
          inherit (vmsh) buildInputs;
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          nativeBuildInputs = ciDeps ++ [
            pkgs.socat
            pkgs.jq # needed for justfile
            pkgs.just
            pkgs.cargo-watch
            pkgs.cargo-deny
            pkgs.pre-commit
            pkgs.rls
            pkgs.git # needed for pre-commit install
            fenixPkgs.rust-analyzer
            pkgs.gdb
            # pkgs.libguestfs-with-appliance # needed for just attach-qemu-img and thus stress-test
            pkgs.gnuplot
            pkgs.cloud-hypervisor
            pkgs.crosvm
            pkgs.firectl
            (pkgs.writeShellScriptBin "firecracker" ''
              exec ${pkgs.firecracker}/bin/firecracker --seccomp-level 0 "$@"
            '')
            
            # for xfstests:
            pkgs.parted
            pkgs.xfsprogs
          ];

          shellHook = ''
            pre-commit install
          '';
          # interesting when supporting aarch64
          #CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER =
          #  "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-gcc";
        };
      })) // {
        nixosModules.linux-ioregionfd = { pkgs, ... }: {
          boot.kernelPackages = self.packageSets.${pkgs.system}.linuxPackages_ioregionfd;
        };
        lib.nixpkgsRev = nixpkgs.shortRev;
      };
}

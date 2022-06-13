{
  description = "Spawn debug shells in virtual machines";

  inputs = {
    # TODO xfsdump is broken in master
    nixpkgs.url = "github:NixOS/nixpkgs/152e9283c5b91174be7b33eef68a1496ccbc1507";

    not-os.url = "github:cleverca22/not-os";
    not-os.flake = false;
    flake-utils.url = "github:numtide/flake-utils";
    microvm.url = "github:Mic92/microvm.nix";
    microvm.inputs.nixpkgs.follows = "nixpkgs";
    microvm.inputs.flake-utils.follows = "flake-utils";
    fenix = {
      url = "github:nix-community/fenix/b3e5ce9985c380c8fe1b9d14879a14b749d1af51";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix, not-os, microvm }:
    (flake-utils.lib.eachSystem ["x86_64-linux"] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};
        rustToolchain = with fenixPkgs; combine [
          stable.cargo
          stable.rustc
          stable.rust-src
          stable.rust-std
          stable.clippy
          stable.rustfmt
          targets.x86_64-unknown-linux-musl.stable.rust-std
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
        });


        measureDeps = [
          # only used in tests/reproduce.py
          pkgs.lsof

          pkgs.numactl
          pkgs.fio
          (pkgs.python3.withPackages (ps: [
            ps.seaborn
            ps.pandas
            ps.psutil
            ps.natsort
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
            ps.lxml

            # linting
            ps.black
            ps.flake8
            ps.isort
            ps.mypy
          ]))
        ] ++ vmsh.nativeBuildInputs ++ measureDeps;

        not-os-image' = (pkgs.callPackage ./nix/not-os-image.nix {
          inherit not-os;
          notos-config = [
            ./nix/modules/not-os-config.nix
          ];
        });
        not-os-image_4_4 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_4_4; }).json;
        not-os-image_4_19 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_4_19; }).json;
        not-os-image_5_10 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_5_10; }).json;
        not-os-image_5_15 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_5_15; }).json;
        not-os-image_5_16 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_5_16; }).json;

        measurement-image = (pkgs.callPackage ./nix/not-os-image.nix {
          inherit not-os;
          notos-config = [
            ./nix/modules/measurement-config.nix
            # we mainly need this for XFS_ONLINE_SCRUB
            self.nixosModules.linux-ioregionfd
          ];
        }).json;

        linux_ioregionfd = pkgs.callPackage ./nix/linux-ioregionfd.nix { };
        kvmtool = pkgs.callPackage ./nix/kvmtool.nix { };
        xfstests = pkgs.callPackage ./nix/xfstests.nix { };
      in {
        # default target for `nix build`
        defaultPackage = vmsh;
        packages = rec {
          inherit vmsh linux_ioregionfd;

          # see justfile/build-linux-shell
          inherit kernel-deps;

          # for testing
          inherit kvmtool;

          # used in tests/xfstests.py
          inherit xfstests;

          # see justfile/not-os
          not-os-image = not-os-image'.json;
          inherit
            not-os-image_4_4
            not-os-image_4_19
            not-os-image_5_10
            not-os-image_5_15
            not-os-image_5_16;
          inherit measurement-image;

          # see justfile/nixos-image
          nixos-image = pkgs.callPackage ./nix/nixos-image.nix {};
          busybox-image = pkgs.callPackage ./nix/busybox-image.nix {};
          passwd-image = pkgs.callPackage ./nix/passwd-image.nix {};
          alpine-sec-scanner = pkgs.callPackage ./nix/alpine-sec-scanner.nix {};
          alpine-db = pkgs.callPackage ./nix/alpine-db.nix {};
          alpine-sec-scanner-image = pkgs.callPackage ./nix/alpine-sec-scanner-image.nix {};
          phoronix-image = pkgs.callPackage ./nix/phoronix-image.nix {};
          alpine-image = pkgs.callPackage ./nix/alpine-image.nix {};
          fat-image = pkgs.callPackage ./nix/fat-image.nix {};

          inherit (microvm.packages.${pkgs.system})
            firecracker-example crosvm-example kvmtool-example qemu-example;

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
            pkgs.rust-analyzer
            pkgs.gdb
            # pkgs.libguestfs-with-appliance # needed for just attach-qemu-img and thus stress-test
            pkgs.gnuplot
            pkgs.cloud-hypervisor
            pkgs.crosvm
            pkgs.firectl
            kvmtool
            pkgs.firecracker

            # for xfstests:
            pkgs.parted
            pkgs.xfsprogs
            xfstests
          ];

          shellHook = ''
            pre-commit install
          '';
          # interesting when supporting aarch64
          #CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER =
          #  "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-gcc";
        };
      })) // {
        nixosModules.linux-ioregionfd = { pkgs, lib, ... }: {
          boot.kernelPackages = lib.mkForce self.packageSets.${pkgs.system}.linuxPackages_ioregionfd;
        };
        lib.nixpkgsRev = nixpkgs.shortRev;
      };
}

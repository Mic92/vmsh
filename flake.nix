{
  description = "Spawn debug shells in virtual machines";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    not-os.url = "github:cleverca22/not-os";
    not-os.flake = false;
    flake-utils.url = "github:numtide/flake-utils";
    microvm.url = "github:Mic92/microvm.nix";
    microvm.inputs.nixpkgs.follows = "nixpkgs";
    microvm.inputs.flake-utils.follows = "flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ { flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } ({ self, lib, ... }: {
      systems = [ "x86_64-linux" ];
      perSystem = { config, pkgs, inputs', ... }:
        let
          fenixPkgs = inputs'.fenix.packages;
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
          ] ++ config.packages.vmsh.nativeBuildInputs ++ measureDeps;

          not-os-image' = (pkgs.callPackage ./nix/not-os-image.nix {
            inherit (inputs) not-os;
            notos-config = [
              ./nix/modules/not-os-config.nix
            ];
          });
        in
        {
          # default target for `nix build`
          packages = {
            vmsh = pkgs.callPackage ./nix/vmsh.nix {
              pkgSrc = self;
              inherit rustPlatform;
            };
            linux_ioregionfd = pkgs.callPackage ./nix/linux-ioregionfd.nix { };
            default = config.packages.vmsh;
            # see justfile/build-linux-shell
            kernel-deps = pkgs.callPackage ./nix/kernel-deps.nix { };

            # used in tests/xfstests.py
            xfstests = pkgs.callPackage ./nix/xfstests.nix { };

            not-os-image = not-os-image'.json;
            not-os-image_4_19 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_4_19; }).json;
            not-os-image_5_10 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_5_10; }).json;
            not-os-image_5_15 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_5_15; }).json;
            not-os-image_6_1 = (not-os-image'.override { linuxPackages = pkgs.linuxPackages_6_1; }).json;

            measurement-image = (pkgs.callPackage ./nix/not-os-image.nix {
              inherit (inputs) not-os;
              notos-config = [
                ./nix/modules/measurement-config.nix
                # we mainly need this for XFS_ONLINE_SCRUB
                self.nixosModules.linux-ioregionfd
              ];
            }).json;

            inherit (inputs'.microvm.packages)
              firecracker-example crosvm-example kvmtool-example qemu-example;

            # see justfile/nixos-image
            nixos-image = pkgs.callPackage ./nix/nixos-image.nix { };
            busybox-image = pkgs.callPackage ./nix/busybox-image.nix { };
            passwd-image = pkgs.callPackage ./nix/passwd-image.nix { };
            alpine-sec-scanner = pkgs.callPackage ./nix/alpine-sec-scanner.nix { };
            alpine-db = pkgs.callPackage ./nix/alpine-db.nix { };
            alpine-sec-scanner-image = pkgs.callPackage ./nix/alpine-sec-scanner-image.nix { };
            phoronix-image = pkgs.callPackage ./nix/phoronix-image.nix { };
            alpine-image = pkgs.callPackage ./nix/alpine-image.nix { };
            fat-image = pkgs.callPackage ./nix/fat-image.nix { };

            phoronix-test-suite = pkgs.callPackage ./nix/phoronix.nix {};
          };
          legacyPackages = {
            linuxPackages_ioregionfd = pkgs.recurseIntoAttrs (pkgs.linuxPackagesFor config.packages.linux_ioregionfd);
          };
          # used by `nix develop`

          devShells = {
            default = pkgs.mkShell {
              inherit (config.packages.vmsh) buildInputs;
              RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
              nativeBuildInputs = ciDeps ++ [
                pkgs.socat
                pkgs.jq # needed for justfile
                pkgs.just
                pkgs.cargo-watch
                pkgs.cargo-deny
                pkgs.rust-analyzer
                pkgs.gdb
                # pkgs.libguestfs-with-appliance # needed for just attach-qemu-img and thus stress-test
                pkgs.gnuplot
                pkgs.cloud-hypervisor
                pkgs.crosvm
                pkgs.firectl
                pkgs.firecracker
                pkgs.kvmtool

                # for xfstests:
                pkgs.parted
                pkgs.xfsprogs
                config.packages.xfstests
              ];

              # interesting when supporting aarch64
              #CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER =
              #  "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-gcc";
            };
            kernel-deps-no-fhs = pkgs.callPackage ./nix/kernel-deps-no-fhs.nix { };
            kernel-deps-shell = (pkgs.callPackage ./nix/kernel-deps.nix {
              runScript = "bash";
            });
            # used in .drone.yml
            ci-shell = pkgs.mkShell {
              inherit (config.packages.vmsh) buildInputs;
              nativeBuildInputs = ciDeps;
            };
          };
        };
      flake = {
        nixosModules.linux-ioregionfd = { pkgs, lib, ... }: {
          boot.kernelPackages = lib.mkForce self.packageSets.${pkgs.system}.linuxPackages_ioregionfd;
        };
        lib.nixpkgsRev = inputs.nixpkgs.shortRev;
      };
    });
}

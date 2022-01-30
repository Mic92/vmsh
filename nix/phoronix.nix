{ lib
, fetchurl
, stdenv
, fetchFromGitHub
, buildFHSUserEnv
, php
, which
, gnused
, makeWrapper
, gnumake
, gcc
, callPackage
, util-linux
, enableBuildDeps ? false
}:
let
  pkg = stdenv.mkDerivation rec {
    pname = "phoronix-test-suite";
    version = "unstable-2021-08-25";

    # fixes partition selection: https://github.com/Mic92/phoronix-test-suite/commit/e9941415d2d8260697ceba18783dc85c75910e85
    src = fetchFromGitHub {
      owner = "Mic92";
      repo = "phoronix-test-suite";
      rev = "e7e1edf3aa5723ed91f589ea45d30db9ab08aeac";
      sha256 = "sha256-Ohd4gNrnb8H2SqYaV2LRUfWfY2A60/vuUNu9faTv5p8=";
    };

    buildInputs = [ php ];
    nativeBuildInputs = [ which gnused makeWrapper ];

    installPhase = ''
      ./install-sh $out
      wrapProgram $out/bin/phoronix-test-suite \
      --set PHP_BIN ${php}/bin/php
    '';

    meta = with lib; {
      description = "Open-Source, Automated Benchmarking";
      homepage = "https://www.phoronix-test-suite.com/";
      license = licenses.gpl3;
      platforms = with platforms; unix;
    };
  };
  phoronix-cache = stdenv.mkDerivation {
    name = "phoronix-cache";
    src = fetchurl {
      url = "https://github.com/Mic92/vmsh/releases/download/assets/phoronix-cache-2022-01-30.tar.gz";
      sha256 = "sha256-sh9ZGB1sTC1dpDG1uK6oygBTi4fRuOmJlOq25CJ1qbw=";
    };
    nativeBuildInputs = [ (fhs true) ];
    buildPhase = ''
      runHook preBuild
      # The trailing slash is crucial here :(
      export PTS_USER_PATH_OVERRIDE=$(pwd)/
      export PTS_DOWNLOAD_CACHE=$(pwd)/download-cache/
      set +o pipefail

      yes | phoronix-test-suite install pts/disk

      set -o pipefail
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      mkdir $out
      cp -r . $out
      cp -r ../.phoronix-test-suite/installed-tests $out/installed-tests
      runHook postInstall
    '';
  };
  fhs = enableBuildDeps': buildFHSUserEnv {
    name = "phoronix-test-suite";
    targetPkgs = pkgs: with pkgs; [
      php
      bash
      coreutils
      util-linux
      popt
      libaio
      pcre
      glibc
      glibc.static
      openmpi
      openssh # ior wants either rsh or ssh
      # for systemd-detect-virt
      systemd
      which
      python2
      pciutils
      zlib
    ] ++ lib.optionals (enableBuildDeps') [
      binutils
      automake
      autoconf
      m4
      bc
      perl
      gcc
      python3
    ];
    multiPkgs = null;
    passthru.phoronix-cache = phoronix-cache;
    extraOutputsToInstall = [ "dev" ];
    profile = ''
      export hardeningDisable=all
    '';
    runScript = "${pkg}/bin/phoronix-test-suite";
  };
in
fhs enableBuildDeps

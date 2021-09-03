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
      rev = "e9941415d2d8260697ceba18783dc85c75910e85";

      sha256 = "sha256-U0eW314nJ0r55OjrYFrR0EY8IS97YFESgOlVa9gs3UI=";
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
      url = "https://github.com/Mic92/vmsh/releases/download/assets/phoronix-2021-08-25.tar.gz";
      sha256 = "sha256-0WDEtCHvefGfuN3wFjKY6GOagMdmgo/7ds9Ty+4bIWc=";
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
      # for systemd-detect-virt
      systemd
      which
    ] ++ lib.optionals (enableBuildDeps') [
      binutils
      automake
      autoconf
      m4
      bc
      perl
      gcc
      python2
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

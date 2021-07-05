{ config, lib, pkgs, ... }:

{
  boot.kernelPackages = let
    linux_patched_pkg =
      { buildLinux, fetchurl, modDirVersionArg ? null, ... }@args:
      buildLinux (args // rec {
        version = "5.12.0";
        modDirVersion = if (modDirVersionArg == null) then
          builtins.replaceStrings [ "-" ] [ ".0-" ] version
        else
          modDirVersionArg;
        src = pkgs.fetchFromGitHub {
          owner = "Mic92";
          repo = "linux";
          rev = "56b6b3611b3a57940a314673e1c7aecbc07976e1";
          sha256 = "sha256-VKtKBIbUoRGp2xJA7VQvjRGPaTaNP04vrjMXDpmOje8=";
        };
        inherit (pkgs.linuxPackages_5_13.kernel) kernelPatches;
        extraMeta.branch = "5.12";
        ignoreConfigErrors = true;
      } // (args.argsOverride or { }));
    linux_patched = pkgs.callPackage linux_patched_pkg { };
  in pkgs.recurseIntoAttrs (pkgs.linuxPackagesFor linux_patched);
}

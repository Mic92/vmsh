{ buildLinux, fetchFromGitHub, linuxPackages_5_13, fetchurl, modDirVersionArg ? null, ... }@args:
buildLinux (args // rec {
  version = "5.12.14";
  modDirVersion = if (modDirVersionArg == null) then
    builtins.replaceStrings [ "-" ] [ ".0-" ] version
      else
    modDirVersionArg;
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "linux";
    rev = "56b6b3611b3a57940a314673e1c7aecbc07976e1";
    sha256 = "sha256-VKtKBIbUoRGp2xJA7VQvjRGPaTaNP04vrjMXDpmOje8=";
  };

  kernelPatches = [{
    name = "enable-kvm-ioregion";
    patch = null;
    extraConfig = ''
      KVM_IOREGION y
    '';
  # 5.12 patch list has one fix we already have in our branch
  }] ++ linuxPackages_5_13.kernel.kernelPatches;
  extraMeta.branch = "5.12";
  ignoreConfigErrors = true;
} // (args.argsOverride or { }))

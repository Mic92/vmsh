{ buildLinux, fetchFromGitHub, linuxPackages_5_15, modDirVersionArg ? null, ... }@args:

buildLinux (args // rec {
  version = "5.12.14";
  modDirVersion =
    if (modDirVersionArg == null) then
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
    # we need XFS_ONLINE_SCRUB this for xfstests
    extraConfig = ''
      KVM_IOREGION y
      XFS_ONLINE_SCRUB y
    '';
    # 5.12 patch list has one fix we already have in our branch
  }] ++ linuxPackages_5_15.kernel.kernelPatches;
  extraMeta.branch = "5.12";
  ignoreConfigErrors = true;
} // (args.argsOverride or { }))

{ buildLinux, fetchFromGitHub, linuxPackages_5_16, ... }@args:
buildLinux (args // rec {
  version = "5.16.2";
  modDirVersion = builtins.replaceStrings [ "-" ] [ ".0-" ] version;
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "linux";
    rev = "892dbc39579f6305bdc6f0c77c9247599a028d7b";
    sha256 = "sha256-k/xGMSdFjM4TwzrIsHsXcM+SLCWUVfUh8SOoAYVaCXU=";
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
  }] ++ linuxPackages_5_16.kernel.kernelPatches;
  extraMeta.branch = linuxPackages_5_16.kernel.meta.branch;
  ignoreConfigErrors = true;
} // (args.argsOverride or { }))

{ buildLinux, fetchFromGitHub, linuxPackages_5_13, fetchurl, modDirVersionArg ? null, ... }@args:
buildLinux (args // rec {
  version = "5.12.15";
  modDirVersion = if (modDirVersionArg == null) then
    builtins.replaceStrings [ "-" ] [ ".0-" ] version
      else
    modDirVersionArg;
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "linux";
    rev = "4b0eade1821b48d288416076d52408609ff08766";
    sha256 = "sha256-gEJdqUlcl6METkGBXQ1Wh5uVTEH5xXCMN8C0/Kqbi3k=";
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

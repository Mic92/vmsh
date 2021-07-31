{ pkgs, lib, modulesPath, ... }:

let
  keys = map (key: "${builtins.getEnv "HOME"}/.ssh/${key}")
    [ "id_rsa.pub" "id_ecdsa.pub" "id_ed25519.pub" ];
in
{
  imports = [
    (modulesPath + "/profiles/qemu-guest.nix")
    ./nested-qemu.nix
  ];
  boot.loader.grub.enable = false;
  boot.initrd.enable = false;
  boot.isContainer = true;
  boot.loader.initScript.enable = true;
  ## login with empty password
  users.extraUsers.root.initialHashedPassword = "";
  services.openssh.enable = true;

  users.users.root.openssh.authorizedKeys.keyFiles = lib.filter builtins.pathExists keys;
  networking.firewall.enable = false;

  fileSystems."/mnt" = {
    device = "home";
    fsType = "9p";
    # skip mount in nested qemu
    options = [ "trans=virtio" "nofail" "msize=104857600" ];
  };

  fileSystems."/linux" = {
    device = "linux";
    fsType = "9p";
    # skip mount in nested qemu
    options = [ "trans=virtio" "nofail" "msize=104857600" ];
  };

  users.users.root.openssh.authorizedKeys.keys = [
    (builtins.readFile ../ssh_key.pub)
  ];

  services.getty.helpLine = ''
    Log in as "root" with an empty password.
    If you are connect via serial console:
    Type Ctrl-a c to switch to the qemu console
    and `quit` to stop the VM.
  '';

  documentation.doc.enable = false;
  documentation.man.enable = false;
  documentation.nixos.enable = false;
  documentation.info.enable = false;
  programs.bash.enableCompletion = false;
  programs.command-not-found.enable = false;

  environment.systemPackages = [
    pkgs.busybox
    pkgs.devmem2
    pkgs.sysbench
  ];
}

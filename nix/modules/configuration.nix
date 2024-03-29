{ pkgs, lib, modulesPath, ... }:

let
  keys = map (key: "${builtins.getEnv "HOME"}/.ssh/${key}")
    [ "id_rsa.pub" "id_ecdsa.pub" "id_ed25519.pub" ];
  myxfstests = pkgs.callPackage ../xfstests.nix { };
in
{
  imports = [
    (modulesPath + "/profiles/qemu-guest.nix")
    ./nested-vms.nix
  ];
  # somehow udev does not pick up hvc0?
  systemd.services."serial-getty" = {
    wantedBy = [ "multi-user.target" ];
    serviceConfig.ExecStart = "${pkgs.util-linux}/sbin/agetty  --login-program ${pkgs.shadow}/bin/login --autologin root hvc0 --keep-baud vt100";
  };
  systemd.services."serial-getty@hvc0".enable = false;

  # slows things down
  systemd.services.systemd-networkd-wait-online.enable = false;

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

  services.getty.autologinUser = lib.mkDefault "root";

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
    pkgs.cloud-hypervisor
    myxfstests
  ];
}

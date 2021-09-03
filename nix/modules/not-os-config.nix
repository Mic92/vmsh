{ lib, pkgs, config, ... }:
{
  environment.systemPackages = [
    pkgs.utillinux
    pkgs.gnugrep
    pkgs.kmod
    pkgs.devmem2
    # for debugging
    pkgs.strace
  ];

  environment.pathsToLink = [ "/lib/modules" ];

  networking.timeServers = [];

  not-os.nix = true;
  not-os.simpleStaticIp = true;
  not-os.preMount = ''
    echo 'nixos' > /proc/sys/kernel/hostname
    ip addr add 127.0.0.1/8 dev lo
    ip addr add ::1/128 dev lo
    ip link set dev lo up
    ip addr add 10.0.2.15/24 dev eth0

    # for stage1 debugging
    #${pkgs.utillinux}/bin/setsid -c ${pkgs.bash}/bin/bash -l
  '';

  boot.initrd.kernelModules = [
    "virtio_console"

    "virtio_mmio"
    # ext4
    "crc16" "mbcache" "jbd2" "crc32c_generic" "ext4"

    # xfs
    "xfs" "libcrc32c"

    # 9p over virtio
    "9pnet" "9p" "9pnet_virtio" "fscache"
  ];

  boot.kernelPatches = [{
    name = "xfs enable extra features";
    patch = null;
    extraConfig = ''
      XFS_ONLINE_SCRUB y
    '';
  }];

  system.activationScripts.vmsh = ''
    mkdir /vmsh
    mount -t 9p vmsh /vmsh -o trans=virtio,msize=104857600
    ln -s /proc/self/fd /dev/fd
    ln -s /proc/mounts /etc/mtab
  '';

  environment.etc = {
    "hosts".text = ''
      127.0.0.1 localhost
      ::1 localhost
      127.0.0.1 nixos
      ::1 nixos
    '';
    "passwd".text = ''
      sys:x:993:991::/var/empty:/run/current-system/sw/bin/nologin
      bin:x:994:992::/var/empty:/run/current-system/sw/bin/nologin
      daemon:x:995:993::/var/empty:/run/current-system/sw/bin/nologin
      fsgqa2:x:996:995::/var/empty:/bin/sh
      fsgqa:x:997:996::/var/empty:/bin/sh
      123456-fsgqa:x:998:996::/var/empty:/bin/sh
      nobody:x:65534:65534:Unprivileged account (don't use!):/var/empty:/run/current-system/sw/bin/nologin
    '';
    "group".text = ''
      sys:x:991:
      bin:x:992:
      daemon:x:993:
      123456-fsgqa:x:994:
      fsgqa2:x:995:
      fsgqa:x:996:
    '';
#    "shadow".text = ''
#      fsgqa2:!:1::::::
#      fsgqa:!:1::::::
#      123456-fsgqa:!:1::::::
#      daemon:!:1::::::
#      sys:!:1::::::
#      bin:!:1::::::
#    '';
    "security/pam_env.conf".text = "

    ";
    "sudoers".text = ''
      root        ALL=(ALL:ALL) SETENV: ALL
    '';
    "pam.d/other".text = ''
      #auth     required pam_warn.so
      #auth     sufficient pam_rootok.so
      auth     sufficient pam_permit.so
      account  required pam_permit.so
      password required pam_permit.so
      session  optional pam_env.so
      # Account management.
      #account required pam_unix.so
      # Authentication management.
#      auth sufficient ${pkgs.linux-pam}/lib/security/pam_rootok.so
      #auth required pam_faillock.so
      #auth sufficient pam_unix.so   likeauth try_first_pass
      #auth required pam_deny.so
      # Password management.
      #password sufficient pam_unix.so nullok sha512
      # Session management.
      #session required pam_env.so conffile=/nix/store/zg2hdxcf62dpxfn9y7v8arwbyhzxnahy-pam-environment readenv=0
      #session required ${pkgs.linux-pam}/lib/security/pam_env.so conffile=${pkgs.linux-pam}/etc/security/pam_env.conf
      #session required ${pkgs.linux-pam}/lib/security/pam_env.so conffile=/etc/pam_env_conf
      #session required pam_env.so
      #session required pam_unix.so
      #session optional pam_unix.so
      #session optional pam_xauth.so 
       #xauthpath=/nix/store/wmww8v2s3rn0r48rai1h3r0diza69c9r-xauth-1.1/bin/xauth systemuser=99
    '';
    "ssh/authorized_keys.d/root" = {
      source = pkgs.writeText "ssh_key" (builtins.readFile ../ssh_key.pub);
      mode = "444";
    };
    "service/shell/run".source = pkgs.writeScript "shell" ''
      #!/bin/sh
      export USER=root
      export HOME=/root
      cd $HOME

      source /etc/profile

      exec < /dev/hvc0 > /dev/hvc0 2>&1
      echo "If you are connect via serial console:"
      echo "Type Ctrl-a c to switch to the qemu console"
      echo "and 'quit' to stop the VM."
      exec ${pkgs.utillinux}/bin/setsid -c ${pkgs.bash}/bin/bash -l
    '';
  };
  environment.etc.profile.text = ''
    export PS1="\e[0;32m[\u@\h \w]\$ \e[0m"
  '';
}

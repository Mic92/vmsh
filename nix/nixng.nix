{ nglib
, busybox
, bash
, nixpkgs
, writeShellScript
}:
nglib.makeSystem {
  inherit nixpkgs;
  system = "x86_64-linux";
  name = "nixng";

  config = ({ ... }: {
    runit.enable = true;

    fstab.entries = {
      "/" = {
        type = "ext4";
        device = "/dev/sda1";
      };
    };

    networking.hostName = "nixng";

    init.services.login-shell = {
      enabled = true;
      script = writeShellScript "login-shell-run" ''
        export USER=root
        export HOME=/root
        cd $HOME
        export TERM=xterm-256color
        export PS1="\e[0;32m[\u@\h \w]\$ \e[0m"

        exec < /dev/hvc0 > /dev/hvc0 2>&1
        exec ${busybox}/bin/setsid ${busybox}/bin/cttyhack ${bash}/bin/bash -l
      '';
    };
  });
}

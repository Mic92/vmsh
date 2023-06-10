{ pkgs }:
let
  buildDiskImage = pkgs.callPackage ./build-disk-image.nix { };
  inherit (pkgs.pkgsStatic) busybox;
  myvim = pkgs.vim_configurable.customize {
    name = "vim";
    vimrcConfig.customRC = builtins.readFile ./modules/vimrc;
    vimrcConfig.packages.nixbundle.start = with pkgs.vimPlugins; [
      vim-sensible
      nerdtree
    ];
  };
in
buildDiskImage {
  packages = with pkgs; [
    busybox
    antigen
    fzf
    tree
    git
    tmux
    psmisc
    # libguestfs-with-appliance
    lazygit
    ack
    ripgrep
    bottom # btm
    myvim
    tcpdump
    htop
    zsh
    antigen
    #doom-emacs
    # rustup
  ];
  extraFiles = {
    "etc/profile" = ''
      export PATH=/bin
    '';
    # "etc/zshrc_actual" = builtins.readFile ./modules/zshrc;
    # "etc/zshrc" = ''
    #   source ${pkgs.antigen}/share/antigen/antigen.zsh
    #   source /etc/zshrc_actual
    # '';
  };
  diskSize = "1G";
  extraCommands = ''
    pushd root
    mkdir bin
    ln -s ${busybox}/bin/sh bin/sh
    ln -s ${busybox}/bin/ls bin/ls
    ln -s ${busybox}/bin/resize bin/resize
    ln -s ${busybox}/bin/ip bin/ip
    ln -s ${busybox}/bin/modprobe bin/modprobe
    ln -s ${myvim}/bin/vim bin/vim
    ln -s ${pkgs.tcpdump}/bin/tcpdump bin/tcpdump
    ln -s ${pkgs.htop}/bin/htop bin/htop
    ln -s ${pkgs.zsh}/bin/zsh bin/zsh
    mkdir -p usr/share/zsh/share
    ln -s ${pkgs.antigen}/share/antigen/antigen.zsh usr/share/zsh/share/antigen.zsh
    mkdir -p proc dev tmp sys
    popd
  '';
}

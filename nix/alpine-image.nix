{ stdenv, fetchurl, cpio, lz4, busybox, pkgsStatic, buildPackages, linuxPackages, zstd }:
stdenv.mkDerivation {
  name = "alpine-initrd";
  src = fetchurl {
    url = "https://github.com/Mic92/vmsh/releases/download/assets/alpine-minirootfs-3.10.0-x86_64.tar.gz";
    sha256 = "sha256-7D2n+19wmhzpEubjH8zFWIQgxfHc7MNixyyYlTLBkXo=";
  };
  unpackPhase = ''
    mkdir alpine-minirootfs
    cd alpine-minirootfs
    tar -xf $src
  '';
  nativeBuildInputs = [ cpio lz4 zstd ];

  installPhase = ''
    install -D -m755 ${pkgsStatic.dropbear.override { enableStatic = false; }}/bin/dropbear usr/bin/dropbear
    install -D -m600 ${./ssh_key.pub} root/.ssh/authorized_keys
    install -d -m600 etc/dropbear

    cp ${busybox}/default.script default.script
    # patch out nix patches from this script
    sed -i -e '1c#!/bin/sh' default.script
    sed -i -e '2c#' default.script
    sed -i -e '3c#' default.script

    cat > init <<EOF
    #! /bin/sh
    mount -t devtmpfs dev /dev
    mkdir /dev/pts
    mount -t devpts devpts /dev/pts
    mount -t proc proc /proc
    mount -t sysfs sysfs /sys
    ip link set up dev lo
    ip link set up dev eth0
    udhcpc -i eth0 -b --script /default.script
    /usr/bin/dropbear -R

    echo "finished booting" > /dev/console
    exec /sbin/getty -n -l /bin/sh 115200 /dev/console
    poweroff -f
    EOF
    chmod +x init

    mkdir $out
    find . -print0 | cpio --null --create --verbose --owner root:root --format=newc | lz4c -l > $out/initramfs.img.lz4

    ${buildPackages.linux.dev}/lib/modules/${buildPackages.linux.modDirVersion}/source/scripts/extract-vmlinux ${linuxPackages.kernel}/bzImage > $out/Image
  '';
}

{ stdenv, fetchurl, cpio, lz4 }:
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
  nativeBuildInputs = [ cpio lz4 ];
  installPhase = ''
    cat > init <<EOF
    #! /bin/sh
    mount -t devtmpfs dev /dev
    mount -t proc proc /proc
    mount -t sysfs sysfs /sys
    ip link set up dev lo

    exec /sbin/getty -n -l /bin/sh 115200 /dev/console
    poweroff -f
    EOF
    chmod +x init

    mkdir $out
    find . -print0 | cpio --null --create --verbose --owner root:root --format=newc | lz4c -l > $out/initramfs.img.lz4
  '';
}

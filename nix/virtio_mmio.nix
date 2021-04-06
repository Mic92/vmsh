{ stdenv, kernel }:

stdenv.mkDerivation {
  name = "virtio_mmio";
  inherit (kernel) src configfile buildInputs nativeBuildInputs;
  postPatch = ''
    patchShebangs scripts
    install -m600 $configfile .config
    cat >> .config <<EOF
    CONFIG_VIRTIO_MMIO_CMDLINE_DEVICES=y
    EOF
  '';
  buildPhase = ''
    make prepare
    make modules_prepare
    make M=drivers/virtio modules
  '';
  installPhase = ''
    install -D drivers/virtio/virtio_mmio.ko $out/lib/modules/virtio/virtio_mmio.ko
  '';
}

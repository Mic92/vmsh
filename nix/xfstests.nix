# taken and adapted from the nixpkgs repo
{ stdenv, acl, attr, autoconf, automake, bash, bc, coreutils, e2fsprogs
, fetchgit, fio, gawk, keyutils, killall, lib, libaio, libcap, libtool
, libuuid, libxfs, lvm2, openssl, perl, procps, quota
, time, util-linux, which, writeScript, xfsprogs, runtimeShell, mktemp
, hostname, gnused, diffutils, findutils, glibc, callPackage, file }:

let
  xfsdump = callPackage ./xfsdump.nix { };
in
stdenv.mkDerivation {
  name = "xfstests-2021-08-22";

  src = fetchgit {
    url = "git://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git";
    rev = "5f8179ce8b001327e0811744dbfdb90a8e934f9c";
    sha256 = "sha256-VV1h3BXaTVeSHfsxGRYzUCo2RcRhOp12xK9od/zaFBo=";
  };

  nativeBuildInputs = [
    autoconf automake libtool
  ];
  buildInputs = [
    acl attr gawk libaio libuuid libxfs openssl perl bc
  ];

  hardeningDisable = [ "format" ];
  enableParallelBuilding = true;

  patchPhase = ''
    # Patch an incompatible awk script with sed. Genious.
    sed -i 's/\/\^\\#\//\/\^#\//' tests/generic/001

    # fix using coreutils
    sed -i 's/SCRATCH_MNT\/ls_on_scratch/SCRATCH_MNT\/ls/' tests/generic/452 tests/generic/452.out


    # needed for qemu-blk. Check if this works natively.
    #sed -i 's/$here\/src\/detached_mounts_propagation/mount $SCRATCH_DEV $SCRATCH_MNT\nmount --make-shared $SCRATCH_MNT\n$here\/src\/detached_mounts_propagation/' tests/generic/632
    # works natively:
    sed -i 's/$DMERROR_DEV/$TEST_DEV/' tests/xfs/006 tests/xfs/264


    # Fix shell-less fsgqa user
    sed -i 's/su $qa_user/su -s \/bin\/sh $qa_user/' common/rc

    substituteInPlace Makefile \
      --replace "cp include/install-sh ." "cp -f include/install-sh ."

    # Patch the destination directory
    sed -i include/builddefs.in -e "s|^PKG_LIB_DIR\s*=.*|PKG_LIB_DIR=$out/lib/xfstests|"

    # Don't canonicalize path to mkfs (in util-linux) - otherwise e.g. mkfs.ext4 isn't found
    sed -i common/config -e 's|^export MKFS_PROG=.*|export MKFS_PROG=mkfs|'

    # Move the Linux-specific test output files to the correct place, or else it will
    # try to move them at runtime. Also nuke all the irix crap.
    for f in tests/*/*.out.linux; do
      mv $f $(echo $f | sed -e 's/\.linux$//')
    done
    rm -f tests/*/*.out.irix

    # Fix up lots of impure paths
    for f in common/* tools/* tests/*/*; do
      sed -i $f -e 's|/bin/bash|${bash}/bin/bash|'
      sed -i $f -e 's|/bin/true|${coreutils}/bin/true|'
      sed -i $f -e 's|/usr/sbin/filefrag|${e2fsprogs}/bin/filefrag|'
      sed -i $f -e 's|hostname -s|hostname|'   # `hostname -s` seems problematic on NixOS
      sed -i $f -e 's|$(_yp_active)|1|'        # NixOS won't ever have Yellow Pages enabled
    done

    for f in src/*.c src/*.sh; do
      sed -e 's|/bin/rm|${coreutils}/bin/rm|' -i $f
      sed -e 's|/usr/bin/time|${time}/bin/time|' -i $f
    done

    patchShebangs .
  '';

  preConfigure = ''
    # The configure scripts really don't like looking in PATH at all...
    export AWK=$(type -P awk)
    export ECHO=$(type -P echo)
    export LIBTOOL=$(type -P libtool)
    export MAKE=$(type -P make)
    export SED=$(type -P sed)
    export SORT=$(type -P sort)

    make configure
  '';

  postInstall = ''
    patchShebangs $out/lib/xfstests

    mkdir -p $out/bin
    substitute $wrapperScript $out/bin/xfstests-check --subst-var out
    chmod a+x $out/bin/xfstests-check
  '';

  # The upstream package is pretty hostile to packaging; it looks up
  # various paths relative to current working directory, and also
  # wants to write temporary files there. So create a temporary
  # to run from and symlink the runtime files to it.
  wrapperScript = writeScript "xfstests-check" ''
    #!${runtimeShell}
    set -e
    export RESULT_BASE="$(pwd)/results"
    export PATH=${ lib.makeBinPath [ mktemp ]}:$PATH

    dir=$(mktemp --tmpdir -d xfstests.XXXXXX)
    trap "rm -rf $dir" EXIT

    chmod a+rx "$dir"
    cd "$dir"
    for f in $(cd @out@/lib/xfstests; echo *); do
      cp -r @out@/lib/xfstests/$f $f
    done

    export PATH=${lib.makeBinPath ([acl attr bc e2fsprogs fio gawk keyutils
                                   libcap lvm2 perl procps killall quota
                                   coreutils
                                   # and some stuff not available in not-os:
                                   hostname
                                   gnused
                                   diffutils
                                   findutils
                                   util-linux
                                   which
                                   xfsprogs
                                   glibc # for getconf
                                   xfsdump
                                   file
                                   ])}:$PATH
    exec ./check "$@"
  '';

  meta = with lib; {
    description = "Torture test suite for filesystems";
    homepage = "https://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git/";
    license = licenses.gpl2;
    maintainers = [ maintainers.dezgeg ];
    platforms = platforms.linux;
  };
}

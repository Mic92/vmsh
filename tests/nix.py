#!/usr/bin/env python3

import functools
import json
import shutil
import subprocess
from contextlib import contextmanager
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any, Iterator

from qemu import VmImage
from root import PROJECT_ROOT


@functools.lru_cache(maxsize=None)
def nix_build(what: str) -> Any:
    path = PROJECT_ROOT.joinpath(".git/nix-results")
    path.mkdir(parents=True, exist_ok=True)
    # gc root to improve caching
    link_name = path.joinpath(what.lstrip(".#"))
    result = subprocess.run(
        [
            "nix",
            "build",
            "--out-link",
            str(link_name),
            "--json",
            what,
        ],
        text=True,
        stdout=subprocess.PIPE,
        check=True,
        cwd=PROJECT_ROOT,
    )
    return json.loads(result.stdout)


def writable_image(name: str) -> Iterator[Path]:
    image = nix_build(name)
    out = image[0]["outputs"]["out"]
    with NamedTemporaryFile() as n:
        with open(out, "rb") as f:
            shutil.copyfileobj(f, n)
        n.flush()
        yield Path(n.name)


@contextmanager
def busybox_image() -> Iterator[Path]:
    yield from writable_image(".#busybox-image")


@contextmanager
def alpine_sec_scanner_image() -> Iterator[Path]:
    yield from writable_image(".#alpine-sec-scanner-image")


@contextmanager
def passwd_image() -> Iterator[Path]:
    yield from writable_image(".#passwd-image")


NOTOS_IMAGE = ".#not-os-image"


def notos_image(nix: str = NOTOS_IMAGE) -> VmImage:
    data = nix_build(nix)
    with open(data[0]["outputs"]["out"]) as f:
        data = json.load(f)
        return VmImage(
            kernel=Path(data["kernel"]),
            kerneldir=Path(data["kerneldir"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]).joinpath("initrd"),
            kernel_params=data["kernelParams"],
        )


def notos_image_custom_kernel(nix: str = NOTOS_IMAGE) -> VmImage:
    """
    This is useful for debugging.
    Make sure to use the same kernel version in your kernel as used in notos
    """
    image = notos_image(nix)
    image.kerneldir = PROJECT_ROOT.joinpath("..", "linux")
    image.kernel = image.kerneldir.joinpath("arch", "x86", "boot")
    return image

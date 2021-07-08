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
    result = subprocess.run(
        ["nix", "build", "--json", what],
        text=True,
        stdout=subprocess.PIPE,
        check=True,
        cwd=PROJECT_ROOT,
    )
    return json.loads(result.stdout)


@contextmanager
def busybox_image() -> Iterator[Path]:
    image = nix_build(".#busybox-image")
    out = image[0]["outputs"]["out"]
    with NamedTemporaryFile() as n:
        with open(out, "rb") as f:
            shutil.copyfileobj(f, n)
        n.flush()
        yield Path(n.name)


def notos_image() -> VmImage:
    data = nix_build(".#not-os-image.json")
    with open(data[0]["outputs"]["out"]) as f:
        data = json.load(f)
        return VmImage(
            kernel=Path(data["kernel"]),
            kerneldir=Path(data["kerneldir"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]),
            kernel_params=data["kernelParams"],
        )


def notos_image_custom_kernel() -> VmImage:
    """
    This is useful for debugging.
    Make sure to use the same kernel version in your kernel as used in notos
    """
    image = notos_image()
    image.kerneldir = PROJECT_ROOT.joinpath("..", "linux")
    image.kernel = image.kerneldir.joinpath("arch", "x86", "boot")
    return image

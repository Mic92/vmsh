import os

import conftest

from nix import notos_image


def test_attach(
    helpers: conftest.Helpers,
    attach_repetitions: int = 1,
    vcpus: int = 1,
    mmio: str = "wrap_syscall",
    image: str = ".#not-os-image",
) -> None:
    with helpers.busybox_image() as img, helpers.spawn_qemu(
        notos_image(image), extra_args=["-smp", str(vcpus)]
    ) as vm:
        vm.wait_for_ssh()
        for i in range(attach_repetitions):
            print(f" ======== repetition {i} ======== ")
            vmsh = helpers.spawn_vmsh_command(
                [
                    "attach",
                    "--backing-file",
                    str(img),
                    "--mmio",
                    mmio,
                    str(vm.pid),
                    "--",
                    "/bin/sh",
                    "-c",
                    "echo works",
                ]
            )

            with vmsh:
                try:
                    vmsh.wait_until_line(
                        "stage1 driver started",
                        lambda line: "stage1 driver started" in line,
                    )
                finally:
                    res = vm.ssh_cmd(["dmesg"], check=False)
                    print("stdout:\n", res.stdout)
                assert res.returncode == 0

                assert (
                    res.stdout.find(
                        "EXT4-fs (vdb): mounted filesystem with ordered data mode"
                    )
                    > 0
                )
            try:
                os.kill(vmsh.pid, 0)
            except ProcessLookupError:
                pass
            else:
                assert False, "vmsh was not terminated properly"

            # See that the VM is still alive after detaching
            res = vm.ssh_cmd(["echo", "ping"], check=False)
            assert res.stdout == "ping\n"
            assert res.returncode == 0


def test_attach_multiple_cpus(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, vcpus=8)


def test_attach_4_4(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_4_4")


def test_attach_4_19(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_4_19")


def test_attach_5_10(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_5_10")


def test_attach_5_15(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_5_15")


def test_attach_5_16(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_5_16")

def test_attach_6_1(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_6_1")

def test_attach_6_3(helpers: conftest.Helpers) -> None:
    test_attach(helpers=helpers, image=".#not-os-image_6_3")

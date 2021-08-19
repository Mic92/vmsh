import conftest

import os


def test_attach(helpers: conftest.Helpers, attach_repetitions: int = 1, vcpus: int = 1, mmio: str = "wrap_syscall") -> None:
    with helpers.busybox_image() as img, helpers.spawn_qemu(
        helpers.notos_image(),
        extra_args=["-smp", str(vcpus)]
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
                        lambda l: "stage1 driver started" in l,
                    )
                finally:
                    res = vm.ssh_cmd(["dmesg"], check=False)
                    print("stdout:\n", res.stdout)
                assert res.returncode == 0

                # with DeviceMmioSpace instead of KvmRunWrapper:
                # assert (
                #     res.stdout.find("virtio_mmio: unknown parameter 'device' ignored") < 0
                # )
                # assert (
                #     res.stdout.find(
                #         "New virtio-mmio devices (version 2) must provide VIRTIO_F_VERSION_1 feature!"
                #     )
                #     >= 0
                # )

                # with KvmRunWrapper:
                assert res.stdout.find("ext4 filesystem being mounted at /tmp/") > 0
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

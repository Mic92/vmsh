import conftest

import os
from root import PROJECT_ROOT


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        vmsh = helpers.spawn_vmsh_command(
            [
                "attach",
                str(vm.pid),
                "--",
                "-i",
                str(PROJECT_ROOT.joinpath("nix", "ssh_key")),
                "-p",
                str(vm.ssh_port),
                "root@127.0.0.1",
            ]
        )

        with vmsh:
            vmsh.wait_until_line(
                "block device driver started",
                lambda l: "block device driver started" in l,
            )

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
            assert (
                res.stdout.find(
                    "virtio_blk virtio3: [vdb] 0 512-byte logical blocks (0 B/0 B)"
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

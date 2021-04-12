import conftest
import subprocess


def test_loading_virtio_mmio(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        res = vm.ssh_cmd(
            ["insmod", "/run/current-system/sw/lib/modules/virtio/virtio_mmio.ko"]
        )
        assert res.returncode == 0
        # assert that virtio_mmio is now loaded
        res = vm.ssh_cmd(["lsmod"])
        assert res.stdout
        assert res.stdout.find("virtio_mmio") >= 0


def test_mprotect(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = helpers.spawn_vmsh_command(
            ["guest_mprotect", str(vm.pid)],
            cargo_executable="test_ioctls",
            stdout=subprocess.PIPE,
        )

        try:
            helpers.vmsh_print_stdout_until(vmsh, "pause\n")
            vm.wait_for_ssh()
            print("ssh available")

            res = vm.ssh_cmd(
                [
                    "devmem2",
                    "0xd0000000",
                    "ww",
                    "0x1337",
                ]
            )
            print("stdout:\n", res.stdout)
            print("stderr:\n", res.stderr)

        finally:
            # we cannot kill sudo, but we can stop vmsh as it drops privileges to our user
            subprocess.run(["pkill", "--parent", str(vmsh.pid)])
            vmsh.wait()
            helpers.vmsh_print_stdout_flush(vmsh)


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = helpers.spawn_vmsh_command(
            ["attach", str(vm.pid)], stdout=subprocess.PIPE
        )

        try:
            vm.wait_for_ssh()
            print("ssh available")

            mmio_config = "0x1000@0xd0000000:5"
            res = vm.ssh_cmd(
                [
                    "insmod",
                    "/run/current-system/sw/lib/modules/virtio/virtio_mmio.ko",
                    f"device={mmio_config}",
                ]
            )
            print("stdout:\n", res.stdout)
            print("stderr:\n", res.stderr)

            helpers.vmsh_print_stdout_until(vmsh, "pause\n")
            res = vm.ssh_cmd(["dmesg"])
            print("stdout:\n", res.stdout)

            assert (
                res.stdout.find("virtio_mmio: unknown parameter 'device' ignored") < 0
            )
            assert (
                res.stdout.find(
                    "New virtio-mmio devices (version 2) must provide VIRTIO_F_VERSION_1 feature!"
                )
                >= 0
            )
        finally:
            # we cannot kill sudo, but we can stop vmsh as it drops privileges to our user
            subprocess.run(["pkill", "--parent", str(vmsh.pid)])
            vmsh.wait()
            helpers.vmsh_print_stdout_flush(vmsh)

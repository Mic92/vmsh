import subprocess

import conftest


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


def test_userfaultfd_completes(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = helpers.spawn_vmsh_command(
            ["guest_userfaultfd", str(vm.pid)],
            cargo_executable="test_ioctls",
            stdout=subprocess.PIPE,
        )

        with vmsh:
            vmsh.wait_until_line("pause")
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


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = helpers.spawn_vmsh_command(
            ["attach", str(vm.pid)], stdout=subprocess.PIPE
        )

        with vmsh:
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

            vmsh.wait_until_line("pause")
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


def test_wrap_syscall(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        # attach vmsh after boot, because it slows the vm down a lot.
        vmsh = helpers.spawn_vmsh_command(
            ["guest_kvm_exits", str(vm.pid)],
            stdout=subprocess.PIPE,
            cargo_executable="test_ioctls",
        )
        with vmsh:
            vmsh.wait_until_line("attached")
            res = vm.ssh_cmd(["devmem2", "0xc0000000", "h"])
            print("read:\n", res.stdout)
            print("stderr:\n", res.stderr)
            assert "0xDEAD" in res.stdout

            res = vm.ssh_cmd(["devmem2", "0xc0000000", "h", "0xBEEF"])
            print("write 0xBEEF:\n", res.stdout)
            print("stderr:\n", res.stderr)

            res = vm.ssh_cmd(["devmem2", "0xc0000000", "h"])
            print("read:\n", res.stdout)
            print("stderr:\n", res.stderr)
            assert "0xDEAD" in res.stdout

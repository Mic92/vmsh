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


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        vmsh = helpers.spawn_vmsh_command(["attach", str(vm.pid)])

        with vmsh:
            vmsh.wait_until_line(
                "mmio dev attached", lambda l: "mmio dev attached" in l
            )

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

            res = vm.ssh_cmd(["dmesg"])
            print("stdout:\n", res.stdout)

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

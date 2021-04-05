import conftest
from multiprocessing import Process


def test_this_test(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        res = vm.ssh_cmd_sh("insmod $MODULE_DRIVERS_DIR/virtio/virtio_mmio.ko.xz")
        assert res.returncode == 0
        # assert that vritio_mmio is now loaded
        res = vm.ssh_cmd(["lsmod"])
        assert res.stdout
        assert res.stdout.find("virtio_mmio") >= 0


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = Process(target=helpers.run_vmsh_command, args=(["attach", str(vm.pid)],))
        vmsh.start()
        vm.wait_for_ssh()
        print("ssh available")

        mmio_config = "0x1000@0xd0000000:5"
        res = vm.ssh_cmd_sh(
            f"insmod $MODULE_DRIVERS_DIR/virtio/virtio_mmio.ko.xz device={mmio_config}"
        )
        print("stdout:\n", res.stdout)
        print("stderr:\n", res.stderr)

        res = vm.ssh_cmd(["dmesg"])
        print("stdout:\n", res.stdout)

        vmsh.kill()
        vmsh.join(10)
        vmsh.terminate()

        assert res.stdout.find("virtio_mmio: unknown parameter 'device' ignored") < 0

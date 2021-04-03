import conftest


def test_this_test(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        print("wait for ssh")
        vm.wait_for_ssh()
        print("ssh available")
        res = vm.ssh_cmd(["lsmod"])
        assert res.stdout
        assert res.stdout.find("virtio_mmio") >= 0


# def test_virtio_device_space(helpers: conftest.Helpers) -> None:
#     with helpers.spawn_qemu(helpers.notos_image()) as vm:
#         print("wait for ssh")
#         vm.wait_for_ssh()
#         print("ssh available")
#         vm.attach()
#         res = vm.ssh_cmd(["insmod", "virtio_mmio"])
#         print("stdout:\n", res.stdout)
#         print("stderr:\n", res.stderr)

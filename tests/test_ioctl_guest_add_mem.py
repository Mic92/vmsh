import conftest


def test_ioctl_guest_add_mem(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["guest_add_mem", str(vm.pid)], cargo_options=["--bin", "test_ioctls"]
        )


# add mem and try to get maps afterwards again
def test_ioctl_guest_add_mem_get_maps(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["guest_add_mem_get_maps", str(vm.pid)],
            cargo_options=["--bin", "test_ioctls"],
        )

import conftest


def test_ioctl_injection(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["vcpu_maps", str(vm.pid)], cargo_executable="test_ioctls"
        )

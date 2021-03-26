import conftest


def test_fd_transfer1(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["fd_transfer1", str(vm.pid)], cargo_options=["--bin", "test_ioctls"]
        )


def test_fd_transfer2(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["fd_transfer2", str(vm.pid)], cargo_options=["--bin", "test_ioctls"]
        )

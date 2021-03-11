import conftest


def test_coredump(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(["coredump", str(vm.pid)])

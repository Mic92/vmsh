import conftest


def test_inspect(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        proc = helpers.run_vmsh_command(["inspect", str(vm.pid)])
        found = False
        while not proc.lines.empty():
            line = proc.lines.get()
            if isinstance(line, int):
                break
            if "found kernel at" in line:
                found = True
                break
        assert found, "could not find kernel"

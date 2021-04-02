import conftest


def test_this_test(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        print("wait for ssh")
        vm.wait_for_ssh()
        print("ssh available")
        res = vm.ssh_cmd(["ls", "-lah", "/"])
        print("stdout:\n", res.stdout)
        # helpers.run_vmsh_command(
        # ["fd_transfer1", str(vm.pid)], cargo_options=["--bin", "test_ioctls"]
        # )

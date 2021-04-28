import conftest


def test_injection(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["inject", str(vm.pid)], cargo_executable="test_ioctls"
        )


def test_alloc_mem(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["alloc_mem", str(vm.pid)], cargo_executable="test_ioctls"
        )


def test_ioctl_guest_add_mem(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["guest_add_mem", str(vm.pid)], cargo_executable="test_ioctls"
        )


# add mem and try to get maps afterwards again
def test_ioctl_guest_add_mem_get_maps(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()  # to be sure qemu won't add any memory we didn't expect
        helpers.run_vmsh_command(
            ["guest_add_mem_get_maps", str(vm.pid)],
            cargo_executable="test_ioctls",
        )


def test_fd_transfer1(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["fd_transfer1", str(vm.pid)], cargo_executable="test_ioctls"
        )


def test_fd_transfer2(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["fd_transfer2", str(vm.pid)], cargo_executable="test_ioctls"
        )


def test_get_vcpu_maps(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        helpers.run_vmsh_command(
            ["vcpu_maps", str(vm.pid)], cargo_executable="test_ioctls"
        )


def test_userfaultfd_completes(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        vmsh = helpers.spawn_vmsh_command(
            ["guest_userfaultfd", str(vm.pid)],
            cargo_executable="test_ioctls",
        )

        with vmsh:
            vmsh.wait_until_line("pause", lambda l: "pause" in l)
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


def test_wrap_syscall(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        # attach vmsh after boot, because it slows the vm down a lot.
        vmsh = helpers.spawn_vmsh_command(
            ["guest_kvm_exits", str(vm.pid)],
            cargo_executable="test_ioctls",
        )
        with vmsh:
            vmsh.wait_until_line("attached", lambda l: "attached" in l)
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

        # check that vm is still responsive
        res = vm.ssh_cmd(["ls"])
        assert res.returncode == 0

import conftest
from qemu import QemuVm


def run_ioctl_test(command: str, vm: QemuVm) -> None:
    conftest.Helpers.run_vmsh_command(
        [command, str(vm.pid)], cargo_executable="examples/test_ioctls"
    )


def spawn_ioctl_test(command: str, vm: QemuVm) -> conftest.VmshPopen:
    return conftest.Helpers.spawn_vmsh_command(
        [command, str(vm.pid)], cargo_executable="examples/test_ioctls"
    )


def test_injection(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("inject", vm)


def test_alloc_mem(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("alloc_mem", vm)


def test_ioctl_cpuid2(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("cpuid2", vm)


def test_ioctl_guest_add_mem(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("guest_add_mem", vm)


# add mem and try to get maps afterwards again
def test_ioctl_guest_add_mem_get_maps(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()  # to be sure qemu won't add any memory we didn't expect
        run_ioctl_test("guest_add_mem_get_maps", vm)


def test_fd_transfer1(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("fd_transfer1", vm)


def test_fd_transfer2(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("fd_transfer2", vm)


def test_get_vcpu_maps(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        run_ioctl_test("vcpu_maps", vm)


# def test_userfaultfd_completes(helpers: conftest.Helpers) -> None:
#    with helpers.spawn_qemu(helpers.notos_image()) as vm:
#        vm.wait_for_ssh()
#        vmsh = spawn_ioctl_test("guest_userfaultfd", vm)
#
#        with vmsh:
#            vmsh.wait_until_line("pause", lambda l: "pause" in l)
#            print("ssh available")
#
#            res = vm.ssh_cmd(
#                [
#                    "devmem2",
#                    "0xd0000000",
#                    "ww",
#                    "0x1337",
#                ]
#            )
#            print("stdout:\n", res.stdout)
#            print("stderr:\n", res.stderr)


def test_wrap_syscall(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        # attach vmsh after boot, because it slows the vm down a lot.
        vmsh = spawn_ioctl_test("guest_kvm_exits", vm)
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

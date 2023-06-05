# import os
#
# import conftest
#
# from nix import alpine_image, passwd_image
#
#
# def test_chpasswd(
#    helpers: conftest.Helpers,
# ) -> None:
#    with passwd_image() as img, helpers.spawn_qemu(alpine_image()) as vm:
#        vm.wait_for_ssh()
#        vmsh = helpers.spawn_vmsh_command(
#            [
#                "attach",
#                "--backing-file",
#                str(img),
#                str(vm.pid),
#                "--",
#                "/bin/sh",
#                "-c",
#                "echo root:passwd | /bin/chpasswd -R /var/lib/vmsh && cat /var/lib/vmsh/etc/shadow",
#            ]
#        )
#
#        with vmsh:
#            try:
#                vmsh.wait_until_line(
#                    "process finished with exit status",
#                    lambda l: "process finished with exit status" in l,
#                )
#            finally:
#                res = vm.ssh_cmd(["dmesg"], check=False)
#                print("stdout:\n", res.stdout)
#            assert res.returncode == 0
#
#        try:
#            os.kill(vmsh.pid, 0)
#        except ProcessLookupError:
#            pass
#        else:
#            assert False, "vmsh was not terminated properly"
#
#        # See that the VM is still alive after detaching
#        # res = vm.ssh_cmd(["cat", "/etc/shadow"], check=False)
#        # breakpoint()
#        # assert res.stdout == "ping\n"
#        # assert res.returncode == 0

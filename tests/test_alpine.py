import conftest

from nix import notos_image, alpine_sec_scanner_image


def test_attach(
    helpers: conftest.Helpers,
) -> None:
    with alpine_sec_scanner_image() as img, helpers.spawn_qemu(notos_image()) as vm:
        vm.wait_for_ssh()
        vmsh = helpers.spawn_vmsh_command(
            [
                "attach",
                "--backing-file",
                str(img),
                str(vm.pid),
                "--",
                "/bin/alpine-sec-scanner",
                "/var/lib/vmsh",
            ]
        )

        with vmsh:
            try:
                vmsh.wait_until_line(
                    "no known insecure packages found",
                    lambda l: "no known insecure packages found" in l,
                )
            finally:
                res = vm.ssh_cmd(["dmesg"], check=False)
                print("stdout:\n", res.stdout)
